use std::mem;
use std::num::NonZeroU16;

use crate::pty::Size;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

/// An unfinished sequence longer than this is passed on instead of waiting, so a program
/// that never finishes one cannot grow the buffer without limit.
const MAX_HELD: usize = 64 * 1024;

/// A copy of a session's screen, so a window that attaches late can be given the current
/// state.
pub struct Screen {
    parser: vt100::Parser,
    /// The start of a sequence or character still waiting for its other half. vt100 has
    /// no way to read or save its parser state, so these bytes stay out of the parser.
    /// That keeps it safe to feed it the grid switches in `snapshot`, and lets the
    /// snapshot hand the same bytes on so the bytes that follow it still make sense.
    held: Vec<u8>,
}

impl Screen {
    pub fn new(size: Size, scrollback: usize) -> Self {
        Self {
            parser: vt100::Parser::new(dimension(size.rows), dimension(size.cols), scrollback),
            held: Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        if self.held.is_empty() {
            let ready = ready_len(bytes);
            self.parser.process(&bytes[..ready]);
            self.held.extend_from_slice(&bytes[ready..]);
        } else {
            let mut joined = mem::take(&mut self.held);
            joined.extend_from_slice(bytes);
            let ready = ready_len(&joined);
            self.parser.process(&joined[..ready]);
            joined.drain(..ready);
            self.held = joined;
        }
    }

    pub fn resize(&mut self, size: Size) {
        self.parser
            .set_size(dimension(size.rows), dimension(size.cols));
    }

    /// Bytes that rebuild this screen when replayed into a terminal of the same size,
    /// whatever that terminal was showing. Takes `&mut self` because vt100 only shows the
    /// hidden main screen through its active grid, which is put back before returning.
    pub fn snapshot(&mut self) -> Vec<u8> {
        let mut out = b"\x1bc".to_vec();

        // Mode 47 switches grids without clearing or saving anything, unlike 1049.
        let alternate = self.parser.screen().alternate_screen();
        if alternate {
            self.parser.process(b"\x1b[?47l");
        }
        self.write_main(&mut out);
        if alternate {
            self.parser.process(b"\x1b[?47h");
            // 1049 saves the cursor just placed, so leaving the full-screen program lands
            // where it would have on the original.
            out.extend_from_slice(b"\x1b[?1049h");
            let screen = self.parser.screen();
            out.extend_from_slice(screen.contents_formatted().as_bytes());
        } else {
            let screen = self.parser.screen();
            out.extend_from_slice(screen.attributes_formatted().as_bytes());
        }

        let screen = self.parser.screen();
        out.extend_from_slice(screen.input_mode_formatted().as_bytes());
        out.extend_from_slice(&self.held);
        out
    }

    /// Writes the main grid top to bottom as ordinary output, so that the rows past the
    /// bottom of the receiver scroll into its own scrollback. Ends with the pen reset, and
    /// the cursor where the main grid has it.
    ///
    /// Walks a copy because reading a scrollback row pads it to full width for good, which
    /// took a short-line session from 27 to 170 MiB at its first attach. The copy costs that
    /// much for the moment instead. The system allocator tends to keep the freed pages
    /// resident, but the next snapshot reuses them, so sessions do not add up.
    fn write_main(&self, out: &mut Vec<u8>) {
        let mut copy = self.parser.screen().clone();
        let screen = &mut copy;
        let (rows, cols) = screen.size();
        let (rows, cols) = (rows.get(), cols.get());
        let mut previous = None;

        for back in (1..=screen.scrollback_len()).rev() {
            screen.set_scrollback(back);
            // One row per offset because rows_formatted forgets wrapping between calls.
            join_rows(out, previous, screen.cell(0, 0), true);
            if let Some(text) = screen.rows_formatted(0, cols).next() {
                out.extend_from_slice(text.as_bytes());
            }
            previous = Some(row_end(screen, 0, cols));
        }

        screen.set_scrollback(0);
        let screen = &*screen;
        for (row, text) in (0..rows).zip(screen.rows_formatted(0, cols)) {
            // Below the first row, vt100 already writes what forces the wrap.
            join_rows(out, previous, screen.cell(row, 0), row == 0);
            out.extend_from_slice(text.as_bytes());
            previous = Some(row_end(screen, row, cols));
        }

        out.extend_from_slice(b"\x1b[m");
        out.extend_from_slice(screen.cursor_state_formatted().as_bytes());
        out.extend_from_slice(b"\x1b[m");
    }

    /// The vt100 screen itself, for tests that compare cells and scrollback. Takes
    /// `&mut self` because reading scrollback means moving the offset.
    #[doc(hidden)]
    pub fn vt(&mut self) -> &mut vt100::Screen {
        self.parser.screen_mut()
    }
}

/// vt100 refuses a zero size, which a window can report while it is being laid out.
fn dimension(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap_or(NonZeroU16::MIN)
}

#[derive(Clone, Copy)]
enum RowEnd {
    Break,
    /// The cursor stops on the last column, so the next character continues this line.
    Wrap,
}

fn row_end(screen: &vt100::Screen, row: u16, cols: u16) -> RowEnd {
    // A flagged row with an empty last column leaves the cursor short of the edge, where
    // the next row's text would land on this row. Better a lost flag than a moved row.
    let reaches_edge = screen
        .cell(row, cols - 1)
        .is_some_and(|cell| cell.has_contents() || cell.is_wide_continuation());
    if screen.row_wrapped(row) && reaches_edge {
        RowEnd::Wrap
    } else {
        RowEnd::Break
    }
}

fn join_rows(
    out: &mut Vec<u8>,
    previous: Option<RowEnd>,
    first: Option<&vt100::Cell>,
    force_wrap: bool,
) {
    // Formatted rows assume they start from the default pen. Resetting before the line
    // feed also keeps a background color out of the blank row that scrolls in.
    out.extend_from_slice(b"\x1b[m");
    match previous {
        None => {}
        Some(RowEnd::Break) => out.extend_from_slice(b"\r\n"),
        // A blank first cell prints nothing, so print and erase a space to make the
        // terminal take the wrap.
        Some(RowEnd::Wrap) if force_wrap && !first.is_some_and(vt100::Cell::has_contents) => {
            out.extend_from_slice(b" \x08\x1b[X");
        }
        Some(RowEnd::Wrap) => {}
    }
}

/// How many leading bytes are safe to parse now. A wrong guess here only delays bytes,
/// since they all reach the parser in order, so this does not have to match vt100's parser
/// exactly.
fn ready_len(bytes: &[u8]) -> usize {
    let ready = unfinished_start(bytes);
    if bytes.len() - ready > MAX_HELD {
        bytes.len()
    } else {
        ready
    }
}

fn unfinished_start(bytes: &[u8]) -> usize {
    let mut pos = 0;
    while let Some(found) = bytes[pos..].iter().position(|&byte| byte == ESC) {
        let start = pos + found;
        match sequence_len(&bytes[start..]) {
            Some(len) => pos = start + len,
            None => return start,
        }
    }
    unfinished_char_start(bytes)
}

/// The length of the sequence opening `seq`, or None if it has not finished yet.
fn sequence_len(seq: &[u8]) -> Option<usize> {
    match *seq.get(1)? {
        ESC => Some(1),
        b'[' => seq[2..]
            .iter()
            .position(|&byte| matches!(byte, 0x40..=0x7e | CAN | SUB))
            .map(|end| end + 3),
        b']' | b'P' | b'X' | b'^' | b'_' => seq[2..]
            .iter()
            .position(|byte| matches!(*byte, BEL | CAN | SUB | ESC))
            // An ESC ends the string and starts its own sequence, usually the terminator.
            .map(|end| {
                if seq[end + 2] == ESC {
                    end + 2
                } else {
                    end + 3
                }
            }),
        _ => seq[1..]
            .iter()
            .position(|byte| !matches!(byte, 0x20..=0x2f))
            .map(|end| end + 2),
    }
}

fn unfinished_char_start(bytes: &[u8]) -> usize {
    let tail = bytes.len().saturating_sub(3);
    for (i, &byte) in bytes.iter().enumerate().skip(tail).rev() {
        let needed = match byte {
            0x80..=0xbf => continue,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            _ => break,
        };
        if bytes.len() - i < needed {
            return i;
        }
        break;
    }
    bytes.len()
}
