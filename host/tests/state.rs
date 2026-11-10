//! Saving sessions to `state.json` and bringing them back when the host starts again.
//! Most of these stop the host with SIGTERM, so they only run on Unix.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use term_host::protocol::{decode_binary, encode_binary};
use term_host::state::{SavedSession, State};
use tokio::net::TcpStream;
use tokio::time::timeout_at;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{WebSocketStream, client_async};

type Ws = WebSocketStream<TcpStream>;

const FIRST: &str = "11111111-1111-4111-8111-111111111111";
const SECOND: &str = "22222222-2222-4222-8222-222222222222";
const THIRD: &str = "33333333-3333-4333-8333-333333333333";

/// Logs its arguments, then waits like a real chat until its terminal hangs up.
const FAKE_CLAUDE: &str =
    "#!/bin/sh\necho \"$*\" >> \"$FAKE_CLAUDE_LOG\"\necho \"fake claude $*\"\nexec sleep 600\n";
/// Stands in for the login shell so `-lic` never runs the system's login files, which would
/// put the system folders ahead of the fake `claude` on the PATH.
const FAKE_SHELL: &str =
    "#!/bin/sh\nif [ \"$1\" = -lic ]; then exec /bin/sh -c \"$2\"; fi\nexec /bin/sh \"$@\"\n";

/// Everything one test's hosts touch, removed on drop: the data folder, a home folder, a Claude
/// config folder, the fake programs, and a folder to start shells in.
struct Dirs(PathBuf);

impl Dirs {
    fn new() -> Dirs {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("term-host-state-{}-{n}", std::process::id()));
        let dirs = Dirs(root);
        for dir in [
            dirs.data(),
            dirs.home(),
            dirs.config(),
            dirs.bin(),
            dirs.work(),
        ] {
            std::fs::create_dir_all(dir).expect("make a test folder");
        }
        #[cfg(unix)]
        {
            write_script(&dirs.bin().join("claude"), FAKE_CLAUDE);
            write_script(&dirs.bin().join("fake-shell"), FAKE_SHELL);
        }
        dirs
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn config(&self) -> PathBuf {
        self.0.join("config")
    }

    fn bin(&self) -> PathBuf {
        self.0.join("bin")
    }

    fn work(&self) -> PathBuf {
        self.0.join("work")
    }

    fn claude_log(&self) -> PathBuf {
        self.0.join("claude.log")
    }

    fn state_file(&self) -> PathBuf {
        self.data().join("state.json")
    }

    fn read_state(&self) -> State {
        let text = std::fs::read_to_string(self.state_file()).expect("read state.json");
        serde_json::from_str(&text).expect("parse state.json")
    }

    fn write_state(&self, state: &State) {
        let text = serde_json::to_string(state).expect("write the state as JSON");
        std::fs::write(self.state_file(), text).expect("seed state.json");
    }

    fn claude_calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.claude_log())
            .map(|log| log.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }
}

impl Drop for Dirs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn write_script(path: &Path, text: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, text).expect("write a fake program");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("make a fake program runnable");
}

