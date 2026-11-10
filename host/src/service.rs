//! `term-host service install|uninstall|status`, which runs the host as a launchd agent on
//! macOS, so it starts at login and launchd starts it again whenever it exits. The plist lives
//! at `~/Library/LaunchAgents/app.terminal.host.plist`. Every `launchctl` call goes through
//! [`Launchctl`], so the logic here can be tested without touching launchd.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};

pub const LABEL: &str = "app.terminal.host";

/// Bootstraps tried before giving up. Right after a bootout, launchd can refuse a bootstrap
/// for a moment while the old job finishes exiting.
const BOOTSTRAP_TRIES: u32 = 5;
const BOOTSTRAP_RETRY_WAIT: Duration = Duration::from_millis(300);

/// The agent's plist. Both launchd and the host write to one log under the data folder.
pub fn plist(exe: &Path, data_dir: &Path) -> String {
    let exe = xml_escape(&exe.to_string_lossy());
    let log = xml_escape(&log_path(data_dir).to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{exe}</string>
	</array>
	<key>WorkingDirectory</key>
	<string>/</string>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#
    )
}

/// Where the plist goes for a user with this home folder.
pub fn plist_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("host.log")
}

/// The program a plist written by [`plist`] runs: the first string after `ProgramArguments`.
pub fn plist_exe(text: &str) -> Option<String> {
    let after = &text[text.find("<key>ProgramArguments</key>")?..];
    let start = after.find("<string>")? + "<string>".len();
    let length = after[start..].find("</string>")?;
    Some(xml_unescape(&after[start..start + length]))
}

fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// Undoes [`xml_escape`]. `&amp;` goes last, so `&amp;lt;` reads back as `&lt;`.
fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The service runs with launchd's environment, not the caller's, so it would not see a
/// custom port or data folder, and a client using them would wait for a host that never comes.
pub fn check_env(port: Option<String>, data_dir: Option<OsString>) -> Result<()> {
    if port.is_some() {
        bail!("TERM_HOST_PORT is set, and the service would not see it. Unset it to install");
    }
    if data_dir.is_some_and(|dir| !dir.is_empty()) {
        bail!("TERM_APP_DATA is set, and the service would not see it. Unset it to install");
    }
    Ok(())
}

/// What one `launchctl` call returned.
pub struct Ran {
    pub success: bool,
    /// Its stdout and stderr together, for an error message.
    pub output: String,
}

/// Runs `launchctl`. Tests use a fake that records the calls.
pub trait Launchctl {
    fn run(&mut self, args: &[String]) -> io::Result<Ran>;

    /// The wait between two bootstrap tries.
    fn pause(&mut self) {
        std::thread::sleep(BOOTSTRAP_RETRY_WAIT);
    }
}

/// The real `launchctl`.
pub struct SystemLaunchctl;

