use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::mem;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
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

use crate::protocol::{
    ClientFrame, HostFrame, PROTO, SessionItem, code, decode_binary, encode_binary,
};
use crate::state::{self, SavedSession, State};
use crate::{ClientEvent, ClientId, Kind, Session, SessionHandle, Size, SpawnSpec, Status};
use crate::{auth, shellinit};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
// A stuck program must not grow the host without limit.
const MAX_QUEUED_INPUT: usize = 16 * 1024 * 1024;
const SAVE_EVERY: Duration = Duration::from_secs(2);
// A restored shell starts before any client says how big it is. Its first attach resizes it.
const RESTORE_SIZE: Size = Size { cols: 80, rows: 24 };

type Ws = WebSocketStream<TcpStream>;
type Out = UnboundedSender<Message>;
/// `Err` is an error frame to send back, which reads better with `?` than building a reply.
type Answer = Result<Option<HostFrame>, HostFrame>;

pub async fn bind(port: u16) -> Result<TcpListener> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .with_context(|| format!("cannot listen on 127.0.0.1:{port}"))
}

/// Brings back the sessions saved in `data_dir`, serves until `shutdown` completes, then saves
/// once more. The caller should exit right after: the programs are still running, and a
/// session that ends from here on stays saved, because a reboot hangs up every shell.
pub async fn serve(
    listener: TcpListener,
    token: String,
    data_dir: PathBuf,
    shutdown: impl Future<Output = ()>,
) {
    let registry = Arc::new(Registry::new(data_dir));
    let restoring = Arc::clone(&registry);
    // Starting shells forks, and looking at folders can stall on a network mount.
    let _ = spawn_blocking(move || {
        // Before any shell starts, since each one loads the hook from there.
        if let Err(error) = shellinit::install(&restoring.data_dir) {
            eprintln!("term-host: shells will not report their folder: {error}");
        }
        restoring.restore();
    })
    .await;
    Arc::clone(&registry).start_saving();
    tokio::select! {
        () = accept(listener, token, Arc::clone(&registry)) => {}
        () = shutdown => {}
    }
    let _ = spawn_blocking(move || registry.shut_down()).await;
}

