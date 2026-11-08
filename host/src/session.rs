use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use bytes::Bytes;
use crossbeam_channel::{Receiver, Sender, after, bounded, never, select, unbounded};
use portable_pty::{ChildKiller, MasterPty};
use uuid::Uuid;

use crate::env::{clean_env, host_env};
use crate::pty::{self, Exit, SharedWriter, Size};

const READ_BUFFER: usize = 64 * 1024;
// Small so a slow actor soon stops the reader, and the terminal then throttles the
// program.
const OUTPUT_CHANNEL_CHUNKS: usize = 16;
const KILL_GRACE: Duration = Duration::from_secs(2);
// A background program can hold the terminal open after the main program ends, so
// the end of output is not waited for forever.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

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
    Output(Bytes),
    /// `clean` means a normal process exit, whatever the exit code.
    Exited {
        clean: bool,
        signal: Option<i32>,
    },
}

pub type ClientTx = Sender<ClientEvent>;

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
            clients: Vec::new(),
            kill_sent: false,
            kill_deadline: never(),
            exit: None,
            announced: None,
        };
        thread::Builder::new()
            .name("session-actor".to_string())
            .spawn(move || actor.run(output_rx, control_rx, exit_rx))?;

        Ok(SessionHandle {
            control: control_tx,
            writer,
            pid,
            next_client: AtomicU64::new(0),
        })
    }
}

/// Dropping the handle ends the program.
pub struct SessionHandle {
    control: Sender<Control>,
    writer: SharedWriter,
    pid: Option<u32>,
    next_client: AtomicU64,
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

    /// Returns once the client is subscribed, so it receives all output produced after
    /// this call and none from before it.
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

    pub fn ack(&self, id: ClientId, bytes: u64) {
        let _ = self.control.send(Control::Ack(id, bytes));
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
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
    // Task 4's throttling reads this.
    #[allow(dead_code)]
    acked: u64,
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
    clients: Vec<Client>,
    kill_sent: bool,
    kill_deadline: Receiver<Instant>,
    // Set when the program ends. `announced` follows once the output has drained.
    exit: Option<Exit>,
    announced: Option<Exit>,
}

impl Actor {
    fn run(
        mut self,
        mut output: Receiver<Bytes>,
        mut control: Receiver<Control>,
        mut exit: Receiver<Exit>,
    ) {
        let mut drain_deadline = never();
        let mut output_done = false;
        let mut handle_gone = false;

        while !(handle_gone && self.announced.is_some()) {
            select! {
                recv(output) -> chunk => match chunk {
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
                recv(drain_deadline) -> _ => {
                    drain_deadline = never();
                    // Lets the reader stop at its next read instead of feeding nobody.
                    output = never();
                    output_done = true;
                },
            }
            if output_done && self.exit.is_some() && self.announced.is_none() {
                self.announce();
            }
        }
    }

    fn forward(&mut self, chunk: Bytes) {
        // Never blocks: a client that can't keep up is dropped so it can't delay the
        // others. It can attach again.
        self.clients.retain(|client| {
            client
                .tx
                .try_send(ClientEvent::Output(chunk.clone()))
                .is_ok()
        });
        // Task 4 feeds the screen copy here, after the clients so parsing never delays
        // the bytes they see.
    }

    fn handle(&mut self, message: Control) {
        match message {
            Control::Attach { id, tx, size, done } => {
                self.resize(size);
                match self.announced {
                    Some(exit) => {
                        let _ = tx.try_send(exit.into());
                    }
                    None => self.clients.push(Client { id, tx, acked: 0 }),
                }
                let _ = done.send(());
            }
            Control::Detach(id) => self.clients.retain(|client| client.id != id),
            Control::Resize(size) => self.resize(size),
            Control::Ack(id, bytes) => {
                if let Some(client) = self.clients.iter_mut().find(|client| client.id == id) {
                    client.acked = bytes;
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
        if let Some(master) = &self.master {
            let _ = master.resize(size.into());
        }
    }

    fn kill(&mut self) {
        if self.kill_sent || self.exit.is_some() {
            return;
        }
        self.kill_sent = true;
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
            for client in self.clients.drain(..) {
                let _ = client.tx.try_send(exit.into());
            }
        }
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
