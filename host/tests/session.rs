use std::path::PathBuf;
use std::time::{Duration, Instant};

#[cfg(unix)]
use bytes::Bytes;
#[cfg(unix)]
use crossbeam_channel::RecvTimeoutError;
use crossbeam_channel::{Receiver, unbounded};
use term_host::{ClientEvent, ClientTx, Kind, Session, SessionHandle, Size, SpawnSpec, clean_env};
#[cfg(unix)]
use term_host::{ClientId, Screen};
use uuid::Uuid;

const SIZE: Size = Size { cols: 80, rows: 24 };
const READ_LIMIT: Duration = Duration::from_secs(2);
const SLOW_LIMIT: Duration = Duration::from_secs(10);
// Long enough to notice a program that is still going. Only ever used to check that
// nothing happens, so a slow machine can't make a correct host fail on it.
#[cfg(unix)]
const QUIET: Duration = Duration::from_millis(300);
#[cfg(unix)]
const FLOOD_LIMIT: u64 = 4 * 1024 * 1024;
#[cfg(unix)]
const MAX_CHUNK: u64 = 64 * 1024;
#[cfg(unix)]
const STALE_LIMIT: Duration = Duration::from_secs(5);

fn spec() -> SpawnSpec {
    SpawnSpec {
        kind: Kind::Shell,
        cwd: PathBuf::from(env!("CARGO_TARGET_TMPDIR")),
        uuid: Uuid::nil(),
    }
}

fn client() -> (ClientTx, Receiver<ClientEvent>) {
    unbounded()
}

/// Collects output until `done` accepts it, or panics with what arrived. Skips the
/// snapshot, so `done` only ever sees bytes produced after the attach.
fn read_until(rx: &Receiver<ClientEvent>, limit: Duration, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + limit;
    let mut seen = Vec::new();
    loop {
        let text = String::from_utf8_lossy(&seen).into_owned();
        if done(&text) {
            return text;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            // Windows' ConPTY sets a title as soon as the program starts.
            Ok(ClientEvent::Snapshot(_) | ClientEvent::Title(_)) => {}
            Ok(ClientEvent::Output(bytes)) => seen.extend_from_slice(&bytes),
            other => panic!("stopped with {other:?}, output so far: {text:?}"),
        }
    }
}

#[cfg(unix)]
fn wait_for_exit(rx: &Receiver<ClientEvent>, limit: Duration) -> (bool, Option<i32>) {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(ClientEvent::Snapshot(_) | ClientEvent::Output(_) | ClientEvent::Title(_)) => {}
            Ok(ClientEvent::Exited { clean, signal }) => return (clean, signal),
            Err(err) => panic!("no exit event: {err}"),
        }
    }
}

#[cfg(unix)]
fn take_snapshot(rx: &Receiver<ClientEvent>) -> Bytes {
    match rx.recv_timeout(READ_LIMIT) {
        Ok(ClientEvent::Snapshot(bytes)) => bytes,
        other => panic!("wanted the snapshot first, got {other:?}"),
    }
}

/// Reads without acking until the client is over the limit, and returns how many bytes it
/// has been sent. Assumes everything sent since the attach is still in `rx`.
#[cfg(unix)]
fn read_past_limit(rx: &Receiver<ClientEvent>) -> u64 {
    let deadline = Instant::now() + SLOW_LIMIT;
    let mut sent = 0;
    while sent <= FLOOD_LIMIT {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(ClientEvent::Snapshot(bytes) | ClientEvent::Output(bytes)) => {
                sent += bytes.len() as u64;
            }
            other => panic!("stopped at {sent} bytes with {other:?}"),
        }
    }
    sent
}

/// Reads and acks the way a client that keeps up would, until `marker` shows up. `taken` is
/// what the client already holds, since acks are running totals.
#[cfg(unix)]
fn read_acking_until(
    handle: &SessionHandle,
    id: ClientId,
    rx: &Receiver<ClientEvent>,
    marker: &str,
    mut taken: u64,
) -> u64 {
    let deadline = Instant::now() + SLOW_LIMIT;
    let mut end = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(ClientEvent::Snapshot(bytes) | ClientEvent::Output(bytes)) => {
                taken += bytes.len() as u64;
                handle.ack(id, taken);
                end.extend_from_slice(&bytes);
                if String::from_utf8_lossy(&end).contains(marker) {
                    return taken;
                }
                // Only the part that could be the start of a marker split across chunks.
                end.drain(..end.len().saturating_sub(marker.len()));
            }
            other => panic!("stopped after {taken} bytes with {other:?}"),
        }
    }
}

