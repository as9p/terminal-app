//! One command on a pty: a reader thread, a writer thread and a thread that waits for the exit.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(not(unix))]
use portable_pty::ChildKiller;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use term_engine::{CellSize, GridSize};

const READ_CHUNK: usize = 64 * 1024;
/// How long the output must stay quiet after the command exited before `Exited` is sent anyway.
/// Only matters when a background child of the command still holds the pty open.
const EXIT_GRACE: Duration = Duration::from_millis(200);

pub enum ShellEvent {
    Output(Vec<u8>),
    Exited(u32),
}

type OnEvent = Box<dyn Fn(ShellEvent) + Send>;

/// The callback runs on the reader thread for output and on the waiter thread for the exit; the
/// lock keeps the two from overlapping and keeps every output ahead of the exit.
struct Delivery {
    on_event: OnEvent,
    exited: bool,
}

/// What the reader is doing, so the waiter can tell "all output is in" from "still reading".
#[derive(Default)]
struct Progress {
    /// The reader reached the end of the pty.
    done: bool,
    /// The reader holds bytes it has not delivered yet.
    busy: bool,
    reads: u64,
}

pub struct Shell {
    master: Box<dyn MasterPty + Send>,
    writes: Sender<Vec<u8>>,
    #[cfg(not(unix))]
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: Option<u32>,
    /// Set by the waiter before it reaps the command, under the lock every signal holds, so a
    /// signal can only reach the command's own group, never a process that reused its number.
    exited: Arc<Mutex<bool>>,
}

impl Shell {
    /// `command` None runs `$SHELL -l` (or `/bin/zsh -l`). `on_event` is called on the shell's
    /// threads, one call at a time; every `Output` comes before the single `Exited`.
    pub fn spawn(
        command: Option<Vec<String>>,
        cwd: Option<PathBuf>,
        grid: GridSize,
        on_event: OnEvent,
    ) -> io::Result<Shell> {
        let pair = native_pty_system()
            .openpty(pty_size(grid, None))
            .map_err(io::Error::other)?;
        let mut builder = match command {
            Some(argv) if argv.is_empty() => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty command"));
            }
            Some(argv) => CommandBuilder::from_argv(argv.into_iter().map(Into::into).collect()),
            None => default_command(),
        };
        builder.env("TERM", "xterm-256color");
        builder.env("COLORTERM", "truecolor");
        builder.env("TERM_PROGRAM", crate::NAME);
        builder.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        if let Some(cwd) = cwd {
            builder.cwd(cwd);
        }
        let mut child = pair
            .slave
            .spawn_command(builder)
            .map_err(io::Error::other)?;
        // Our copy of the child side would keep the pty open after the command exits, and the
        // reader would never see the end.
        drop(pair.slave);
        let master = pair.master;
        let reader = master.try_clone_reader().map_err(io::Error::other);
        let writer = master.take_writer().map_err(io::Error::other);
        let (reader, writer) = match (reader, writer) {
            (Ok(reader), Ok(writer)) => (reader, writer),
            (Err(e), _) | (_, Err(e)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };

        let pid = child.process_id();
        #[cfg(not(unix))]
        let killer = child.clone_killer();
        let exited = Arc::new(Mutex::new(false));
        let delivery = Arc::new(Mutex::new(Delivery {
            on_event,
            exited: false,
        }));
        let progress = Arc::new((Mutex::new(Progress::default()), Condvar::new()));
        let (writes, queue) = mpsc::channel::<Vec<u8>>();
        let progress_w = progress.clone();

        let spawned = (|| {
            let (delivery_r, progress_r) = (delivery.clone(), progress.clone());
            thread::Builder::new()
                .name("shell reader".into())
                .spawn(move || read_loop(reader, &delivery_r, &progress_r))?;
            thread::Builder::new()
                .name("shell writer".into())
                .spawn(move || write_loop(writer, queue))?;
            let exited_w = exited.clone();
            thread::Builder::new()
                .name("shell waiter".into())
                .spawn(move || {
                    wait_loop(child, pid, &delivery, &progress_w, &exited_w);
                })?;
            io::Result::Ok(())
        })();
        let shell = Shell {
            master,
            writes,
            #[cfg(not(unix))]
            killer: Mutex::new(killer),
            pid,
            exited,
        };
        // Dropping the half-built shell ends the command if a thread could not start.
        spawned.map(|()| shell)
    }