impl Launchctl for SystemLaunchctl {
    fn run(&mut self, args: &[String]) -> io::Result<Ran> {
        let output = std::process::Command::new("/bin/launchctl")
            .args(args)
            .stdin(std::process::Stdio::null())
            .output()?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        Ok(Ran {
            success: output.status.success(),
            output: text.trim().to_string(),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Installed {
    /// The plist was already this one and launchd had it loaded.
    Unchanged,
    /// The plist was already this one, and launchd has now loaded it.
    Loaded,
    /// The plist was written, replacing any other, and launchd has loaded it.
    Written,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Uninstalled {
    Removed,
    NothingInstalled,
}

pub struct Status {
    pub plist: PathBuf,
    pub present: bool,
    /// The program the plist runs, if there is a plist and it names one.
    pub exe: Option<String>,
    /// Whether launchd has the job loaded.
    pub loaded: bool,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let present = if self.present { "present" } else { "missing" };
        writeln!(f, "plist: {} ({present})", self.plist.display())?;
        writeln!(f, "exe: {}", self.exe.as_deref().unwrap_or("none"))?;
        let loaded = if self.loaded { "loaded" } else { "not loaded" };
        write!(f, "launchd: {loaded}")
    }
}

/// The service for one user.
pub struct Service {
    pub home: PathBuf,
    pub data_dir: PathBuf,
    pub uid: u32,
}

impl Service {
    pub fn plist_path(&self) -> PathBuf {
        plist_path(&self.home)
    }

    fn domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn target(&self) -> String {
        format!("gui/{}/{LABEL}", self.uid)
    }

    /// Writes the plist for `exe` and has launchd load it. A plist that is already this one is
    /// left alone, so a running host is not restarted; another one is booted out first.
    pub fn install(&self, exe: &Path, launchctl: &mut impl Launchctl) -> Result<Installed> {
        let path = self.plist_path();
        let text = plist(exe, &self.data_dir);
        // launchd opens the log but does not make its folder.
        let logs = log_path(&self.data_dir);
        let logs = logs.parent().expect("the log has a folder");
        fs::create_dir_all(logs).with_context(|| format!("cannot make {}", logs.display()))?;

        let same = match fs::read_to_string(&path) {
            Ok(old) => old == text,
            Err(error) if error.kind() == ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        let loaded = self.loaded(launchctl)?;
        if same && loaded {
            return Ok(Installed::Unchanged);
        }
        if same {
            self.bootstrap(&path, launchctl)?;
            return Ok(Installed::Loaded);
        }
        if loaded {
            self.bootout(launchctl)?;
        }
        write_plist(&path, &text).with_context(|| format!("cannot write {}", path.display()))?;
        self.bootstrap(&path, launchctl)?;
        Ok(Installed::Written)
    }

    /// Boots the job out and removes the plist. With nothing installed it does nothing.
    pub fn uninstall(&self, launchctl: &mut impl Launchctl) -> Result<Uninstalled> {
        let path = self.plist_path();
        let loaded = self.loaded(launchctl)?;
        if loaded {
            self.bootout(launchctl)?;
        }
        let removed = match fs::remove_file(&path) {
            Ok(()) => true,
            Err(error) if error.kind() == ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error).with_context(|| format!("cannot remove {}", path.display()));
            }
        };
        Ok(if loaded || removed {
            Uninstalled::Removed
        } else {
            Uninstalled::NothingInstalled
        })
    }

    pub fn status(&self, launchctl: &mut impl Launchctl) -> Result<Status> {
        let path = self.plist_path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", path.display()));
            }
        };
        Ok(Status {
            present: text.is_some(),
            exe: text.as_deref().and_then(plist_exe),
            loaded: self.loaded(launchctl)?,
            plist: path,
        })
    }

    fn loaded(&self, launchctl: &mut impl Launchctl) -> Result<bool> {
        let ran = run(launchctl, &["print".to_string(), self.target()])?;
        Ok(ran.success)
    }

    fn bootout(&self, launchctl: &mut impl Launchctl) -> Result<()> {
        let ran = run(launchctl, &["bootout".to_string(), self.target()])?;
        if !ran.success {
            bail!("launchctl bootout {} failed: {}", self.target(), ran.output);
        }
        Ok(())
    }

    fn bootstrap(&self, path: &Path, launchctl: &mut impl Launchctl) -> Result<()> {
        let args = [
            "bootstrap".to_string(),
            self.domain(),
            path.to_string_lossy().into_owned(),
        ];
        let mut tries = 0;
        loop {
            let ran = run(launchctl, &args)?;
            if ran.success {
                return Ok(());
            }
            tries += 1;
            if tries == BOOTSTRAP_TRIES {
                bail!(
                    "launchctl bootstrap {} failed {tries} times: {}",
                    self.domain(),
                    ran.output
                );
            }
            launchctl.pause();
        }
    }
}

fn run(launchctl: &mut impl Launchctl, args: &[String]) -> Result<Ran> {
    launchctl
        .run(args)
        .with_context(|| format!("cannot run launchctl {}", args.join(" ")))
}

/// Written beside it and renamed, so launchd never reads half a plist. Mode 0644: launchd
/// refuses an agent that others can write.
fn write_plist(path: &Path, text: &str) -> io::Result<()> {
    let folder = path.parent().expect("the plist has a folder");
    fs::create_dir_all(folder)?;
    let temp = path.with_extension("plist.tmp");
    match fs::remove_file(&temp) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o644);
    }
    let mut file = options.open(&temp)?;
    file.write_all(text.as_bytes())?;
    // The mode given to open is cut by the umask.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path)
}

/// Runs `term-host service <action>` and returns the exit code.
pub fn main(action: Option<OsString>) -> ExitCode {
    match cli(action.as_deref().map(|action| action.to_string_lossy())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("term-host service: {error:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(target_os = "macos")]
fn cli(action: Option<std::borrow::Cow<'_, str>>) -> Result<()> {
    use anyhow::anyhow;

    let home = dirs::home_dir().ok_or_else(|| anyhow!("this computer has no home folder"))?;
    let service = Service {
        home,
        data_dir: crate::paths::data_dir()?,
        // SAFETY: getuid has no preconditions and cannot fail.
        uid: unsafe { libc::getuid() },
    };
    let mut launchctl = SystemLaunchctl;
    match action.as_deref() {
        Some("install") => {
            check_env(
                std::env::var("TERM_HOST_PORT").ok(),
                std::env::var_os("TERM_APP_DATA"),
            )?;
            let exe = std::env::current_exe().context("cannot find this program")?;
            // The same path the app compares against, with no symlink or `..` in it.
            let exe = fs::canonicalize(&exe)
                .with_context(|| format!("cannot resolve {}", exe.display()))?;
            match service.install(&exe, &mut launchctl)? {
                Installed::Unchanged => println!("Already installed for {}", exe.display()),
                Installed::Loaded => println!("Loaded {}", service.plist_path().display()),
                Installed::Written => println!("Installed {}", service.plist_path().display()),
            }
        }
        Some("uninstall") => {
            if service.uninstall(&mut launchctl)? == Uninstalled::Removed {
                println!("Removed {}", service.plist_path().display());
            }
        }
        Some("status") => println!("{}", service.status(&mut launchctl)?),
        _ => bail!("usage: term-host service install|uninstall|status"),
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn cli(_action: Option<std::borrow::Cow<'_, str>>) -> Result<()> {
    bail!("not supported on this system yet. Only macOS runs the host as a service for now")
}
