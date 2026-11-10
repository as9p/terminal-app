use std::net::{IpAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{WebSocketStream, client_async};

const LIMIT_BYTES: usize = 16 * 1024 * 1024;

type Ws = WebSocketStream<TcpStream>;

/// A data folder that is removed on drop. The host creates it unless a test seeds it first.
struct DataDir(PathBuf);

impl DataDir {
    fn fresh() -> DataDir {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        DataDir(std::env::temp_dir().join(format!("term-host-test-{}-{n}", std::process::id())))
    }
}

impl Drop for DataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The real host binary. Dropping it kills the process and then removes its folder,
/// including when a test panics.
struct Host {
    child: Child,
    dir: DataDir,
    port: u16,
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Host {
    fn spawn(port: u16, dir: DataDir, stderr: Stdio) -> Host {
        term_host::prepare_for_fork();
        let child = Command::new(env!("CARGO_BIN_EXE_term-host"))
            .env("TERM_APP_DATA", &dir.0)
            .env("TERM_HOST_PORT", port.to_string())
            // So a test can never start the real login shell, which is slow and varies.
            .env("SHELL", "/bin/sh")
            // So a login shell never reads the real startup files.
            .env("HOME", &dir.0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .expect("start the host");
        Host { child, dir, port }
    }

    fn start_in(dir: DataDir) -> Host {
        let mut host = Host::spawn(free_port(), dir, Stdio::inherit());
        host.wait_ready();
        host
    }

    fn start() -> Host {
        Host::start_in(DataDir::fresh())
    }

    /// Ready means the port answers and a whole token is on disk.
    fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().expect("check the host") {
                panic!("host exited before it was ready: {status}");
            }
            let listening = std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok();
            let token_written = std::fs::read(self.token_path()).is_ok_and(|t| t.len() == 64);
            if listening && token_written {
                return;
            }
            assert!(Instant::now() < deadline, "host was not ready in time");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn token_path(&self) -> PathBuf {
        token_path(&self.dir.0)
    }

    fn token(&self) -> String {
        std::fs::read_to_string(self.token_path()).expect("read the token file")
    }
}

fn token_path(dir: &Path) -> PathBuf {
    dir.join("host-token")
}

/// A port no other test has, and that this one found free. Taken from below the range the
/// system picks outgoing ports from: a port the system hands out as free can be taken by
/// another test's outgoing connection before the host gets to bind it.
fn free_port() -> u16 {
    static NEXT: AtomicU16 = AtomicU16::new(0);
    let base = 20_000 + (std::process::id() % 5_000) as u16 * 2;
    loop {
        let port = base + NEXT.fetch_add(1, Ordering::Relaxed);
        // Connecting, not binding: a listener open here for a moment would be copied into any
        // child another test forks in that moment, and then the host could not bind the port.
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            return port;
        }
    }
}

async fn connect(port: u16, origins: &[&str]) -> Result<Ws, Box<WsError>> {
    let mut request = format!("ws://127.0.0.1:{port}/")
        .into_client_request()
        .expect("build the request");
    for origin in origins {
        let value = origin.parse().expect("build the Origin header");
        request.headers_mut().append(ORIGIN, value);
    }
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect to the host");
    stream.set_nodelay(true).expect("set TCP_NODELAY");
    let (ws, _) = client_async(request, stream).await.map_err(Box::new)?;
    Ok(ws)
}

async fn open(port: u16) -> Ws {
    connect(port, &[]).await.expect("upgrade to a WebSocket")
}

async fn say(ws: &mut Ws, frame: Value) {
    ws.send(Message::text(frame.to_string()))
        .await
        .expect("send a frame");
}

async fn hear(ws: &mut Ws) -> Value {
    let frame = timeout(Duration::from_secs(5), ws.next())
        .await
        .expect("no frame arrived in time")
        .expect("the connection ended")
        .expect("read a frame");
    match frame {
        Message::Text(text) => serde_json::from_str(text.as_str()).expect("parse a host frame"),
        other => panic!("expected a text frame, got {other:?}"),
    }
}

/// Passes only if the host ends the connection without sending any frame first.
async fn hear_close(ws: &mut Ws) {
    let frame = timeout(Duration::from_secs(8), ws.next())
        .await
        .expect("the connection stayed open");
    match frame {
        None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {}
        Some(Ok(other)) => panic!("expected a close, got {other:?}"),
    }
}

async fn expect_hello_ok(port: u16, ws: &mut Ws, token: &str) {
    say(ws, json!({"type": "hello", "token": token, "proto": 3})).await;
    let reply = hear(ws).await;
    assert_eq!(reply["type"], "hello_ok", "port {port}: {reply}");
    assert_eq!(reply["proto"], 3, "{reply}");
    assert!(reply["host_version"].is_string(), "{reply}");
}

async fn signed_in(host: &Host) -> Ws {
    let mut ws = open(host.port).await;
    expect_hello_ok(host.port, &mut ws, &host.token()).await;
    ws
}

async fn assert_refused_403(port: u16, origins: &[&str]) {
    match connect(port, origins).await {
        Ok(_) => panic!("origins {origins:?} were allowed to upgrade"),
        Err(error) => match *error {
            WsError::Http(response) => {
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{origins:?}")
            }
            other => panic!("origins {origins:?} failed the wrong way: {other}"),
        },
    }
}

#[tokio::test]
async fn refuses_foreign_origin() {
    let host = Host::start();
    let foreign = [
        "https://example.com",
        "null",
        // Near misses, because the match must be exact.
        "tauri://localhost.example.com",
        "tauri://localhost/",
        "TAURI://LOCALHOST",
        "http://tauri.localhost.example.com",
        "https://tauri.localhost",
        "http://localhost",
        "http://localhost:14200",
        "http://127.0.0.1:1420",
    ];
    for origin in foreign {
        assert_refused_403(host.port, &[origin]).await;
    }
    // A foreign origin must not slip past by sharing a request with an allowed one.
    assert_refused_403(host.port, &["tauri://localhost", "https://example.com"]).await;
    assert_refused_403(host.port, &["https://example.com", "tauri://localhost"]).await;
}

#[tokio::test]
async fn allows_app_origins() {
    let host = Host::start();
    let token = host.token();
    for origin in ["tauri://localhost", "http://tauri.localhost"] {
        let mut ws = connect(host.port, &[origin]).await.expect(origin);
        expect_hello_ok(host.port, &mut ws, &token).await;
    }

    // Browsers always send Origin on a WebSocket, so only a non-browser client gets here
    // without one, and it still needs the token.
    let mut ws = connect(host.port, &[]).await.expect("no Origin header");
    expect_hello_ok(host.port, &mut ws, &token).await;

    let dev_server = "http://localhost:1420";
    if cfg!(debug_assertions) {
        let mut ws = connect(host.port, &[dev_server]).await.expect(dev_server);
        expect_hello_ok(host.port, &mut ws, &token).await;
    } else {
        assert_refused_403(host.port, &[dev_server]).await;
    }
}

#[tokio::test]
async fn refuses_bad_token() {
    let host = Host::start();
    let token = host.token();

    // These stay silent while the checks below run, which also shows that a client
    // waiting on its hello does not hold up anyone else.
    let silent_since = Instant::now();
    let mut silent = open(host.port).await;
    let no_upgrade = TcpStream::connect(("127.0.0.1", host.port))
        .await
        .expect("connect to the host");

    let last = token.chars().last().expect("a token");
    let flipped_last = format!("{}{}", &token[..63], if last == '0' { '1' } else { '0' });
    for wrong in [
        "wrong".to_string(),
        String::new(),
        token[..63].to_string(),
        format!("{token}0"),
        flipped_last,
        token.to_uppercase(),
    ] {
        let mut ws = open(host.port).await;
        say(
            &mut ws,
            json!({"type": "hello", "token": wrong, "proto": 3}),
        )
        .await;
        hear_close(&mut ws).await;
    }

    // A wrong token learns nothing about the protocol version either.
    let mut ws = open(host.port).await;
    say(
        &mut ws,
        json!({"type": "hello", "token": "wrong", "proto": 4}),
    )
    .await;
    hear_close(&mut ws).await;

    // Anything but a well formed hello as the first frame closes.
    for first in [
        Message::text(r#"{"type":"list"}"#),
        Message::text("not json"),
        Message::text(json!({"type": "hello", "proto": 3}).to_string()),
        Message::binary(
            json!({"type": "hello", "token": token, "proto": 3})
                .to_string()
                .into_bytes(),
        ),
        Message::Ping(vec![1].into()),
    ] {
        let mut ws = open(host.port).await;
        ws.send(first).await.expect("send the first frame");
        hear_close(&mut ws).await;
    }

    // The right token with the wrong version is told why, and then closed.
    let mut ws = open(host.port).await;
    say(
        &mut ws,
        json!({"type": "hello", "token": token, "proto": 4}),
    )
    .await;
    let reply = hear(&mut ws).await;
    assert_eq!(reply["type"], "error", "{reply}");
    assert_eq!(reply["code"], "proto_mismatch", "{reply}");
    hear_close(&mut ws).await;

    // The right token still gets in, so the refusals above are not a dead host.
    let mut ws = open(host.port).await;
    expect_hello_ok(host.port, &mut ws, &token).await;

    hear_close(&mut silent).await;
    let waited = silent_since.elapsed();
    assert!(
        waited >= Duration::from_secs(4) && waited <= Duration::from_secs(7),
        "a silent client was closed after {waited:?}"
    );

    // A client that never even upgrades must not hold a connection open forever.
    let closed = timeout(Duration::from_secs(3), async {
        let mut buffer = [0u8; 16];
        loop {
            no_upgrade.readable().await.expect("wait on the socket");
            match no_upgrade.try_read(&mut buffer) {
                Ok(0) => return true,
                Ok(_) => return false,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => return true,
            }
        }
    })
    .await
    .expect("a client that never upgraded stayed connected");
    assert!(
        closed,
        "the host sent bytes to a client that never upgraded"
    );
}

#[tokio::test]
async fn proto_2_client_is_refused() {
    let host = Host::start();
    let mut ws = open(host.port).await;
    say(
        &mut ws,
        json!({"type": "hello", "token": host.token(), "proto": 2}),
    )
    .await;
    let reply = hear(&mut ws).await;
    assert_eq!(reply["type"], "error", "{reply}");
    assert_eq!(reply["code"], "proto_mismatch", "{reply}");
    assert!(
        reply["message"].as_str().is_some_and(|m| m.contains('3')),
        "{reply}"
    );
    hear_close(&mut ws).await;
}

#[tokio::test]
async fn binds_loopback_only() {
    let host = Host::start();

    // The positive half, so this test cannot pass with nothing listening.
    let mut ws = open(host.port).await;
    expect_hello_ok(host.port, &mut ws, &host.token()).await;

    let Some(address) = non_loopback_address() else {
        println!("binds_loopback_only: this computer has no non-loopback address, skipped");
        return;
    };
    println!("binds_loopback_only: trying {address}:{}", host.port);
    let attempt = timeout(
        Duration::from_secs(5),
        TcpStream::connect((address, host.port)),
    )
    .await;
    assert!(
        !matches!(attempt, Ok(Ok(_))),
        "the host answered on {address}"
    );
}

/// A connected UDP socket picks a route and sends nothing. 192.0.2.1 is a reserved
/// documentation address.
fn non_loopback_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[tokio::test]
async fn second_host_exits() {
    let mut first = Host::start();
    let token_before = first.token();
    let mut ws = signed_in(&first).await;

    let mut second = Host::spawn(first.port, DataDir::fresh(), Stdio::piped());
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = second.child.try_wait().expect("check the second host") {
            break status;
        }
        assert!(Instant::now() < deadline, "the second host kept running");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(!status.success(), "the second host exited with {status}");

    let mut stderr = String::new();
    std::io::Read::read_to_string(
        second
            .child
            .stderr
            .as_mut()
            .expect("the second host's stderr"),
        &mut stderr,
    )
    .expect("read the second host's stderr");
    assert_eq!(stderr.lines().count(), 1, "stderr was: {stderr:?}");
    assert!(
        !second.dir.0.exists(),
        "the second host touched the disk before finding the port taken"
    );

    // The first host and its existing connection did not notice.
    let first_status = first.child.try_wait().expect("check the first host");
    assert_eq!(first_status, None, "the first host exited");
    assert_eq!(first.token(), token_before);
    ws.send(Message::text("not json")).await.expect("send");
    assert_eq!(hear(&mut ws).await["code"], "bad_request");
    signed_in(&first).await;
}

#[tokio::test]
async fn bad_request_after_hello_stays_open() {
    let host = Host::start();
    let mut ws = signed_in(&host).await;
    for bad in [
        "not json".to_string(),
        json!({"type": "no_such_frame"}).to_string(),
        json!({"type": "attach", "session": "one"}).to_string(),
        json!({"type": "hello", "token": host.token(), "proto": 3}).to_string(),
    ] {
        ws.send(Message::text(bad.clone())).await.expect("send");
        let reply = hear(&mut ws).await;
        assert_eq!(reply["type"], "error", "{bad}: {reply}");
        assert_eq!(reply["code"], "bad_request", "{bad}: {reply}");
    }
}

/// Written out as raw JSON so a change to the wire format fails here. None of these reach a
/// shell, so this runs everywhere.
#[tokio::test]
async fn session_frames_get_named_errors() {
    let host = Host::start();
    let mut ws = signed_in(&host).await;
    let uuid = "7f1d9b3e-2a4c-4e8f-9a6b-0c1d2e3f4a5b";
    for (frame, expected) in [
        (
            json!({"type": "create", "uuid": uuid, "kind": "fish", "cwd": "/", "cols": 80, "rows": 24}),
            "bad_kind",
        ),
        (
            json!({"type": "create", "uuid": "nope", "kind": "shell", "cwd": "/", "cols": 80, "rows": 24}),
            "bad_request",
        ),
        (
            json!({"type": "create", "uuid": uuid, "kind": "shell", "cwd": "/", "cols": 0, "rows": 24}),
            "bad_size",
        ),
        (
            json!({"type": "create", "uuid": uuid, "kind": "shell", "cwd": "/no/such/folder", "cols": 80, "rows": 24}),
            "bad_cwd",
        ),
        (
            json!({"type": "attach", "session": 9, "cols": 80, "rows": 0}),
            "bad_size",
        ),
        (
            json!({"type": "attach", "session": 9, "cols": 80, "rows": 24}),
            "no_session",
        ),
        (json!({"type": "detach", "session": 9}), "no_session"),
        (
            json!({"type": "resize", "session": 9, "cols": 80, "rows": 24}),
            "no_session",
        ),
        (json!({"type": "kill", "session": 9}), "no_session"),
    ] {
        say(&mut ws, frame.clone()).await;
        let reply = hear(&mut ws).await;
        assert_eq!(reply["type"], "error", "{frame}: {reply}");
        assert_eq!(reply["code"], expected, "{frame}: {reply}");
    }

    ws.send(Message::binary(vec![9, 0, 0, 0, b'x']))
        .await
        .expect("send");
    assert_eq!(hear(&mut ws).await["code"], "no_session");
    ws.send(Message::binary(vec![9, 0, 0])).await.expect("send");
    assert_eq!(hear(&mut ws).await["code"], "bad_request");

    // A late ack is not an error, so the next thing heard is the answer to `list`.
    say(&mut ws, json!({"type": "ack", "session": 9, "bytes": 1})).await;
    say(&mut ws, json!({"type": "list"})).await;
    assert_eq!(
        hear(&mut ws).await,
        json!({"type": "sessions", "items": []})
    );
}

#[tokio::test]
async fn tree_get_answers_an_empty_tree() {
    let host = Host::start();
    let mut ws = signed_in(&host).await;
    say(&mut ws, json!({"type": "tree_get"})).await;
    assert_eq!(
        hear(&mut ws).await,
        json!({"type": "tree", "tree": {"groups": [], "ungrouped": [], "closed": [], "defaults": {}}})
    );
}

#[tokio::test]
async fn tree_op_gets_named_errors() {
    let host = Host::start();
    let mut ws = signed_in(&host).await;
    for (op, code) in [
        (json!({"type": "close_tab", "tab": "t9"}), "no_tab"),
        (
            json!({"type": "rename_group", "group": "folder:none", "name": "x"}),
            "no_group",
        ),
        (json!({"type": "reopen_tab"}), "bad_op"),
        (
            json!({"type": "open_folder", "path": "/no/such/folder/here"}),
            "bad_cwd",
        ),
    ] {
        say(&mut ws, json!({"type": "tree_op", "op": op})).await;
        let reply = hear(&mut ws).await;
        assert_eq!(reply["type"], "error", "{reply}");
        assert_eq!(reply["code"], code, "{op} {reply}");
    }
}

#[tokio::test]
async fn frame_over_the_limit_closes() {
    let host = Host::start();
    let mut ws = signed_in(&host).await;

    let at_limit = "x".repeat(LIMIT_BYTES);
    ws.send(Message::text(at_limit)).await.expect("send");
    assert_eq!(hear(&mut ws).await["code"], "bad_request");

    let over_limit = "x".repeat(LIMIT_BYTES + 1);
    // The host may close while this is still being written, so a failed send is fine.
    let _ = ws.send(Message::text(over_limit)).await;
    hear_close(&mut ws).await;
}

#[cfg(unix)]
#[tokio::test]
async fn token_file_is_private() {
    use std::os::unix::fs::PermissionsExt;

    let host = Host::start();
    let mode = |path: &Path| {
        std::fs::metadata(path)
            .expect("read the mode")
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode(&host.dir.0), 0o700);
    assert_eq!(mode(&host.token_path()), 0o600);
    // `shell` holds the shell hook, and may not be written yet.
    let names: Vec<_> = std::fs::read_dir(&host.dir.0)
        .expect("list the folder")
        .map(|entry| entry.expect("read an entry").file_name())
        .filter(|name| name != "shell")
        .collect();
    assert_eq!(names, ["host-token"], "the temp file must not stay behind");

    let token = host.token();
    assert_eq!(token.len(), 64, "{token:?}");
    assert!(
        token.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
        "{token:?}"
    );
}

#[tokio::test]
async fn existing_token_is_reused() {
    let token = "ab".repeat(32);
    let dir = DataDir::fresh();
    std::fs::create_dir_all(&dir.0).expect("seed the folder");
    std::fs::write(token_path(&dir.0), &token).expect("seed the token");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let loose = std::fs::Permissions::from_mode(0o644);
        std::fs::set_permissions(token_path(&dir.0), loose).expect("loosen the token file");
    }

    let host = Host::start_in(dir);
    // A hello that succeeds means the host has finished with the token file.
    let mut ws = open(host.port).await;
    expect_hello_ok(host.port, &mut ws, &token).await;
    assert_eq!(host.token(), token);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(host.token_path())
            .expect("read the mode")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

/// A crash can leave a short file behind. It must never become the token.
#[tokio::test]
async fn partial_token_file_is_replaced() {
    for partial in ["", "abc"] {
        let dir = DataDir::fresh();
        std::fs::create_dir_all(&dir.0).expect("seed the folder");
        std::fs::write(token_path(&dir.0), partial).expect("seed the token");

        let host = Host::start_in(dir);
        let mut refused = open(host.port).await;
        say(
            &mut refused,
            json!({"type": "hello", "token": partial, "proto": 3}),
        )
        .await;
        hear_close(&mut refused).await;
        signed_in(&host).await;
    }
}

#[cfg(unix)]
mod sessions {
    use super::*;
    use term_host::protocol::{decode_binary, encode_binary};

    const FLOOD_LIMIT: usize = 4 * 1024 * 1024;
    const LARGEST_CHUNK: usize = 64 * 1024;
    const UUID: &str = "7f1d9b3e-2a4c-4e8f-9a6b-0c1d2e3f4a5b";
    const PATIENCE: Duration = Duration::from_secs(10);

    enum Heard {
        Text(Value),
        Bytes(u32, Vec<u8>),
    }

    async fn hear_any(ws: &mut Ws, wait: Duration) -> Option<Heard> {
        loop {
            let frame = timeout(wait, ws.next()).await.ok()?;
            match frame.expect("the connection ended").expect("read a frame") {
                Message::Text(text) => {
                    let value = serde_json::from_str(text.as_str()).expect("parse a host frame");
                    return Some(Heard::Text(value));
                }
                Message::Binary(frame) => {
                    let (session, bytes) = decode_binary(&frame).expect("a session number");
                    return Some(Heard::Bytes(session, bytes.to_vec()));
                }
                _ => {}
            }
        }
    }

    /// The next text frame. Output on the way there is thrown away.
    async fn hear_text(ws: &mut Ws) -> Value {
        loop {
            match hear_any(ws, PATIENCE).await.expect("no frame in time") {
                Heard::Text(value) => return value,
                Heard::Bytes(..) => {}
            }
        }
    }

    async fn create(ws: &mut Ws) -> u32 {
        say(
            ws,
            json!({"type": "create", "uuid": UUID, "kind": "shell", "cwd": "/", "cols": 80, "rows": 24}),
        )
        .await;
        let reply = hear_text(ws).await;
        assert_eq!(reply["type"], "created", "{reply}");
        assert_eq!(reply["uuid"], UUID, "{reply}");
        reply["session"].as_u64().expect("a session number") as u32
    }

    /// Returns the snapshot.
    async fn attach_at(ws: &mut Ws, session: u32, cols: u16, rows: u16) -> Vec<u8> {
        say(
            ws,
            json!({"type": "attach", "session": session, "cols": cols, "rows": rows}),
        )
        .await;
        let reply = hear_text(ws).await;
        assert_eq!(reply["type"], "attached", "{reply}");
        assert_eq!(reply["session"], session, "{reply}");
        match hear_any(ws, PATIENCE).await.expect("no snapshot in time") {
            Heard::Bytes(from, bytes) => {
                assert_eq!(from, session);
                assert_eq!(reply["snapshot_bytes"], bytes.len(), "{reply}");
                bytes
            }
            Heard::Text(other) => panic!("expected the snapshot, got {other}"),
        }
    }

    async fn attach(ws: &mut Ws, session: u32) -> Vec<u8> {
        attach_at(ws, session, 80, 24).await
    }

    async fn type_line(ws: &mut Ws, session: u32, line: &str) {
        let bytes = format!("{line}\n");
        ws.send(Message::binary(encode_binary(session, bytes.as_bytes())))
            .await
            .expect("send input");
    }

    async fn ack(ws: &mut Ws, session: u32, bytes: usize) {
        say(
            ws,
            json!({"type": "ack", "session": session, "bytes": bytes}),
        )
        .await;
    }

    /// Everything the session printed up to and including `needle`.
    async fn read_until(ws: &mut Ws, session: u32, needle: &str) -> String {
        let deadline = Instant::now() + PATIENCE;
        let mut seen = Vec::new();
        loop {
            let text = String::from_utf8_lossy(&seen);
            if text.contains(needle) {
                return text.into_owned();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match hear_any(ws, left).await {
                Some(Heard::Bytes(from, bytes)) if from == session => seen.extend(bytes),
                Some(Heard::Bytes(..)) => {}
                Some(Heard::Text(value)) => panic!("expected output, got {value}"),
                None => panic!("never saw {needle:?} in {text:?}"),
            }
        }
    }

    /// Reads and acks a flood until `total` reaches `target`.
    async fn read_acking(ws: &mut Ws, session: u32, total: &mut usize, target: usize) {
        while *total < target {
            match hear_any(ws, PATIENCE).await {
                Some(Heard::Bytes(_, bytes)) => {
                    *total += bytes.len();
                    ack(ws, session, *total).await;
                }
                Some(Heard::Text(value)) => panic!("expected output, got {value}"),
                None => panic!("the flood stopped at {total} bytes, wanted {target}"),
            }
        }
    }

    /// Reads a flood until it goes quiet, which only counts once `total` is past `at_least`.
    async fn read_until_quiet(
        ws: &mut Ws,
        session: u32,
        total: &mut usize,
        at_least: usize,
        acking: bool,
    ) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match hear_any(ws, Duration::from_millis(300)).await {
                Some(Heard::Bytes(_, bytes)) => {
                    *total += bytes.len();
                    if acking {
                        ack(ws, session, *total).await;
                    }
                }
                Some(Heard::Text(value)) => panic!("expected output, got {value}"),
                None if *total >= at_least => return,
                None => assert!(Instant::now() < deadline, "only {total} bytes arrived"),
            }
        }
    }

    async fn list(ws: &mut Ws) -> Vec<Value> {
        say(ws, json!({"type": "list"})).await;
        loop {
            let reply = hear_text(ws).await;
            if reply["type"] == "sessions" {
                return reply["items"].as_array().expect("an item list").clone();
            }
        }
    }

    /// Waits for the session list to pass `check`, because a frame takes effect a moment
    /// after it is sent.
    async fn wait_for_list(ws: &mut Ws, check: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let items = list(ws).await;
            if check(&items) {
                return items;
            }
            assert!(
                Instant::now() < deadline,
                "the list never settled: {items:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// A report as the hook prints it, typed by hand because the test host's shell is
    /// `/bin/sh`, which gets no hook.
    const REPORT_TMP: &str = r#"printf '\033]7;file://%s/tmp\007' "$(hostname)""#;

    #[tokio::test]
    async fn cwd_frame_reaches_client() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        attach(&mut ws, session).await;
        type_line(&mut ws, session, REPORT_TMP).await;
        loop {
            let frame = hear_text(&mut ws).await;
            if frame["type"] == "cwd" {
                assert_eq!(
                    frame,
                    json!({"type": "cwd", "session": session, "cwd": real_tmp()})
                );
                break;
            }
        }
    }

    #[tokio::test]
    async fn list_carries_cwd() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        let items = list(&mut ws).await;
        assert_eq!(items[0]["cwd"], Value::Null, "{items:?}");
        attach(&mut ws, session).await;
        type_line(&mut ws, session, REPORT_TMP).await;
        wait_for_list(&mut ws, |items| items[0]["cwd"] == real_tmp().as_str()).await;
    }

    #[tokio::test]
    async fn late_attach_gets_current_cwd() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        attach(&mut ws, session).await;
        type_line(&mut ws, session, REPORT_TMP).await;
        wait_for_list(&mut ws, |items| items[0]["cwd"] == real_tmp().as_str()).await;

        let mut late = signed_in(&host).await;
        attach(&mut late, session).await;
        loop {
            let frame = hear_text(&mut late).await;
            if frame["type"] == "cwd" {
                assert_eq!(
                    frame,
                    json!({"type": "cwd", "session": session, "cwd": real_tmp()})
                );
                break;
            }
        }
    }

    /// `/tmp` is a link on macOS, and reported folders arrive resolved.
    fn real_tmp() -> String {
        let tmp = std::fs::canonicalize("/tmp").expect("resolve /tmp");
        tmp.to_str().expect("a UTF-8 folder").to_string()
    }

    #[tokio::test]
    async fn create_attach_type_echo() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        attach(&mut ws, session).await;
        // Split so the terminal echoing the typed line cannot satisfy the wait.
        type_line(&mut ws, session, "echo round''trip").await;
        read_until(&mut ws, session, "roundtrip").await;

        let items = list(&mut ws).await;
        assert_eq!(
            items,
            [json!({
                "session": session, "uuid": UUID, "kind": "shell", "title": null,
                "cwd": null, "exited": null, "cols": 80, "rows": 24,
            })]
        );
    }

    #[tokio::test]
    async fn sessions_survive_disconnect() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        attach(&mut ws, session).await;
        type_line(&mut ws, session, "echo pid=$$ mark''er").await;
        let before = read_until(&mut ws, session, "marker").await;
        drop(ws);
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut ws = signed_in(&host).await;
        assert_eq!(list(&mut ws).await[0]["exited"], Value::Null);
        let snapshot = attach(&mut ws, session).await;
        assert!(
            String::from_utf8_lossy(&snapshot).contains("marker"),
            "the screen did not survive: {:?}",
            String::from_utf8_lossy(&snapshot)
        );
        type_line(&mut ws, session, "echo pid=$$ sec''ond").await;
        let after = read_until(&mut ws, session, "second").await;
        assert_eq!(pid_in(&before), pid_in(&after));
    }

    fn pid_in(text: &str) -> String {
        text.match_indices("pid=")
            .map(|(at, _)| {
                text[at + 4..]
                    .chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            })
            .find(|digits| !digits.is_empty())
            .unwrap_or_else(|| panic!("no pid in {text:?}"))
    }

    #[tokio::test]
    async fn ack_lets_a_flood_continue() {
        let host = Host::start();

        let mut acking = signed_in(&host).await;
        let session = create(&mut acking).await;
        let mut total = attach(&mut acking, session).await.len();
        type_line(&mut acking, session, "yes flood").await;
        read_acking(&mut acking, session, &mut total, 3 * FLOOD_LIMIT).await;

        let mut silent = signed_in(&host).await;
        let session = create(&mut silent).await;
        let mut total = attach(&mut silent, session).await.len();
        type_line(&mut silent, session, "yes flood").await;
        read_until_quiet(&mut silent, session, &mut total, FLOOD_LIMIT, false).await;
        assert!(
            total <= FLOOD_LIMIT + LARGEST_CHUNK,
            "a client that never acks got {total} bytes"
        );

        // Catching up opens it again.
        ack(&mut silent, session, total).await;
        let target = total + FLOOD_LIMIT;
        read_acking(&mut silent, session, &mut total, target).await;
    }

    #[tokio::test]
    async fn stale_client_hears_detached() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        attach(&mut ws, session).await;
        type_line(&mut ws, session, "yes flood").await;

        assert_eq!(
            hear_text(&mut ws).await,
            json!({"type": "detached", "session": session})
        );
        type_line(&mut ws, session, "echo late").await;
        assert_eq!(hear_text(&mut ws).await["code"], "not_attached");

        let total = attach(&mut ws, session).await.len();
        ack(&mut ws, session, total).await;
    }

    #[tokio::test]
    async fn title_reaches_the_client() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        attach(&mut ws, session).await;
        assert_eq!(list(&mut ws).await[0]["title"], Value::Null);

        let set_title = r"printf '\033]2;%s\007' 'tab one'";
        type_line(&mut ws, session, set_title).await;
        let title = json!({"type": "title", "session": session, "title": "tab one"});
        assert_eq!(hear_text(&mut ws).await, title);
        assert_eq!(list(&mut ws).await[0]["title"], "tab one");

        // The same title again is not a change, so the next text frame is the list.
        type_line(&mut ws, session, set_title).await;
        type_line(&mut ws, session, "echo sa''me").await;
        read_until(&mut ws, session, "same").await;
        let items = list(&mut ws).await;
        assert_eq!(items[0]["title"], "tab one");

        // A later attach hears it right after the snapshot.
        let mut other = signed_in(&host).await;
        attach(&mut other, session).await;
        assert_eq!(hear_text(&mut other).await, title);
    }

    #[tokio::test]
    async fn kill_removes_a_session() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;

        // Running: the exit is announced, then it goes.
        let running = create(&mut ws).await;
        attach(&mut ws, running).await;
        say(&mut ws, json!({"type": "kill", "session": running})).await;
        assert_eq!(
            hear_text(&mut ws).await,
            json!({"type": "exited", "session": running, "clean": false})
        );
        wait_for_list(&mut ws, |items| items.is_empty()).await;
        say(&mut ws, json!({"type": "kill", "session": running})).await;
        assert_eq!(hear_text(&mut ws).await["code"], "no_session");

        // Finished: it stays, with its screen and exit, until a kill removes it.
        let finished = create(&mut ws).await;
        assert!(finished > running, "session numbers must not be reused");
        attach(&mut ws, finished).await;
        type_line(&mut ws, finished, "echo last wor''ds; exit 0").await;
        assert_eq!(
            hear_text(&mut ws).await,
            json!({"type": "exited", "session": finished, "clean": true})
        );
        type_line(&mut ws, finished, "too late").await;
        assert_eq!(hear_text(&mut ws).await["code"], "not_attached");
        drop(ws);

        let mut ws = signed_in(&host).await;
        assert_eq!(list(&mut ws).await[0]["exited"], true);
        let snapshot = attach(&mut ws, finished).await;
        assert!(String::from_utf8_lossy(&snapshot).contains("last words"));
        assert_eq!(
            hear_text(&mut ws).await,
            json!({"type": "exited", "session": finished, "clean": true})
        );
        say(&mut ws, json!({"type": "kill", "session": finished})).await;
        wait_for_list(&mut ws, |items| items.is_empty()).await;
    }

    #[tokio::test]
    async fn closing_the_socket_detaches() {
        let host = Host::start();
        let mut watching = signed_in(&host).await;
        let session = create(&mut watching).await;
        let mut total = attach(&mut watching, session).await.len();

        // Never reads or acks, so the flood stops once this one is 4 MiB behind.
        let mut holding = signed_in(&host).await;
        say(
            &mut holding,
            json!({"type": "attach", "session": session, "cols": 80, "rows": 24}),
        )
        .await;
        type_line(&mut watching, session, "yes flood").await;
        read_until_quiet(&mut watching, session, &mut total, FLOOD_LIMIT / 2, true).await;
        let held_at = total;

        drop(holding);
        let closed = Instant::now();
        let target = held_at + 1;
        timeout(
            Duration::from_secs(1),
            read_acking(&mut watching, session, &mut total, target),
        )
        .await
        .expect("the flood was still held 1 s after the socket closed");
        println!("flood resumed {:?} after the close", closed.elapsed());
    }

    #[tokio::test]
    async fn input_needs_an_attachment() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;

        type_line(&mut ws, session, "echo nope").await;
        assert_eq!(hear_text(&mut ws).await["code"], "not_attached");
        say(
            &mut ws,
            json!({"type": "resize", "session": session, "cols": 90, "rows": 20}),
        )
        .await;
        assert_eq!(hear_text(&mut ws).await["code"], "not_attached");

        attach(&mut ws, session).await;
        // Attaching again replaces the first attachment, so one line prints once.
        attach(&mut ws, session).await;
        type_line(&mut ws, session, "echo on''ce; echo do''ne").await;
        let seen = read_until(&mut ws, session, "done").await;
        assert_eq!(seen.matches("once").count(), 1, "{seen:?}");

        say(&mut ws, json!({"type": "detach", "session": session})).await;
        type_line(&mut ws, session, "echo nope").await;
        assert_eq!(hear_text(&mut ws).await["code"], "not_attached");

        // Another connection's attachment does not count here.
        let mut other = signed_in(&host).await;
        attach(&mut other, session).await;
        type_line(&mut ws, session, "echo nope").await;
        assert_eq!(hear_text(&mut ws).await["code"], "not_attached");
    }

    #[tokio::test]
    async fn last_resize_wins() {
        let host = Host::start();
        let mut first = signed_in(&host).await;
        let session = create(&mut first).await;
        attach(&mut first, session).await;

        let mut second = signed_in(&host).await;
        attach_at(&mut second, session, 100, 30).await;
        type_line(&mut second, session, "stty size").await;
        read_until(&mut second, session, "30 100").await;

        say(
            &mut first,
            json!({"type": "resize", "session": session, "cols": 90, "rows": 20}),
        )
        .await;
        wait_for_list(&mut first, |items| items[0]["cols"] == 90).await;
        type_line(&mut first, session, "stty size").await;
        read_until(&mut first, session, "20 90").await;

        say(
            &mut first,
            json!({"type": "resize", "session": session, "cols": 0, "rows": 20}),
        )
        .await;
        assert_eq!(hear_text(&mut first).await["code"], "bad_size");
    }

    #[tokio::test]
    async fn attaching_again_replaces_the_old_attachment() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        attach(&mut ws, session).await;
        attach(&mut ws, session).await;

        type_line(&mut ws, session, "echo on''ce").await;
        let mut seen = read_until(&mut ws, session, "once").await;
        while let Some(Heard::Bytes(_, bytes)) = hear_any(&mut ws, Duration::from_millis(300)).await
        {
            seen.push_str(&String::from_utf8_lossy(&bytes));
        }
        assert_eq!(seen.matches("once").count(), 1, "{seen:?}");
    }

    #[tokio::test]
    async fn stuck_program_drops_input() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        let mut other = signed_in(&host).await;
        let free = create(&mut other).await;
        attach(&mut other, free).await;
        attach(&mut ws, session).await;

        // Raw, because a full line buffer in canonical mode throws input away, not blocks.
        type_line(&mut ws, session, "stty raw -echo; echo rea''dy; sleep 60").await;
        read_until(&mut ws, session, "ready").await;

        let chunk = encode_binary(session, &[b'x'; 1024 * 1024]);
        for _ in 0..20 {
            ws.send(Message::binary(chunk.clone())).await.expect("send");
        }
        assert_eq!(hear_text(&mut ws).await["code"], "input_dropped");

        // Nothing else is held up, on this connection or by this session.
        assert_eq!(list(&mut ws).await.len(), 2);
        type_line(&mut other, free, "echo st''ill here").await;
        read_until(&mut other, free, "still here").await;
    }

    const OTHER_UUID: &str = "8e2c0a4f-3b5d-4f9a-8b7c-1d2e3f4a5b6c";

    /// Asks for the tree and returns it. Pushed trees on the way are skipped.
    async fn tree(ws: &mut Ws) -> Value {
        say(ws, json!({"type": "tree_get"})).await;
        pushed_tree(ws, |_| true).await
    }

    /// The next tree frame that passes `check`.
    async fn pushed_tree(ws: &mut Ws, check: impl Fn(&Value) -> bool) -> Value {
        loop {
            let frame = hear_text(ws).await;
            assert_ne!(frame["type"], "error", "{frame}");
            if frame["type"] == "tree" && check(&frame["tree"]) {
                return frame["tree"].clone();
            }
        }
    }

    async fn tree_op(ws: &mut Ws, op: Value) {
        say(ws, json!({"type": "tree_op", "op": op})).await;
    }

    fn leaf(session: &str) -> Value {
        json!({"type": "leaf", "session": session})
    }

    fn scratch_folder(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("term-host-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make a folder");
        dir.canonicalize().expect("resolve the folder")
    }

    #[tokio::test]
    async fn created_session_gets_a_tab() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        create(&mut ws).await;
        let tree = tree(&mut ws).await;
        assert_eq!(tree["ungrouped"][0]["split"], leaf(UUID), "{tree}");
    }

    #[tokio::test]
    async fn tree_pushed_to_second_client_after_op() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        create(&mut ws).await;
        let tab = tree(&mut ws).await["ungrouped"][0]["id"].clone();
        let mut other = signed_in(&host).await;
        tree(&mut other).await;

        tree_op(
            &mut ws,
            json!({"type": "rename_tab", "tab": tab, "name": "build"}),
        )
        .await;
        let pushed = pushed_tree(&mut other, |_| true).await;
        assert_eq!(pushed["ungrouped"][0]["name"], "build", "{pushed}");
    }

    #[tokio::test]
    async fn create_with_split_place_adds_pane() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        create(&mut ws).await;
        say(
            &mut ws,
            json!({"type": "create", "uuid": OTHER_UUID, "kind": "shell", "cwd": "/", "cols": 80,
                "rows": 24, "place": {"type": "split_of", "session": UUID, "dir": "right"}}),
        )
        .await;
        assert_eq!(hear_text(&mut ws).await["type"], "created");
        let tree = tree(&mut ws).await;
        let tabs = tree["ungrouped"].as_array().expect("tabs");
        assert_eq!(tabs.len(), 1, "{tree}");
        assert_eq!(tabs[0]["split"]["type"], "row", "{tree}");
        assert_eq!(
            tabs[0]["split"]["children"],
            json!([leaf(UUID), leaf(OTHER_UUID)])
        );
    }

    #[tokio::test]
    async fn closed_tab_reopens_through_frames() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        create(&mut ws).await;
        let tab = tree(&mut ws).await["ungrouped"][0]["id"].clone();

        tree_op(&mut ws, json!({"type": "close_tab", "tab": tab})).await;
        let closed = pushed_tree(&mut ws, |tree| tree["closed"][0]["panes"][0].is_object()).await;
        assert_eq!(closed["ungrouped"], json!([]), "{closed}");
        assert_eq!(closed["closed"][0]["panes"][0]["uuid"], UUID, "{closed}");
        assert_eq!(closed["closed"][0]["panes"][0]["kind"], "shell", "{closed}");
        wait_for_list(&mut ws, |items| items.is_empty()).await;

        tree_op(&mut ws, json!({"type": "reopen_tab"})).await;
        let reopened = pushed_tree(&mut ws, |tree| tree["closed"] == json!([])).await;
        assert_eq!(reopened["ungrouped"][0]["split"], leaf(UUID), "{reopened}");
        let items = wait_for_list(&mut ws, |items| items.len() == 1).await;
        assert_eq!(items[0]["uuid"], UUID, "{items:?}");
        assert_eq!(items[0]["exited"], Value::Null, "{items:?}");
        let session = items[0]["session"].as_u64().expect("a session number") as u32;
        attach(&mut ws, session).await;
    }

    #[tokio::test]
    async fn opened_folder_is_never_an_empty_group() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let folder = scratch_folder("open");
        tree(&mut ws).await;

        tree_op(&mut ws, json!({"type": "open_folder", "path": folder})).await;
        let opened = pushed_tree(&mut ws, |tree| tree["groups"] != json!([])).await;
        let group = &opened["groups"][0];
        assert_eq!(
            group["path"],
            folder.to_str().expect("a UTF-8 folder"),
            "{opened}"
        );
        let tabs = group["tabs"].as_array().expect("tabs");
        assert_eq!(tabs.len(), 1, "{opened}");
        let shell = tabs[0]["split"]["session"]
            .as_str()
            .expect("a pane")
            .to_string();
        let items = list(&mut ws).await;
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0]["uuid"], shell.as_str(), "{items:?}");
        assert_eq!(items[0]["kind"], "shell", "{items:?}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[tokio::test]
    async fn clean_exit_removes_its_tab() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        tree(&mut ws).await;
        attach(&mut ws, session).await;
        type_line(&mut ws, session, "exit 0").await;
        pushed_tree(&mut ws, |tree| tree["ungrouped"] == json!([])).await;
    }

    #[tokio::test]
    async fn kill_removes_its_tab() {
        let host = Host::start();
        let mut ws = signed_in(&host).await;
        let session = create(&mut ws).await;
        tree(&mut ws).await;
        say(&mut ws, json!({"type": "kill", "session": session})).await;
        let tree = pushed_tree(&mut ws, |tree| tree["ungrouped"] == json!([])).await;
        assert_eq!(tree["closed"], json!([]), "{tree}");
    }
}