/// Leaves a program finished while its client is over the limit and its last output waits.
#[cfg(unix)]
fn held_back_past_exit(shell: &str) -> (SessionHandle, Receiver<ClientEvent>, ClientId, u64) {
    let handle = Session::spawn_with_env(spec(), SIZE, shell_env(shell)).unwrap();
    let (tx, rx) = client();
    let id = handle.attach(tx, SIZE);
    handle.write(b"go\n").unwrap();
    let sent = read_past_limit(&rx);
    handle.write(b"go\n").unwrap();
    // Well past the drain grace, with the program gone and its tail waiting.
    assert_no_events(&rx, Duration::from_millis(1500));
    (handle, rx, id, sent)
}

#[cfg(unix)]
fn assert_no_events(rx: &Receiver<ClientEvent>, wait: Duration) {
    assert_eq!(rx.recv_timeout(wait), Err(RecvTimeoutError::Timeout));
}

#[cfg(unix)]
mod fixtures {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    pub struct Fixtures {
        pub cat: String,
        pub hup_ignoring_cat: String,
        pub hup_exiting_shell: String,
        pub numbered_lines: String,
        pub flood: String,
        pub counting_flood: String,
        pub flood_then_tail: String,
        pub flood_then_background: String,
        pub flood_until_interrupted: String,
        pub wide_history: String,
        pub root: PathBuf,
        pub home: String,
        pub login_home: String,
    }

    fn script(dir: &Path, name: &str, body: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path.to_str().unwrap().to_string()
    }

    // Written once before any test forks: a fork while a script is still open for
    // writing makes exec fail with ETXTBSY on Linux.
    pub fn get() -> &'static Fixtures {
        static FIXTURES: OnceLock<Fixtures> = OnceLock::new();
        FIXTURES.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("session-fixtures");
            let home = root.join("home");
            let login_home = root.join("login-home");
            std::fs::create_dir_all(&home).unwrap();
            std::fs::create_dir_all(&login_home).unwrap();
            let profile = "PATH=\"/login-shell-marker:$PATH\"\nexport PATH\n";
            for name in [".profile", ".bash_profile", ".zprofile"] {
                std::fs::write(login_home.join(name), profile).unwrap();
            }
            // Keeps zsh from offering its first-run setup menu in the empty home.
            std::fs::write(login_home.join(".zshrc"), "").unwrap();
            Fixtures {
                cat: script(&root, "cat-shell", "exec cat"),
                hup_ignoring_cat: script(&root, "hup-ignoring-cat", "trap '' HUP\nexec cat"),
                hup_exiting_shell: script(
                    &root,
                    "hup-exiting-shell",
                    "trap 'exit 0' HUP\nwhile read -r line; do echo \"got-$line\"; done",
                ),
                // Fewer lines than the scrollback holds, so an attach running late can't
                // lose the start. The pauses leave plenty of time to attach mid-stream, and
                // the blank lines at the end mean a late attach still sees later bytes.
                numbered_lines: script(
                    &root,
                    "numbered-lines",
                    "i=0\nwhile [ $i -lt 15000 ]; do\n  printf '\\033[3%dmn-%d\\033[m\\n' $((i % 7)) $i\n  i=$((i+1))\n  if [ $((i % 100)) = 0 ]; then sleep 0.01; fi\ndone\nyes '' | head -n 200\nexec cat",
                ),
                flood: script(&root, "flood", "exec yes $(printf '%01000d' 0)"),
                // Writes down how far it got, named by pid, so a test can see the program
                // itself stop and not only its output.
                counting_flood: script(
                    &root,
                    "counting-flood",
                    &format!(
                        "line=$(printf '%01000d' 0)\ni=0\nwhile :; do\n  echo \"$line\"\n  i=$((i+1))\n  echo \"$i\" > \"{}/flood-$$\"\ndone",
                        root.display()
                    ),
                ),
                // Waits on input at both ends, so a test can have the program finish only
                // once output is being held back. No newlines, so the terminal adds no
                // bytes and the flood is sure to pass the limit by a little.
                flood_then_tail: script(
                    &root,
                    "flood-then-tail",
                    &format!(
                        "read -r _\nhead -c {} /dev/zero | tr '\\0' x\nread -r _\nprintf tail-marker\nexit 0",
                        super::FLOOD_LIMIT + 1024
                    ),
                ),
                // The same, leaving a background program with the terminal open. On Linux the
                // end of output then never comes and only the drain grace ends the wait. macOS
                // closes the terminal when the program exits, so there this path cannot run.
                flood_then_background: script(
                    &root,
                    "flood-then-background",
                    &format!(
                        "read -r _\nhead -c {} /dev/zero | tr '\\0' x\nread -r _\n(trap '' HUP; exec sleep 8) &\nprintf tail-marker\nexit 0",
                        super::FLOOD_LIMIT + 1024
                    ),
                ),
                // Floods until Ctrl+C, then says so and exits normally.
                flood_until_interrupted: script(
                    &root,
                    "flood-until-interrupted",
                    "trap 'printf interrupted-marker; exit 0' INT\nyes $(printf '%01000d' 0)\nsleep 30",
                ),
                // Enough long lines that the snapshot alone is over the limit.
                wide_history: script(
                    &root,
                    "wide-history",
                    "yes $(printf '%0399d' 0) | head -n 12000\necho generated\nexec cat",
                ),
                root: root.clone(),
                home: home.to_str().unwrap().to_string(),
                login_home: login_home.to_str().unwrap().to_string(),
            }
        })
    }
}

