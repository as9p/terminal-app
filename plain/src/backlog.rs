//! Backpressure from the view to the pty: the reader thread waits while too much output is
//! queued and not yet fed, so a flood is slowed by the kernel instead of filling memory.

use std::ops::Deref;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use crate::shell::ShellEvent;

/// The most output handed over and not yet fed before the reader waits.
pub const QUEUE_LIMIT: usize = 4 * 1024 * 1024;

/// Counts bytes handed to the event loop and not yet fed to the view.
pub struct Backlog {
    limit: usize,
    /// The queued total, and whether the backlog is closed.
    state: Mutex<(usize, bool)>,
    drained: Condvar,
}

impl Backlog {
    pub fn new(limit: usize) -> Backlog {
        Backlog {
            limit,
            state: Mutex::new((0, false)),
            drained: Condvar::new(),
        }
    }

    /// Called on the reader thread after it has sent `bytes` to the loop. Blocks while the
    /// queued total is over the limit and the backlog is open.
    pub fn added(&self, bytes: usize) {
        let mut state = self.lock();
        // Counted before the wait, so one read bigger than the limit waits for the view to take
        // it rather than for room that can never come.
        state.0 = state.0.saturating_add(bytes);
        let _state = self
            .drained
            .wait_while(state, |(queued, closed)| *queued > self.limit && !*closed)
            .unwrap_or_else(PoisonError::into_inner);
    }

    /// Called on the loop thread after feeding `bytes` to the view. Wakes a blocked reader.
    pub fn fed(&self, bytes: usize) {
        let mut state = self.lock();
        state.0 = state.0.saturating_sub(bytes);
        if state.0 <= self.limit {
            self.drained.notify_all();
        }
    }

    /// Called before the session is dropped. A blocked reader returns at once, and `added` never
    /// blocks again.
    pub fn close(&self) {
        self.lock().1 = true;
        self.drained.notify_all();
    }

    /// Only the tests read the total; the loop and the reader act on it through the calls above.
    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.lock().0
    }

    /// A panic elsewhere must not stop the count, or the reader would wait forever.
    fn lock(&self) -> MutexGuard<'_, (usize, bool)> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Closes the backlog when dropped. Its owner holds it ahead of the session, so whatever drops
/// the session, on any path, lets a blocked reader go first.
pub struct CloseOnDrop(pub Arc<Backlog>);

impl Deref for CloseOnDrop {
    type Target = Backlog;

    fn deref(&self) -> &Backlog {
        &self.0
    }
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.0.close();
    }
}

/// The shell's event callback around `send`: each output is handed over first and then counted,
/// so the reader waits only on bytes the other side already has. The exit is never held up.
pub fn counted(
    backlog: Arc<Backlog>,
    send: impl Fn(ShellEvent) + Send + 'static,
) -> Box<dyn Fn(ShellEvent) + Send> {
    Box::new(move |event| {
        let bytes = match &event {
            ShellEvent::Output(bytes) => bytes.len(),
            ShellEvent::Exited(_) => 0,
        };
        send(event);
        if bytes > 0 {
            backlog.added(bytes);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    /// Long enough for a reader that is not blocked to have finished.
    const BLOCKED: Duration = Duration::from_millis(50);
    /// A reader that was let go finishes well within this.
    const SOON: Duration = Duration::from_secs(1);

    /// Calls `added` for each size on its own thread; the receiver hears once all have returned.
    fn reader(backlog: &Arc<Backlog>, sizes: &[usize]) -> mpsc::Receiver<()> {
        let (done, finished) = mpsc::channel();
        let backlog = backlog.clone();
        let sizes = sizes.to_vec();
        thread::spawn(move || {
            for bytes in sizes {
                backlog.added(bytes);
            }
            let _ = done.send(());
        });
        finished
    }

    #[test]
    fn under_the_limit_never_blocks() {
        let backlog = Arc::new(Backlog::new(100));
        assert!(reader(&backlog, &[60]).recv_timeout(SOON).is_ok());
        assert_eq!(backlog.queued(), 60);
    }

    #[test]
    fn over_the_limit_blocks_until_fed() {
        let backlog = Arc::new(Backlog::new(100));
        let finished = reader(&backlog, &[60, 60]);
        assert!(finished.recv_timeout(BLOCKED).is_err());
        assert_eq!(backlog.queued(), 120);
        backlog.fed(60);
        assert!(finished.recv_timeout(SOON).is_ok());
        assert_eq!(backlog.queued(), 60);
    }

    #[test]
    fn one_oversized_read_gets_through() {
        let backlog = Arc::new(Backlog::new(100));
        let finished = reader(&backlog, &[500]);
        assert!(finished.recv_timeout(BLOCKED).is_err());
        backlog.fed(500);
        assert!(finished.recv_timeout(SOON).is_ok());
        assert_eq!(backlog.queued(), 0);
    }

    #[test]
    fn close_releases_a_blocked_reader() {
        let backlog = Arc::new(Backlog::new(100));
        let finished = reader(&backlog, &[500]);
        assert!(finished.recv_timeout(BLOCKED).is_err());
        backlog.close();
        assert!(finished.recv_timeout(SOON).is_ok());
        assert!(reader(&backlog, &[500]).recv_timeout(SOON).is_ok());
    }

    #[test]
    fn fed_never_underflows() {
        let backlog = Backlog::new(100);
        backlog.added(10);
        backlog.fed(50);
        assert_eq!(backlog.queued(), 0);
    }
}
