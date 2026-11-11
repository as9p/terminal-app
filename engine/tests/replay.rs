//! Replays recorded program output through a view as a window would draw it, and checks that
//! nothing is drawn while a synchronized update is open.

use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

use term_engine::{GridSize, Platform, Record, TerminalView, Theme, read_recording};

const BSU: &[u8] = b"\x1b[?2026h";

struct Report {
    frames: usize,
    drawn_during_sync: usize,
    saw_sync: bool,
    kitty_flags: u8,
    rows: Vec<String>,
}

/// The parser stamps an open update's deadline with the real clock, but a replay runs much
/// faster than the recording did. So while an update is open, time is told from the feed that
/// opened it: that feed's real instant plus the recording time since.
struct Clock {
    /// The open update's deadline, the real instant before the feed that opened it, and that
    /// feed's recording time.
    opened: Option<(Instant, Instant, u64)>,
}

impl Clock {
    fn at(&self, ms: u64) -> Instant {
        match self.opened {
            Some((_, fed_at, opened_ms)) => {
                fed_at + Duration::from_millis(ms.saturating_sub(opened_ms))
            }
            None => Instant::now(),
        }
    }

    fn after_feed(&mut self, view: &TerminalView, fed_at: Instant, ms: u64) {
        self.opened = match (view.screen().sync_deadline(), self.opened) {
            (None, _) => None,
            (Some(deadline), Some(open)) if open.0 == deadline => Some(open),
            (Some(deadline), _) => Some((deadline, fed_at, ms)),
        };
    }
}

fn draw(view: &mut TerminalView, report: &mut Report) {
    if view.needs_draw() {
        report.frames += 1;
        if view.screen().sync_deadline().is_some() {
            report.drawn_during_sync += 1;
        }
        view.mark_drawn();
    }
}

/// Feeds the records into a view at the first resize's grid. With `merged`, a run of
/// neighbouring outputs is fed as one, at the run's last time.
fn replay(records: &[Record], merged: bool) -> Report {
    let grid = records
        .iter()
        .find_map(|record| match record {
            Record::Resize { grid, .. } => Some(*grid),
            Record::Output { .. } => None,
        })
        .expect("a recording starts with its grid");
    let mut view = TerminalView::new(grid, &Theme::builtin(), Platform::Mac);
    let mut clock = Clock { opened: None };
    let mut report = Report {
        frames: 0,
        drawn_during_sync: 0,
        saw_sync: false,
        kitty_flags: 0,
        rows: Vec::new(),
    };
    // The end of the stream so far, so a marker cut across two feeds is still found.
    let mut tail: Vec<u8> = Vec::new();
    draw(&mut view, &mut report);
    let mut index = 0;
    while index < records.len() {
        match &records[index] {
            Record::Resize { grid, .. } => {
                view.set_grid(*grid);
                draw(&mut view, &mut report);
                index += 1;
            }
            Record::Output { ms, bytes } => {
                let mut ms = *ms;
                let mut chunk = bytes.clone();
                index += 1;
                while merged
                    && let Some(Record::Output {
                        ms: next_ms,
                        bytes: next,
                    }) = records.get(index)
                {
                    chunk.extend_from_slice(next);
                    ms = *next_ms;
                    index += 1;
                }
                // An update the program left open too long ends before the next bytes arrive.
                view.tick(clock.at(ms));
                draw(&mut view, &mut report);
                let fed_at = Instant::now();
                view.feed(&chunk);
                view.take_output();
                view.take_events();
                clock.after_feed(&view, fed_at, ms);
                view.tick(clock.at(ms));
                draw(&mut view, &mut report);
                tail.extend_from_slice(&chunk);
                if tail.windows(BSU.len()).any(|window| window == BSU) {
                    report.saw_sync = true;
                }
                tail.drain(..tail.len().saturating_sub(BSU.len() - 1));
                report.kitty_flags = report.kitty_flags.max(view.screen().modes().kitty_flags);
            }
        }
    }
    report.rows = (0..view.grid().rows)
        .map(|row| view.screen().row_text(row))
        .collect();
    report
}

/// The first row that differs and how many differ, so a failure never prints screen text.
fn row_difference(a: &[String], b: &[String]) -> Option<(usize, usize)> {
    if a.len() != b.len() {
        return Some((a.len().min(b.len()), a.len().abs_diff(b.len())));
    }
    let differing: Vec<usize> = (0..a.len()).filter(|&row| a[row] != b[row]).collect();
    differing.first().map(|&first| (first, differing.len()))
}

fn output(ms: u64, bytes: &[u8]) -> Record {
    Record::Output {
        ms,
        bytes: bytes.to_vec(),
    }
}

#[test]
fn synthetic_full_screen_redraw_never_shows_half_a_frame() {
    let records = vec![
        Record::Resize {
            ms: 0,
            grid: GridSize { cols: 40, rows: 10 },
        },
        output(0, b"\x1b[?1049h\x1b[Hstale frame"),
        // One update that clears the screen and paints three rows, cut inside the opening
        // marker, between `ESC [` and its final byte, and inside the closing marker.
        output(10, b"\x1b[?20"),
        output(13, b"26h\x1b[2J\x1b[Hrow one\r\n\x1b["),
        output(16, b"1mrow two\x1b[0m\r\nrow th"),
        output(19, b"ree\x1b[?2026"),
        output(22, b"l"),
    ];
    let unmerged = replay(&records, false);
    let merged = replay(&records, true);
    assert_eq!(unmerged.drawn_during_sync, 0);
    assert_eq!(merged.drawn_during_sync, 0);
    assert!(unmerged.saw_sync);
    assert_eq!(
        unmerged.rows[..3],
        ["row one", "row two", "row three"],
        "the update's three rows replace the stale frame"
    );
    assert!(unmerged.rows[3..].iter().all(String::is_empty));
    assert_eq!(merged.rows, unmerged.rows);
}

#[test]
#[ignore = "needs engine/tests/recordings/claude.local.rec from tools/plain/record-claude.sh"]
fn claude_session_replays_clean() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/recordings/claude.local.rec");
    let file = File::open(&path).unwrap_or_else(|e| {
        panic!(
            "no recording at {} ({e}); make one with: zsh tools/plain/record-claude.sh",
            path.display()
        )
    });
    let records = read_recording(file).expect("the recording reads back");
    let unmerged = replay(&records, false);
    let merged = replay(&records, true);
    println!(
        "REPLAY frames={} kitty_flags={} records={}",
        unmerged.frames,
        unmerged.kitty_flags,
        records.len()
    );
    assert_eq!(
        unmerged.drawn_during_sync, 0,
        "frames drawn while an update was open"
    );
    assert_eq!(
        merged.drawn_during_sync, 0,
        "frames drawn while an update was open, merged"
    );
    assert!(
        unmerged.saw_sync,
        "the program never opened a synchronized update"
    );
    if let Some((first, count)) = row_difference(&merged.rows, &unmerged.rows) {
        panic!("merged and unmerged replays differ in {count} rows, the first is row {first}");
    }
    assert!(
        unmerged.rows.iter().any(|row| !row.trim().is_empty()),
        "the final screen is empty"
    );
}