#[cfg(unix)]
fn shell_env(shell: &str) -> std::vec::IntoIter<(String, String)> {
    vec![
        ("SHELL".to_string(), shell.to_string()),
        ("HOME".to_string(), fixtures::get().home.clone()),
        ("PATH".to_string(), std::env::var("PATH").unwrap()),
    ]
    .into_iter()
}

#[cfg(unix)]
fn spawn_shell(shell: &str) -> (SessionHandle, Receiver<ClientEvent>) {
    let handle = Session::spawn_with_env(spec(), SIZE, shell_env(shell)).unwrap();
    let (tx, rx) = client();
    handle.attach(tx, SIZE);
    (handle, rx)
}

#[cfg(unix)]
#[test]
fn echo_round_trip() {
    let (handle, rx) = spawn_shell(&fixtures::get().cat);
    handle.write(b"hello\n").unwrap();
    // Twice: once from the terminal's own echo and once from cat.
    read_until(&rx, READ_LIMIT, |out| out.matches("hello").count() >= 2);
}

// Restoring a window opens many terminals at the same moment. On macOS those opens
// race inside the kernel and can fail with error -6.
#[cfg(unix)]
#[test]
fn concurrent_spawns_all_succeed() {
    const THREADS: usize = 16;
    const ROUNDS: usize = 20;
    let cat = &fixtures::get().cat;
    for round in 0..ROUNDS {
        let barrier = std::sync::Barrier::new(THREADS);
        let failures: Vec<String> = std::thread::scope(|scope| {
            let spawns: Vec<_> = (0..THREADS)
                .map(|thread| {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        spawn_and_echo_own_token(cat, &format!("token-{round}-{thread}-end")).err()
                    })
                })
                .collect();
            spawns
                .into_iter()
                .filter_map(|spawn| spawn.join().unwrap())
                .collect()
        });
        assert_eq!(failures, Vec::<String>::new());
    }
}

