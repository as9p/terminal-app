use std::net::{IpAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
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

/// Asks the system for a free port. Parallel tests must not be handed the same one.
fn free_port() -> u16 {
    static HANDED_OUT: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    let mut handed_out = HANDED_OUT.lock().unwrap();
    loop {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("find a free port");
        let port = listener.local_addr().expect("read the free port").port();
        if !handed_out.contains(&port) {
            handed_out.push(port);
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
    say(ws, json!({"type": "hello", "token": token, "proto": 1})).await;
    let reply = hear(ws).await;
    assert_eq!(reply["type"], "hello_ok", "port {port}: {reply}");
    assert_eq!(reply["proto"], 1, "{reply}");
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
            json!({"type": "hello", "token": wrong, "proto": 1}),
        )
        .await;
        hear_close(&mut ws).await;
    }

    // A wrong token learns nothing about the protocol version either.
    let mut ws = open(host.port).await;
    say(
        &mut ws,
        json!({"type": "hello", "token": "wrong", "proto": 2}),
    )
    .await;
    hear_close(&mut ws).await;

    // Anything but a well formed hello as the first frame closes.
    for first in [
        Message::text(r#"{"type":"list"}"#),
        Message::text("not json"),
        Message::text(json!({"type": "hello", "proto": 1}).to_string()),
        Message::binary(
            json!({"type": "hello", "token": token, "proto": 1})
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
        json!({"type": "hello", "token": token, "proto": 2}),
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
        json!({"type": "hello", "token": host.token(), "proto": 1}).to_string(),
    ] {
        ws.send(Message::text(bad.clone())).await.expect("send");
        let reply = hear(&mut ws).await;
        assert_eq!(reply["type"], "error", "{bad}: {reply}");
        assert_eq!(reply["code"], "bad_request", "{bad}: {reply}");
    }
}

/// Written out as raw JSON so a change to the wire format fails here.
#[tokio::test]
async fn session_frames_are_understood_but_not_implemented() {
    let host = Host::start();
    let mut ws = signed_in(&host).await;
    let uuid = "7f1d9b3e-2a4c-4e8f-9a6b-0c1d2e3f4a5b";
    for frame in [
        json!({"type": "create", "uuid": uuid, "kind": "shell", "cwd": "/", "cols": 80, "rows": 24}),
        json!({"type": "attach", "session": 1, "cols": 80, "rows": 24}),
        json!({"type": "detach", "session": 1}),
        json!({"type": "resize", "session": 1, "cols": 80, "rows": 24}),
        json!({"type": "kill", "session": 1}),
        json!({"type": "ack", "session": 1, "bytes": 4096}),
        json!({"type": "list"}),
        json!({"type": "layout_get"}),
        json!({"type": "layout_set", "json": "{}"}),
    ] {
        say(&mut ws, frame.clone()).await;
        let reply = hear(&mut ws).await;
        assert_eq!(reply["type"], "error", "{frame}: {reply}");
        assert_eq!(reply["code"], "not_implemented", "{frame}: {reply}");
    }
    ws.send(Message::binary(vec![1, 0, 0, 0, b'x']))
        .await
        .expect("send");
    assert_eq!(hear(&mut ws).await["code"], "not_implemented");
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
    let names: Vec<_> = std::fs::read_dir(&host.dir.0)
        .expect("list the folder")
        .map(|entry| entry.expect("read an entry").file_name())
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
            json!({"type": "hello", "token": partial, "proto": 1}),
        )
        .await;
        hear_close(&mut refused).await;
        signed_in(&host).await;
    }
}
