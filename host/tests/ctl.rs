//! The local socket that hook calls reach the host through, the `term-host hook` command and
//! the Claude settings file that runs it. The hook command must never slow Claude down or
//! show it an error, whatever state the host is in.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use interprocess::local_socket::ListenerOptions;
use interprocess::local_socket::prelude::*;
use serde_json::Value;
use term_host::claudecfg;
use term_host::ctl::{self, CtlRequest, Tokens};

/// What `term-host hook` may take, with room for starting a process on a busy CI machine.
const HOOK_LIMIT: Duration = Duration::from_millis(300);

/// A temp folder for one test, removed on drop. Kept short: a Unix socket path has a small
/// length limit.
struct Root(PathBuf);

impl Root {
    fn new() -> Root {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("th-ctl-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).expect("make the test folder");
        Root(path)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn request(token: &str, event: &str) -> CtlRequest {
    CtlRequest {
        token: token.to_string(),
        session: "s1".to_string(),
        event: event.to_string(),
        payload: serde_json::json!({ "hook_event_name": event }),
    }
}

/// Writes one raw line to the socket and returns whatever the host sends back before it
/// closes the connection.
fn send_line(data_dir: &Path, line: &str) -> Vec<u8> {
    let mut stream = LocalSocketStream::connect(ctl::socket_name(data_dir).expect("socket name"))
        .expect("connect to the host");
    stream.write_all(line.as_bytes()).expect("send the line");
    stream.set_recv_timeout(Some(Duration::from_secs(5))).ok();
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    reply
}

/// Runs `term-host hook stop` as Claude would: the session's env, `stdin` fed by `feed`.
fn run_hook(
    data_dir: &Path,
    feed: impl FnOnce(std::process::ChildStdin) + Send + 'static,
) -> (Output, Duration) {
    let started = Instant::now();
    let mut child = Command::new(env!("CARGO_BIN_EXE_term-host"))
        .args(["hook", "Stop"])
        .env("TA_SESSION", "s1")
        .env("TA_TOKEN", "good")
        .env("TA_DATA", data_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start term-host hook");
    let stdin = child.stdin.take().expect("the hook's stdin");
    std::thread::spawn(move || feed(stdin));
    let output = child.wait_with_output().expect("wait for term-host hook");
    (output, started.elapsed())
}

/// Starts the binary once so the first timed run does not pay for the system's first look
/// at a new executable.
fn warm_up() {
    let _ = Command::new(env!("CARGO_BIN_EXE_term-host"))
        .args(["hook", "Stop"])
        .env_remove("TA_SESSION")
        .stdin(Stdio::null())
        .output();
}

fn assert_quiet_and_fast((output, took): (Output, Duration)) {
    assert!(output.status.success(), "exit status {:?}", output.status);
    assert!(output.stdout.is_empty(), "stdout {:?}", output.stdout);
    assert!(
        output.stderr.is_empty(),
        "stderr {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(took < HOOK_LIMIT, "took {took:?}");
}

fn stop_json(message_len: usize) -> Vec<u8> {
    let payload = serde_json::json!({
        "session_id": "22222222-2222-4222-8222-222222222222",
        "hook_event_name": "Stop",
        "last_assistant_message": "x".repeat(message_len),
        "background_tasks": [],
    });
    serde_json::to_vec(&payload).expect("encode the payload")
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_token_is_ignored() {
    let root = Root::new();
    let tokens = Tokens::default();
    tokens.insert("s1", "good");
    let (tx, rx) = mpsc::channel();
    ctl::serve(&root.0, tokens.clone(), move |req| {
        let _ = tx.send(req);
    })
    .expect("serve the socket");

    let dir = root.0.clone();
    tokio::task::spawn_blocking(move || {
        let line = |req: &CtlRequest| format!("{}\n", serde_json::to_string(req).unwrap());
        // A wrong token, an unknown session and a line that is not a request get no reply.
        assert!(send_line(&dir, &line(&request("bad", "Stop"))).is_empty());
        let mut stranger = request("good", "Stop");
        stranger.session = "s2".to_string();
        assert!(send_line(&dir, &line(&stranger)).is_empty());
        assert!(send_line(&dir, "not json\n").is_empty());
        assert!(send_line(&dir, &line(&request("good", "Prompt"))).is_empty());
    })
    .await
    .expect("the client side");

    let got = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the good request arrives");
    assert_eq!(got, request("good", "Prompt"));
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());

    // A token taken back no longer works.
    tokens.remove("s1");
    let dir = root.0.clone();
    tokio::task::spawn_blocking(move || {
        let line = format!(
            "{}\n",
            serde_json::to_string(&request("good", "End")).unwrap()
        );
        send_line(&dir, &line);
    })
    .await
    .expect("the client side");
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn hook_exits_zero_fast_with_no_host() {
    let root = Root::new();
    warm_up();
    assert_quiet_and_fast(run_hook(&root.0, |mut stdin| {
        let _ = stdin.write_all(&stop_json(10));
    }));

    // Stdin that stays open and never sends anything.
    let (keep_tx, keep_rx) = mpsc::channel::<()>();
    assert_quiet_and_fast(run_hook(&root.0, move |stdin| {
        let _ = keep_rx.recv_timeout(Duration::from_secs(5));
        drop(stdin);
    }));
    drop(keep_tx);

    // Nothing to send without the session's env.
    let output = Command::new(env!("CARGO_BIN_EXE_term-host"))
        .args(["hook", "Stop"])
        .env_remove("TA_SESSION")
        .env_remove("TA_TOKEN")
        .env_remove("TA_DATA")
        .stdin(Stdio::null())
        .output()
        .expect("run term-host hook");
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

#[test]
fn hook_exits_zero_fast_when_host_never_answers() {
    let root = Root::new();
    #[cfg(unix)]
    fs::create_dir_all(root.0.join("ctl")).expect("make the socket folder");
    let listener = ListenerOptions::new()
        .name(ctl::socket_name(&root.0).expect("socket name"))
        .create_sync()
        .expect("listen on the socket");
    // Accepts every connection and keeps it open without reading a byte.
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming().flatten() {
            held.push(stream);
        }
    });
    warm_up();

    // Big enough to fill any socket buffer, so the write itself blocks.
    assert_quiet_and_fast(run_hook(&root.0, |mut stdin| {
        let _ = stdin.write_all(&stop_json(1 << 20));
    }));
    // And with stdin that never closes after the JSON.
    let (keep_tx, keep_rx) = mpsc::channel::<()>();
    assert_quiet_and_fast(run_hook(&root.0, move |mut stdin| {
        let _ = stdin.write_all(&stop_json(10));
        let _ = keep_rx.recv_timeout(Duration::from_secs(5));
    }));
    drop(keep_tx);
}

#[test]
fn claude_settings_file_has_every_hook() {
    let root = Root::new();
    let exe = Path::new("/Applications/Term App/term-host");
    let path = claudecfg::write(&root.0, exe).expect("write the settings file");
    assert_eq!(path, root.0.join("claude-hooks.json"));
    let names: Vec<_> = fs::read_dir(&root.0)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, ["claude-hooks.json"], "no temp file is left behind");

    let settings: Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).expect("valid JSON");
    let hooks = settings["hooks"].as_object().expect("a hooks object");
    let mut events: Vec<_> = hooks.keys().map(String::as_str).collect();
    events.sort_unstable();
    assert_eq!(
        events,
        [
            "Notification",
            "PermissionRequest",
            "PostToolUse",
            "PreToolUse",
            "SessionEnd",
            "SessionStart",
            "Stop",
            "SubagentStop",
            "UserPromptSubmit",
        ]
    );
    for (event, entries) in hooks {
        let entries = entries.as_array().expect("a list of matchers");
        assert_eq!(entries.len(), 1, "{event}");
        let hook = &entries[0]["hooks"][0];
        assert_eq!(hook["type"], "command", "{event}");
        let quoted = if cfg!(windows) {
            format!("\"{}\"", exe.display())
        } else {
            format!("'{}'", exe.display())
        };
        assert_eq!(hook["command"], format!("{quoted} hook {event}"), "{event}");
        let matcher = &entries[0]["matcher"];
        match event.as_str() {
            "PreToolUse" => assert_eq!(matcher, "Edit|Write|Agent"),
            _ => assert!(matcher.is_null(), "{event} has no matcher"),
        }
    }

    // Writing again replaces the file.
    claudecfg::write(&root.0, Path::new("/opt/it's here/term-host")).expect("write again");
    let text = fs::read_to_string(&path).unwrap();
    let settings: Value = serde_json::from_str(&text).expect("valid JSON");
    let command = settings["hooks"]["Stop"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .to_string();
    if cfg!(unix) {
        assert_eq!(command, r#"'/opt/it'\''s here/term-host' hook Stop"#);
    }
}

/// The socket sits in a folder only its owner can enter, made that way before the socket is
/// bound, so no other user can reach it even before its own mode is set.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn socket_folder_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let root = Root::new();
    // A folder left open by an older host is closed again.
    fs::create_dir_all(root.0.join("ctl")).expect("make an open folder");
    fs::set_permissions(root.0.join("ctl"), fs::Permissions::from_mode(0o755))
        .expect("open the folder");
    ctl::serve(&root.0, Tokens::default(), |_| {}).expect("serve the socket");
    let folder = fs::metadata(root.0.join("ctl")).expect("the socket folder");
    assert_eq!(folder.permissions().mode() & 0o777, 0o700);
    let socket = fs::metadata(root.0.join("ctl").join("ctl.sock")).expect("the socket");
    assert_eq!(socket.permissions().mode() & 0o777, 0o600);
}

#[tokio::test(flavor = "multi_thread")]
async fn hook_request_reaches_serve() {
    let root = Root::new();
    let tokens = Tokens::default();
    tokens.insert("s1", "good");
    let (tx, rx) = mpsc::channel();
    ctl::serve(&root.0, tokens, move |req| {
        let _ = tx.send(req);
    })
    .expect("serve the socket");

    let dir = root.0.clone();
    let run = tokio::task::spawn_blocking(move || {
        warm_up();
        run_hook(&dir, |mut stdin| {
            let _ = stdin.write_all(&stop_json(10));
        })
    })
    .await
    .expect("run the hook");
    assert_quiet_and_fast(run);

    let got = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the hook's request arrives");
    assert_eq!(got.token, "good");
    assert_eq!(got.session, "s1");
    assert_eq!(got.event, "Stop");
    let expected: Value = serde_json::from_slice(&stop_json(10)).unwrap();
    assert_eq!(got.payload, expected);
}

/// The real host binary with a data folder of its own, killed on drop.
#[cfg(unix)]
struct LiveHost {
    child: std::process::Child,
    port: u16,
    token: String,
}

#[cfg(unix)]
impl Drop for LiveHost {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
impl LiveHost {
    fn start(data: &Path) -> LiveHost {
        term_host::prepare_for_fork();
        let port = 41_000 + (std::process::id() % 4_000) as u16;
        let child = Command::new(env!("CARGO_BIN_EXE_term-host"))
            .env("TERM_APP_DATA", data)
            .env("TERM_HOST_PORT", port.to_string())
            .env("SHELL", "/bin/sh")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start the host");
        let mut host = LiveHost {
            child,
            port,
            token: String::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let listening = std::net::TcpStream::connect(("127.0.0.1", port)).is_ok();
            if let Ok(token) = fs::read_to_string(data.join("host-token"))
                && listening
                && token.len() == 64
            {
                host.token = token;
                return host;
            }
            assert!(Instant::now() < deadline, "host was not ready in time");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(unix)]
type Ws = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;

#[cfg(unix)]
async fn say(ws: &mut Ws, frame: Value) {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::Message;
    ws.send(Message::text(frame.to_string()))
        .await
        .expect("send a frame");
}

/// The first text frame that `wanted` accepts, skipping output and other frames.
#[cfg(unix)]
async fn hear(ws: &mut Ws, wanted: impl Fn(&Value) -> bool) -> Value {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let frame = tokio::time::timeout_at(deadline, ws.next())
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

/// The state the frame gives `uuid`, or None if it lists no state for it.
#[cfg(unix)]
fn state_of(frame: &Value, uuid: &str) -> Option<String> {
    frame["items"]
        .as_array()?
        .iter()
        .find(|item| item["session"] == uuid)?["state"]
        .as_str()
        .map(String::from)
}

#[cfg(unix)]
#[tokio::test]
async fn hook_reaches_host() {
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    const UUID: &str = "44444444-4444-4444-8444-444444444444";
    let root = Root::new();
    let data = root.0.join("data");
    let host = LiveHost::start(&data);
    let request = format!("ws://127.0.0.1:{}/", host.port)
        .into_client_request()
        .expect("build the request");
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", host.port))
        .await
        .expect("connect to the host");
    let (mut ws, _) = tokio_tungstenite::client_async(request, stream)
        .await
        .expect("upgrade to a WebSocket");
    say(
        &mut ws,
        serde_json::json!({"type": "hello", "token": host.token, "proto": 3}),
    )
    .await;
    hear(&mut ws, |frame| frame["type"] == "hello_ok").await;
    say(&mut ws, serde_json::json!({"type": "tree_get"})).await;
    let first = hear(&mut ws, |frame| frame["type"] == "states").await;
    assert_eq!(state_of(&first, UUID), None, "{first}");

    let cwd = root.0.to_str().expect("a UTF-8 folder");
    say(
        &mut ws,
        serde_json::json!({"type": "create", "uuid": UUID, "kind": "shell", "cwd": cwd, "cols": 80, "rows": 24}),
    )
    .await;
    let created = hear(&mut ws, |frame| frame["type"] == "created").await;
    let session = created["session"].as_u64().expect("a session number") as u32;
    say(
        &mut ws,
        serde_json::json!({"type": "attach", "session": session, "cols": 80, "rows": 24}),
    )
    .await;
    hear(&mut ws, |frame| frame["type"] == "attached").await;

    // The shell has TA_SESSION, TA_TOKEN and TA_DATA, as a Claude it started would.
    let exe = env!("CARGO_BIN_EXE_term-host").replace('\'', r"'\''");
    let typed =
        format!("echo '{{}}' | '{exe}' hook UserPromptSubmit; echo '{{}}' | '{exe}' hook Stop\n");
    {
        use futures_util::SinkExt;
        ws.send(Message::binary(term_host::protocol::encode_binary(
            session,
            typed.as_bytes(),
        )))
        .await
        .expect("type the hook calls");
    }
    let working = hear(&mut ws, |frame| {
        frame["type"] == "states" && state_of(frame, UUID).is_some()
    })
    .await;
    assert_eq!(
        state_of(&working, UUID).as_deref(),
        Some("working"),
        "{working}"
    );
    let done = hear(&mut ws, |frame| {
        frame["type"] == "states" && state_of(frame, UUID).as_deref() == Some("done")
    })
    .await;
    assert!(done["items"][0]["since_ms"].as_u64().is_some(), "{done}");

    say(
        &mut ws,
        serde_json::json!({"type": "seen", "session": UUID}),
    )
    .await;
    hear(&mut ws, |frame| {
        frame["type"] == "states" && state_of(frame, UUID).as_deref() == Some("idle")
    })
    .await;
}