/// Fails if the session can't be written to or its output isn't its own, which is what
/// two terminals crossing over during a concurrent open would look like.
#[cfg(unix)]
fn spawn_and_echo_own_token(shell: &str, token: &str) -> Result<(), String> {
    let handle = Session::spawn_with_env(spec(), SIZE, shell_env(shell))
        .map_err(|err| format!("spawn: {err}"))?;
    let (tx, rx) = client();
    handle.attach(tx, SIZE);
    handle
        .write(format!("{token}\n").as_bytes())
        .map_err(|err| {
            let mut events = Vec::new();
            while let Ok(event) = rx.recv_timeout(READ_LIMIT) {
                let exited = matches!(event, ClientEvent::Exited { .. });
                events.push(event);
                if exited {
                    break;
                }
            }
            format!("write: {err:?}, then {events:?}")
        })?;
    let deadline = Instant::now() + SLOW_LIMIT;
    let mut seen = Vec::new();
    loop {
        let text = String::from_utf8_lossy(&seen);
        if text.contains("token-") && !text.contains(token) && text.contains("-end") {
            return Err(format!("wanted {token}, got {text:?}"));
        }
        if text.matches(token).count() >= 2 {
            return Ok(());
        }
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(ClientEvent::Snapshot(_)) => {}
            Ok(ClientEvent::Output(bytes)) => seen.extend_from_slice(&bytes),
            other => return Err(format!("{other:?}, output so far: {text:?}")),
        }
    }
}

// A shell spec has no way to carry `cmd /c more`, so this runs `more` inside COMSPEC.
#[cfg(windows)]
#[test]
fn echo_round_trip() {
    let handle = Session::spawn(spec(), SIZE).unwrap();
    let (tx, rx) = client();
    handle.attach(tx, SIZE);
    handle.write(b"more\r\nhello\r\n").unwrap();
    read_until(&rx, READ_LIMIT, |out| out.contains("hello"));
}