/// The real host binary. Dropping it kills the process, including when a test panics.
struct Host {
    child: Child,
    port: u16,
    token: String,
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Host {
    fn start(dirs: &Dirs) -> Host {
        term_host::prepare_for_fork();
        let port = free_port();
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![dirs.bin()];
        paths.extend(std::env::split_paths(&path));
        let child = Command::new(env!("CARGO_BIN_EXE_term-host"))
            .env("TERM_APP_DATA", dirs.data())
            .env("TERM_HOST_PORT", port.to_string())
            .env("SHELL", dirs.bin().join("fake-shell"))
            .env("HOME", dirs.home())
            .env("CLAUDE_CONFIG_DIR", dirs.config())
            .env("PATH", std::env::join_paths(paths).expect("build PATH"))
            .env("FAKE_CLAUDE_LOG", dirs.claude_log())
            .env_remove("CLAUDECODE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start the host");
        let mut host = Host {
            child,
            port,
            token: String::new(),
        };
        host.wait_ready(&dirs.data().join("host-token"));
        host
    }

    fn wait_ready(&mut self, token_path: &Path) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().expect("check the host") {
                panic!("host exited before it was ready: {status}");
            }
            let listening = std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok();
            if let Ok(token) = std::fs::read_to_string(token_path)
                && listening
                && token.len() == 64
            {
                self.token = token;
                return;
            }
            assert!(Instant::now() < deadline, "host was not ready in time");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Sends SIGTERM, as a shutdown does, and waits for a clean exit.
    #[cfg(unix)]
    fn stop(&mut self) {
        let pid = self.child.id() as i32;
        assert_eq!(
            unsafe { libc::kill(pid, libc::SIGTERM) },
            0,
            "signal the host"
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().expect("check the host") {
                assert!(status.success(), "host exit after SIGTERM: {status}");
                return;
            }
            assert!(Instant::now() < deadline, "host did not exit after SIGTERM");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// A port no other test has and that was free when found. Kept apart from the range the
/// server tests use, and below the range the system picks outgoing ports from.
fn free_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(0);
    let base = 31_000 + (std::process::id() % 4_000) as u16 * 2;
    loop {
        let port = base + NEXT.fetch_add(1, Ordering::Relaxed);
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            return port;
        }
    }
}

async fn signed_in(host: &Host) -> Ws {
    let request = format!("ws://127.0.0.1:{}/", host.port)
        .into_client_request()
        .expect("build the request");
    let stream = TcpStream::connect(("127.0.0.1", host.port))
        .await
        .expect("connect to the host");
    let (mut ws, _) = client_async(request, stream)
        .await
        .expect("upgrade to a WebSocket");
    say(
        &mut ws,
        json!({"type": "hello", "token": host.token, "proto": 2}),
    )
    .await;
    let reply = hear(&mut ws, |frame| frame["type"] == "hello_ok").await;
    assert_eq!(reply["proto"], 2, "{reply}");
    ws
}

async fn say(ws: &mut Ws, frame: Value) {
    ws.send(Message::text(frame.to_string()))
        .await
        .expect("send a frame");
}

/// The first text frame that `wanted` accepts. Output in between is skipped, and an error
/// frame fails the test.
async fn hear(ws: &mut Ws, wanted: impl Fn(&Value) -> bool) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let frame = timeout_at(deadline, ws.next())
            .await
            .expect("no matching frame arrived in time")
            .expect("the connection ended")
            .expect("read a frame");
        if let Message::Text(text) = frame {
            let frame: Value = serde_json::from_str(text.as_str()).expect("parse a host frame");
            assert_ne!(frame["type"], "error", "{frame}");
            if wanted(&frame) {
                return frame;
            }
        }
    }
}

async fn create(ws: &mut Ws, uuid: &str, kind: &str, cwd: &Path) -> u32 {
    let cwd = cwd.to_str().expect("a UTF-8 folder");
    say(
        ws,
        json!({"type": "create", "uuid": uuid, "kind": kind, "cwd": cwd, "cols": 200, "rows": 24}),
    )
    .await;
    let created = hear(ws, |frame| {
        frame["type"] == "created" && frame["uuid"] == uuid
    })
    .await;
    created["session"].as_u64().expect("a session number") as u32
}

async fn attach(ws: &mut Ws, session: u32) {
    say(
        ws,
        json!({"type": "attach", "session": session, "cols": 200, "rows": 24}),
    )
    .await;
    hear(ws, |frame| {
        frame["type"] == "attached" && frame["session"] == session
    })
    .await;
}

async fn list(ws: &mut Ws) -> Vec<Value> {
    say(ws, json!({"type": "list"})).await;
    let reply = hear(ws, |frame| frame["type"] == "sessions").await;
    reply["items"]
        .as_array()
        .expect("a list of sessions")
        .clone()
}

async fn type_in(ws: &mut Ws, session: u32, text: &str) {
    ws.send(Message::binary(encode_binary(session, text.as_bytes())))
        .await
        .expect("send input");
}

/// Waits for `@@<tag>:<value>@@` in the session's output and returns the value. Commands
/// spell the tag split in two (`@@cw""d:`) so their own echo never matches.
async fn read_value(ws: &mut Ws, session: u32, tag: &str) -> String {
    let needle = format!("@@{tag}:");
    let mut output = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let text = String::from_utf8_lossy(&output);
        if let Some(at) = text.find(&needle) {
            let rest = &text[at + needle.len()..];
            if let Some(end) = rest.find("@@") {
                return rest[..end].to_string();
            }
        }
        let frame = timeout_at(deadline, ws.next())
            .await
            .unwrap_or_else(|_| panic!("no {tag} in time, output so far: {text:?}"))
            .expect("the connection ended")
            .expect("read a frame");
        if let Message::Binary(frame) = frame
            && let Some((from, bytes)) = decode_binary(&frame)
            && from == session
        {
            output.extend_from_slice(bytes);
        }
    }
}

async fn expect_exit(ws: &mut Ws, session: u32, clean: bool) {
    let exited = hear(ws, |frame| {
        frame["type"] == "exited" && frame["session"] == session
    })
    .await;
    assert_eq!(exited["clean"], clean, "{exited}");
}

