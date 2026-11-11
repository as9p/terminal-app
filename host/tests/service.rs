//! The host as a launchd service: the plist text, and install, uninstall and status run
//! against a temp home folder and a fake `launchctl` that records each call and runs nothing.
//! launchd exists only on macOS (the CLI refuses elsewhere), and these paths and plist texts
//! are macOS ones: `<` and `>` cannot even appear in a Windows file name.

#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use term_host::service::{self, Installed, Launchctl, Ran, Service, Uninstalled};

const UID: u32 = 501;
const TARGET: &str = "gui/501/app.terminal.host";

/// Removed on drop. Its folders have a space and an ampersand, so every path needs escaping.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new() -> Sandbox {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("service-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("home & co")).expect("make the home folder");
        Sandbox(root)
    }

    fn service(&self) -> Service {
        Service {
            home: self.0.join("home & co"),
            data_dir: self.0.join("data <1>"),
            uid: UID,
        }
    }

    fn exe(&self, name: &str) -> PathBuf {
        self.0.join("Terminal & Co.app/Contents/MacOS").join(name)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A fake `launchctl` that keeps one job's loaded state, like launchd would.
#[derive(Default)]
struct FakeLaunchctl {
    loaded: bool,
    /// How many bootstraps fail before one works, as right after a bootout.
    bootstrap_failures: u32,
    calls: Vec<String>,
    pauses: u32,
}

impl Launchctl for FakeLaunchctl {
    fn run(&mut self, args: &[String]) -> io::Result<Ran> {
        self.calls.push(args.join(" "));
        let success = match args[0].as_str() {
            "print" => self.loaded,
            "bootout" => std::mem::replace(&mut self.loaded, false),
            "bootstrap" if self.loaded => false,
            "bootstrap" if self.bootstrap_failures > 0 => {
                self.bootstrap_failures -= 1;
                false
            }
            "bootstrap" => {
                self.loaded = true;
                true
            }
            other => panic!("unexpected launchctl {other}"),
        };
        let output = if success {
            String::new()
        } else {
            "Bootstrap failed: 5: Input/output error".to_string()
        };
        Ok(Ran { success, output })
    }

    fn pause(&mut self) {
        self.pauses += 1;
    }
}

fn bootstrap_call(plist: &Path) -> String {
    format!("bootstrap gui/501 {}", plist.display())
}

#[test]
fn service_takes_exactly_one_known_action() {
    let args = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
    assert_eq!(
        service::action(&args(&["install"])),
        Some(service::Action::Install)
    );
    assert_eq!(
        service::action(&args(&["uninstall"])),
        Some(service::Action::Uninstall)
    );
    assert_eq!(
        service::action(&args(&["status"])),
        Some(service::Action::Status)
    );
    for wrong in [
        &[][..],
        &["--help"],
        &["help"],
        &["-h"],
        &["Install"],
        &["install", "now"],
        &["status", "--help"],
    ] {
        assert_eq!(service::action(&args(wrong)), None, "{wrong:?}");
    }
}

#[test]
fn plist_runs_the_host_with_no_arguments() {
    let text = service::plist(
        Path::new("/Applications/Terminal.app/Contents/MacOS/term-host"),
        Path::new("/Users/me/Library/Application Support/terminal-app"),
    );
    let start = text
        .find("<key>ProgramArguments</key>")
        .expect("the arguments");
    let array = &text[start..start + text[start..].find("</array>").expect("their end")];
    assert_eq!(array.matches("<string>").count(), 1, "{array}");
}

#[test]
fn plist_has_keepalive_and_run_at_load() {
    let text = service::plist(
        Path::new("/Applications/Terminal.app/Contents/MacOS/term-host"),
        Path::new("/Users/me/Library/Application Support/terminal-app"),
    );
    assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
    assert!(text.contains("<key>Label</key>\n\t<string>app.terminal.host</string>"));
    assert!(text.contains("<key>KeepAlive</key>\n\t<true/>"));
    assert!(text.contains("<key>RunAtLoad</key>\n\t<true/>"));
    assert!(text.contains(
        "<key>ProgramArguments</key>\n\t<array>\n\t\t<string>/Applications/Terminal.app/Contents/MacOS/term-host</string>\n\t</array>"
    ));
    let log = "<string>/Users/me/Library/Application Support/terminal-app/logs/host.log</string>";
    assert!(text.contains(&format!("<key>StandardOutPath</key>\n\t{log}")));
    assert!(text.contains(&format!("<key>StandardErrorPath</key>\n\t{log}")));
    assert!(text.trim_end().ends_with("</plist>"));
}

#[test]
fn plist_escapes_paths_with_spaces_and_ampersands() {
    let text = service::plist(
        Path::new("/Apps/Tom & Jerry's <\"best\">/term-host"),
        Path::new("/data/R&D <x>"),
    );
    assert!(text.contains(
        "<string>/Apps/Tom &amp; Jerry&apos;s &lt;&quot;best&quot;&gt;/term-host</string>"
    ));
    assert!(text.contains("<string>/data/R&amp;D &lt;x&gt;/logs/host.log</string>"));
    assert!(!text.contains("Tom & Jerry"));
    assert!(!text.contains("R&D"));
    // Reads back as the path it was given.
    assert_eq!(
        service::plist_exe(&text).as_deref(),
        Some("/Apps/Tom & Jerry's <\"best\">/term-host")
    );
}

#[test]
fn plist_path_is_in_the_launch_agents_folder() {
    assert_eq!(
        service::plist_path(Path::new("/Users/me")),
        PathBuf::from("/Users/me/Library/LaunchAgents/app.terminal.host.plist")
    );
}

#[test]
fn install_writes_the_plist_and_bootstraps_it() {
    let sandbox = Sandbox::new();
    let service = sandbox.service();
    let exe = sandbox.exe("term-host");
    let mut launchctl = FakeLaunchctl::default();

    let installed = service.install(&exe, &mut launchctl).expect("install");

    assert_eq!(installed, Installed::Written);
    let path = service.plist_path();
    assert_eq!(
        launchctl.calls,
        [format!("print {TARGET}"), bootstrap_call(&path)]
    );
    let written = fs::read_to_string(&path).expect("read the plist");
    assert_eq!(written, service::plist(&exe, &service.data_dir));
    assert!(
        service.data_dir.join("logs").is_dir(),
        "launchd does not make the log folder"
    );
    let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        leftovers,
        ["app.terminal.host.plist"],
        "no temp file is left"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }
}

