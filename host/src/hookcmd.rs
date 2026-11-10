//! `term-host hook <event>`, which Claude Code runs for each hook. It reads the hook's JSON on
//! stdin and sends it to the host as one `CtlRequest`. Claude waits for it, so it gives up on
//! a stdin that never sends and a host that is gone or never reads, prints nothing, and its
//! caller always exits 0. `term-host hook ClaudeExit`, which the shell hook's `claude` runs
//! when a typed `claude` returns, has no JSON and does not read stdin.

use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::ctl::{self, CtlRequest, MAX_LINE};
use crate::states::CLAUDE_EXIT;

/// How long to wait for the hook's JSON on stdin.
const READ_WAIT: Duration = Duration::from_millis(100);

/// How long the whole call may take before it gives up on the host, well inside the 300 ms
/// promised, to leave time for the process to start and exit.
const DEADLINE: Duration = Duration::from_millis(200);

/// Sends the hook call, giving up at the deadline. The work runs on threads that are left
/// behind when the process exits, so a blocked read or write cannot hold it.
pub fn run(event: &str) {
    let started = Instant::now();
    // A panic must not print where Claude would show it.
    std::panic::set_hook(Box::new(|_| {}));
    let session = std::env::var("TA_SESSION").unwrap_or_default();
    let token = std::env::var("TA_TOKEN").unwrap_or_default();
    let Some(data_dir) = std::env::var_os("TA_DATA").map(PathBuf::from) else {
        return;
    };
    if session.is_empty() || token.is_empty() {
        return;
    }

    let payload = if event.eq_ignore_ascii_case(CLAUDE_EXIT) {
        Value::Object(Default::default())
    } else {
        let Some(payload) = read_payload() else {
            return;
        };
        payload
    };

    let request = CtlRequest {
        token,
        session,
        event: event.to_string(),
        payload,
    };
    let (sent_tx, sent_rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = ctl::send(&data_dir, &request);
        let _ = sent_tx.send(());
    });
    let _ = sent_rx.recv_timeout(DEADLINE.saturating_sub(started.elapsed()));
}

/// The hook's JSON from stdin, or None if none came in time.
fn read_payload() -> Option<Value> {
    let (payload_tx, payload_rx) = mpsc::channel();
    thread::spawn(move || {
        // One JSON value, read without waiting for the end of stdin.
        let stdin = io::stdin().lock().take(MAX_LINE);
        let first = serde_json::Deserializer::from_reader(stdin)
            .into_iter::<Value>()
            .next();
        if let Some(Ok(payload)) = first {
            let _ = payload_tx.send(payload);
        }
    });
    // With no JSON there is nothing safe to report: a `Stop` read as empty could mark a
    // session done while its helper still runs.
    payload_rx.recv_timeout(READ_WAIT).ok()
}
