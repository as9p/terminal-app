//! Finds the folder reports a shell hook prints: `ESC ] 7 ; file://<host><path>`, ended by BEL
//! or `ESC \`, with the path percent-encoded. Only reads the output, never changes it.

use std::path::PathBuf;
use std::sync::OnceLock;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
// Longer than any real report, so a program that never ends its sequence costs no memory.
const MAX_OSC: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OscEvent {
    Cwd { host: String, path: PathBuf },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    Osc,
    /// An `ESC` inside a sequence, which `\` makes its end.
    OscEsc,
    /// Over `MAX_OSC`, so the rest of the sequence is skipped.
    TooLong,
    TooLongEsc,
}

/// Keeps its place between calls, so a report split across reads is still found.
pub struct Scanner {
    state: State,
    osc: Vec<u8>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner {
    pub fn new() -> Scanner {
        Scanner {
            state: State::Ground,
            osc: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        let mut events = Vec::new();
        for &byte in bytes {
            self.state = match (self.state, byte) {
                (State::Osc, BEL) | (State::OscEsc, b'\\') => {
                    events.extend(parse(&self.osc));
                    State::Ground
                }
                (State::Osc, ESC) => State::OscEsc,
                (State::Osc, _) if self.osc.len() >= MAX_OSC => State::TooLong,
                (State::Osc, _) => {
                    self.osc.push(byte);
                    State::Osc
                }
                (State::TooLong, BEL) | (State::TooLongEsc, b'\\') => State::Ground,
                (State::TooLong, ESC) => State::TooLongEsc,
                (State::TooLong, _) => State::TooLong,
                // Any other `ESC` sequence ends an unfinished one, and may start the next.
                (State::Esc | State::OscEsc | State::TooLongEsc, b']') => {
                    self.osc.clear();
                    State::Osc
                }
                (_, ESC) => State::Esc,
                _ => State::Ground,
            };
        }
        events
    }
}

/// Only a well formed folder report gives an event.
fn parse(osc: &[u8]) -> Option<OscEvent> {
    let rest = osc.strip_prefix(b"7;file://")?;
    let slash = rest.iter().position(|&byte| byte == b'/')?;
    let host = String::from_utf8(rest[..slash].to_vec()).ok()?;
    // Skipped when not UTF-8, since the state file could not hold it.
    let path = String::from_utf8(percent_decode(&rest[slash..])?).ok()?;
    Some(OscEvent::Cwd {
        host,
        path: native_path(path),
    })
}

fn percent_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(text.len());
    let mut bytes = text.iter();
    while let Some(&byte) = bytes.next() {
        if byte == b'%' {
            let high = (*bytes.next()? as char).to_digit(16)?;
            let low = (*bytes.next()? as char).to_digit(16)?;
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    Some(decoded)
}

#[cfg(not(windows))]
fn native_path(path: String) -> PathBuf {
    PathBuf::from(path)
}

/// `/C:/Users/x` becomes `C:\Users\x`.
#[cfg(windows)]
fn native_path(path: String) -> PathBuf {
    let bytes = path.as_bytes();
    let path = if bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b':' {
        &path[1..]
    } else {
        &path
    };
    PathBuf::from(path.replace('/', "\\"))
}

/// Whether a report's host part names this computer. An empty one does too, since the hook
/// may not know its name.
pub(crate) fn is_this_host(host: &str) -> bool {
    host.is_empty() || this_host().is_some_and(|name| name.eq_ignore_ascii_case(host))
}

fn this_host() -> Option<&'static str> {
    static NAME: OnceLock<Option<String>> = OnceLock::new();
    NAME.get_or_init(host_name).as_deref()
}

/// The name a shell's `$HOST` and `$HOSTNAME` hold.
#[cfg(unix)]
fn host_name() -> Option<String> {
    let mut buffer = [0u8; 256];
    let failed = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } != 0;
    if failed {
        return None;
    }
    let end = buffer.iter().position(|&byte| byte == 0)?;
    String::from_utf8(buffer[..end].to_vec()).ok()
}

/// The name PowerShell's `$env:COMPUTERNAME` holds.
#[cfg(not(unix))]
fn host_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd(host: &str, path: &str) -> OscEvent {
        OscEvent::Cwd {
            host: host.to_string(),
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn finds_cwd_split_across_feeds() {
        let mut scanner = Scanner::new();
        let mut events = Vec::new();
        for byte in b"before\x1b]7;file://mac/Users/x\x07after" {
            events.extend(scanner.feed(&[*byte]));
        }
        assert_eq!(events, [cwd("mac", "/Users/x")]);
    }

    #[cfg(unix)]
    #[test]
    fn decodes_percent_and_unicode() {
        let mut scanner = Scanner::new();
        let events =
            scanner.feed(b"\x1b]7;file://mac/Users/x/My%20Stuff/%E6%97%A5%E6%9C%AC/50%25%23\x07");
        assert_eq!(events, [cwd("mac", "/Users/x/My Stuff/日本/50%#")]);
    }

    #[cfg(unix)]
    #[test]
    fn accepts_bel_and_st_endings() {
        let mut scanner = Scanner::new();
        let events = scanner.feed(b"\x1b]7;file://mac/a\x07\x1b]7;file:///b\x1b\\");
        assert_eq!(events, [cwd("mac", "/a"), cwd("", "/b")]);
    }

    #[cfg(unix)]
    #[test]
    fn ignores_other_osc_and_oversized() {
        let mut scanner = Scanner::new();
        assert_eq!(scanner.feed(b"\x1b]0;a title\x07\x1b]2;file:///x\x07"), []);

        let mut oversized = b"\x1b]7;file:///".to_vec();
        oversized.extend(std::iter::repeat_n(b'a', 5000));
        assert_eq!(scanner.feed(&oversized), []);
        assert_eq!(scanner.feed(b"\x07"), []);
        // Still reads the next one.
        assert_eq!(scanner.feed(b"\x1b]7;file:///c\x07"), [cwd("", "/c")]);
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_path() {
        let mut scanner = Scanner::new();
        let events = scanner.feed(b"\x1b]7;file://pc/C:/Users/x\x07");
        assert_eq!(events, [cwd("pc", r"C:\Users\x")]);
    }
}
