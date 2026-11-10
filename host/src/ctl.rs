//! The local socket that hook calls reach the host through: `<data_dir>/ctl.sock`, or a named
//! pipe on Windows. A request is one JSON line. The host never replies, so a hook call never
//! waits on it, and a request whose token is not its session's is dropped.

use std::collections::HashMap;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use interprocess::local_socket::tokio::Stream as TokioStream;
use interprocess::local_socket::traits::tokio::Listener as _;
use interprocess::local_socket::{ListenerOptions, Name, prelude::*};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::auth;

/// The largest request line read. A hook's JSON can carry a whole file a tool wrote.
pub(crate) const MAX_LINE: u64 = 16 << 20;

/// How long a connection may take to send its line before it is dropped.
const READ_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CtlRequest {
    pub token: String,
    /// The session uuid from `TA_SESSION`.
    pub session: String,
    /// The hook's event name, as given to `term-host hook`.
    pub event: String,
    /// The hook's JSON from Claude Code, as it came.
    pub payload: serde_json::Value,
}

/// The token of each live session, by session uuid. Cloned handles share one map: the
/// server adds a session's token when it spawns it and removes it when the session ends.
#[derive(Clone, Debug, Default)]
pub struct Tokens(Arc<Mutex<HashMap<String, String>>>);

impl Tokens {
    pub fn insert(&self, session: &str, token: &str) {
        self.map().insert(session.to_string(), token.to_string());
    }

    pub fn remove(&self, session: &str) {
        self.map().remove(session);
    }

    pub fn matches(&self, session: &str, token: &str) -> bool {
        self.map()
            .get(session)
            .is_some_and(|expected| auth::tokens_match(token, expected))
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The socket's name for a data folder.
#[cfg(not(windows))]
pub fn socket_name(data_dir: &Path) -> io::Result<Name<'static>> {
    use interprocess::local_socket::GenericFilePath;

    data_dir.join("ctl.sock").to_fs_name::<GenericFilePath>()
}

/// The pipe's name for a data folder: `\\.\pipe\terminal-app-ctl-<user>-<hash>`. Pipes are
/// machine wide, so the user and a hash of the data folder keep two hosts, or two test runs,
/// apart.
#[cfg(windows)]
pub fn socket_name(data_dir: &Path) -> io::Result<Name<'static>> {
    use interprocess::local_socket::GenericNamespaced;

    let user: String = std::env::var("USERNAME")
        .unwrap_or_else(|_| "user".to_string())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    // FNV-1a, so the host and the hook command get the same hash in any build.
    let hash = data_dir
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("terminal-app-ctl-{user}-{hash:016x}").to_ns_name::<GenericNamespaced>()
}

/// Listens on the socket and calls `on` with each request whose token matches `tokens`, one
/// at a time, in the order they arrive. Returns once listening; the work runs on the current
/// Tokio runtime, so this must be called from inside one. `on` runs on a runtime task, so it
/// must not block for long.
pub fn serve(
    data_dir: &Path,
    tokens: Tokens,
    on: impl Fn(CtlRequest) + Send + 'static,
) -> io::Result<()> {
    let runtime = tokio::runtime::Handle::try_current().map_err(io::Error::other)?;
    let _entered = runtime.enter();
    // A socket file left by a host that died is replaced. Only the host holding the port gets
    // here, so no live host is pushed off it.
    let listener = ListenerOptions::new()
        .name(socket_name(data_dir)?)
        .try_overwrite(true)
        .max_spin_time(Duration::from_secs(1))
        .create_tokio()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            data_dir.join("ctl.sock"),
            std::fs::Permissions::from_mode(0o600),
        )?;
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    runtime.spawn(async move {
        while let Some(request) = rx.recv().await {
            on(request);
        }
    });
    runtime.spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    let tokens = tokens.clone();
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        if let Some(request) = read_request(stream, &tokens).await {
                            let _ = tx.send(request);
                        }
                    });
                }
                // Out of file handles or the like: wait rather than spin.
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    });
    Ok(())
}

/// The connection's one request, or None for a slow, broken or unknown one.
async fn read_request(stream: TokioStream, tokens: &Tokens) -> Option<CtlRequest> {
    let mut reader = BufReader::new(stream.take(MAX_LINE));
    let mut line = Vec::new();
    tokio::time::timeout(READ_WAIT, reader.read_until(b'\n', &mut line))
        .await
        .ok()?
        .ok()?;
    let request: CtlRequest = serde_json::from_slice(&line).ok()?;
    tokens
        .matches(&request.session, &request.token)
        .then_some(request)
}

/// Sends one request and returns once it is written. Blocks: a caller that must not wait on
/// the host runs this on a thread it can leave behind.
pub fn send(data_dir: &Path, request: &CtlRequest) -> io::Result<()> {
    let mut line = serde_json::to_vec(request)?;
    line.push(b'\n');
    let mut stream = LocalSocketStream::connect(socket_name(data_dir)?)?;
    stream.write_all(&line)?;
    stream.flush()
}
