use std::collections::HashMap;
use std::io;
use std::mem;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::{Receiver, Sender, unbounded};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::spawn_blocking;
use tokio::time::{Instant, sleep, timeout, timeout_at};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::{WebSocketStream, accept_hdr_async_with_config};
use uuid::Uuid;

use crate::auth;
use crate::protocol::{
    ClientFrame, HostFrame, PROTO, SessionItem, code, decode_binary, encode_binary,
};
use crate::{ClientEvent, ClientId, Kind, Session, SessionHandle, Size, SpawnSpec};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
// A stuck program must not grow the host without limit.
const MAX_QUEUED_INPUT: usize = 16 * 1024 * 1024;

type Ws = WebSocketStream<TcpStream>;
type Out = UnboundedSender<Message>;
/// `Err` is an error frame to send back, which reads better with `?` than building a reply.
type Answer = Result<Option<HostFrame>, HostFrame>;

pub async fn bind(port: u16) -> Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .with_context(|| format!("cannot listen on 127.0.0.1:{port}"))
}

pub async fn serve(listener: TcpListener, token: String) {
    let token: Arc<str> = token.into();
    let registry = Arc::new(Registry::default());
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(serve_connection(
                    stream,
                    Arc::clone(&token),
                    Arc::clone(&registry),
                ));
            }
            // Running out of file descriptors fails here repeatedly, so do not spin on it.
            Err(error) => {
                eprintln!("term-host: accept failed: {error}");
                sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

// The Callback trait in tungstenite fixes the return type, so the error cannot be boxed.
#[allow(clippy::result_large_err)]
async fn serve_connection(stream: TcpStream, token: Arc<str>, registry: Arc<Registry>) {
    // A delayed small packet would add typing latency.
    if stream.set_nodelay(true).is_err() {
        return;
    }

    // The deadline covers the upgrade as well, so a client that never upgrades cannot hold
    // a connection open either.
    let deadline = Instant::now() + HELLO_TIMEOUT;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let upgrade = accept_hdr_async_with_config(
        stream,
        |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
            if origin_passes(request) {
                Ok(response)
            } else {
                let mut refusal = ErrorResponse::new(None);
                *refusal.status_mut() = StatusCode::FORBIDDEN;
                Err(refusal)
            }
        },
        Some(config),
    );
    let Ok(Ok(mut ws)) = timeout_at(deadline, upgrade).await else {
        return;
    };

    let first = match timeout_at(deadline, ws.next()).await {
        Ok(Some(Ok(frame))) => Some(frame),
        _ => None,
    };
    match judge_hello(first, &token) {
        Hello::Accepted => {}
        Hello::WrongProto => {
            let message = format!("this host speaks protocol {PROTO}");
            let refusal = HostFrame::error(code::PROTO_MISMATCH, &message);
            let _ = timeout(CLOSE_TIMEOUT, send_frame(&mut ws, &refusal)).await;
            close(&mut ws).await;
            return;
        }
        // Says nothing about why, so a caller cannot tell a wrong token from a bad frame.
        Hello::Refused => {
            close(&mut ws).await;
            return;
        }
    }

    let welcome = HostFrame::HelloOk {
        proto: PROTO,
        host_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    if send_frame(&mut ws, &welcome).await.is_ok() {
        serve_sessions(ws, registry).await;
    }
}

/// A missing Origin is allowed: browsers always send one on a WebSocket, and the token still
/// guards a non-browser client. More than one is never sent honestly, so it is refused.
fn origin_passes(request: &Request) -> bool {
    let mut origins = request.headers().get_all(ORIGIN).iter();
    match (origins.next(), origins.next()) {
        (None, _) => true,
        (Some(origin), None) => origin.to_str().is_ok_and(auth::origin_allowed),
        (Some(_), Some(_)) => false,
    }
}

enum Hello {
    Accepted,
    WrongProto,
    Refused,
}

/// The token is checked before the version, so only a caller holding the token learns
/// anything from the reply.
fn judge_hello(first: Option<Message>, token: &str) -> Hello {
    let Some(Message::Text(text)) = first else {
        return Hello::Refused;
    };
    let Ok(ClientFrame::Hello {
        token: given,
        proto,
    }) = serde_json::from_str(text.as_str())
    else {
        return Hello::Refused;
    };
    if !auth::tokens_match(&given, token) {
        Hello::Refused
    } else if proto != PROTO {
        Hello::WrongProto
    } else {
        Hello::Accepted
    }
}

async fn serve_sessions(ws: Ws, registry: Arc<Registry>) {
    let (mut sink, mut stream) = ws.split();
    // Unbounded because unacked bytes are what bound it: a socket that stops reading costs
    // at most the flood limit per session, since no acks arrive.
    let (out, mut outgoing) = unbounded_channel();
    let writer = tokio::spawn(async move {
        while let Some(message) = outgoing.recv().await {
            if sink.send(message).await.is_err() {
                return;
            }
        }
        let _ = sink.close().await;
    });
    let stop_writer = writer.abort_handle();

    let mut connection = Connection {
        registry,
        out,
        attachments: HashMap::new(),
    };
    while let Some(frame) = stream.next().await {
        match frame {
            Ok(Message::Text(text)) => connection.answer_text(text.as_str()).await,
            Ok(Message::Binary(frame)) => connection.take_input(&frame),
            Ok(Message::Close(_)) | Err(_) => break,
            // tungstenite answers pings itself.
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
        }
    }

    // Otherwise a gone client holds a program back until its stale clock runs out.
    connection.detach_all();
    drop(connection);
    if timeout(CLOSE_TIMEOUT, writer).await.is_err() {
        stop_writer.abort();
    }
}

/// One accepted client. Frames are handled one at a time, so they take effect in order.
struct Connection {
    registry: Arc<Registry>,
    out: Out,
    attachments: HashMap<u32, Attachment>,
}

struct Attachment {
    id: ClientId,
    // Held while a frame is handed to the writer task, so once this is cleared nothing from
    // this attachment can land behind the `attached` of a later one. Also cleared when the
    // attachment ends by itself.
    wanted: Arc<Mutex<bool>>,
}

impl Connection {
    async fn answer_text(&mut self, text: &str) {
        let answer = match serde_json::from_str(text) {
            Ok(frame) => self.answer(frame).await,
            Err(_) => Err(HostFrame::error(
                code::BAD_REQUEST,
                "not a frame this host understands",
            )),
        };
        self.reply(answer);
    }

    fn take_input(&mut self, frame: &[u8]) {
        let answer = self.write_input(frame);
        self.reply(answer);
    }

    fn reply(&self, answer: Answer) {
        match answer {
            Ok(Some(frame)) | Err(frame) => hand_text(&self.out, &frame),
            Ok(None) => {}
        }
    }

    fn write_input(&mut self, frame: &[u8]) -> Answer {
        let Some((session, bytes)) = decode_binary(frame) else {
            return Err(HostFrame::error(
                code::BAD_REQUEST,
                "a binary frame starts with a session number",
            ));
        };
        let (served, _) = self.attached(session)?;
        if served.input.push(bytes) {
            Ok(None)
        } else {
            Err(HostFrame::error(
                code::INPUT_DROPPED,
                "this program is not reading its input",
            ))
        }
    }

    async fn answer(&mut self, frame: ClientFrame) -> Answer {
        match frame {
            ClientFrame::Hello { .. } => {
                Err(HostFrame::error(code::BAD_REQUEST, "already said hello"))
            }
            ClientFrame::Create {
                uuid,
                kind,
                cwd,
                cols,
                rows,
            } => self.create(uuid, kind, cwd, cols, rows).await,
            ClientFrame::Attach {
                session,
                cols,
                rows,
            } => self.attach(session, cols, rows).await,
            ClientFrame::Detach { session } => {
                self.attached(session)?;
                self.drop_attachment(session);
                Ok(None)
            }
            ClientFrame::Resize {
                session,
                cols,
                rows,
            } => {
                let size = checked_size(cols, rows)?;
                let (served, _) = self.attached(session)?;
                served.handle.resize(size);
                Ok(None)
            }
            ClientFrame::Kill { session } => {
                self.registry.kill(session)?;
                Ok(None)
            }
            ClientFrame::Ack { session, bytes } => {
                // An ack can cross an exit or a detach on the wire, so a late one is no error.
                if let Ok((served, id)) = self.attached(session) {
                    served.handle.ack(id, bytes);
                }
                Ok(None)
            }
            ClientFrame::List => Ok(Some(HostFrame::Sessions {
                items: self.registry.items(),
            })),
            ClientFrame::LayoutGet => Ok(Some(HostFrame::Layout {
                json: lock(&self.registry.layout).clone(),
            })),
            ClientFrame::LayoutSet { json } => {
                *lock(&self.registry.layout) = Some(json);
                Ok(None)
            }
        }
    }

    async fn create(
        &mut self,
        uuid: String,
        kind: String,
        cwd: String,
        cols: u16,
        rows: u16,
    ) -> Answer {
        let size = checked_size(cols, rows)?;
        let parsed = Uuid::parse_str(&uuid)
            .map_err(|_| HostFrame::error(code::BAD_REQUEST, "uuid does not parse"))?;
        let (kind, kind_name) = match kind.as_str() {
            "shell" => (Kind::Shell, "shell"),
            // The uuid is also the chat id. Resuming a chat is not offered yet.
            "claude" => (
                Kind::Claude {
                    chat: parsed,
                    resume: false,
                },
                "claude",
            ),
            _ => return Err(HostFrame::error(code::BAD_KIND, "unknown kind")),
        };
        let spec = SpawnSpec {
            kind,
            cwd: PathBuf::from(cwd),
            uuid: parsed,
        };

        // Forking is slow, and looking at a folder can stall on a network mount.
        let spawned = spawn_blocking(move || {
            if !spec.cwd.is_dir() {
                return Err(HostFrame::error(code::BAD_CWD, "no such folder"));
            }
            let spawn_failed = |error: String| HostFrame::error(code::SPAWN_FAILED, &error);
            let handle = Session::spawn(spec, size).map_err(|e| spawn_failed(format!("{e:#}")))?;
            let handle = Arc::new(handle);
            let input =
                InputQueue::start(Arc::clone(&handle)).map_err(|e| spawn_failed(e.to_string()))?;
            Ok((handle, input))
        })
        .await;
        let (handle, input) = match spawned {
            Ok(result) => result?,
            Err(_) => {
                let message = "starting the program failed";
                return Err(HostFrame::error(code::SPAWN_FAILED, message));
            }
        };

        let session = self.registry.add(Served {
            uuid: uuid.clone(),
            kind: kind_name,
            handle,
            input,
            kill_asked: AtomicBool::new(false),
        });
        Ok(Some(HostFrame::Created { session, uuid }))
    }

    async fn attach(&mut self, session: u32, cols: u16, rows: u16) -> Answer {
        let size = checked_size(cols, rows)?;
        let served = self.registry.get(session).ok_or_else(no_session)?;
        self.drop_attachment(session);

        let (tx, rx) = unbounded();
        let wanted = Arc::new(Mutex::new(true));
        let pump = {
            let out = self.out.clone();
            let wanted = Arc::clone(&wanted);
            thread::Builder::new()
                .name("session-pump".to_string())
                .spawn(move || pump_events(session, rx, out, wanted))
        };
        if pump.is_err() {
            return Ok(Some(HostFrame::Detached { session }));
        }

        // Takes a snapshot, which can be slow on a long scrollback. If this panics, `tx` is
        // gone with it and the pump answers `detached`.
        let handle = Arc::clone(&served.handle);
        if let Ok(id) = spawn_blocking(move || handle.attach(tx, size)).await {
            self.attachments.insert(session, Attachment { id, wanted });
        }
        Ok(None)
    }

    /// The session, if this connection is still attached to it.
    fn attached(&mut self, session: u32) -> Result<(Arc<Served>, ClientId), HostFrame> {
        let served = self.registry.get(session).ok_or_else(no_session)?;
        let not_attached = || HostFrame::error(code::NOT_ATTACHED, "not attached to this session");
        let attachment = self.attachments.get(&session).ok_or_else(not_attached)?;
        if *lock(&attachment.wanted) {
            Ok((served, attachment.id))
        } else {
            self.attachments.remove(&session);
            Err(not_attached())
        }
    }

    fn drop_attachment(&mut self, session: u32) {
        let Some(attachment) = self.attachments.remove(&session) else {
            return;
        };
        let was_wanted = mem::replace(&mut *lock(&attachment.wanted), false);
        if was_wanted && let Some(served) = self.registry.get(session) {
            served.handle.detach(attachment.id);
        }
    }

    fn detach_all(&mut self) {
        let sessions: Vec<u32> = self.attachments.keys().copied().collect();
        for session in sessions {
            self.drop_attachment(session);
        }
    }
}

/// Runs on its own thread per attachment, taking a session's events to the connection's
/// writer task. Ends when the attachment does.
fn pump_events(session: u32, events: Receiver<ClientEvent>, out: Out, wanted: Arc<Mutex<bool>>) {
    loop {
        let event = events.recv();
        let mut wanted = lock(&wanted);
        if !*wanted {
            return;
        }
        match event {
            Ok(ClientEvent::Snapshot(bytes)) => {
                let snapshot_bytes = bytes.len() as u64;
                let attached = HostFrame::Attached {
                    session,
                    snapshot_bytes,
                };
                hand_text(&out, &attached);
                hand_binary(&out, session, &bytes);
            }
            Ok(ClientEvent::Output(bytes)) => hand_binary(&out, session, &bytes),
            Ok(ClientEvent::Title(title)) => {
                hand_text(&out, &HostFrame::Title { session, title });
            }
            Ok(ClientEvent::Exited { clean, .. }) => {
                *wanted = false;
                hand_text(&out, &HostFrame::Exited { session, clean });
                return;
            }
            // The session dropped this client. The app answers by attaching again.
            Err(_) => {
                *wanted = false;
                hand_text(&out, &HostFrame::Detached { session });
                return;
            }
        }
    }
}

/// Everything the host serves, shared by all connections. The locks only cover quick
/// lookups, so they are safe to take on the async runtime.
#[derive(Default)]
struct Registry {
    last_session: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<Served>>>,
    layout: Mutex<Option<String>>,
}

impl Registry {
    fn add(&self, served: Served) -> u32 {
        let session = self.last_session.fetch_add(1, Ordering::Relaxed) + 1;
        lock(&self.sessions).insert(session, Arc::new(served));
        session
    }

    fn get(&self, session: u32) -> Option<Arc<Served>> {
        lock(&self.sessions).get(&session).cloned()
    }

    fn remove(&self, session: u32) {
        let removed = lock(&self.sessions).remove(&session);
        drop(removed);
    }

    fn items(&self) -> Vec<SessionItem> {
        let sessions = lock(&self.sessions);
        let mut items: Vec<SessionItem> = sessions
            .iter()
            .map(|(&session, served)| {
                let status = served.handle.status();
                SessionItem {
                    session,
                    uuid: served.uuid.clone(),
                    kind: served.kind.to_string(),
                    title: status.title,
                    exited: status.exited,
                    cols: status.size.cols,
                    rows: status.size.rows,
                }
            })
            .collect();
        items.sort_by_key(|item| item.session);
        items
    }

    /// A finished session goes now. A running one is killed and goes once its exit has been
    /// announced, so clients attached to it hear why.
    fn kill(self: &Arc<Self>, session: u32) -> Result<(), HostFrame> {
        let served = self.get(session).ok_or_else(no_session)?;
        if served.handle.status().exited.is_some() {
            self.remove(session);
            return Ok(());
        }
        if served.kill_asked.swap(true, Ordering::Relaxed) {
            return Ok(());
        }
        served.handle.kill();

        let announced = served.handle.announced();
        let registry = Arc::clone(self);
        // Without this thread the session stays listed as finished, and a later kill removes it.
        let _ = thread::Builder::new()
            .name("session-reaper".to_string())
            .spawn(move || {
                let _ = announced.recv();
                registry.remove(session);
            });
        Ok(())
    }
}

/// A session and what the host knows about it. A finished one stays here, so a client
/// reconnecting later sees its screen and exit.
struct Served {
    uuid: String,
    kind: &'static str,
    handle: Arc<SessionHandle>,
    input: InputQueue,
    kill_asked: AtomicBool,
}

/// Each session has its own writer thread, because a write blocks while the program is not
/// reading, and that must hold up neither the runtime nor another session. A Ctrl+C sits
/// behind any input already queued here.
struct InputQueue {
    tx: Sender<Vec<u8>>,
    queued: Arc<AtomicUsize>,
}

impl InputQueue {
    fn start(handle: Arc<SessionHandle>) -> io::Result<InputQueue> {
        let (tx, rx) = unbounded::<Vec<u8>>();
        let queued = Arc::new(AtomicUsize::new(0));
        let taken = Arc::clone(&queued);
        thread::Builder::new()
            .name("session-input".to_string())
            .spawn(move || {
                for bytes in rx {
                    // Fails once the program has gone, which is not worth telling anyone.
                    let _ = handle.write(&bytes);
                    taken.fetch_sub(bytes.len(), Ordering::Relaxed);
                }
            })?;
        Ok(InputQueue { tx, queued })
    }

    /// False if the input was dropped because the queue is full.
    fn push(&self, bytes: &[u8]) -> bool {
        let reserved = self
            .queued
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |queued| {
                let after = queued + bytes.len();
                (after <= MAX_QUEUED_INPUT).then_some(after)
            });
        reserved.is_ok() && self.tx.send(bytes.to_vec()).is_ok()
    }
}

fn checked_size(cols: u16, rows: u16) -> Result<Size, HostFrame> {
    if cols == 0 || rows == 0 {
        Err(HostFrame::error(
            code::BAD_SIZE,
            "cols and rows must not be zero",
        ))
    } else {
        Ok(Size { cols, rows })
    }
}

fn no_session() -> HostFrame {
    HostFrame::error(code::NO_SESSION, "no such session")
}

// The writer task only goes away with the connection, when nobody is left to tell.
fn hand_text(out: &Out, frame: &HostFrame) {
    if let Ok(text) = serde_json::to_string(frame) {
        let _ = out.send(Message::text(text));
    }
}

fn hand_binary(out: &Out, session: u32, bytes: &[u8]) {
    let _ = out.send(Message::binary(encode_binary(session, bytes)));
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

async fn send_frame(ws: &mut Ws, frame: &HostFrame) -> Result<()> {
    let text = serde_json::to_string(frame)?;
    ws.send(Message::text(text)).await?;
    Ok(())
}

async fn close(ws: &mut Ws) {
    let _ = timeout(CLOSE_TIMEOUT, ws.close(None)).await;
}
