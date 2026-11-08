use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use term_host::{ClientEvent, ClientTx, Kind, Session, SessionHandle, Size, SpawnSpec, clean_env};
use uuid::Uuid;

const SIZE: Size = Size { cols: 80, rows: 24 };
const READ_LIMIT: Duration = Duration::from_secs(2);
const SLOW_LIMIT: Duration = Duration::from_secs(10);

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

/// Collects output until `done` accepts it, or panics with what arrived.
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
            Ok(ClientEvent::Output(_)) => {}
            Ok(ClientEvent::Exited { clean, signal }) => return (clean, signal),
            Err(err) => panic!("no exit event: {err}"),
        }
    }
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
    handle.write(b"after\n").unwrap();
    let out = read_until(&second, READ_LIMIT, |out| out.matches("after").count() >= 2);
    assert!(!out.contains("before"), "{out:?}");
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
    // The actor dropped this client's sender, and it never saw any output.
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
    handle.write(b"exit\n").unwrap();
    wait_for_exit(&rx, SLOW_LIMIT);

    let (tx, late) = client();
    handle.attach(tx, SIZE);
    assert_eq!(wait_for_exit(&late, READ_LIMIT), (true, None));
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
