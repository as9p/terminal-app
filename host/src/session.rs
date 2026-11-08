use std::io::{self, Read, Write};
use std::path::PathBuf;
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
    /// `clean` means a normal process exit, whatever the exit code.
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
        let status = Arc::new(Mutex::new(Status {
            title: None,
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
            clients: Vec::new(),
            kill_sent: false,
            kill_deadline: never(),
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
    clients: Vec<Client>,
    kill_sent: bool,
    kill_deadline: Receiver<Instant>,
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
                recv(stale_deadline) -> _ => self.drop_stale_clients(),
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
        self.screen.feed(&chunk);
        if let Some(title) = self.screen.take_new_title() {
            self.update_status(|status| status.title = Some(title.clone()));
            for client in &self.clients {
                let _ = client.tx.try_send(ClientEvent::Title(title.clone()));
            }
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
        self.update_status(|status| status.size = size);
        self.screen.resize(size);
        if let Some(master) = &self.master {
            let _ = master.resize(size.into());
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