async fn pwd(ws: &mut Ws, session: u32) -> PathBuf {
    type_in(ws, session, "echo \"@@cw\"\"d:$(pwd -P)@@\"\n").await;
    PathBuf::from(read_value(ws, session, "cwd").await)
}

fn real(path: &Path) -> PathBuf {
    path.canonicalize().expect("resolve a folder")
}

fn uuids(state: &State) -> Vec<&str> {
    state
        .sessions
        .iter()
        .map(|session| session.uuid.as_str())
        .collect()
}

fn saved_claude(uuid: &str, cwd: &Path) -> SavedSession {
    SavedSession {
        uuid: uuid.to_string(),
        kind: "claude".to_string(),
        cwd: cwd.to_path_buf(),
        title: None,
        chat: Some(uuid.parse().expect("a chat id")),
    }
}

async fn wait_for_claude_calls(dirs: &Dirs, count: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let calls = dirs.claude_calls();
        if calls.len() >= count {
            return calls;
        }
        assert!(Instant::now() < deadline, "claude ran {calls:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn clean_exit_removes() {
    let dirs = Dirs::new();
    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    let typed = create(&mut ws, FIRST, "shell", &dirs.work()).await;
    let closed = create(&mut ws, SECOND, "shell", &dirs.work()).await;
    create(&mut ws, THIRD, "shell", &dirs.work()).await;

    attach(&mut ws, typed).await;
    type_in(&mut ws, typed, "exit\n").await;
    expect_exit(&mut ws, typed, true).await;
    say(&mut ws, json!({"type": "kill", "session": closed})).await;
    // `kill` has no reply, and frames are handled in order, so this one's reply means the kill
    // landed before the host is stopped.
    list(&mut ws).await;

    host.stop();
    assert_eq!(uuids(&dirs.read_state()), [THIRD]);
}

#[cfg(unix)]
#[tokio::test]
async fn signal_exit_keeps() {
    let dirs = Dirs::new();
    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    let hung = create(&mut ws, FIRST, "shell", &dirs.work()).await;
    let trapped = create(&mut ws, SECOND, "shell", &dirs.work()).await;

    attach(&mut ws, hung).await;
    type_in(&mut ws, hung, "echo \"@@pi\"\"d:$$@@\"\n").await;
    let pid: i32 = read_value(&mut ws, hung, "pid")
        .await
        .parse()
        .expect("a pid");
    assert_eq!(
        unsafe { libc::kill(pid, libc::SIGHUP) },
        0,
        "hang up the shell"
    );
    expect_exit(&mut ws, hung, false).await;

    // Exits normally when the terminal hangs up, which happens once the host has stopped.
    attach(&mut ws, trapped).await;
    type_in(
        &mut ws,
        trapped,
        "trap 'exit 0' HUP; echo \"@@tr\"\"ap:set@@\"\n",
    )
    .await;
    assert_eq!(read_value(&mut ws, trapped, "trap").await, "set");

    host.stop();
    assert_eq!(uuids(&dirs.read_state()), [FIRST, SECOND]);
}

#[cfg(unix)]
#[tokio::test]
async fn restart_restores_shell_in_folder() {
    let dirs = Dirs::new();
    let start = dirs.work().join("project");
    let inner = start.join("inner");
    std::fs::create_dir_all(&inner).expect("make the folders");

    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    let session = create(&mut ws, FIRST, "shell", &start).await;
    attach(&mut ws, session).await;
    type_in(&mut ws, session, "cd inner\n").await;
    assert_eq!(pwd(&mut ws, session).await, real(&inner));
    host.stop();

    let state = dirs.read_state();
    assert_eq!(uuids(&state), [FIRST]);
    assert_eq!(real(&state.sessions[0].cwd), real(&inner));

    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    let items = list(&mut ws).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["uuid"], FIRST);
    assert_eq!(items[0]["kind"], "shell");
    assert_eq!(items[0]["exited"], Value::Null);
    let session = items[0]["session"].as_u64().expect("a session number") as u32;
    attach(&mut ws, session).await;
    assert_eq!(pwd(&mut ws, session).await, real(&inner));
    host.stop();

    std::fs::remove_dir_all(&start).expect("remove the folder");
    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    let items = list(&mut ws).await;
    assert_eq!(items.len(), 1, "{items:?}");
    let session = items[0]["session"].as_u64().expect("a session number") as u32;
    attach(&mut ws, session).await;
    assert_eq!(pwd(&mut ws, session).await, real(&dirs.home()));
    host.stop();
}