#[cfg(unix)]
#[test]
fn spawn_reads_shell_from_host_env() {
    const CHILD_FLAG: &str = "TERM_HOST_TEST_SPAWN_CHILD";
    if std::env::var_os(CHILD_FLAG).is_some() {
        let handle = Session::spawn(spec(), SIZE).unwrap();
        let (tx, rx) = client();
        handle.attach(tx, SIZE);
        handle.write(b"hello\n").unwrap();
        read_until(&rx, READ_LIMIT, |out| out.matches("hello").count() >= 2);
        return;
    }
    // A fresh process, because changing this process's environment would race the
    // other tests.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "spawn_reads_shell_from_host_env"])
        .env("SHELL", &fixtures::get().cat)
        .env(CHILD_FLAG, "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

#[cfg(unix)]
#[test]
fn spawn_uses_cwd() {
    let (handle, rx) = spawn_shell("/bin/sh");
    handle.write(b"echo \"[$(pwd -P)]\"\n").unwrap();
    let expected = std::fs::canonicalize(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let expected = format!("[{}]", expected.display());
    read_until(&rx, SLOW_LIMIT, |out| out.contains(&expected));
}

#[cfg(unix)]
#[test]
fn program_gets_cleaned_env() {
    let env = shell_env("/bin/sh").chain([("CLAUDECODE".to_string(), "1".to_string())]);
    let handle = Session::spawn_with_env(spec(), SIZE, env).unwrap();
    let (tx, rx) = client();
    handle.attach(tx, SIZE);
    handle.write(b"echo \"[$CLAUDECODE][$TERM]\"\n").unwrap();
    read_until(&rx, SLOW_LIMIT, |out| out.contains("[][xterm-256color]"));
}

#[cfg(unix)]
#[test]
fn resize_reaches_program() {
    let (handle, rx) = spawn_shell("/bin/sh");
    handle.resize(Size {
        cols: 100,
        rows: 40,
    });
    handle.write(b"stty size\n").unwrap();
    read_until(&rx, SLOW_LIMIT, |out| out.contains("40 100"));
}

// What readline does by accident when a resize lands while it returns to its prompt, made
// to happen every time.
#[cfg(unix)]
#[test]
fn undone_resize_is_applied_again() {
    let (handle, rx) = spawn_shell("/bin/sh");
    handle
        .write(
            b"trap 'trap - WINCH; stty rows 5 cols 5' WINCH; echo rea''dy; \
              sleep 1 & wait; sleep 0.3; stty size\n",
        )
        .unwrap();
    read_until(&rx, SLOW_LIMIT, |out| out.contains("ready"));
    handle.resize(Size {
        cols: 100,
        rows: 40,
    });
    let out = read_until(&rx, SLOW_LIMIT, |out| {
        out.contains("40 100") || out.contains("5 5\r\n")
    });
    assert!(out.contains("40 100"), "the undone size stayed: {out:?}");
}

#[cfg(unix)]
#[test]
fn attach_resizes_first() {
    let handle = Session::spawn_with_env(spec(), SIZE, shell_env("/bin/sh")).unwrap();
    let (tx, rx) = client();
    handle.attach(
        tx,
        Size {
            cols: 120,
            rows: 50,
        },
    );
    handle.write(b"stty size\n").unwrap();
    read_until(&rx, SLOW_LIMIT, |out| out.contains("50 120"));
}

#[cfg(unix)]
#[test]
fn attach_starts_at_an_exact_point() {
    let (handle, first) = spawn_shell(&fixtures::get().cat);
    handle.write(b"before\n").unwrap();
    read_until(&first, READ_LIMIT, |out| out.matches("before").count() >= 2);

    let (tx, second) = client();
    handle.attach(tx, SIZE);
    // What came before arrives once, in the snapshot, and is not replayed as output.
    let snapshot = take_snapshot(&second);
    assert!(String::from_utf8_lossy(&snapshot).contains("before"));
    handle.write(b"after\n").unwrap();
    let out = read_until(&second, READ_LIMIT, |out| out.matches("after").count() >= 2);
    assert!(!out.contains("before"), "{out:?}");
}

#[cfg(unix)]
#[test]
fn attach_mid_stream_no_loss_no_dup() {
    let (handle, first) = spawn_shell(&fixtures::get().numbered_lines);
    read_until(&first, SLOW_LIMIT, |out| out.contains("n-"));

    let (tx, second) = client();
    handle.attach(tx, SIZE);
    let mut replay = Screen::new(SIZE, 20_000);
    replay.feed(&take_snapshot(&second));
    let mut later_lines = 0;
    while later_lines < 200 {
        match second.recv_timeout(SLOW_LIMIT) {
            Ok(ClientEvent::Output(bytes)) => {
                replay.feed(&bytes);
                later_lines += bytes.iter().filter(|&&byte| byte == b'\n').count();
            }
            other => panic!("stopped after {later_lines} lines with {other:?}"),
        }
    }

    let vt = replay.vt();
    let cols = vt.size().1.get();
    let mut rows = Vec::new();
    for back in (1..=vt.scrollback_len()).rev() {
        vt.set_scrollback(back);
        rows.extend(vt.rows(0, cols).next());
    }
    vt.set_scrollback(0);
    rows.extend(vt.rows(0, cols));
    let mut numbers: Vec<usize> = rows
        .iter()
        .filter_map(|row| row.strip_prefix("n-")?.parse().ok())
        .collect();
    // The last line may still be arriving, which would read as a smaller number.
    numbers.pop();
    assert!(numbers.len() >= 100, "only {} lines", numbers.len());
    let expected: Vec<usize> = (0..numbers.len()).collect();
    assert_eq!(numbers, expected);
}

#[cfg(unix)]
#[test]
fn flood_stays_bounded() {
    let (handle, rx) = spawn_shell(&fixtures::get().counting_flood);
    let sent = read_past_limit(&rx);
    assert!(sent <= FLOOD_LIMIT + MAX_CHUNK, "sent {sent}");
    assert_no_events(&rx, QUIET);

    // The program itself stops once the channel and terminal are full.
    let progress = fixtures::get()
        .root
        .join(format!("flood-{}", handle.pid().unwrap()));
    let deadline = Instant::now() + SLOW_LIMIT;
    let mut last = None;
    let lines: u64 = loop {
        let now = std::fs::read_to_string(&progress)
            .ok()
            .and_then(|text| text.trim().parse().ok());
        if let (Some(lines), true) = (now, now == last) {
            break lines;
        }
        assert!(Instant::now() < deadline, "still writing at {now:?}");
        last = now;
        std::thread::sleep(QUIET);
    };
    // 1000 digits and the end of line. The slack is the output channel and the terminal.
    assert!(
        lines * 1002 <= sent + 2 * 1024 * 1024,
        "{lines} lines with {sent} sent"
    );

    let (other, other_rx) = spawn_shell(&fixtures::get().cat);
    other.write(b"hello\n").unwrap();
    read_until(&other_rx, READ_LIMIT, |out| {
        out.matches("hello").count() >= 2
    });

    // Control messages still get through: this one only returns once the actor answers.
    let (tx, late) = client();
    handle.attach(tx, SIZE);
    take_snapshot(&late);

    // The next thing this client hears is the exit: it gets no more output, even though
    // the kill makes the host read the program's last bytes.
    handle.kill();
    let exited = ClientEvent::Exited {
        clean: false,
        signal: Some(libc::SIGHUP),
    };
    assert_eq!(rx.recv_timeout(SLOW_LIMIT), Ok(exited));
    assert_eq!(
        wait_for_exit(&late, READ_LIMIT),
        (false, Some(libc::SIGHUP))
    );
    let _ = std::fs::remove_file(progress);
}

#[cfg(unix)]
#[test]
fn dropping_the_handle_ends_a_held_back_program() {
    let (handle, rx) = spawn_shell(&fixtures::get().flood);
    read_past_limit(&rx);
    let pid = handle.pid().unwrap() as i32;
    drop(handle);
    assert_eq!(wait_for_exit(&rx, SLOW_LIMIT), (false, Some(libc::SIGHUP)));
    // Signal 0 only checks that the process exists.
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "pid {pid} still running");
}

#[cfg(unix)]
#[test]
fn stale_client_dropped() {
    let handle = Session::spawn_with_env(spec(), SIZE, shell_env(&fixtures::get().flood)).unwrap();
    // The live one first, so by the time the stale one has a chunk the live one has too.
    let (tx, live) = client();
    let live_id = handle.attach(tx, SIZE);
    let (tx, stale) = client();
    handle.attach(tx, SIZE);

    read_past_limit(&stale);
    let held_at = Instant::now();
    let taken = live.try_iter().fold(0, |sum, event| match event {
        ClientEvent::Snapshot(bytes) | ClientEvent::Output(bytes) => sum + bytes.len() as u64,
        other => panic!("{other:?}"),
    });
    handle.ack(live_id, taken);
    assert_no_events(&live, QUIET);

    // Dropped, not merely quiet: the channel reports its sender has gone.
    assert_eq!(
        stale.recv_timeout(SLOW_LIMIT),
        Err(RecvTimeoutError::Disconnected)
    );
    assert!(held_at.elapsed() >= STALE_LIMIT - Duration::from_secs(2));

    // The throttle has lifted for the client that is left.
    assert!(matches!(
        live.recv_timeout(SLOW_LIMIT),
        Ok(ClientEvent::Output(_))
    ));
}

#[cfg(unix)]
#[test]
fn idle_client_under_the_limit_is_kept() {
    let (handle, rx) = spawn_shell(&fixtures::get().cat);
    std::thread::sleep(STALE_LIMIT + Duration::from_millis(500));
    handle.write(b"hello\n").unwrap();
    read_until(&rx, READ_LIMIT, |out| out.matches("hello").count() >= 2);
}

#[cfg(unix)]
#[test]
fn output_tail_survives_a_held_back_client_at_exit() {
    let (handle, rx, id, sent) = held_back_past_exit(&fixtures::get().flood_then_tail);
    handle.ack(id, sent);
    read_until(&rx, READ_LIMIT, |out| out.contains("tail-marker"));
    assert_eq!(wait_for_exit(&rx, READ_LIMIT), (true, None));
}

#[cfg(unix)]
#[test]
fn exit_is_announced_when_a_hold_lifts_and_the_terminal_stays_open() {
    let (handle, rx, id, sent) = held_back_past_exit(&fixtures::get().flood_then_background);
    handle.ack(id, sent);
    read_until(&rx, READ_LIMIT, |out| out.contains("tail-marker"));
    assert_eq!(wait_for_exit(&rx, READ_LIMIT), (true, None));
}

#[cfg(unix)]
#[test]
fn ctrl_c_stops_a_held_back_flood_within_a_second() {
    let handle = Session::spawn_with_env(
        spec(),
        SIZE,
        shell_env(&fixtures::get().flood_until_interrupted),
    )
    .unwrap();
    let (tx, rx) = client();
    let id = handle.attach(tx, SIZE);
    let sent = read_past_limit(&rx);

    let pressed = Instant::now();
    handle.write(b"\x03").unwrap();
    handle.ack(id, sent);
    read_acking_until(&handle, id, &rx, "interrupted-marker", sent);
    let took = pressed.elapsed();

    assert!(took < Duration::from_secs(1), "took {took:?}");
    assert_eq!(wait_for_exit(&rx, READ_LIMIT), (true, None));
}

#[cfg(unix)]
#[test]
fn kill_after_exit_ends_the_wait_for_a_held_back_client() {
    let (handle, rx, _id, _sent) = held_back_past_exit(&fixtures::get().flood_then_tail);
    handle.kill();
    // Sooner than the stale limit, and clean because the program ended before the kill.
    assert_eq!(wait_for_exit(&rx, READ_LIMIT), (true, None));
}

#[cfg(unix)]
#[test]
fn snapshot_over_the_limit_is_kept_if_acked() {
    const WIDE: Size = Size {
        cols: 400,
        rows: 24,
    };
    let handle =
        Session::spawn_with_env(spec(), WIDE, shell_env(&fixtures::get().wide_history)).unwrap();
    let (tx, first) = client();
    let first_id = handle.attach(tx, WIDE);
    read_acking_until(&handle, first_id, &first, "generated", 0);
    handle.detach(first_id);

    let (tx, rx) = client();
    let id = handle.attach(tx, WIDE);
    let snapshot = take_snapshot(&rx).len() as u64;
    assert!(snapshot > FLOOD_LIMIT, "snapshot is only {snapshot}");

    // An ack that leaves it over the limit still restarts the clock, so this client
    // outlives the limit from attaching. Over the limit means its output is held back.
    handle.write(b"hello\n").unwrap();
    assert_no_events(&rx, Duration::from_secs(3));
    handle.ack(id, 1);
    assert_no_events(&rx, Duration::from_secs(3));
    handle.ack(id, snapshot);
    read_until(&rx, READ_LIMIT, |out| out.matches("hello").count() >= 2);
}

#[cfg(unix)]
#[test]
fn detach_stops_output() {
    let (handle, kept) = spawn_shell(&fixtures::get().cat);
    let (tx, gone) = client();
    let id = handle.attach(tx, SIZE);
    handle.detach(id);
    handle.ack(id, 10);
    handle.write(b"hello\n").unwrap();
    read_until(&kept, READ_LIMIT, |out| out.matches("hello").count() >= 2);
    // The actor dropped this client's sender, and it saw nothing after its snapshot.
    take_snapshot(&gone);
    assert!(gone.recv_timeout(READ_LIMIT).is_err());
}

#[cfg(unix)]
#[test]
fn exit_reports_clean_or_signal() {
    let (exiting, rx) = spawn_shell("/bin/sh");
    exiting.write(b"exit\n").unwrap();
    assert_eq!(wait_for_exit(&rx, SLOW_LIMIT), (true, None));

    let (killed, rx) = spawn_shell(&fixtures::get().cat);
    killed.kill();
    assert_eq!(wait_for_exit(&rx, SLOW_LIMIT), (false, Some(libc::SIGHUP)));
}

#[cfg(unix)]
#[test]
fn exit_code_is_not_mistaken_for_a_signal() {
    // 129 is what a shell reports for a child killed by SIGHUP, but here the
    // program itself exits normally.
    let (handle, rx) = spawn_shell("/bin/sh");
    handle.write(b"exit 129\n").unwrap();
    assert_eq!(wait_for_exit(&rx, SLOW_LIMIT), (true, None));
}

#[cfg(unix)]
#[test]
fn normal_exit_after_kill_is_not_clean() {
    // Restore keeps a session the host ended, so a shell answering SIGHUP with a normal
    // exit must not look like the person typing exit.
    let (handle, rx) = spawn_shell(&fixtures::get().hup_exiting_shell);
    // Prove the trap is set before signalling. Startup output could come before attach
    // and be missed, so this waits on a reply to input instead.
    handle.write(b"ready\n").unwrap();
    read_until(&rx, READ_LIMIT, |out| out.contains("got-ready"));
    handle.kill();
    assert_eq!(wait_for_exit(&rx, SLOW_LIMIT), (false, None));
}

#[cfg(unix)]
#[test]
fn kill_escalates_when_sighup_is_ignored() {
    let (handle, rx) = spawn_shell(&fixtures::get().hup_ignoring_cat);
    // Prove the trap ran and cat took over before signalling.
    handle.write(b"ready\n").unwrap();
    read_until(&rx, READ_LIMIT, |out| out.matches("ready").count() >= 2);
    handle.kill();
    assert_eq!(wait_for_exit(&rx, SLOW_LIMIT), (false, Some(libc::SIGKILL)));
}

#[cfg(unix)]
#[test]
fn attach_after_exit_reports_the_exit() {
    let (handle, rx) = spawn_shell("/bin/sh");
    let long = "a".repeat(60);
    handle
        .write(format!("echo {long}; exit\n").as_bytes())
        .unwrap();
    wait_for_exit(&rx, SLOW_LIMIT);

    let (tx, late) = client();
    handle.attach(tx, SIZE);
    let snapshot = take_snapshot(&late);
    assert!(String::from_utf8_lossy(&snapshot).contains(&long));
    assert_eq!(wait_for_exit(&late, READ_LIMIT), (true, None));

    // The terminal has gone, but the screen copy still takes a new size.
    let (tx, narrow) = client();
    handle.attach(tx, Size { cols: 20, rows: 24 });
    let snapshot = take_snapshot(&narrow);
    assert!(!String::from_utf8_lossy(&snapshot).contains(&long));
    assert_eq!(wait_for_exit(&narrow, READ_LIMIT), (true, None));
}

#[cfg(unix)]
#[test]
fn dropping_the_handle_ends_the_program() {
    let (handle, _rx) = spawn_shell(&fixtures::get().cat);
    let pid = handle.pid().unwrap() as i32;
    drop(handle);
    let deadline = Instant::now() + SLOW_LIMIT;
    // Signal 0 only checks that the process exists.
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(Instant::now() < deadline, "pid {pid} still running");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn clean_env_strips() {
    let input = [
        ("PATH", "/usr/bin"),
        ("LANG", "C"),
        ("CLAUDECODE", "1"),
        ("CLAUDE_CONFIG_DIR", "/x"),
        ("CLAUDE_CODE_ENTRYPOINT", "cli"),
        ("KITTY_WINDOW_ID", "1"),
        ("ORCA_PANE", "2"),
        ("TERM", "dumb"),
        ("COLORTERM", "no"),
        ("CLAUDE_OTHER", "kept"),
    ]
    .map(|(key, value)| (key.to_string(), value.to_string()));

    let mut env = clean_env(input.into_iter());
    env.sort();

    let expected = [
        ("CLAUDE_OTHER", "kept"),
        ("COLORTERM", "truecolor"),
        ("LANG", "C"),
        ("PATH", "/usr/bin"),
        ("TERM", "xterm-256color"),
    ]
    .map(|(key, value)| (key.to_string(), value.to_string()));
    assert_eq!(env, expected);
}

#[cfg(unix)]
#[test]
fn login_shell_finds_path() {
    // No PATH and no SHELL, like a host started by launchd. HOME only points the
    // login shell at a profile this test controls, not the real dotfiles.
    let env = [("HOME".to_string(), fixtures::get().login_home.clone())];
    let handle = Session::spawn_with_env(spec(), SIZE, env.into_iter()).unwrap();
    let (tx, rx) = client();
    handle.attach(tx, SIZE);
    handle
        .write(b"printf 'PATHIS=<%s>' \"$PATH\"; echo\n")
        .unwrap();
    let out = read_until(&rx, SLOW_LIMIT, |out| out.contains("PATHIS=</"));
    assert!(out.contains("/login-shell-marker"), "{out:?}");
}
