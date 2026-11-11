//! One shell tied to one view: what the program writes reaches the screen, and what the view
//! answers or the user types reaches the program.

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};
use std::time::Instant;

use term_engine::{CellSize, GridSize, Platform, Recorder, TerminalView, Theme};

use crate::shell::Shell;

/// Creates the recording file, so a bad path fails before the command runs, not after it has
/// done its work. Says why on stderr when it cannot.
pub fn can_record(path: Option<&Path>) -> bool {
    match path.map(File::create) {
        Some(Err(e)) => {
            eprintln!("paw: cannot write the recording: {e}");
            false
        }
        _ => true,
    }
}

pub struct Session {
    pub view: TerminalView,
    shell: Shell,
    recorder: Option<Recorder<BufWriter<File>>>,
    start: Instant,
}

impl Session {
    pub fn new(shell: Shell, grid: GridSize, record: Option<PathBuf>) -> io::Result<Session> {
        let platform = if cfg!(target_os = "macos") {
            Platform::Mac
        } else {
            Platform::Other
        };
        let mut view = TerminalView::new(grid, &Theme::builtin(), platform);
        view.set_identity(&format!("{} {}", crate::NAME, env!("CARGO_PKG_VERSION")));
        // The recording holds the grid the screen really has, which can differ from the one
        // asked for.
        let recorder = match record {
            Some(path) => Some(Recorder::new(
                BufWriter::new(File::create(path)?),
                view.grid(),
            )?),
            None => None,
        };
        Ok(Session {
            view,
            shell,
            recorder,
            start: Instant::now(),
        })
    }

    /// Record, feed the view, then send back whatever the view answered.
    pub fn output(&mut self, bytes: &[u8]) {
        let ms = self.ms();
        self.record(|recorder| recorder.output(ms, bytes));
        self.view.feed(bytes);
        self.flush();
    }

    /// Sends the view's pending bytes (query answers, encoded keys, pastes) to the shell.
    pub fn flush(&mut self) {
        let bytes = self.view.take_output();
        self.shell.write(bytes);
    }

    /// Bytes for the program as they are, past the view's input policy.
    pub fn write(&mut self, bytes: Vec<u8>) {
        self.shell.write(bytes);
    }

    /// The view, the pty and the recording change together so they never disagree.
    pub fn set_grid(&mut self, grid: GridSize, cell: CellSize) {
        let before = self.view.grid();
        self.view.set_grid(grid);
        let now = self.view.grid();
        self.shell.resize(now, cell);
        if now != before {
            let ms = self.ms();
            self.record(|recorder| recorder.resize(ms, now));
        }
    }

    pub fn kill(&self) {
        self.shell.kill();
    }

    fn ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// A recording that cannot be written must not stop the terminal: say so once and stop.
    fn record(&mut self, write: impl FnOnce(&mut Recorder<BufWriter<File>>) -> io::Result<()>) {
        if let Some(recorder) = &mut self.recorder
            && let Err(e) = write(recorder)
        {
            eprintln!("paw: recording stopped: {e}");
            self.recorder = None;
        }
    }
}

impl Drop for Session {
    /// Dropping the buffer would hide a failed last write.
    fn drop(&mut self) {
        self.record(|recorder| recorder.flush());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use term_engine::{Record, read_recording};

    #[test]
    fn resize_is_recorded() {
        let path = std::env::temp_dir().join(format!("paw-resize-{}", std::process::id()));
        let grid = GridSize { cols: 80, rows: 24 };
        let shell = Shell::spawn(
            Some(vec!["/bin/cat".to_string()]),
            None,
            grid,
            Box::new(|_| {}),
        )
        .unwrap();
        let mut session = Session::new(shell, grid, Some(path.clone())).unwrap();
        session.set_grid(
            GridSize {
                cols: 100,
                rows: 30,
            },
            CellSize { w: 8, h: 20 },
        );
        // An unchanged grid only passes the pixel size on; it is not a new record.
        session.set_grid(
            GridSize {
                cols: 100,
                rows: 30,
            },
            CellSize { w: 9, h: 21 },
        );
        drop(session);
        let records = read_recording(File::open(&path).unwrap());
        let _ = std::fs::remove_file(&path);
        let grids: Vec<GridSize> = records
            .unwrap()
            .into_iter()
            .filter_map(|record| match record {
                Record::Resize { grid, .. } => Some(grid),
                Record::Output { .. } => None,
            })
            .collect();
        assert_eq!(
            grids,
            [
                grid,
                GridSize {
                    cols: 100,
                    rows: 30
                }
            ]
        );
    }
}
