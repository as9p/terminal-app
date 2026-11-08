use std::ffi::OsString;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use bytes::Bytes;
use portable_pty::{
    Child, ChildKiller, CommandBuilder, MasterPty, PtyPair, PtySize, native_pty_system,
};

use crate::session::{Kind, SpawnSpec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl From<Size> for PtySize {
    fn from(size: Size) -> Self {
        PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Exit {
    pub clean: bool,
    pub signal: Option<i32>,
}

pub(crate) type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

pub(crate) struct Pty {
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: SharedWriter,
    pub child: Box<dyn Child + Send + Sync>,
    pub killer: Box<dyn ChildKiller + Send + Sync>,
}

/// When terminals open at the same moment, even from other apps, the macOS kernel can
/// hand back its internal "raced, open again" code (EREDRIVEOPEN, -6) once its own
/// retries run out. Opening again is what that code asks for. portable-pty only
/// reports it as text, so this matches the text.
fn open_pty(size: Size) -> Result<PtyPair> {
    const RETRIES: u64 = 10;

    let system = native_pty_system();
    let mut retry = 0;
    loop {
        match system.openpty(size.into()) {
            Err(err) if retry < RETRIES && is_open_race(&err.to_string()) => {
                retry += 1;
                std::thread::sleep(Duration::from_millis(retry));
            }
            result => return result,
        }
    }
}

fn is_open_race(message: &str) -> bool {
    message.contains("code: -6,")
}

/// A macOS fork child runs libnotify's after-fork step, which does run-once setup if the
/// parent never did. A fork can catch that gate held by a parent thread that the child
/// does not have, and the child then dies with SIGKILL before it reaches exec. Running
/// the setup before any fork leaves the child nothing to wait on.
#[cfg(target_os = "macos")]
fn prepare_for_fork() {
    use std::sync::Once;

    unsafe extern "C" {
        fn notify_is_valid_token(token: std::os::raw::c_int) -> bool;
    }
    static PREPARED: Once = Once::new();
    PREPARED.call_once(|| {
        unsafe { notify_is_valid_token(0) };
    });
}

#[cfg(not(target_os = "macos"))]
fn prepare_for_fork() {}

pub(crate) fn spawn(spec: &SpawnSpec, size: Size, env: Vec<(String, String)>) -> Result<Pty> {
    prepare_for_fork();
    let pair = open_pty(size)?;
    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;

    let mut command = CommandBuilder::new_default_prog();
    command.env_clear();
    for (key, value) in env {
        command.env(key, value);
    }
    command.cwd(&spec.cwd);
    // get_shell reads SHELL or COMSPEC from the environment set above, and on Unix
    // falls back to the account's shell when a bare environment has no SHELL.
    let shell = command.get_shell();
    *command.get_argv_mut() = argv(&spec.kind, &shell, cfg!(windows))
        .into_iter()
        .map(OsString::from)
        .collect();

    let child = pair.slave.spawn_command(command)?;
    let killer = child.clone_killer();
    // The reader only sees end of file once every copy of the slave side is closed.
    drop(pair.slave);

    Ok(Pty {
        master: pair.master,
        reader,
        writer: Arc::new(Mutex::new(writer)),
        child,
        killer,
    })
}

fn argv(kind: &Kind, shell: &str, windows: bool) -> Vec<String> {
    let (flag, id) = match kind {
        Kind::Shell if windows => return vec![shell.to_string()],
        Kind::Shell => return vec![shell.to_string(), "-l".to_string()],
        Kind::Claude { chat, resume: true } => ("--resume", chat),
        Kind::Claude {
            chat,
            resume: false,
        } => ("--session-id", chat),
    };
    if windows {
        ["cmd", "/c", "claude", flag, &id.to_string()]
            .map(String::from)
            .to_vec()
    } else {
        vec![
            shell.to_string(),
            "-lic".to_string(),
            format!("exec claude {flag} {id}"),
        ]
    }
}

/// Blocks until the program ends.
#[cfg(unix)]
pub(crate) fn wait_for_exit(child: &mut (dyn Child + Send + Sync)) -> Exit {
    use std::os::unix::process::ExitStatusExt;

    // portable-pty only reports a signal as strsignal text such as "Hangup", so
    // this reaps the pid directly to get the number.
    let unknown = Exit {
        clean: false,
        signal: None,
    };
    let Some(pid) = child.process_id() else {
        return unknown;
    };
    let mut status = 0;
    loop {
        let reaped = unsafe { libc::waitpid(pid as i32, &mut status, 0) };
        if reaped == pid as i32 {
            break;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return unknown;
        }
    }
    let signal = std::process::ExitStatus::from_raw(status).signal();
    Exit {
        clean: signal.is_none(),
        signal,
    }
}

/// Blocks until the program ends.
#[cfg(not(unix))]
pub(crate) fn wait_for_exit(child: &mut (dyn Child + Send + Sync)) -> Exit {
    Exit {
        clean: child.wait().is_ok(),
        signal: None,
    }
}

#[cfg(unix)]
pub(crate) fn output_filter(_writer: &SharedWriter) -> impl FnMut(Bytes) -> Bytes + Send + use<> {
    |chunk| chunk
}

/// ConPTY asks for the cursor position once at startup and stalls until it gets a
/// reply. A session may have no client yet, and a client's terminal would answer a
/// second time into the program's input, so the host answers and hides the query.
#[cfg(windows)]
pub(crate) fn output_filter(writer: &SharedWriter) -> impl FnMut(Bytes) -> Bytes + Send + use<> {
    const CURSOR_REPLY: &[u8] = b"\x1b[1;1R";
    // The query comes before any program output. Past this, a query belongs to the
    // program and must reach the client.
    const STARTUP_WINDOW: usize = 256;

    let writer = writer.clone();
    let mut seen = 0;
    move |chunk| {
        if seen >= STARTUP_WINDOW {
            return chunk;
        }
        seen += chunk.len();
        let Some(stripped) = strip_cursor_query(&chunk) else {
            return chunk;
        };
        seen = STARTUP_WINDOW;
        let mut writer = writer.lock().unwrap_or_else(|err| err.into_inner());
        let _ = writer.write_all(CURSOR_REPLY).and_then(|()| writer.flush());
        stripped
    }
}

#[cfg(any(windows, test))]
fn strip_cursor_query(chunk: &[u8]) -> Option<Bytes> {
    const CURSOR_QUERY: &[u8] = b"\x1b[6n";
    let at = chunk
        .windows(CURSOR_QUERY.len())
        .position(|window| window == CURSOR_QUERY)?;
    let mut stripped = Vec::with_capacity(chunk.len() - CURSOR_QUERY.len());
    stripped.extend_from_slice(&chunk[..at]);
    stripped.extend_from_slice(&chunk[at + CURSOR_QUERY.len()..]);
    Some(stripped.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const CHAT: Uuid = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef);
    const CHAT_TEXT: &str = "01234567-89ab-cdef-0123-456789abcdef";

    fn claude(resume: bool) -> Kind {
        Kind::Claude { chat: CHAT, resume }
    }

    #[test]
    fn only_the_kernel_race_counts_as_an_open_race() {
        let race = std::io::Error::from_raw_os_error(-6);
        assert!(is_open_race(&format!("failed to openpty: {race:?}")));
        let out_of_ptys = std::io::Error::from_raw_os_error(6);
        assert!(!is_open_race(&format!(
            "failed to openpty: {out_of_ptys:?}"
        )));
    }

    #[test]
    fn shell_argv() {
        assert_eq!(argv(&Kind::Shell, "/bin/zsh", false), ["/bin/zsh", "-l"]);
        assert_eq!(argv(&Kind::Shell, "cmd.exe", true), ["cmd.exe"]);
    }

    #[test]
    fn claude_argv_on_unix() {
        assert_eq!(
            argv(&claude(false), "/bin/zsh", false),
            [
                "/bin/zsh",
                "-lic",
                &format!("exec claude --session-id {CHAT_TEXT}")
            ]
        );
        assert_eq!(
            argv(&claude(true), "/bin/zsh", false),
            [
                "/bin/zsh",
                "-lic",
                &format!("exec claude --resume {CHAT_TEXT}")
            ]
        );
    }

    #[test]
    fn claude_argv_on_windows() {
        assert_eq!(
            argv(&claude(false), "cmd.exe", true),
            ["cmd", "/c", "claude", "--session-id", CHAT_TEXT]
        );
        assert_eq!(
            argv(&claude(true), "cmd.exe", true),
            ["cmd", "/c", "claude", "--resume", CHAT_TEXT]
        );
    }

    #[test]
    fn cursor_query_is_stripped() {
        assert_eq!(
            strip_cursor_query(b"\x1b[?9001h\x1b[6nrest").as_deref(),
            Some(&b"\x1b[?9001hrest"[..])
        );
        assert_eq!(strip_cursor_query(b"\x1b[6n").as_deref(), Some(&b""[..]));
        assert_eq!(strip_cursor_query(b"no query"), None);
    }
}