async fn accept(listener: TcpListener, token: String, registry: Arc<Registry>) {
    let token: Arc<str> = token.into();
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
        if served.running()?.input.push(bytes) {
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
                served.running()?.handle.resize(size);
                Ok(None)
            }
            ClientFrame::Kill { session } => {
                self.registry.kill(session)?;
                Ok(None)
            }
            ClientFrame::Ack { session, bytes } => {
                // An ack can cross an exit or a detach on the wire, so a late one is no error.
                if let Ok((served, id)) = self.attached(session)
                    && let Ok(running) = served.running()
                {
                    running.handle.ack(id, bytes);
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
            // The uuid is also the chat id. A new chat never resumes; a restored one can.
            "claude" => (
                Kind::Claude {
                    chat: parsed,
                    resume: false,
                },
                "claude",
            ),
            _ => return Err(HostFrame::error(code::BAD_KIND, "unknown kind")),
        };
        let chat = match kind {
            Kind::Claude { chat, .. } => Some(chat),
            Kind::Shell => None,
        };
        let spec = SpawnSpec {
            kind,
            cwd: PathBuf::from(cwd),
            uuid: parsed,
            data_dir: self.registry.data_dir.clone(),
        };
        let cwd = spec.cwd.clone();

        // Forking is slow, and looking at a folder can stall on a network mount.
        let spawned = spawn_blocking(move || {
            if !spec.cwd.is_dir() {
                return Err(HostFrame::error(code::BAD_CWD, "no such folder"));
            }
            start_program(spec, size)
        })
        .await;
        let running = spawned.unwrap_or_else(|_| Err(start_failed()))?;

        let served = Served::new(uuid.clone(), parsed, kind_name, chat, cwd, None);
        let _ = served.running.set(running);
        let session = self.registry.add(served);
        Ok(Some(HostFrame::Created { session, uuid }))
    }

    async fn attach(&mut self, session: u32, cols: u16, rows: u16) -> Answer {
        let size = checked_size(cols, rows)?;
        let served = self.registry.get(session).ok_or_else(no_session)?;
        self.drop_attachment(session);
        if served.running.get().is_none() {
            // A restored Claude session starts here, so a reboot does not start every chat at
            // once.
            let starting = Arc::clone(&served);
            let data_dir = self.registry.data_dir.clone();
            let started = spawn_blocking(move || starting.start(size, &data_dir)).await;
            started.unwrap_or_else(|_| Err(start_failed()))?;
        }
        let running = served.running()?;

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
        let handle = Arc::clone(&running.handle);
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
        if was_wanted
            && let Some(served) = self.registry.get(session)
            && let Ok(running) = served.running()
        {
            running.handle.detach(attachment.id);
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
            // A folder whose name is not UTF-8 cannot go in a text frame.
            Ok(ClientEvent::Cwd(path)) => {
                if let Some(cwd) = path.to_str() {
                    let cwd = cwd.to_string();
                    hand_text(&out, &HostFrame::Cwd { session, cwd });
                }
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
struct Registry {
    last_session: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<Served>>>,
    layout: Mutex<Option<String>>,
    data_dir: PathBuf,
    /// What is in `state.json` now. Held while saving, so saves never cross.
    saved: Mutex<State>,
    /// Set before the last save on the way out. Nothing is saved after it, so the sessions a
    /// shutdown or reboot ends stay saved.
    shutting_down: AtomicBool,
}

impl Registry {
    fn new(data_dir: PathBuf) -> Registry {
        Registry {
            last_session: AtomicU32::new(0),
            sessions: Mutex::new(HashMap::new()),
            layout: Mutex::new(None),
            data_dir,
            saved: Mutex::new(State::default()),
            shutting_down: AtomicBool::new(false),
        }
    }

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

    /// A restored Claude session that has not started yet reports a size of 0 by 0.
    fn items(&self) -> Vec<SessionItem> {
        let sessions = lock(&self.sessions);
        let mut items: Vec<SessionItem> = sessions
            .iter()
            .map(|(&session, served)| {
                let status = served.running.get().map(|running| running.handle.status());
                let size = status
                    .as_ref()
                    .map_or(Size { cols: 0, rows: 0 }, |status| status.size);
                SessionItem {
                    session,
                    uuid: served.uuid.clone(),
                    kind: served.kind.to_string(),
                    title: served.title(status.as_ref()),
                    cwd: status
                        .as_ref()
                        .and_then(|status| status.cwd.as_ref()?.to_str().map(String::from)),
                    exited: status.and_then(|status| status.exited),
                    cols: size.cols,
                    rows: size.rows,
                }
            })
            .collect();
        items.sort_by_key(|item| item.session);
        items
    }

    /// A finished session goes now. A running one is killed and goes once its exit has been
    /// announced, so clients attached to it hear why. Either way it is no longer saved.
    fn kill(self: &Arc<Self>, session: u32) -> Result<(), HostFrame> {
        let served = self.get(session).ok_or_else(no_session)?;
        let Some(running) = served.running.get() else {
            self.remove(session);
            return Ok(());
        };
        if running.handle.status().exited.is_some() {
            self.remove(session);
            return Ok(());
        }
        if served.kill_asked.swap(true, Ordering::Relaxed) {
            return Ok(());
        }
        running.handle.kill();

        let announced = running.handle.announced();
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

    /// Puts back what `state.json` holds. Shells start now. Claude sessions wait for their
    /// first attach.
    fn restore(&self) {
        let loaded = state::load(&self.data_dir);
        *lock(&self.layout) = loaded.layout.clone();
        for saved in &loaded.sessions {
            let Some(served) = Served::restored(saved) else {
                eprintln!("term-host: skipped a saved session that does not make sense");
                continue;
            };
            let shell = served.chat.is_none();
            let session = self.add(served);
            if shell
                && let Some(served) = self.get(session)
                && let Err(HostFrame::Error { message, .. }) =
                    served.start(RESTORE_SIZE, &self.data_dir)
            {
                // Stays listed, and its first attach tries again.
                eprintln!("term-host: could not restore a shell: {message}");
            }
        }
        *lock(&self.saved) = loaded;
    }

    fn start_saving(self: Arc<Self>) {
        let started = thread::Builder::new()
            .name("state-saver".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(SAVE_EVERY);
                    self.save();
                }
            });
        if let Err(error) = started {
            eprintln!("term-host: sessions will not be saved: {error}");
        }
    }

    /// Writes `state.json` if anything changed since the last write.
    fn save(&self) {
        let mut saved = lock(&self.saved);
        if self.shutting_down.load(Ordering::Relaxed) {
            return;
        }
        self.write_state(&mut saved);
    }

    /// The last save. Taken while every program still runs, so their folders can be read.
    fn shut_down(&self) {
        let mut saved = lock(&self.saved);
        self.shutting_down.store(true, Ordering::Relaxed);
        self.write_state(&mut saved);
    }

    fn write_state(&self, saved: &mut State) {
        let state = self.state();
        if state == *saved {
            return;
        }
        // Left as it was on failure, so the next save tries again.
        match state::save(&self.data_dir, &state) {
            Ok(()) => *saved = state,
            Err(error) => eprintln!("term-host: cannot save sessions: {error}"),
        }
    }

    fn state(&self) -> State {
        let served: Vec<(u32, Arc<Served>)> = lock(&self.sessions)
            .iter()
            .map(|(&session, served)| (session, Arc::clone(served)))
            .collect();
        let mut numbered: Vec<(u32, SavedSession)> = served
            .into_iter()
            .filter_map(|(session, served)| Some((session, served.saved()?)))
            .collect();
        numbered.sort_by_key(|(session, _)| *session);
        State {
            sessions: numbered.into_iter().map(|(_, saved)| saved).collect(),
            layout: lock(&self.layout).clone(),
        }
    }
}

/// A session and what the host knows about it. A finished one stays here, so a client
/// reconnecting later sees its screen and exit.
struct Served {
    uuid: String,
    id: Uuid,
    kind: &'static str,
    chat: Option<Uuid>,
    /// Where the program was last seen, or where it is to start.
    cwd: Mutex<PathBuf>,
    /// Saved before a restart, and shown until the program sets a title of its own.
    saved_title: Option<String>,
    /// Empty only for a restored session that has not started yet.
    running: OnceLock<Running>,
    starting: Mutex<()>,
    kill_asked: AtomicBool,
}

struct Running {
    handle: Arc<SessionHandle>,
    input: InputQueue,
}

impl Served {
    fn new(
        uuid: String,
        id: Uuid,
        kind: &'static str,
        chat: Option<Uuid>,
        cwd: PathBuf,
        saved_title: Option<String>,
    ) -> Served {
        Served {
            uuid,
            id,
            kind,
            chat,
            cwd: Mutex::new(cwd),
            saved_title,
            running: OnceLock::new(),
            starting: Mutex::new(()),
            kill_asked: AtomicBool::new(false),
        }
    }

    fn restored(saved: &SavedSession) -> Option<Served> {
        let id = Uuid::parse_str(&saved.uuid).ok()?;
        let (kind, chat) = match (saved.kind.as_str(), saved.chat) {
            ("shell", _) => ("shell", None),
            ("claude", Some(chat)) => ("claude", Some(chat)),
            _ => return None,
        };
        let served = Served::new(
            saved.uuid.clone(),
            id,
            kind,
            chat,
            saved.cwd.clone(),
            saved.title.clone(),
        );
        Some(served)
    }

    /// Only a session with no attachment yet can be waiting to start, so its absence reads as
    /// not attached.
    fn running(&self) -> Result<&Running, HostFrame> {
        self.running
            .get()
            .ok_or_else(|| HostFrame::error(code::NOT_ATTACHED, "not attached to this session"))
    }

    fn title(&self, status: Option<&Status>) -> Option<String> {
        status
            .and_then(|status| status.title.clone())
            .or_else(|| self.saved_title.clone())
    }

    /// Starts a restored session, in its saved folder or the home folder if that is gone. A
    /// Claude chat resumes only if it has a transcript, since one that never got a first
    /// message cannot be resumed and starts fresh with the same id. Blocks.
    fn start(&self, size: Size, data_dir: &Path) -> Result<(), HostFrame> {
        let _gate = lock(&self.starting);
        if self.running.get().is_some() {
            return Ok(());
        }
        let wanted = lock(&self.cwd).clone();
        let cwd = if wanted.is_dir() {
            wanted
        } else {
            fallback_dir()
        };
        let kind = match self.chat {
            Some(chat) => Kind::Claude {
                chat,
                resume: state::transcript_exists(chat),
            },
            None => Kind::Shell,
        };
        *lock(&self.cwd) = cwd.clone();
        let spec = SpawnSpec {
            kind,
            cwd,
            uuid: self.id,
            data_dir: data_dir.to_path_buf(),
        };
        let _ = self.running.set(start_program(spec, size)?);
        Ok(())
    }

    /// What to save, or None if the session should not come back: it was closed with `kill`,
    /// or it ended cleanly on its own, as a typed `exit` does. One ended by a signal stays,
    /// since a reboot hangs up every shell before the host is told to stop.
    fn saved(&self) -> Option<SavedSession> {
        if self.kill_asked.load(Ordering::Relaxed) {
            return None;
        }
        let running = self.running.get();
        let status = running.map(|running| running.handle.status());
        let exited = status.as_ref().and_then(|status| status.exited);
        if exited == Some(true) {
            return None;
        }
        // The shell hook's report first, since it holds even once the program has exited.
        // Without one, ask the system, but only while running: once it has exited its pid
        // may belong to something else.
        if let Some(dir) = status.as_ref().and_then(|status| status.cwd.clone()) {
            *lock(&self.cwd) = dir;
        } else if exited.is_none()
            && let Some(pid) = running.and_then(|running| running.handle.pid())
            && let Some(dir) = state::current_dir(pid)
        {
            *lock(&self.cwd) = dir;
        }
        Some(SavedSession {
            uuid: self.uuid.clone(),
            kind: self.kind.to_string(),
            cwd: lock(&self.cwd).clone(),
            title: self.title(status.as_ref()),
            chat: self.chat,
        })
    }
}

/// Starts the program and its input thread. Blocks.
fn start_program(spec: SpawnSpec, size: Size) -> Result<Running, HostFrame> {
    let spawn_failed = |error: String| HostFrame::error(code::SPAWN_FAILED, &error);
    let handle = Session::spawn(spec, size).map_err(|e| spawn_failed(format!("{e:#}")))?;
    let handle = Arc::new(handle);
    let input = InputQueue::start(Arc::clone(&handle)).map_err(|e| spawn_failed(e.to_string()))?;
    Ok(Running { handle, input })
}

fn start_failed() -> HostFrame {
    HostFrame::error(code::SPAWN_FAILED, "starting the program failed")
}

/// Where a restored session starts when its folder is gone.
fn fallback_dir() -> PathBuf {
    dirs::home_dir()
        .filter(|home| home.is_dir())
        .unwrap_or_else(|| PathBuf::from("/"))
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
