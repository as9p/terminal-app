//! The shell hook: a shell reports its folder at each prompt, and the user's own startup files
//! still load as they would without it. Every test has a home and a data folder of its own.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, unbounded};
use term_host::{ClientEvent, Kind, Session, SessionHandle, Size, SpawnSpec, shellinit};
use uuid::Uuid;

const SIZE: Size = Size { cols: 80, rows: 24 };
const PATIENCE: Duration = Duration::from_secs(10);

/// Removed on drop.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Sandbox {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("shellinit-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home")).expect("make the home folder");
        shellinit::install(&root.join("data")).expect("install the hook");
        Sandbox(root)
    }

    fn home(&self) -> PathBuf {
        self.0.join("home")
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }

    /// Writes a file under the home folder, making its folder.
    #[cfg(unix)]
    fn write(&self, name: &str, text: &str) {
        let path = self.home().join(name);
        fs::create_dir_all(path.parent().expect("a parent")).expect("make the folder");
        fs::write(path, text).expect("write the file");
    }

    /// The lines the startup files appended to `order`.
    #[cfg(unix)]
    fn order(&self) -> Vec<String> {
        fs::read_to_string(self.home().join("order"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn start(&self, shell: &str, extra: &[(&str, String)]) -> Watch {
        let home = self.home().display().to_string();
        #[cfg(unix)]
        let mut env = vec![
            ("HOME".to_string(), home),
            ("SHELL".to_string(), shell.to_string()),
            (
                "PATH".to_string(),
                std::env::var("PATH").unwrap_or_default(),
            ),
        ];
        // PowerShell needs more of the system's environment than a Unix shell to start.
        #[cfg(windows)]
        let mut env: Vec<(String, String)> = std::env::vars()
            .filter(|(key, _)| {
                !["COMSPEC", "HOME", "USERPROFILE"].contains(&key.to_uppercase().as_str())
            })
            .chain([
                ("COMSPEC".to_string(), shell.to_string()),
                ("HOME".to_string(), home.clone()),
                ("USERPROFILE".to_string(), home),
            ])
            .collect();
        env.extend(
            extra
                .iter()
                .map(|(key, value)| (key.to_string(), value.clone())),
        );
        let spec = SpawnSpec {
            kind: Kind::Shell,
            cwd: self.home(),
            uuid: Uuid::from_u128(1),
            data_dir: self.data(),
            known_cwd: false,
        };
        let handle = Session::spawn_with_env(spec, SIZE, env.into_iter()).expect("spawn");
        let (tx, rx) = unbounded();
        handle.attach(tx, SIZE);
        Watch {
            handle,
            rx,
            output: String::new(),
            cwds: Vec::new(),
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A running shell and everything it has sent so far.
struct Watch {
    handle: SessionHandle,
    rx: Receiver<ClientEvent>,
    output: String,
    cwds: Vec<PathBuf>,
}

impl Watch {
    fn type_line(&self, line: &str) {
        // What the Enter key sends.
        let enter = if cfg!(windows) { "\r" } else { "\n" };
        self.handle
            .write(format!("{line}{enter}").as_bytes())
            .expect("type");
    }

    fn until(&mut self, what: &str, done: impl Fn(&Watch) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(self) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left) {
                Ok(ClientEvent::Output(bytes)) => {
                    self.output.push_str(&String::from_utf8_lossy(&bytes));
                }
                Ok(ClientEvent::Cwd(path)) => self.cwds.push(path),
                Ok(ClientEvent::Snapshot(_) | ClientEvent::Title(_)) => {}
                other => panic!(
                    "waiting for {what}, stopped with {other:?}; folders {:?}, output {:?}",
                    self.cwds, self.output
                ),
            }
        }
    }

    #[cfg(unix)]
    fn until_output(&mut self, needle: &str) {
        self.until(needle, |watch| watch.output.contains(needle));
    }

    fn until_cwd(&mut self, wanted: &Path) {
        let wanted = real(wanted);
        self.until(&wanted.display().to_string(), |watch| {
            watch.cwds.contains(&wanted)
        });
    }
}

/// Resolved the way the host resolves a reported folder, without the `\\?\` Windows adds.
fn real(path: &Path) -> PathBuf {
    let real = path
        .canonicalize()
        .unwrap_or_else(|error| panic!("resolve {}: {error}", path.display()));
    if cfg!(windows) {
        let text = real.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    real
}

/// Startup file text that appends `name` to `$HOME/order`.
#[cfg(unix)]
fn appends(name: &str) -> String {
    format!("echo {name} >> \"$HOME/order\"\n")
}

#[cfg(unix)]
#[test]
fn zsh_reports_folder_at_prompt() {
    let sandbox = Sandbox::new();
    sandbox.write(".zshrc", "PS1='%# '\n");
    let mut shell = sandbox.start("/bin/zsh", &[]);
    shell.until_cwd(&sandbox.home());
    shell.type_line("cd /tmp");
    shell.until_cwd(&std::fs::canonicalize("/tmp").expect("resolve /tmp"));
}

#[cfg(unix)]
#[test]
fn symlinked_folder_reports_its_real_path() {
    let sandbox = Sandbox::new();
    sandbox.write(".zshrc", "PS1='%# '\n");
    let target = sandbox.home().join("target");
    std::fs::create_dir(&target).expect("make the target");
    std::os::unix::fs::symlink(&target, sandbox.home().join("link")).expect("make the link");
    let mut shell = sandbox.start("/bin/zsh", &[]);
    shell.until_cwd(&sandbox.home());
    shell.type_line("cd link");
    shell.until_cwd(&target);
}

#[cfg(unix)]
#[test]
fn zsh_profile_order_is_unchanged() {
    let sandbox = Sandbox::new();
    for name in ["zshenv", "zprofile", "zshrc", "zlogin"] {
        sandbox.write(&format!(".{name}"), &appends(name));
    }
    let mut shell = sandbox.start("/bin/zsh", &[]);
    shell.type_line("echo \"Z=${ZDOTDIR-unset}=Z\"");
    shell.until_output("Z=unset=Z");
    assert_eq!(sandbox.order(), ["zshenv", "zprofile", "zshrc", "zlogin"]);
}

#[cfg(unix)]
#[test]
fn zsh_keeps_a_custom_zdotdir() {
    let sandbox = Sandbox::new();
    let custom = sandbox.home().join("z");
    sandbox.write(".zshrc", &appends("home-zshrc"));
    sandbox.write("z/.zshenv", &appends("z-zshenv"));
    sandbox.write("z/.zshrc", &appends("z-zshrc"));
    let mut shell = sandbox.start("/bin/zsh", &[("ZDOTDIR", custom.display().to_string())]);
    shell.until_cwd(&sandbox.home());
    shell.type_line("echo \"Z=$ZDOTDIR=Z\"");
    shell.until_output(&format!("Z={}=Z", custom.display()));
    assert_eq!(sandbox.order(), ["z-zshenv", "z-zshrc"]);
}

#[cfg(unix)]
#[test]
fn zsh_user_precmd_still_runs() {
    let sandbox = Sandbox::new();
    sandbox.write(".zshrc", "precmd() { echo user-precmd-ran }\n");
    let mut shell = sandbox.start("/bin/zsh", &[]);
    shell.until_output("user-precmd-ran");
    shell.until_cwd(&sandbox.home());
}

#[cfg(unix)]
#[test]
fn bash_reports_folder_and_loads_profile() {
    let sandbox = Sandbox::new();
    sandbox.write(".bash_profile", &(appends("bash_profile") + "PS1='$ '\n"));
    sandbox.write(".profile", &appends("profile"));
    let mut shell = sandbox.start("/bin/bash", &[]);
    shell.until_cwd(&sandbox.home());
    shell.type_line("cd /tmp");
    shell.until_cwd(Path::new("/tmp"));
    assert_eq!(sandbox.order(), ["bash_profile"]);
}

#[cfg(unix)]
#[test]
fn unknown_shell_is_untouched() {
    let sandbox = Sandbox::new();
    let mut env = vec![("HOME".to_string(), sandbox.home().display().to_string())];
    let before = env.clone();
    let argv = shellinit::wrap(Path::new("/bin/sh"), &sandbox.data(), &mut env);
    assert_eq!(argv, ["/bin/sh", "-l"].map(std::ffi::OsString::from));
    assert_eq!(env, before);
}

#[cfg(unix)]
#[test]
fn report_from_another_host_is_dropped() {
    let sandbox = Sandbox::new();
    let mut shell = sandbox.start("/bin/sh", &[]);
    // The second report names this computer in capitals, which still counts.
    shell.type_line(
        "printf '\\033]7;file://elsewhere/tmp\\007'; \
         printf '\\033]7;file://%s%s\\007' \"$(hostname | tr a-z A-Z)\" \"$HOME\"",
    );
    shell.until("a folder", |watch| !watch.cwds.is_empty());
    assert_eq!(real(&shell.cwds[0]), real(&sandbox.home()));
}

#[cfg(unix)]
#[test]
fn report_for_a_missing_folder_is_dropped() {
    let sandbox = Sandbox::new();
    let mut shell = sandbox.start("/bin/sh", &[]);
    shell.type_line(
        "printf '\\033]7;file://%s/missing\\007' \"$HOME\"; \
         printf '\\033]7;file://%s\\007' \"$HOME\"",
    );
    shell.until("a folder", |watch| !watch.cwds.is_empty());
    assert_eq!(real(&shell.cwds[0]), real(&sandbox.home()));
}

#[cfg(windows)]
#[test]
fn powershell_reports_folder_at_prompt() {
    let sandbox = Sandbox::new();
    let other = sandbox.home().join("other folder");
    fs::create_dir_all(&other).expect("make a folder");
    let mut shell = sandbox.start("powershell.exe", &[]);
    shell.until_cwd(&sandbox.home());
    shell.type_line(&format!("cd '{}'", other.display()));
    shell.until_cwd(&other);
}

/// Prints each argument in brackets between `got:` and `:end`, which a typed line never holds.
#[cfg(unix)]
const FAKE_CLAUDE: &str = "#!/bin/sh\nprintf 'go''t:'\nfor a in \"$@\"; do printf '[%s]' \"$a\"; done\nprintf ':end\\n'\n";

#[cfg(unix)]
const SHELLS: [&str; 2] = ["/bin/zsh", "/bin/bash"];

/// A shell with the fake `claude` first on its PATH, the hooks file in place, and `flags` as
/// the flags file.
#[cfg(unix)]
fn claude_shell(sandbox: &Sandbox, shell: &str, flags: &str, extra: &[(&str, String)]) -> Watch {
    use std::os::unix::fs::PermissionsExt;
    let bin = sandbox.0.join("bin");
    fs::create_dir_all(&bin).expect("make the bin folder");
    let fake = bin.join("claude");
    fs::write(&fake, FAKE_CLAUDE).expect("write the fake claude");
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("make it runnable");
    fs::write(sandbox.data().join("claude-hooks.json"), "{}").expect("write the hooks file");
    fs::write(sandbox.data().join("claude-args"), flags).expect("write the flags file");
    let mut watch = sandbox.start(shell, extra);
    watch.until_cwd(&sandbox.home());
    // Typed rather than passed in, since a login shell's startup files can reorder PATH.
    watch.type_line(&format!("PATH='{}':\"$PATH\"", bin.display()));
    watch
}

/// The arguments of every call to the fake `claude` so far.
#[cfg(unix)]
fn claude_calls(watch: &Watch) -> Vec<String> {
    watch
        .output
        .split("got:")
        .skip(1)
        .filter_map(|rest| rest.split_once(":end").map(|(args, _)| args.to_string()))
        .collect()
}

/// Types `line` and waits for the call it makes.
#[cfg(unix)]
fn call_claude(watch: &mut Watch, line: &str) -> String {
    let before = claude_calls(watch).len();
    watch.type_line(line);
    watch.until(line, |watch| claude_calls(watch).len() > before);
    claude_calls(watch)[before].clone()
}

#[cfg(unix)]
fn settings_args(sandbox: &Sandbox) -> String {
    format!(
        "[--settings][{}]",
        sandbox.data().join("claude-hooks.json").display()
    )
}

#[cfg(unix)]
#[test]
fn claude_gets_settings_and_flags() {
    for shell in SHELLS {
        let sandbox = Sandbox::new();
        let mut watch = claude_shell(&sandbox, shell, "--dangerously-skip-permissions\n", &[]);
        let got = call_claude(&mut watch, "claude --model haiku");
        let settings = settings_args(&sandbox);
        assert_eq!(
            got,
            format!("{settings}[--dangerously-skip-permissions][--model][haiku]"),
            "{shell}"
        );
    }
}

#[cfg(unix)]
#[test]
fn claude_subcommand_gets_nothing() {
    for shell in SHELLS {
        let sandbox = Sandbox::new();
        let mut watch = claude_shell(&sandbox, shell, "--dangerously-skip-permissions\n", &[]);
        assert_eq!(
            call_claude(&mut watch, "claude mcp list"),
            "[mcp][list]",
            "{shell}"
        );
    }
}

#[cfg(unix)]
#[test]
fn claude_flag_not_repeated() {
    for shell in SHELLS {
        let sandbox = Sandbox::new();
        let mut watch = claude_shell(&sandbox, shell, "--dangerously-skip-permissions\n", &[]);
        let got = call_claude(&mut watch, "claude -p hi --dangerously-skip-permissions");
        let settings = settings_args(&sandbox);
        assert_eq!(
            got,
            format!("{settings}[-p][hi][--dangerously-skip-permissions]"),
            "{shell}"
        );
    }
}

#[cfg(unix)]
#[test]
fn claude_flags_off_still_gets_settings() {
    for shell in SHELLS {
        let sandbox = Sandbox::new();
        let mut watch = claude_shell(&sandbox, shell, "", &[]);
        let got = call_claude(&mut watch, "claude");
        assert_eq!(got, settings_args(&sandbox), "{shell}");
    }
}

/// When a typed `claude` returns, even with an error, the function tells the host through
/// `$TA_HOST` and hands back claude's status.
#[cfg(unix)]
#[test]
fn claude_exit_is_reported_and_status_kept() {
    use std::os::unix::fs::PermissionsExt;
    for shell in SHELLS {
        let sandbox = Sandbox::new();
        let bin = sandbox.0.join("bin");
        let log = sandbox.0.join("calls.log");
        fs::create_dir_all(&bin).expect("make the bin folder");
        // The function hides the host's output, so both programs write to a log instead.
        let fake_host = bin.join("fake-host");
        let host_script = format!("#!/bin/sh\necho \"host $*\" >> '{}'\n", log.display());
        fs::write(&fake_host, host_script).expect("write the fake host");
        fs::set_permissions(&fake_host, fs::Permissions::from_mode(0o755))
            .expect("make it runnable");
        let failing = format!("#!/bin/sh\necho claude >> '{}'\nexit 3\n", log.display());
        let host = [("TA_HOST", fake_host.display().to_string())];
        let mut watch = claude_shell(&sandbox, shell, "", &host);
        fs::write(bin.join("claude"), failing).expect("make claude fail");
        watch.type_line("claude; echo \"st\"\"atus:${?}:end\"");
        watch.until_output("status:3:end");
        let calls = fs::read_to_string(&log).unwrap_or_default();
        assert_eq!(calls, "claude\nhost hook ClaudeExit\n", "{shell}");
        // A subcommand starts no chat, so nothing is reported for it.
        watch.type_line("claude mcp list; echo \"do\"\"ne:end\"");
        watch.until_output("done:end");
        let calls = fs::read_to_string(&log).unwrap_or_default();
        assert_eq!(calls, "claude\nhost hook ClaudeExit\nclaude\n", "{shell}");
    }
}

#[cfg(unix)]
#[test]
fn changed_flags_apply_in_an_open_shell() {
    for shell in SHELLS {
        let sandbox = Sandbox::new();
        let mut watch = claude_shell(&sandbox, shell, "--dangerously-skip-permissions\n", &[]);
        let settings = settings_args(&sandbox);
        let first = call_claude(&mut watch, "claude");
        assert_eq!(
            first,
            format!("{settings}[--dangerously-skip-permissions]"),
            "{shell}"
        );
        fs::write(
            sandbox.data().join("claude-args"),
            "--verbose\n--model\nhaiku\n",
        )
        .expect("change the flags");
        let second = call_claude(&mut watch, "claude");
        assert_eq!(
            second,
            format!("{settings}[--verbose][--model][haiku]"),
            "{shell}"
        );
    }
}