#[cfg(unix)]
#[tokio::test]
async fn restored_shell_lists_its_saved_folder() {
    let dirs = Dirs::new();
    let folder = dirs.work().join("saved");
    std::fs::create_dir_all(&folder).expect("make the folder");
    dirs.write_state(&State {
        sessions: vec![SavedSession {
            uuid: FIRST.to_string(),
            kind: "shell".to_string(),
            cwd: folder.clone(),
            title: None,
            chat: None,
        }],
        ..State::default()
    });

    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    let items = list(&mut ws).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(
        items[0]["cwd"],
        real(&folder).to_str().expect("a UTF-8 folder")
    );
    host.stop();
}

#[cfg(unix)]
#[tokio::test]
async fn claude_restore_uses_resume() {
    let dirs = Dirs::new();
    let transcripts = dirs.config().join("projects").join("-some-project");
    std::fs::create_dir_all(&transcripts).expect("make the projects folder");
    std::fs::write(transcripts.join(format!("{FIRST}.jsonl")), "{}\n").expect("write a transcript");
    dirs.write_state(&State {
        sessions: vec![
            saved_claude(FIRST, &dirs.work()),
            saved_claude(SECOND, &dirs.work()),
        ],
        layout: None,
    });

    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    let items = list(&mut ws).await;
    assert_eq!(items.len(), 2, "{items:?}");
    for item in &items {
        attach(
            &mut ws,
            item["session"].as_u64().expect("a session number") as u32,
        )
        .await;
    }
    let mut calls = wait_for_claude_calls(&dirs, 2).await;
    calls.sort();
    assert_eq!(
        calls,
        [
            format!("--resume {FIRST}"),
            format!("--session-id {SECOND}")
        ]
    );
    host.stop();
}

#[cfg(unix)]
#[tokio::test]
async fn claude_restore_is_lazy() {
    let dirs = Dirs::new();
    let mut seeded = saved_claude(FIRST, &dirs.work());
    seeded.title = Some("planning".to_string());
    dirs.write_state(&State {
        sessions: vec![seeded],
        layout: None,
    });

    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    let items = list(&mut ws).await;
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["uuid"], FIRST);
    assert_eq!(items[0]["kind"], "claude");
    assert_eq!(items[0]["title"], "planning");
    assert_eq!(items[0]["exited"], Value::Null);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(dirs.claude_calls().is_empty(), "{:?}", dirs.claude_calls());

    let session = items[0]["session"].as_u64().expect("a session number") as u32;
    attach(&mut ws, session).await;
    let calls = wait_for_claude_calls(&dirs, 1).await;
    assert_eq!(calls, [format!("--session-id {FIRST}")]);
    host.stop();

    // A chat the host ended by shutting down is still there for next time.
    let state = dirs.read_state();
    assert_eq!(uuids(&state), [FIRST]);
    assert_eq!(state.sessions[0].title.as_deref(), Some("planning"));
}

#[cfg(unix)]
#[tokio::test]
async fn layout_survives_restart() {
    let dirs = Dirs::new();
    let layout = r#"{"tabs":[{"split":"row"}]}"#;
    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    say(&mut ws, json!({"type": "layout_set", "json": layout})).await;
    // `layout_set` has no reply. Frames are handled in order, so this reply means it landed.
    list(&mut ws).await;
    host.stop();

    let mut host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    say(&mut ws, json!({"type": "layout_get"})).await;
    let reply = hear(&mut ws, |frame| frame["type"] == "layout").await;
    assert_eq!(reply["json"], layout);
    host.stop();
}

#[tokio::test]
async fn corrupt_state_moved_aside() {
    let dirs = Dirs::new();
    std::fs::write(dirs.state_file(), "{not json").expect("write a bad state file");

    let host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    assert!(list(&mut ws).await.is_empty());

    let moved: Vec<PathBuf> = std::fs::read_dir(dirs.data())
        .expect("list the data folder")
        .map(|entry| entry.expect("read an entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("state.json.bad"))
        })
        .collect();
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(
        std::fs::read_to_string(&moved[0]).expect("read the moved file"),
        "{not json"
    );
}

#[tokio::test]
async fn non_utf8_state_moved_aside() {
    let dirs = Dirs::new();
    let bytes = [0xff, 0xfe, 0x00, 0x80];
    std::fs::write(dirs.state_file(), bytes).expect("write a non-UTF-8 state file");

    let host = Host::start(&dirs);
    let mut ws = signed_in(&host).await;
    assert!(list(&mut ws).await.is_empty());

    assert_ne!(std::fs::read(dirs.state_file()).ok(), Some(bytes.to_vec()));
    let moved: Vec<PathBuf> = std::fs::read_dir(dirs.data())
        .expect("list the data folder")
        .map(|entry| entry.expect("read an entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("state.json.bad"))
        })
        .collect();
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(
        std::fs::read(&moved[0]).expect("read the moved file"),
        bytes
    );
}
