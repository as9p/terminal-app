//! Finds the host on its port, or starts it so that it outlives this app.

use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;

// Source of truth for these rules: host/src/paths.rs and host/src/auth.rs. The host's library
// also brings its server, PTY and screen code, so a small copy beats a dependency on it.
const DEFAULT_PORT: u16 = 47211;
const APP_FOLDER: &str = "terminal-app";
const TOKEN_FILE: &str = "host-token";
const TOKEN_HEX_LEN: usize = 64;

const START_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(200);

/// Two calls at once, say a page reload during a start, must not both see a silent port.
static START_LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize)]
pub struct HostConnection {
    port: u16,
    token: String,
    /// The page needs this for the `cwd` of a new session.
    home: String,
}

pub fn find_or_start() -> Result<HostConnection, String> {
    let _guard = START_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let home = dirs::home_dir()
        .ok_or("This computer has no home folder")?
        .to_string_lossy()
        .into_owned();
    let port = port_from(std::env::var("TERM_HOST_PORT").ok())?;
    let data_dir = data_dir_from(std::env::var_os("TERM_APP_DATA"), dirs::data_dir())?;
    let token_path = data_dir.join(TOKEN_FILE);

    let mut started = if answers(port) {
        None
    } else {
        Some(start_host()?)
    };
    let token = wait_until_ready(port, &token_path, started.as_mut());
    if let Some(mut host) = started {
        // Collects its exit status if it ends while the app runs. The app quitting does not
        // touch it: nothing here ever signals the host.
        std::thread::spawn(move || host.wait());
    }
    Ok(HostConnection {
        port,
        token: token?,
        home,
    })
}

fn port_from(value: Option<String>) -> Result<u16, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_PORT);
    };
    match value.parse() {
        Ok(0) => Err(
            "TERM_HOST_PORT is 0, and the app could not find a port the system picked".to_string(),
        ),
        Ok(port) => Ok(port),
        Err(_) => Err(format!("TERM_HOST_PORT is not a port: {value:?}")),
    }
}

fn data_dir_from(value: Option<OsString>, system_dir: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(dir) = value.filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let base = system_dir.ok_or("This computer has no data folder")?;
    Ok(base.join(APP_FOLDER))
}

fn answers(port: u16) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).is_ok()
}

/// Both builds keep the host beside this program, and it is never looked up on PATH. A bundle
/// has it in Contents/MacOS through bundle.externalBin, and a dev build runs from target/debug,
/// where cargo puts term-host.
fn host_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("Cannot find where this app is running from: {error}"))?;
    let name = format!("term-host{}", std::env::consts::EXE_SUFFIX);
    let path = exe.with_file_name(name);
    if !path.is_file() {
        return Err(format!(
            "The host program is missing at {}. In a dev build, run: cargo build -p term-host",
            path.display()
        ));
    }
    Ok(path)
}

fn start_host() -> Result<Child, String> {
    let binary = host_binary()?;
    let mut command = Command::new(&binary);
    command
        // The host runs for days, so it must not hold on to the folder the app started in.
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Signals sent to the app's process group, such as Ctrl+C in a shell, must not reach it.
        command.process_group(0);
    }
    command
        .spawn()
        .map_err(|error| format!("Cannot start the host at {}: {error}", binary.display()))
}

fn wait_until_ready(
    port: u16,
    token_path: &Path,
    mut started: Option<&mut Child>,
) -> Result<String, String> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        let port_answers = answers(port);
        if port_answers && let Some(token) = read_token(token_path) {
            return Ok(token);
        }
        // Only when the port is silent: losing the race to a host from another app is fine.
        if !port_answers
            && let Some(host) = started.as_deref_mut()
            && let Ok(Some(status)) = host.try_wait()
        {
            return Err(format!(
                "The host exited with {status} before it answered on 127.0.0.1:{port}"
            ));
        }
        if Instant::now() >= deadline {
            return Err(timeout_message(port, port_answers, token_path));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// The host listens before it writes the token, and it replaces a short file left by a crash,
/// so a short or odd token here means "not ready yet", not a token to use.
fn read_token(path: &Path) -> Option<String> {
    let token = std::fs::read_to_string(path).ok()?;
    let whole = token.len() == TOKEN_HEX_LEN && token.bytes().all(|b| b.is_ascii_hexdigit());
    whole.then_some(token)
}

fn timeout_message(port: u16, port_answers: bool, token_path: &Path) -> String {
    let seconds = START_TIMEOUT.as_secs();
    if port_answers {
        format!(
            "Something answers on 127.0.0.1:{port}, but after {seconds} s there is no usable token at {}. Another program may be using the port.",
            token_path.display()
        )
    } else {
        format!("The host did not answer on 127.0.0.1:{port} within {seconds} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_and_data_folder_follow_the_host_rules() {
        assert_eq!(port_from(None), Ok(47211));
        assert_eq!(port_from(Some("50000".to_string())), Ok(50000));
        assert!(port_from(Some("0".to_string())).is_err());
        assert!(port_from(Some(String::new())).is_err());
        assert!(port_from(Some("70000".to_string())).is_err());

        let system = Some(PathBuf::from("/system"));
        assert_eq!(
            data_dir_from(Some("/chosen".into()), system.clone()),
            Ok(PathBuf::from("/chosen"))
        );
        assert_eq!(
            data_dir_from(Some("".into()), system.clone()),
            Ok(PathBuf::from("/system/terminal-app"))
        );
        assert_eq!(
            data_dir_from(None, system),
            Ok(PathBuf::from("/system/terminal-app"))
        );
        assert!(data_dir_from(None, None).is_err());
    }
}
