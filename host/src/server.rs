use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io;
use std::mem;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
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

use crate::ctl::{self, CtlRequest, Tokens};
use crate::env::host_env;
use crate::protocol::{
    ClientFrame, HostFrame, PROTO, SessionItem, StateItem, code, decode_binary, encode_binary,
};
use crate::session::resolve_dir;
use crate::state::{self, SavedSession, State};
use crate::states::{self, HookEvent, Tracker};
use crate::tree::{Op, Place, Tree, TreeError};
use crate::{ClientEvent, ClientId, Kind, Session, SessionHandle, Size, SpawnSpec, Status};
use crate::{auth, claudecfg, project, shellinit};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
// A stuck program must not grow the host without limit.
const MAX_QUEUED_INPUT: usize = 16 * 1024 * 1024;
// The most output one binary frame carries when waiting chunks are merged.
const MERGED_OUTPUT_BYTES: usize = 64 * 1024;
const SAVE_EVERY: Duration = Duration::from_secs(2);
// How often a finished turn is checked for having waited long enough to show as done.
const STATE_TICK: Duration = Duration::from_millis(250);
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
        // Also before any shell starts, since a `claude` in one reads both files.
        restoring.write_claude_files();
        restoring.restore();
    })
    .await;
    Arc::clone(&registry).start_hooks();
    Arc::clone(&registry).start_saving();
    Arc::clone(&registry).start_ticking();
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
        watcher: registry.next_watcher.fetch_add(1, Ordering::Relaxed),
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
    lock(&connection.registry.watchers).remove(&connection.watcher);
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
    /// This connection's key in `Registry::watchers`, which it joins with `tree_get`.
    watcher: u64,
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
                place,
            } => self.create(uuid, kind, cwd, cols, rows, place).await,
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
            // Sent by the registry, so it cannot land behind a newer pushed tree.
            ClientFrame::TreeGet => {
                self.registry.watch(self.watcher, &self.out);
                Ok(None)
            }
            ClientFrame::Seen { session } => {
                self.registry.saw_seen(&session);
                Ok(None)
            }
            ClientFrame::TreeOp { op } => {
                // `project::resolve` looks at folders, and an op can start or end programs.
                let registry = Arc::clone(&self.registry);
                let applied = spawn_blocking(move || registry.tree_op(op)).await;
                applied.unwrap_or_else(|_| Err(start_failed()))?;
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
        place: Place,
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
            known_cwd: false,
        };
        let cwd = spec.cwd.clone();

        // Forking is slow, and looking at a folder can stall on a network mount.
        let tokens = self.registry.tokens.clone();
        let spawned = spawn_blocking(move || {
            let Some(folder) = resolve_dir(&spec.cwd) else {
                return Err(HostFrame::error(code::BAD_CWD, "no such folder"));
            };
            let project = project::resolve(&folder);
            Ok((start_program(spec, size, &tokens)?, project))
        })
        .await;
        let (running, project) = spawned.unwrap_or_else(|_| Err(start_failed()))?;

        // The pane goes in before the session is listed. Its folders and exits reach the tree
        // only through the registry, so none can arrive before there is a pane for them.
        let shell = kind_name == "shell";
        lock(&self.registry.tree).add_session_in(&uuid, place, &project, shell);
        let served = Served::new(uuid.clone(), parsed, kind_name, chat, cwd, None);
        let _ = served.running.set(running);
        let session = self.registry.add(served);
        self.registry.push_tree();
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
            let tokens = self.registry.tokens.clone();
            let started = spawn_blocking(move || starting.start(size, &data_dir, &tokens)).await;
            started.unwrap_or_else(|_| Err(start_failed()))?;
        }
        let running = served.running()?;

        let (tx, rx) = unbounded();
        let wanted = Arc::new(Mutex::new(true));
        let pump = {
            let pumped = Pumped {
                session,
                served: Arc::clone(&served),
                registry: Arc::clone(&self.registry),
                out: self.out.clone(),
                wanted: Arc::clone(&wanted),
            };
            thread::Builder::new()
                .name("session-pump".to_string())
                .spawn(move || pump_events(pumped, rx))
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

/// What a pump thread needs from its attachment.
struct Pumped {
    session: u32,
    served: Arc<Served>,
    registry: Arc<Registry>,
    out: Out,
    wanted: Arc<Mutex<bool>>,
}

/// Runs on its own thread per attachment, taking a session's events to the connection's
/// writer task and folders and exits to the tree. Ends when the attachment does.
fn pump_events(pumped: Pumped, events: Receiver<ClientEvent>) {
    let Pumped {
        session,
        served,
        registry,
        out,
        wanted,
    } = pumped;
    // An event that was waiting but could not join merged output. It goes next.
    let mut pending = None;
    loop {
        let event = pending.take().map_or_else(|| events.recv(), Ok);
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
            Ok(ClientEvent::Output(bytes)) => {
                let (merged, next) = merge_waiting(bytes, || events.try_recv().ok());
                pending = next;
                hand_binary(&out, session, &merged);
            }
            Ok(ClientEvent::Title(title)) => {
                hand_text(&out, &HostFrame::Title { session, title });
            }
            // A folder whose name is not UTF-8 cannot go in a text frame.
            Ok(ClientEvent::Cwd(path)) => {
                if let Some(cwd) = path.to_str() {
                    let cwd = cwd.to_string();
                    hand_text(&out, &HostFrame::Cwd { session, cwd });
                }
                // Let go first: `attached` takes this lock on the runtime, and the tree looks
                // at folders.
                drop(wanted);
                registry.saw_cwd(&served, &path);
            }
            Ok(ClientEvent::Exited { clean, .. }) => {
                *wanted = false;
                hand_text(&out, &HostFrame::Exited { session, clean });
                drop(wanted);
                registry.saw_exit(&served, clean);
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

/// Adds the output already waiting behind `first` to it, so a burst costs the client one frame
/// per `MERGED_OUTPUT_BYTES` rather than one per terminal read. `next` must only take what is
/// already there, never wait, so a lone chunk such as a key's echo goes out at once and as it
/// is. Returns the first waiting event that could not join, which must be handled next.
/// Acks count bytes, not frames, so merging leaves them as they were.
fn merge_waiting(
    first: Bytes,
    mut next: impl FnMut() -> Option<ClientEvent>,
) -> (Bytes, Option<ClientEvent>) {
    let mut merged: Option<BytesMut> = None;
    let mut len = first.len();
    loop {
        match next() {
            Some(ClientEvent::Output(chunk)) if len + chunk.len() <= MERGED_OUTPUT_BYTES => {
                len += chunk.len();
                merged
                    .get_or_insert_with(|| {
                        let mut merged = BytesMut::with_capacity(MERGED_OUTPUT_BYTES);
                        merged.extend_from_slice(&first);
                        merged
                    })
                    .extend_from_slice(&chunk);
            }
            other => return (merged.map_or(first, BytesMut::freeze), other),
        }
    }
}

/// Everything the host serves, shared by all connections. The locks only cover quick
/// lookups, so they are safe to take on the async runtime.
///
/// Lock order: `watchers` before `tree`, and only to copy the tree. `tree` is otherwise held
/// alone, and only for work in memory: folders are resolved before it is taken, and it is
/// never held while calling into a session, sending to a client or saving.
struct Registry {
    last_session: AtomicU32,
    sessions: Mutex<HashMap<u32, Arc<Served>>>,
    tree: Mutex<Tree>,
    /// The connections that asked for the tree, each sent the whole tree after every change.
    watchers: Mutex<HashMap<u64, Out>>,
    next_watcher: AtomicU64,
    data_dir: PathBuf,
    /// What is in `state.json` now. Held while saving, so saves never cross.
    saved: Mutex<State>,
    /// Set before the last save on the way out. Nothing is saved after it, so the sessions a
    /// shutdown or reboot ends stay saved.
    shutting_down: AtomicBool,
    /// Each session's Claude state, by uuid.
    states: Mutex<Tracker>,
    /// Each running program's `TA_TOKEN`, by uuid. Only in memory.
    tokens: Tokens,
}

impl Registry {
    fn new(data_dir: PathBuf) -> Registry {
        Registry {
            last_session: AtomicU32::new(0),
            sessions: Mutex::new(HashMap::new()),
            tree: Mutex::new(Tree::default()),
            watchers: Mutex::new(HashMap::new()),
            next_watcher: AtomicU64::new(0),
            data_dir,
            saved: Mutex::new(State::default()),
            shutting_down: AtomicBool::new(false),
            states: Mutex::new(Tracker::default()),
            tokens: Tokens::default(),
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
        // A pane closed by hand goes like one that exited cleanly.
        if lock(&self.tree).on_exit(&served.uuid, true) {
            self.push_tree();
        }
        self.forget(&served);
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
    /// The tree comes back too, less panes whose session did not, and a restored session in
    /// no saved tab gets a tab of its own.
    fn restore(&self) {
        let loaded = state::load(&self.data_dir);
        let restored: Vec<&SavedSession> = loaded
            .sessions
            .iter()
            .filter(|saved| self.bring_back(saved))
            .collect();
        let kept: HashSet<&str> = restored.iter().map(|saved| saved.uuid.as_str()).collect();
        let mut tree = loaded.tree.clone();
        tree.retain_sessions(|uuid| kept.contains(uuid));
        for saved in restored {
            tree.add_session(
                &saved.uuid,
                Place::NewTab,
                &saved.cwd,
                saved.kind == "shell",
            );
        }
        *lock(&self.tree) = tree;
        *lock(&self.saved) = loaded;
    }

    /// Lists a saved session again. A shell starts now, a Claude session at its first attach.
    /// Returns false for one that does not make sense. Blocks.
    fn bring_back(&self, saved: &SavedSession) -> bool {
        let Some(served) = Served::restored(saved) else {
            eprintln!("term-host: skipped a saved session that does not make sense");
            return false;
        };
        let shell = served.chat.is_none();
        let session = self.add(served);
        if shell
            && let Some(served) = self.get(session)
            && let Err(HostFrame::Error { message, .. }) =
                served.start(RESTORE_SIZE, &self.data_dir, &self.tokens)
        {
            // Stays listed, and its first attach tries again.
            eprintln!("term-host: could not restore a shell: {message}");
        }
        true
    }

    /// The live session with this uuid. One being killed does not count, since a reopened tab
    /// brings its uuid back while the old program may still be on its way out.
    fn find(&self, uuid: &str) -> Option<(u32, Arc<Served>)> {
        lock(&self.sessions)
            .iter()
            .filter(|(_, served)| served.uuid == uuid && !served.kill_asked.load(Ordering::Relaxed))
            .max_by_key(|(session, _)| **session)
            .map(|(&session, served)| (session, Arc::clone(served)))
    }

    /// Adds a connection to the watchers and sends it the tree. Done under the watchers lock,
    /// so a newer tree pushed meanwhile always lands after it.
    fn watch(&self, watcher: u64, out: &Out) {
        let mut watchers = lock(&self.watchers);
        watchers.insert(watcher, out.clone());
        let tree = lock(&self.tree).clone();
        hand_text(out, &HostFrame::Tree { tree });
        hand_text(out, &self.states_frame());
    }

    /// Sends every state to every watcher, the same way as `push_tree`.
    fn push_states(&self) {
        let watchers = lock(&self.watchers);
        if watchers.is_empty() {
            return;
        }
        let frame = self.states_frame();
        for out in watchers.values() {
            hand_text(out, &frame);
        }
    }

    /// Lists only sessions with a state, so one whose state cleared drops out.
    fn states_frame(&self) -> HostFrame {
        let now = std::time::Instant::now();
        let wall = SystemTime::now();
        let items = lock(&self.states)
            .all()
            .into_iter()
            .map(|(session, state, since)| {
                let began = wall
                    .checked_sub(now.saturating_duration_since(since))
                    .unwrap_or(wall);
                let since_ms = began
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_millis() as u64);
                StateItem {
                    session,
                    state: Some(state.name().to_string()),
                    since_ms,
                }
            })
            .collect();
        HostFrame::States { items }
    }

    /// One hook call from a Claude in some tab, taken in the order the calls came. Blocks.
    fn on_hook(&self, request: &CtlRequest) {
        let Some(event) = states::parse(&request.event, &request.payload) else {
            return;
        };
        // A shut down kills the shell before its `claude` returns, and the tab must still
        // come back as the chat.
        if request.event.eq_ignore_ascii_case(states::CLAUDE_EXIT)
            && self.shutting_down.load(Ordering::Relaxed)
        {
            return;
        }
        let session = request.session.as_str();
        // A call sent just before its session ended must not bring its state back.
        let Some((_, served)) = self.find(session) else {
            return;
        };
        let ended = event == HookEvent::End;
        let change = lock(&self.states).on_event(session, event, std::time::Instant::now());
        if change.changed {
            self.push_states();
        }
        if let Some(folder) = change
            .first_edit
            .as_deref()
            .and_then(Path::parent)
            .filter(|folder| !folder.as_os_str().is_empty())
        {
            // Looked at before the tree lock, as for a reported folder.
            let project = project::resolve(folder);
            if lock(&self.tree).on_first_edit_in(session, &project) {
                self.push_tree();
            }
        }
        if let Some(chat) = change.chat.and_then(|chat| Uuid::parse_str(&chat).ok()) {
            *lock(&served.typed_chat) = Some(chat);
        } else if ended {
            *lock(&served.typed_chat) = None;
        }
    }

    /// The user saw the session's tab.
    fn saw_seen(&self, session: &str) {
        if lock(&self.states).seen(session).changed {
            self.push_states();
        }
    }

    /// A session's program is ending: its token stops working and its state clears. The token
    /// stays if a reopened tab with the same uuid already has a new one.
    fn forget(&self, served: &Served) {
        if let Some(running) = served.running.get()
            && self.tokens.matches(&served.uuid, &running.token)
        {
            self.tokens.remove(&served.uuid);
        }
        let now = std::time::Instant::now();
        if lock(&self.states)
            .on_event(&served.uuid, HookEvent::End, now)
            .changed
        {
            self.push_states();
        }
    }

    /// Sends the tree to every watcher. Pushes never cross, so the last one sent is the newest.
    fn push_tree(&self) {
        let watchers = lock(&self.watchers);
        if watchers.is_empty() {
            return;
        }
        let tree = lock(&self.tree).clone();
        let frame = HostFrame::Tree { tree };
        for out in watchers.values() {
            hand_text(out, &frame);
        }
    }

    /// Carries out a `tree_op`, with the kills and spawns it asks for. Blocks.
    fn tree_op(self: &Arc<Self>, op: Op) -> Result<(), HostFrame> {
        let op = match op {
            Op::OpenFolder { path } => return self.open_folder(&path),
            Op::CloseTab { tab } => return self.close_tab(&tab),
            op => op,
        };
        // Resolved between two holds of the lock. If the closed list changes in between, the
        // tree falls back to the folder as is rather than looking at the disk itself.
        let folder = lock(&self.tree).folder_for(&op);
        let project = folder.map(|folder| project::resolve(&folder));
        // A reopened pane is in the tree before its session is listed, as in `create`.
        let effects = lock(&self.tree).apply_in(op, project).map_err(tree_error)?;
        for uuid in &effects.kill {
            if let Some((session, _)) = self.find(uuid) {
                let _ = self.kill(session);
            }
        }
        for saved in &effects.spawn {
            self.bring_back(saved);
        }
        self.push_tree();
        Ok(())
    }

    /// Reads what the tab's sessions need to come back, then closes it with that in one hold
    /// of the lock, so no `reopen_tab` can find its entry empty. Then ends the sessions.
    fn close_tab(self: &Arc<Self>, tab: &str) -> Result<(), HostFrame> {
        let sessions = lock(&self.tree)
            .tab_sessions(tab)
            .ok_or_else(|| tree_error(TreeError::NoTab))?;
        // Asks the sessions, so not under the tree lock. A pane split in meanwhile has no
        // entry here and does not come back.
        let panes: Vec<SavedSession> = sessions
            .iter()
            .filter_map(|uuid| self.find(uuid)?.1.saved())
            .collect();
        let effects = lock(&self.tree).close_tab(tab, panes).map_err(tree_error)?;
        for uuid in &effects.kill {
            if let Some((session, _)) = self.find(uuid) {
                let _ = self.kill(session);
            }
        }
        self.push_tree();
        Ok(())
    }

    /// Starts a shell in the folder, then makes its group and tab in one go, so no one sees
    /// the group empty. Blocks.
    fn open_folder(self: &Arc<Self>, path: &Path) -> Result<(), HostFrame> {
        let folder =
            resolve_dir(path).ok_or_else(|| HostFrame::error(code::BAD_CWD, "no such folder"))?;
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|_| start_failed())?;
        let id = uuid::Builder::from_random_bytes(bytes).into_uuid();
        let spec = SpawnSpec {
            kind: Kind::Shell,
            cwd: folder.clone(),
            uuid: id,
            data_dir: self.data_dir.clone(),
            known_cwd: false,
        };
        let project = project::resolve(&folder);
        // Its first attach gives it a real size.
        let running = start_program(spec, RESTORE_SIZE, &self.tokens)?;
        let uuid = id.to_string();

        // The group and its pane go in before the session is listed, as in `create`.
        let opened = {
            let mut tree = lock(&self.tree);
            let path = folder.clone();
            let opened = tree.apply_in(Op::OpenFolder { path }, Some(project.clone()));
            if opened.is_ok() {
                tree.add_session_in(&uuid, Place::NewTab, &project, true);
            }
            opened
        };
        if let Err(error) = opened {
            running.handle.kill();
            return Err(tree_error(error));
        }
        let served = Served::new(uuid, id, "shell", None, folder, None);
        let _ = served.running.set(running);
        self.add(served);
        self.push_tree();
        Ok(())
    }

    /// A session reported its folder. Only a new one reaches the tree, since every attachment
    /// passes on the same report.
    fn saw_cwd(&self, served: &Served, cwd: &Path) {
        if served.kill_asked.load(Ordering::Relaxed) {
            return;
        }
        {
            let mut seen = lock(&served.seen_cwd);
            if seen.as_deref() == Some(cwd) {
                return;
            }
            *seen = Some(cwd.to_path_buf());
        }
        let project = project::resolve(cwd);
        if lock(&self.tree).on_cwd_in(&served.uuid, &project) {
            self.push_tree();
        }
    }

    /// A session's program ended. A killed one is already out of the tree, and its uuid may
    /// belong to a reopened tab by now.
    fn saw_exit(&self, served: &Served, clean: bool) {
        if served.kill_asked.load(Ordering::Relaxed)
            || served.seen_exit.swap(true, Ordering::Relaxed)
        {
            return;
        }
        if lock(&self.tree).on_exit(&served.uuid, clean) {
            self.push_tree();
        }
        self.forget(served);
    }

    /// Passes on folders and exits of sessions no one is attached to, which have no pump.
    fn notice(&self) {
        let served: Vec<Arc<Served>> = lock(&self.sessions).values().cloned().collect();
        for served in served {
            let Some(running) = served.running.get() else {
                continue;
            };
            let status = running.handle.status();
            if let Some(cwd) = &status.cwd {
                self.saw_cwd(&served, cwd);
            }
            if let Some(clean) = status.exited {
                self.saw_exit(&served, clean);
            }
        }
    }

    fn start_saving(self: Arc<Self>) {
        let started = thread::Builder::new()
            .name("state-saver".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(SAVE_EVERY);
                    self.notice();
                    self.save();
                    // Picks up a change to the flags in the app's settings. Already reported
                    // at startup if it fails.
                    let _ = claudecfg::write_args(&self.data_dir);
                }
            });
        if let Err(error) = started {
            eprintln!("term-host: sessions will not be saved: {error}");
        }
    }

    /// The Claude settings file with the hooks, and the flags file. Blocks.
    fn write_claude_files(&self) {
        let written =
            std::env::current_exe().and_then(|exe| claudecfg::write(&self.data_dir, &exe));
        if let Err(error) = written {
            eprintln!("term-host: Claude tabs will show no state: {error}");
        }
        if let Err(error) = claudecfg::write_args(&self.data_dir) {
            eprintln!("term-host: claude will not get its flags: {error}");
        }
    }

    /// Listens for hook calls. They are handled on one thread of their own, in the order they
    /// came, since looking at a folder can stall. Needs the runtime.
    fn start_hooks(self: Arc<Self>) {
        let (calls, received) = unbounded::<CtlRequest>();
        let listening = ctl::serve(&self.data_dir, self.tokens.clone(), move |request| {
            let _ = calls.send(request);
        });
        if let Err(error) = listening {
            eprintln!("term-host: Claude tabs will show no state: {error}");
            return;
        }
        let started = thread::Builder::new()
            .name("hook-calls".to_string())
            .spawn(move || {
                for request in received {
                    self.on_hook(&request);
                }
            });
        if let Err(error) = started {
            eprintln!("term-host: Claude tabs will show no state: {error}");
        }
    }

    /// Turns a finished turn into done once it has waited long enough.
    fn start_ticking(self: Arc<Self>) {
        let started = thread::Builder::new()
            .name("state-ticker".to_string())
            .spawn(move || {
                loop {
                    thread::sleep(STATE_TICK);
                    let done = lock(&self.states).tick(std::time::Instant::now());
                    if !done.is_empty() {
                        self.push_states();
                    }
                }
            });
        if let Err(error) = started {
            eprintln!("term-host: Claude tabs will not show done: {error}");
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
            tree: lock(&self.tree).clone(),
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
    /// The last folder passed to the tree.
    seen_cwd: Mutex<Option<PathBuf>>,
    /// Set once the exit has been passed to the tree.
    seen_exit: AtomicBool,
    /// The chat of a `claude` typed in this tab, from its first hook call, until it ends.
    typed_chat: Mutex<Option<Uuid>>,
}

struct Running {
    handle: Arc<SessionHandle>,
    input: InputQueue,
    /// The program's `TA_TOKEN`.
    token: String,
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
            seen_cwd: Mutex::new(None),
            seen_exit: AtomicBool::new(false),
            typed_chat: Mutex::new(None),
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
    fn start(&self, size: Size, data_dir: &Path, tokens: &Tokens) -> Result<(), HostFrame> {
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
            known_cwd: kind == Kind::Shell,
            kind,
            cwd,
            uuid: self.id,
            data_dir: data_dir.to_path_buf(),
        };
        let _ = self.running.set(start_program(spec, size, tokens)?);
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
        // A `claude` typed in a shell tab comes back as a Claude tab resuming its chat.
        let chat = lock(&self.typed_chat).or(self.chat);
        let kind = if chat.is_some() { "claude" } else { self.kind };
        Some(SavedSession {
            uuid: self.uuid.clone(),
            kind: kind.to_string(),
            cwd: lock(&self.cwd).clone(),
            title: self.title(status.as_ref()),
            chat,
        })
    }
}

/// Starts the program and its input thread, with a new `TA_TOKEN` that only this program
/// gets, so a hook call proves which tab it came from, and `TA_HOST`, this program's path,
/// which the shell hook's `claude` runs to say a typed `claude` returned. Blocks.
fn start_program(spec: SpawnSpec, size: Size, tokens: &Tokens) -> Result<Running, HostFrame> {
    let spawn_failed = |error: String| HostFrame::error(code::SPAWN_FAILED, &error);
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| start_failed())?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let uuid = spec.uuid.to_string();
    let host_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.into_os_string().into_string().ok());
    let env = host_env()
        .filter(|(key, _)| {
            !key.eq_ignore_ascii_case("TA_TOKEN") && !key.eq_ignore_ascii_case("TA_HOST")
        })
        .chain([("TA_TOKEN".to_string(), token.clone())])
        .chain(host_exe.map(|exe| ("TA_HOST".to_string(), exe)));
    // In place before the program starts, so its first hook call cannot beat it.
    tokens.insert(&uuid, &token);
    let started = Session::spawn_with_env(spec, size, env)
        .map_err(|e| spawn_failed(format!("{e:#}")))
        .and_then(|handle| {
            let handle = Arc::new(handle);
            let input =
                InputQueue::start(Arc::clone(&handle)).map_err(|e| spawn_failed(e.to_string()))?;
            Ok((handle, input))
        });
    match started {
        Ok((handle, input)) => Ok(Running {
            handle,
            input,
            token,
        }),
        Err(error) => {
            if tokens.matches(&uuid, &token) {
                tokens.remove(&uuid);
            }
            Err(error)
        }
    }
}

fn tree_error(error: TreeError) -> HostFrame {
    HostFrame::error(error.code(), &error.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn output(text: &str) -> ClientEvent {
        ClientEvent::Output(Bytes::copy_from_slice(text.as_bytes()))
    }

    /// What a client is handed for `queued`, everything already waiting, the way
    /// `pump_events` takes it.
    fn pump(queued: Vec<ClientEvent>) -> Vec<ClientEvent> {
        let mut queue = VecDeque::from(queued);
        let mut handed = Vec::new();
        let mut pending = None;
        while let Some(event) = pending.take().or_else(|| queue.pop_front()) {
            match event {
                ClientEvent::Output(bytes) => {
                    let (merged, next) = merge_waiting(bytes, || queue.pop_front());
                    pending = next;
                    handed.push(ClientEvent::Output(merged));
                }
                other => handed.push(other),
            }
        }
        handed
    }

    fn payloads(handed: &[ClientEvent]) -> Vec<Vec<u8>> {
        handed
            .iter()
            .map(|event| match event {
                ClientEvent::Output(bytes) => bytes.to_vec(),
                other => panic!("expected output, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn lone_chunk_goes_out_as_it_is() {
        let chunk = Bytes::from_static(b"a");
        let mut asked = 0;
        let (merged, next) = merge_waiting(chunk.clone(), || {
            asked += 1;
            None
        });
        assert_eq!(merged, chunk);
        assert_eq!(
            merged.as_ptr(),
            chunk.as_ptr(),
            "a lone chunk is not copied"
        );
        assert_eq!(next, None);
        assert_eq!(asked, 1, "only one look at the queue, never a wait");
    }

    #[test]
    fn queued_chunks_go_out_as_one_frame_in_order() {
        let handed = pump(vec![output("one "), output("two "), output("three")]);
        assert_eq!(payloads(&handed), vec![b"one two three".to_vec()]);
    }

    #[test]
    fn merged_frame_stays_within_the_cap() {
        // A macOS terminal read is about 1013 bytes.
        let input: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let queued = input
            .chunks(1013)
            .map(|chunk| ClientEvent::Output(Bytes::copy_from_slice(chunk)))
            .collect::<Vec<_>>();
        let chunks = queued.len();
        let frames = payloads(&pump(queued));
        assert!(
            frames
                .iter()
                .all(|frame| frame.len() <= MERGED_OUTPUT_BYTES),
            "a frame went over {MERGED_OUTPUT_BYTES} bytes"
        );
        assert_eq!(frames.concat(), input);
        assert!(
            frames.len() < chunks / 10,
            "{} frames for {chunks} chunks",
            frames.len()
        );
    }

    #[test]
    fn chunk_that_would_go_over_waits_for_the_next_frame() {
        let full = "x".repeat(MERGED_OUTPUT_BYTES - 1);
        let handed = pump(vec![output(&full), output("ab"), output("c")]);
        assert_eq!(payloads(&handed), vec![full.into_bytes(), b"abc".to_vec()]);
    }

    #[test]
    fn other_events_keep_their_place() {
        let title = ClientEvent::Title("t".to_string());
        let cwd = ClientEvent::Cwd(PathBuf::from("/tmp"));
        let exited = ClientEvent::Exited {
            clean: true,
            signal: None,
        };
        let handed = pump(vec![
            output("a"),
            output("b"),
            title.clone(),
            output("c"),
            cwd.clone(),
            cwd.clone(),
            output("d"),
            output("e"),
            exited.clone(),
        ]);
        assert_eq!(
            handed,
            vec![
                output("ab"),
                title,
                output("c"),
                cwd.clone(),
                cwd,
                output("de"),
                exited,
            ]
        );
    }

    #[test]
    fn snapshot_is_never_merged() {
        let snapshot = ClientEvent::Snapshot(Bytes::from_static(b"screen"));
        let handed = pump(vec![snapshot.clone(), output("a"), output("b")]);
        assert_eq!(handed, vec![snapshot, output("ab")]);
    }
}
