use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use bytes::Bytes;
use crossbeam_channel::{Receiver, Sender, after, at, bounded, never, select, unbounded};
use portable_pty::{ChildKiller, MasterPty};
use uuid::Uuid;

use crate::env::{clean_env, host_env};
use crate::osc::{self, OscEvent, Scanner};
use crate::pty::{self, Exit, SharedWriter, Size};
use crate::screen::Screen;

const READ_BUFFER: usize = 64 * 1024;
// Small so a slow actor soon stops the reader, and the terminal then throttles the
// program.
const OUTPUT_CHANNEL_CHUNKS: usize = 16;
const KILL_GRACE: Duration = Duration::from_secs(2);
// A background program can hold the terminal open after the main program ends, so
// the end of output is not waited for forever.
const DRAIN_GRACE: Duration = Duration::from_millis(500);
// Matches Alex's kitty.
const SCROLLBACK: usize = 20_000;
// How far a client can fall behind before the program is made to wait for it.
const FLOOD_LIMIT: u64 = 4 * 1024 * 1024;
// A client over the limit this long without acking is holding everyone back for nothing.
const STALE_LIMIT: Duration = Duration::from_secs(5);
// How long after a resize to check that it held. See `Actor::check_size`.
const SIZE_SETTLE: Duration = Duration::from_millis(50);
const SIZE_RETRIES: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Shell,
    Claude { chat: Uuid, resume: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnSpec {
    pub kind: Kind,
    pub cwd: PathBuf,
    pub uuid: Uuid,
    /// Holds the shell hook. The program gets it as `TA_DATA` and the uuid as `TA_SESSION`.
    pub data_dir: PathBuf,
    /// Shows `cwd` as the folder from the start, as for a restored shell, whose hook reports
    /// only at its first prompt.
    pub known_cwd: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClientId(u64);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientEvent {
    /// Rebuilds the screen. Always the first event a client gets, once per attach.
    Snapshot(Bytes),
    Output(Bytes),
    /// The window title changed. Also sent right after the snapshot when there is one.
    Title(String),
    /// The shell hook reported a folder other than the last one. Also sent right after the
    /// snapshot when there is one.
    Cwd(PathBuf),
    /// `clean` means a normal process exit, whatever the exit code, except that on Windows an
    /// error status code (a crash or Ctrl+C) is not clean.
    Exited {
        clean: bool,
        signal: Option<i32>,
    },
}

pub type ClientTx = Sender<ClientEvent>;

/// What a session looks like from outside, readable without waiting for its actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub title: Option<String>,
    /// The folder the shell hook last reported, if it has.
    pub cwd: Option<PathBuf>,
    /// None until the exit has been announced, then whether it was clean.
    pub exited: Option<bool>,
    pub size: Size,
}

pub struct Session;

impl Session {
    pub fn spawn(spec: SpawnSpec, size: Size) -> Result<SessionHandle> {
        Self::spawn_with_env(spec, size, host_env())
    }

    /// Like `spawn`, but takes the environment instead of reading the host's.
    pub fn spawn_with_env(
        spec: SpawnSpec,
        size: Size,
        env: impl Iterator<Item = (String, String)>,
    ) -> Result<SessionHandle> {
        let cwd = if spec.known_cwd {
            resolve_dir(&spec.cwd)
        } else {
            None
        };
        let pty::Pty {
            master,
            reader,
            writer,
            mut child,
            killer,
        } = pty::spawn(&spec, size, clean_env(env))?;
        let pid = child.process_id();

        let (output_tx, output_rx) = bounded(OUTPUT_CHANNEL_CHUNKS);
        let (exit_tx, exit_rx) = bounded(1);
        let (control_tx, control_rx) = unbounded();
        let (announced_tx, announced_rx) = bounded(0);
        let (checked_tx, checked_rx) = unbounded();
        let status = Arc::new(Mutex::new(Status {
            title: None,
            cwd: cwd.clone(),
            exited: None,
            size,
        }));

        let filter = pty::output_filter(&writer);
        thread::Builder::new()
            .name("session-reader".to_string())
            .spawn(move || read_output(reader, output_tx, filter))?;
        // Separate from the reader because ConPTY's output only ends once the actor
        // closes it, which it does on hearing the program has exited.
        thread::Builder::new()
            .name("session-wait".to_string())
            .spawn(move || {
                let _ = exit_tx.send(pty::wait_for_exit(child.as_mut()));
            })?;

        let actor = Actor {
            master: Some(master),
            killer,
            pid,
            size,
            screen: Screen::new(size, SCROLLBACK),
            scanner: Scanner::new(),
            cwd,
            checking: false,
            pending: None,
            checked_tx,
            checked_rx,
            clients: Vec::new(),
            kill_sent: false,
            kill_deadline: never(),
            size_check: never(),
            size_retries: 0,
            exit: None,
            announced: None,
            announced_signal: Some(announced_tx),
            status: Arc::clone(&status),
        };
        thread::Builder::new()
            .name("session-actor".to_string())
            .spawn(move || actor.run(output_rx, control_rx, exit_rx))?;

        Ok(SessionHandle {
            control: control_tx,
            writer,
            pid,
            next_client: AtomicU64::new(0),
            status,
            announced: announced_rx,
        })
    }
}

/// Dropping the handle ends the program.
pub struct SessionHandle {
    control: Sender<Control>,
    writer: SharedWriter,
    pid: Option<u32>,
    next_client: AtomicU64,
    status: Arc<Mutex<Status>>,
    announced: Receiver<()>,
}

impl SessionHandle {
    // Skips the actor: a program blocked on output can stop reading input, so an actor
    // blocked writing that input would never drain the output and both would hang.
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = self.writer.lock().unwrap_or_else(|err| err.into_inner());
        writer.write_all(bytes)?;
        writer.flush()
    }

    pub fn resize(&self, size: Size) {
        let _ = self.control.send(Control::Resize(size));
    }

    /// Asks the program to end with SIGHUP, then SIGKILL if it is still running after
    /// a grace period. On Windows this terminates it right away.
    pub fn kill(&self) {
        let _ = self.control.send(Control::Kill);
    }

    /// Returns once the client is subscribed. Its first event is a snapshot of the screen
    /// at that point, then it gets all output after it, so nothing is lost or repeated.
    pub fn attach(&self, tx: ClientTx, size: Size) -> ClientId {
        let id = ClientId(self.next_client.fetch_add(1, Ordering::Relaxed));
        let (done_tx, done_rx) = bounded(1);
        let attach = Control::Attach {
            id,
            tx,
            size,
            done: done_tx,
        };
        if self.control.send(attach).is_ok() {
            let _ = done_rx.recv();
        }
        id
    }

    pub fn detach(&self, id: ClientId) {
        let _ = self.control.send(Control::Detach(id));
    }

    /// `bytes` is the running total the client has taken in since attaching, snapshot
    /// included. Any ack restarts the client's stale clock, even one that leaves it over
    /// the limit.
    pub fn ack(&self, id: ClientId, bytes: u64) {
        let _ = self.control.send(Control::Ack(id, bytes));
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    /// Nothing is ever sent on this. It disconnects once the exit has been announced.
    pub fn announced(&self) -> Receiver<()> {
        self.announced.clone()
    }
}

enum Control {
    Attach {
        id: ClientId,
        tx: ClientTx,
        size: Size,
        done: Sender<()>,
    },
    Detach(ClientId),
    Resize(Size),
    Ack(ClientId, u64),
    Kill,
}

struct Client {
    id: ClientId,
    tx: ClientTx,
    sent: u64,
    acked: u64,
    // When this client's stale clock started. Set only while it is over the limit.
    over_since: Option<Instant>,
    // Stopped getting output during a kill. Stays set, because bytes after a gap would
    // draw garbage. The exit event still reaches it.
    cut_off: bool,
}

impl Client {
    fn new(id: ClientId, tx: ClientTx) -> Self {
        Self {
            id,
            tx,
            sent: 0,
            acked: 0,
            over_since: None,
            cut_off: false,
        }
    }

    fn over_limit(&self) -> bool {
        self.sent.saturating_sub(self.acked) > FLOOD_LIMIT
    }

    fn count_sent(&mut self, bytes: usize) {
        self.sent += bytes as u64;
        if self.over_since.is_none() && self.over_limit() {
            self.over_since = Some(Instant::now());
        }
    }

    fn count_acked(&mut self, bytes: u64) {
        self.acked = bytes;
        self.over_since = self.over_limit().then(Instant::now);
    }
}

fn read_output(
    mut reader: Box<dyn Read + Send>,
    output: Sender<Bytes>,
    mut filter: impl FnMut(Bytes) -> Bytes,
) {
    let mut buffer = vec![0; READ_BUFFER];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return,
            Ok(count) => {
                let chunk = filter(Bytes::copy_from_slice(&buffer[..count]));
                if !chunk.is_empty() && output.send(chunk).is_err() {
                    return;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            // Linux reports EIO, not end of file, once the program side has closed.
            Err(_) => return,
        }
    }
}

struct Actor {
    // None once the program has exited.
    master: Option<Box<dyn MasterPty + Send>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pid: Option<u32>,
    size: Size,
    screen: Screen,
    scanner: Scanner,
    cwd: Option<PathBuf>,
    // One folder check runs at a time. A report arriving meanwhile waits, replacing any
    // older one waiting, so a flood costs one check at a time and the last report wins.
    checking: bool,
    pending: Option<PathBuf>,
    checked_tx: Sender<Option<PathBuf>>,
    checked_rx: Receiver<Option<PathBuf>>,
    clients: Vec<Client>,
    kill_sent: bool,
    kill_deadline: Receiver<Instant>,
    size_check: Receiver<Instant>,
    size_retries: u8,
    // Set when the program ends. `announced` follows once the output has drained.
    exit: Option<Exit>,
    announced: Option<Exit>,
    // Dropped when `announced` is set, which is what `SessionHandle::announced` waits for.
    announced_signal: Option<Sender<()>>,
    status: Arc<Mutex<Status>>,
}

impl Actor {
    fn run(
        mut self,
        mut output: Receiver<Bytes>,
        mut control: Receiver<Control>,
        mut exit: Receiver<Exit>,
    ) {
        // Listened to in place of the output while holding back. Nothing gets read, so the
        // channel and then the terminal fill up and the program blocks on its next write.
        let held_output = never();
        let mut drain_deadline = never();
        let mut drain_paused = false;
        let mut output_done = false;
        let mut handle_gone = false;

        while !(handle_gone && self.announced.is_some()) {
            let next_output = if self.holding_back() {
                &held_output
            } else {
                &output
            };
            let stale_deadline = self.stale_deadline();
            select! {
                recv(next_output) -> chunk => match chunk {
                    Ok(chunk) => self.forward(chunk),
                    Err(_) => {
                        output = never();
                        output_done = true;
                    }
                },
                recv(control) -> message => match message {
                    Ok(message) => self.handle(message),
                    Err(_) => {
                        control = never();
                        handle_gone = true;
                        self.kill();
                    }
                },
                recv(exit) -> status => {
                    exit = never();
                    self.program_exited(status.unwrap_or(Exit { clean: false, signal: None }));
                    drain_deadline = after(DRAIN_GRACE);
                },
                recv(self.kill_deadline) -> _ => {
                    self.kill_deadline = never();
                    self.force_kill();
                },
                recv(self.checked_rx) -> resolved => self.checked(resolved.unwrap_or(None)),
                recv(stale_deadline) -> _ => self.drop_stale_clients(),
                recv(self.size_check) -> _ => self.check_size(),
                recv(drain_deadline) -> _ => {
                    drain_deadline = never();
                    // Lets the reader stop at its next read instead of feeding nobody.
                    output = never();
                    output_done = true;
                },
            }
            // The grace is for a background program holding the terminal open, not for
            // output the actor itself is sitting on, so a program's last words wait for a
            // slow client.
            if self.exit.is_some() && !output_done {
                if self.holding_back() {
                    drain_deadline = never();
                    drain_paused = true;
                } else if drain_paused {
                    drain_deadline = after(DRAIN_GRACE);
                    drain_paused = false;
                }
            }
            if output_done && self.exit.is_some() && self.announced.is_none() {
                self.announce();
            }
        }
    }

    fn forward(&mut self, chunk: Bytes) {
        // Never blocks: a client that can't keep up is dropped so it can't delay the
        // others. It can attach again.
        self.clients.retain_mut(|client| {
            // Only possible during a kill, since otherwise output is held back first. A
            // program ignoring SIGHUP can flood until SIGKILL, and this keeps that out of
            // the client's queue.
            client.cut_off |= client.over_limit();
            if client.cut_off {
                return true;
            }
            let sent = client.tx.try_send(ClientEvent::Output(chunk.clone()));
            client.count_sent(chunk.len());
            sent.is_ok()
        });
        // After the clients so parsing never delays the bytes they see.
        for event in self.scanner.feed(&chunk) {
            self.report(event);
        }
        self.screen.feed(&chunk);
        if let Some(title) = self.screen.take_new_title() {
            self.update_status(|status| status.title = Some(title.clone()));
            for client in &self.clients {
                let _ = client.tx.try_send(ClientEvent::Title(title.clone()));
            }
        }
    }

    /// A folder counts only if it is on this computer and exists, since a shell over SSH
    /// reports the other computer's folders.
    fn report(&mut self, event: OscEvent) {
        let OscEvent::Cwd { host, path } = event;
        if !osc::is_this_host(&host) {
            return;
        }
        if self.checking {
            self.pending = Some(path);
        } else {
            self.check(path);
        }
    }

    // On its own thread because looking at a folder can block, as on a stalled network
    // drive, and that must not hold up the session's output.
    fn check(&mut self, path: PathBuf) {
        let done = self.checked_tx.clone();
        let started = thread::Builder::new()
            .name("session-cwd".to_string())
            .spawn(move || {
                let _ = done.send(resolve_dir(&path));
            });
        self.checking = started.is_ok();
    }

    /// Compares the resolved folder, so a link and its target count as one folder.
    fn checked(&mut self, resolved: Option<PathBuf>) {
        self.checking = false;
        // A newer report makes this result stale.
        if let Some(path) = self.pending.take() {
            self.check(path);
            return;
        }
        let Some(path) = resolved else {
            return;
        };
        if self.cwd.as_ref() == Some(&path) {
            return;
        }
        self.cwd = Some(path.clone());
        self.update_status(|status| status.cwd = Some(path.clone()));
        for client in &self.clients {
            let _ = client.tx.try_send(ClientEvent::Cwd(path.clone()));
        }
    }

    fn update_status(&self, change: impl FnOnce(&mut Status)) {
        change(&mut self.status.lock().unwrap_or_else(|err| err.into_inner()));
    }

    // A kill ends the hold because macOS won't let a program finish exiting while its
    // terminal has unread output, so a held back program could never die.
    fn holding_back(&self) -> bool {
        !self.kill_sent
            && self
                .clients
                .iter()
                .any(|client| client.over_since.is_some())
    }

    // Only makes a timer while holding back, and no output is read then, so a flood the
    // clients keep up with costs no timers. During a kill nobody is being delayed, so a
    // client over the limit stays to hear about the exit.
    fn stale_deadline(&self) -> Receiver<Instant> {
        if !self.holding_back() {
            return never();
        }
        self.clients
            .iter()
            .filter_map(|client| client.over_since)
            .min()
            .map_or_else(never, |since| at(since + STALE_LIMIT))
    }

    fn drop_stale_clients(&mut self) {
        let now = Instant::now();
        self.clients.retain(|client| {
            client
                .over_since
                .is_none_or(|since| now < since + STALE_LIMIT)
        });
    }

    fn handle(&mut self, message: Control) {
        match message {
            Control::Attach { id, tx, size, done } => {
                self.resize(size);
                let snapshot = Bytes::from(self.screen.snapshot());
                let bytes = snapshot.len();
                // A client with no room for the snapshot gets nothing more, since nothing
                // else would make sense without it. It can attach again.
                if tx.try_send(ClientEvent::Snapshot(snapshot)).is_ok() {
                    if let Some(title) = self.screen.title() {
                        let _ = tx.try_send(ClientEvent::Title(title.to_string()));
                    }
                    if let Some(cwd) = &self.cwd {
                        let _ = tx.try_send(ClientEvent::Cwd(cwd.clone()));
                    }
                    match self.announced {
                        Some(exit) => {
                            let _ = tx.try_send(exit.into());
                        }
                        None => {
                            let mut client = Client::new(id, tx);
                            client.count_sent(bytes);
                            self.clients.push(client);
                        }
                    }
                }
                let _ = done.send(());
            }
            Control::Detach(id) => self.clients.retain(|client| client.id != id),
            Control::Resize(size) => self.resize(size),
            Control::Ack(id, bytes) => {
                if let Some(client) = self.clients.iter_mut().find(|client| client.id == id) {
                    client.count_acked(bytes);
                }
            }
            Control::Kill => self.kill(),
        }
    }

    fn resize(&mut self, size: Size) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.screen.resize(size);
        self.size_retries = SIZE_RETRIES;
        self.apply_size();
    }

    fn apply_size(&mut self) {
        if let Some(master) = &self.master {
            let _ = master.resize(self.size.into());
        }
        self.size_check = after(SIZE_SETTLE);
    }

    // A resize can be undone with no error: readline reads the size and writes it back each
    // time it returns to its prompt, so a resize landing between the two is lost. Nothing
    // reports that, so look again shortly after. `list` shows a size only once it has held,
    // and the retries stop a program that keeps setting its own size from fighting forever.
    fn check_size(&mut self) {
        self.size_check = never();
        let size = self.size;
        let undone = self
            .master
            .as_ref()
            .and_then(|master| master.get_size().ok())
            .is_some_and(|now| now.cols != size.cols || now.rows != size.rows);
        if undone && self.size_retries > 0 {
            self.size_retries -= 1;
            self.apply_size();
        } else {
            self.update_status(|status| status.size = size);
        }
    }

    fn kill(&mut self) {
        if self.kill_sent {
            return;
        }
        // Noted even when the program has already gone, because it ends the hold, and with
        // it the drain's wait for a slow client. Nobody is waiting for the last words now.
        self.kill_sent = true;
        if self.exit.is_some() {
            return;
        }
        let _ = self.killer.kill();
        self.kill_deadline = after(KILL_GRACE);
    }

    fn force_kill(&self) {
        #[cfg(unix)]
        if let (None, Some(pid)) = (self.exit, self.pid) {
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
    }

    fn program_exited(&mut self, exit: Exit) {
        // A program that exits normally because we told it to end has not finished on its
        // own: a shell can answer SIGHUP with an exit code, and Windows has no signals.
        let clean = exit.clean && !self.kill_sent;
        self.exit = Some(Exit { clean, ..exit });
        // On its own thread because closing ConPTY can block until its output has been
        // read, and this thread is what reads it.
        if let Some(master) = self.master.take() {
            thread::spawn(move || drop(master));
        }
    }

    fn announce(&mut self) {
        self.announced = self.exit;
        if let Some(exit) = self.announced {
            self.update_status(|status| status.exited = Some(exit.clean));
            for client in self.clients.drain(..) {
                let _ = client.tx.try_send(exit.into());
            }
        }
        self.announced_signal = None;
    }
}

impl From<Exit> for ClientEvent {
    fn from(exit: Exit) -> Self {
        ClientEvent::Exited {
            clean: exit.clean,
            signal: exit.signal,
        }
    }
}

/// The folder with every link resolved, or None if it is not a folder. Blocks.
fn resolve_dir(path: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    if !real.is_dir() {
        return None;
    }
    Some(without_verbatim(real))
}

/// Drops the `\\?\` that `canonicalize` puts in front of a Windows path, so the client sees
/// `C:\Users\x`.
fn without_verbatim(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let Some(text) = path.to_str() else {
        return path;
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => path,
    }
}