#[test]
fn install_with_the_same_exe_does_not_reload() {
    let sandbox = Sandbox::new();
    let service = sandbox.service();
    let exe = sandbox.exe("term-host");
    let mut launchctl = FakeLaunchctl::default();
    service
        .install(&exe, &mut launchctl)
        .expect("first install");
    launchctl.calls.clear();

    let installed = service
        .install(&exe, &mut launchctl)
        .expect("second install");

    assert_eq!(installed, Installed::Unchanged);
    assert_eq!(launchctl.calls, [format!("print {TARGET}")]);
}

#[test]
fn install_loads_an_unchanged_plist_that_launchd_lost() {
    let sandbox = Sandbox::new();
    let service = sandbox.service();
    let exe = sandbox.exe("term-host");
    let mut launchctl = FakeLaunchctl::default();
    service
        .install(&exe, &mut launchctl)
        .expect("first install");
    launchctl.loaded = false;
    launchctl.calls.clear();

    let installed = service
        .install(&exe, &mut launchctl)
        .expect("second install");

    assert_eq!(installed, Installed::Loaded);
    assert_eq!(
        launchctl.calls,
        [
            format!("print {TARGET}"),
            bootstrap_call(&service.plist_path())
        ]
    );
    assert!(launchctl.loaded);
}