    /// Queued to the writer thread, so this never waits for the program to read. After the
    /// command exited the bytes are dropped.
    pub fn write(&self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            let _ = self.writes.send(bytes);
        }
    }

    /// Programs read the pixel size from the pty as well as the grid, so both are passed on.
    pub fn resize(&self, grid: GridSize, cell: CellSize) {
        let _ = self.master.resize(pty_size(grid, Some(cell)));
    }

    /// Ends the command for good: the whole process group on Unix, so a child of the command
    /// that ignores hang-ups goes too.
    pub fn kill(&self) {
        self.signal(Kill::Now);
    }

    /// Returns whether a signal was sent. After the command has exited nothing is sent: dropping
    /// the pty is what tells leftover jobs the terminal is gone.
    fn signal(&self, how: Kill) -> bool {
        let exited = lock(&self.exited);
        if *exited {
            return false;
        }
        #[cfg(unix)]
        {
            let Some(pid) = self.pid.and_then(|pid| i32::try_from(pid).ok()) else {
                return false;
            };
            use nix::sys::signal::{Signal, killpg};
            use nix::unistd::Pid;
            let signal = match how {
                Kill::Now => Signal::SIGKILL,
                Kill::HangUp => Signal::SIGHUP,
            };
            // The library starts the command in its own session, so its pid is its group. The
            // command is not reaped while `exited` is held false, so the number is still ours.
            killpg(Pid::from_raw(pid), signal).is_ok()
        }
        #[cfg(not(unix))]
        {
            // The killer holds a process handle, which cannot point at another process.
            let _ = how;
            lock(&self.killer).kill().is_ok()
        }
    }
}

#[derive(Clone, Copy)]
enum Kill {
    Now,
    /// As when a terminal closes, which lets a shell save its history first.
    HangUp,
}

impl Drop for Shell {
    /// A shell that outlives its owner would hold a pty and a process nobody can reach.
    fn drop(&mut self) {
        self.signal(Kill::HangUp);
    }
}

#[cfg(unix)]
fn default_command() -> CommandBuilder {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string());
    let mut builder = CommandBuilder::new(shell);
    builder.arg("-l");
    builder
}

#[cfg(not(unix))]
fn default_command() -> CommandBuilder {
    CommandBuilder::new("cmd.exe")
}

fn pty_size(grid: GridSize, cell: Option<CellSize>) -> PtySize {
    let cell = cell.unwrap_or(CellSize { w: 0, h: 0 });
    let pixels = |cells: u16, each: u32| u16::try_from(u32::from(cells) * each).unwrap_or(u16::MAX);
    PtySize {
        rows: grid.rows,
        cols: grid.cols,
        pixel_width: pixels(grid.cols, cell.w),
        pixel_height: pixels(grid.rows, cell.h),
    }
}

/// A panic in the caller's callback must not take the other shell threads down with it.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn read_loop(
    mut reader: Box<dyn Read + Send>,
    delivery: &Mutex<Delivery>,
    progress: &(Mutex<Progress>, Condvar),
) {
    let (state, changed) = progress;
    let mut buf = vec![0; READ_CHUNK];
    loop {
        lock(state).busy = false;
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            // macOS and Linux report the closed child side as an error rather than an end.
            Err(_) => break,
        };
        {
            let mut state = lock(state);
            state.busy = true;
            state.reads += 1;
        }
        let delivery = lock(delivery);
        if delivery.exited {
            // Output after the exit belongs to a background child that kept the pty open.
            break;
        }
        (delivery.on_event)(ShellEvent::Output(buf[..n].to_vec()));
    }
    let mut state = lock(state);
    state.done = true;
    state.busy = false;
    changed.notify_all();
}

fn write_loop(mut writer: Box<dyn Write + Send>, queue: mpsc::Receiver<Vec<u8>>) {
    // Ends when the shell is dropped or the pty refuses a write (the command is gone); the
    // receiver goes with it, and later sends fail quietly.
    while let Ok(bytes) = queue.recv() {
        if writer
            .write_all(&bytes)
            .and_then(|()| writer.flush())
            .is_err()
        {
            break;
        }
    }
}

fn wait_loop(
    mut child: Box<dyn Child + Send + Sync>,
    pid: Option<u32>,
    delivery: &Mutex<Delivery>,
    progress: &(Mutex<Progress>, Condvar),
    exited: &Mutex<bool>,
) {
    // Reaping frees the pid for reuse, so the exit is marked first, under the signal lock.
    #[cfg(unix)]
    {
        if let Some(pid) = pid {
            await_exit(pid);
        }
        *lock(exited) = true;
    }
    // The library gives a killed command code 1 and the signal only by name.
    let code = match child.wait() {
        Ok(status) => status.exit_code(),
        Err(_) => 1,
    };
    #[cfg(not(unix))]
    {
        let _ = pid;
        *lock(exited) = true;
    }
    // What the command wrote is already in the pty; wait until the reader has delivered it.
    let (state, changed) = progress;
    let mut state = lock(state);
    let mut seen = state.reads;
    let mut quiet_since = Instant::now();
    loop {
        if state.done {
            break;
        }
        if state.busy || state.reads != seen {
            seen = state.reads;
            quiet_since = Instant::now();
        } else if quiet_since.elapsed() >= EXIT_GRACE {
            break;
        }
        state = changed
            .wait_timeout(
                state,
                EXIT_GRACE
                    .saturating_sub(quiet_since.elapsed())
                    .max(Duration::from_millis(10)),
            )
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .0;
    }
    drop(state);
    let mut delivery = lock(delivery);
    delivery.exited = true;
    (delivery.on_event)(ShellEvent::Exited(code));
}

