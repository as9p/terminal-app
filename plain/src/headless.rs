//! The windowless mode: the command's output feeds a session, keys are typed from a file at
//! their times, and the final screen is printed when the command exits.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use term_engine::{CellSize, GridSize, TerminalView};

use crate::backlog::{Backlog, CloseOnDrop, QUEUE_LIMIT, counted};
use crate::session::{self, Session};
use crate::shell::{Shell, ShellEvent};

/// There is no font without a window, but programs ask for the pixel size, so give them one.
const CELL: CellSize = CellSize { w: 8, h: 20 };
/// The exit code `timeout(1)` uses, so scripts can tell a stuck command from a failed one.
pub const TIMED_OUT: i32 = 124;

pub struct Options {
    pub grid: GridSize,
    pub keys: Vec<Key>,
    pub timeout: Option<Duration>,
    pub record: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    pub command: Option<Vec<String>>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Key {
    pub at: Duration,
    pub bytes: Vec<u8>,
}

/// Returns the process exit code.
pub fn run(options: Options) -> i32 {
    let (tx, events) = mpsc::channel();
    let backlog = Arc::new(Backlog::new(QUEUE_LIMIT));
    let on_event = counted(backlog.clone(), move |event| {
        // The receiver is gone only when this function has returned, and then nobody listens.
        let _ = tx.send(event);
    });
    if !session::can_record(options.record.as_deref()) {
        return 1;
    }
    let shell = match Shell::spawn(options.command, options.cwd, options.grid, on_event) {
        Ok(shell) => shell,
        Err(e) => {
            eprintln!("snowflake: cannot start the command: {e}");
            return 127;
        }
    };
    let mut session = match Session::new(shell, options.grid, options.record) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("snowflake: cannot write the recording: {e}");
            return 1;
        }
    };
    session.set_grid(options.grid, CELL);
    // Made after the session, so it drops first on every return below.
    let backlog = CloseOnDrop(backlog);

    let start = Instant::now();
    let deadline = options.timeout.map(|timeout| start + timeout);
    let mut keys = options.keys.into_iter().peekable();
    loop {
        let now = Instant::now();
        while let Some(key) = keys.next_if(|key| start + key.at <= now) {
            session.write(key.bytes);
        }
        if deadline.is_some_and(|deadline| now >= deadline) {
            session.kill();
            eprintln!("snowflake: the command did not exit in time and was killed");
            return TIMED_OUT;
        }
        let wake = keys
            .peek()
            .map(|key| start + key.at)
            .into_iter()
            .chain(deadline)
            .min();
        let event = match wake {
            Some(wake) => match events.recv_timeout(wake.saturating_duration_since(now)) {
                Ok(event) => event,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match events.recv() {
                Ok(event) => event,
                Err(_) => break,
            },
        };
        match event {
            ShellEvent::Output(bytes) => {
                session.output(&bytes);
                backlog.fed(bytes.len());
                // Titles and bells mean nothing without a window; drop them so they do not pile up.
                session.view.take_events();
            }
            ShellEvent::Exited(code) => {
                print_screen(&session.view);
                // Unix keeps the low 8 bits; Windows codes are full 32-bit values.
                return code as i32;
            }
        }
    }
    eprintln!("snowflake: lost the command without an exit code");
    1
}

/// One line per row, without the blank rows below the last text.
fn print_screen(view: &TerminalView) {
    let screen = view.screen();
    let mut rows: Vec<String> = (0..screen.size().rows)
        .map(|row| screen.row_text(row))
        .collect();
    while rows.last().is_some_and(|row| row.trim().is_empty()) {
        rows.pop();
    }
    let mut out = io::stdout().lock();
    for row in rows {
        // A closed stdout is the reader's choice, not a reason to change the exit code.
        if writeln!(out, "{row}").is_err() {
            return;
        }
    }
    let _ = out.flush();
}

/// Lines of `<milliseconds since start> <text>`; the error names the bad line.
pub fn parse_keys(file: &[u8]) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut last = Duration::ZERO;
    for (index, line) in file.split(|b| *b == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        let bad = |why: &str| format!("keys file line {}: {why}", index + 1);
        let space = line
            .iter()
            .position(|b| *b == b' ')
            .ok_or_else(|| bad("no space after the time"))?;
        let ms = std::str::from_utf8(&line[..space])
            .ok()
            .filter(|ms| ms.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|ms| ms.parse::<u64>().ok())
            .ok_or_else(|| bad("the time is not a whole number of milliseconds"))?;
        let at = Duration::from_millis(ms);
        if at < last {
            return Err(bad("the time is earlier than the line before"));
        }
        last = at;
        let bytes = unescape(&line[space + 1..]).map_err(bad)?;
        keys.push(Key { at, bytes });
    }
    Ok(keys)
}

fn unescape(text: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.iter().copied();
    while let Some(b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        out.push(match bytes.next() {
            Some(b'r') => b'\r',
            Some(b'n') => b'\n',
            Some(b't') => b'\t',
            Some(b'\\') => b'\\',
            Some(b'x') => {
                let hex = [bytes.next(), bytes.next()];
                let [Some(high), Some(low)] = hex else {
                    return Err("\\x needs two hex digits");
                };
                std::str::from_utf8(&[high, low])
                    .ok()
                    .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                    .ok_or("\\x needs two hex digits")?
            }
            _ => return Err("unknown escape; use \\r \\n \\t \\\\ or \\xNN"),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_file_lines_are_parsed() {
        let keys = parse_keys(b"300 hi there\\r\n\n600 \\x04\\\\\\t\\n").unwrap();
        assert_eq!(
            keys,
            [
                Key {
                    at: Duration::from_millis(300),
                    bytes: b"hi there\r".to_vec()
                },
                Key {
                    at: Duration::from_millis(600),
                    bytes: b"\x04\\\t\n".to_vec()
                },
            ]
        );
        assert!(parse_keys(b"300\n").is_err());
        assert!(parse_keys(b"x1 a\n").is_err());
        assert!(parse_keys(b"300 \\q\n").is_err());
        assert!(parse_keys(b"300 \\x4\n").is_err());
        assert!(parse_keys(b"600 a\n300 b\n").is_err());
    }
}