#[test]
fn install_with_another_exe_boots_out_then_bootstraps() {
    let sandbox = Sandbox::new();
    let service = sandbox.service();
    let mut launchctl = FakeLaunchctl::default();
    service
        .install(&sandbox.exe("term-host"), &mut launchctl)
        .expect("first install");
    launchctl.calls.clear();
    let moved = sandbox.0.join("Moved.app/Contents/MacOS/term-host");

    let installed = service
        .install(&moved, &mut launchctl)
        .expect("second install");

    assert_eq!(installed, Installed::Written);
    assert_eq!(
        launchctl.calls,
        [
            format!("print {TARGET}"),
            format!("bootout {TARGET}"),
            bootstrap_call(&service.plist_path()),
        ]
    );
    let written = fs::read_to_string(service.plist_path()).unwrap();
    assert_eq!(written, service::plist(&moved, &service.data_dir));
}

#[test]
fn install_retries_a_bootstrap_that_fails_right_after_a_bootout() {
    let sandbox = Sandbox::new();
    let service = sandbox.service();
    let mut launchctl = FakeLaunchctl {
        bootstrap_failures: 2,
        ..FakeLaunchctl::default()
    };

    service
        .install(&sandbox.exe("term-host"), &mut launchctl)
        .expect("install");

    assert_eq!(launchctl.pauses, 2);
    assert!(launchctl.loaded);
}

#[test]
fn install_reports_a_bootstrap_that_keeps_failing() {
    let sandbox = Sandbox::new();
    let service = sandbox.service();
    let mut launchctl = FakeLaunchctl {
        bootstrap_failures: u32::MAX,
        ..FakeLaunchctl::default()
    };

    let error = service
        .install(&sandbox.exe("term-host"), &mut launchctl)
        .expect_err("install fails");

    let message = format!("{error:#}");
    assert!(message.contains("Bootstrap failed: 5"), "{message}");
    assert!(launchctl.calls.len() > 2, "it tried more than once");
}

#[test]
fn uninstall_boots_out_and_removes_the_file() {
    let sandbox = Sandbox::new();
    let service = sandbox.service();
    let mut launchctl = FakeLaunchctl::default();
    service
        .install(&sandbox.exe("term-host"), &mut launchctl)
        .expect("install");
    launchctl.calls.clear();

    let uninstalled = service.uninstall(&mut launchctl).expect("uninstall");

    assert_eq!(uninstalled, Uninstalled::Removed);
    assert_eq!(
        launchctl.calls,
        [format!("print {TARGET}"), format!("bootout {TARGET}")]
    );
    assert!(!service.plist_path().exists());
    assert!(!launchctl.loaded);
}

#[test]
fn uninstall_with_nothing_installed_succeeds() {
    let sandbox = Sandbox::new();
    let service = sandbox.service();
    let mut launchctl = FakeLaunchctl::default();

    let uninstalled = service.uninstall(&mut launchctl).expect("uninstall");

    assert_eq!(uninstalled, Uninstalled::NothingInstalled);
    assert_eq!(launchctl.calls, [format!("print {TARGET}")]);
}

#[test]
fn status_reports_the_file_its_exe_and_whether_it_is_loaded() {
    let sandbox = Sandbox::new();
    let service = sandbox.service();
    let exe = sandbox.exe("term-host");
    let mut launchctl = FakeLaunchctl::default();

    let before = service.status(&mut launchctl).expect("status");
    assert!(!before.present);
    assert_eq!(before.exe, None);
    assert!(!before.loaded);
    assert_eq!(before.plist, service.plist_path());

    service.install(&exe, &mut launchctl).expect("install");
    let after = service.status(&mut launchctl).expect("status");
    assert!(after.present);
    assert_eq!(after.exe.as_deref(), Some(exe.to_str().unwrap()));
    assert!(after.loaded);
    let printed = after.to_string();
    assert!(printed.contains(exe.to_str().unwrap()), "{printed}");
    assert!(printed.contains("launchd: loaded"), "{printed}");
    assert!(before.to_string().contains("launchd: not loaded"));
}

#[test]
fn install_refuses_a_custom_port_or_data_folder() {
    assert!(service::check_env(None, None).is_ok());
    assert!(service::check_env(Some("50000".into()), None).is_err());
    assert!(service::check_env(None, Some("/tmp/x".into())).is_err());
    // An empty TERM_APP_DATA means the default, as in paths.rs.
    assert!(service::check_env(None, Some("".into())).is_ok());
}