/// Blocks until the command has exited, leaving it unreaped. If the exit cannot be watched this
/// returns at once, and the command then gets no signals at all rather than risk a stranger.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn await_exit(pid: u32) {
    use nix::errno::Errno;
    use nix::sys::wait::{Id, WaitPidFlag, waitid};
    use nix::unistd::Pid;
    let Ok(pid) = i32::try_from(pid) else { return };
    let flags = WaitPidFlag::WEXITED | WaitPidFlag::WNOWAIT;
    while let Err(Errno::EINTR) = waitid(Id::Pid(Pid::from_raw(pid)), flags) {}
}

/// As above; these systems have no `waitid` in nix, but kqueue reports an exit without reaping.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd"
))]
fn await_exit(pid: u32) {
    use nix::errno::Errno;
    use nix::sys::event::{EventFilter, EventFlag, FilterFlag, KEvent, Kqueue};
    let Ok(queue) = Kqueue::new() else { return };
    let watch = KEvent::new(
        pid as usize,
        EventFilter::EVFILT_PROC,
        EventFlag::EV_ADD | EventFlag::EV_ONESHOT,
        FilterFlag::NOTE_EXIT,
        0,
        0,
    );
    let mut fired = [watch];
    // An already exited command fails to register with ESRCH, which also ends the wait.
    while let Ok(0) | Err(Errno::EINTR) = queue.kevent(&[watch], &mut fired, None) {}
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd"
    ))
))]
fn await_exit(_pid: u32) {}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use term_engine::{CellSize, GridSize};

    #[test]
    fn write_after_exit_does_not_panic() {
        let (tx, rx) = mpsc::channel();
        let shell = Shell::spawn(
            Some(vec!["/usr/bin/true".to_string()]),
            None,
            GridSize { cols: 80, rows: 24 },
            Box::new(move |event| {
                let _ = tx.send(event);
            }),
        )
        .unwrap();
        loop {
            match rx.recv_timeout(Duration::from_secs(30)).expect("an event") {
                ShellEvent::Exited(code) => {
                    assert_eq!(code, 0);
                    break;
                }
                ShellEvent::Output(_) => {}
            }
        }
        let started = Instant::now();
        shell.write(vec![b'x'; 1024]);
        assert!(started.elapsed() < Duration::from_millis(100));
        shell.resize(
            GridSize {
                cols: 100,
                rows: 30,
            },
            CellSize { w: 8, h: 20 },
        );
        // The writer thread meets the closed pty only now; give it the chance to fail loudly.
        shell.write(vec![b'y'; 1024]);
    }

    fn spawn_sh(script: &str) -> (Shell, mpsc::Receiver<ShellEvent>) {
        let (tx, rx) = mpsc::channel();
        let command = ["/bin/sh", "-c", script].map(String::from).to_vec();
        let on_event = Box::new(move |event| {
            let _ = tx.send(event);
        });
        let shell = Shell::spawn(
            Some(command),
            None,
            GridSize { cols: 80, rows: 24 },
            on_event,
        );
        (shell.unwrap(), rx)
    }

    /// Ends the background `sleep 30` a test started, by its own pid and only while that pid is
    /// still a sleep, also when an assertion fails.
    struct Sleeper(i32);

    impl Drop for Sleeper {
        fn drop(&mut self) {
            use nix::sys::signal::{Signal, kill};
            use nix::unistd::Pid;
            let name = std::process::Command::new("/bin/ps")
                .args(["-p", &self.0.to_string(), "-o", "comm="])
                .output();
            if name.is_ok_and(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .ends_with("sleep")
            }) {
                let _ = kill(Pid::from_raw(self.0), Signal::SIGKILL);
            }
        }
    }

    #[test]
    fn no_signal_after_the_command_has_exited() {
        let (shell, rx) = spawn_sh("sleep 30 & echo \"pid $!\"; exit 0");
        let mut text = String::new();
        let mut sleeper = None;
        loop {
            match rx.recv_timeout(Duration::from_secs(30)).expect("an event") {
                ShellEvent::Output(bytes) => {
                    text.push_str(&String::from_utf8_lossy(&bytes));
                    if sleeper.is_none()
                        && let Some(line) = text
                            .split("pid ")
                            .nth(1)
                            .and_then(|rest| rest.split_once('\n'))
                        && let Ok(pid) = line.0.trim().parse()
                    {
                        sleeper = Some(Sleeper(pid));
                    }
                }
                ShellEvent::Exited(code) => {
                    assert_eq!(code, 0);
                    break;
                }
            }
        }
        assert!(sleeper.is_some(), "no pid in {text:?}");
        assert!(!shell.signal(Kill::Now));
        assert!(!shell.signal(Kill::HangUp));
        shell.kill();
        drop(shell);
    }

    #[test]
    fn a_live_command_is_signalled() {
        let (shell, rx) = spawn_sh("exec /bin/cat");
        assert!(shell.signal(Kill::Now));
        loop {
            match rx.recv_timeout(Duration::from_secs(30)).expect("an event") {
                ShellEvent::Exited(code) => {
                    assert_ne!(code, 0);
                    break;
                }
                ShellEvent::Output(_) => {}
            }
        }
    }
}
