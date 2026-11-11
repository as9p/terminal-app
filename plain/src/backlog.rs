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

    /// Reader thread, before handing `bytes` to the loop: counts them. Never blocks. Counting
    /// first means the loop can never feed, and give back, bytes that were not counted yet.
    pub fn reserve(&self, bytes: usize) {
        let mut state = self.lock();
        state.0 = state.0.saturating_add(bytes);
    }

    /// Reader thread, after the hand-over: blocks while the queued total is over the limit and
    /// the backlog is open. One read bigger than the limit waits for the view to take it, which
    /// it can, since the loop already has it.
    pub fn wait(&self) {
        let state = self.lock();
        let _state = self
            .drained
            .wait_while(state, |(queued, closed)| *queued > self.limit && !*closed)
            .unwrap_or_else(PoisonError::into_inner);
    }

    /// Loop thread, after feeding `bytes` to the view (or reader thread, when a hand-over
    /// failed). Wakes a blocked reader.
    pub fn fed(&self, bytes: usize) {
        let mut state = self.lock();
        // Every byte fed was reserved first, so the count cannot run short; if it does, the
        // accounting is wrong and the clamp below would only hide it.
        debug_assert!(
            bytes <= state.0,
            "fed {bytes} bytes with only {} queued",
            state.0
        );
        state.0 = state.0.saturating_sub(bytes);
        if state.0 <= self.limit {
            self.drained.notify_all();
        }
    }

    /// Called before the session is dropped. A blocked reader returns at once, and `wait` never
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

/// The shell's event callback around `send`, which says whether the loop took the event: each
/// output is counted, handed over, then waited on, so the reader waits only on bytes the loop
/// already has. Bytes the loop did not take are given back at once. The exit is never held up.
pub fn counted(
    backlog: Arc<Backlog>,
    send: impl Fn(ShellEvent) -> bool + Send + 'static,
) -> Box<dyn Fn(ShellEvent) + Send> {
    Box::new(move |event| {
        let bytes = match &event {
            ShellEvent::Output(bytes) => bytes.len(),
            ShellEvent::Exited(_) => 0,
        };
        if bytes == 0 {
            send(event);
            return;
        }
        backlog.reserve(bytes);
        if send(event) {
            backlog.wait();
        } else {
            backlog.fed(bytes);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    /// Long enough for a reader that is not blocked to have finished.
    const BLOCKED: Duration = Duration::from_millis(50);
    /// A reader that was let go finishes well within this.
    const SOON: Duration = Duration::from_secs(1);
    /// The longest a loaded machine may take to start a thread and run it to its first wait.
    const STARTED: Duration = Duration::from_secs(2);

    /// Runs `work` on its own thread; the receiver hears once it has returned.
    fn spawn(
        backlog: &Arc<Backlog>,
        work: impl FnOnce(&Backlog) + Send + 'static,
    ) -> mpsc::Receiver<()> {
        let (done, finished) = mpsc::channel();
        let backlog = backlog.clone();
        thread::spawn(move || {
            work(&backlog);
            let _ = done.send(());
        });
        finished
    }

    /// Hands over each size as the reader does: counted first, then the wait.
    fn reader(backlog: &Arc<Backlog>, sizes: &[usize]) -> mpsc::Receiver<()> {
        let sizes = sizes.to_vec();
        spawn(backlog, move |backlog| {
            for bytes in sizes {
                backlog.reserve(bytes);
                backlog.wait();
            }
        })
    }

    /// Waits until the reader's count is in, so a thread that has not run yet is never taken for
    /// one that is blocked.
    fn until_queued(backlog: &Backlog, total: usize) {
        let start = Instant::now();
        while backlog.queued() != total && start.elapsed() < STARTED {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(backlog.queued(), total);
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
        until_queued(&backlog, 120);
        assert!(finished.recv_timeout(BLOCKED).is_err());
        backlog.fed(60);
        assert!(finished.recv_timeout(SOON).is_ok());
        assert_eq!(backlog.queued(), 60);
    }

    #[test]
    fn one_oversized_read_gets_through() {
        let backlog = Arc::new(Backlog::new(100));
        let finished = reader(&backlog, &[500]);
        until_queued(&backlog, 500);
        assert!(finished.recv_timeout(BLOCKED).is_err());
        backlog.fed(500);
        assert!(finished.recv_timeout(SOON).is_ok());
        assert_eq!(backlog.queued(), 0);
    }

    #[test]
    fn close_releases_a_blocked_reader() {
        let backlog = Arc::new(Backlog::new(100));
        let finished = reader(&backlog, &[500]);
        until_queued(&backlog, 500);
        assert!(finished.recv_timeout(BLOCKED).is_err());
        backlog.close();
        assert!(finished.recv_timeout(SOON).is_ok());
        assert!(reader(&backlog, &[500]).recv_timeout(SOON).is_ok());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn fed_without_a_reserve_is_a_bug() {
        let backlog = Backlog::new(100);
        backlog.reserve(10);
        backlog.fed(50);
    }

    #[test]
    fn reserve_never_blocks() {
        let backlog = Backlog::new(100);
        backlog.reserve(500);
        assert_eq!(backlog.queued(), 500);
    }

    #[test]
    fn fed_before_wait_leaves_nothing_counted() {
        let backlog = Arc::new(Backlog::new(100));
        backlog.reserve(500);
        backlog.fed(500);
        assert!(spawn(&backlog, Backlog::wait).recv_timeout(SOON).is_ok());
        assert_eq!(backlog.queued(), 0);
    }
}
