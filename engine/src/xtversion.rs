//! XTVERSION, `CSI > q` or `CSI > 0 q`: a program asks the terminal its name and version. The
//! screen library ignores the query, so the screen finds it itself, with a scan cheap enough to
//! run over every byte of output: a SIMD search for `>`, then a look at the bytes around it.
//!
//! The scan only knows the query's own five bytes, and that is enough. The parser lets an escape
//! end any string sequence (a title, a DCS payload) and start a new sequence, so these bytes are
//! always this query to the parser too. The rarer spellings the parser also accepts, such as
//! `CSI > 00 q` or a control byte inside the sequence, get no reply.

const ESC: u8 = 0x1b;

/// Who the terminal says it is when no program has said otherwise.
const DEFAULT_IDENTITY: &str = concat!("term-engine ", env!("CARGO_PKG_VERSION"));

/// How much of a query the output read so far ends with. Every query starts with an escape, so
/// a query cut across reads is at most four bytes into the last one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Progress {
    #[default]
    None,
    /// `ESC`
    Escape,
    /// `ESC [`
    Csi,
    /// `ESC [ >`
    Marker,
    /// `ESC [ > 0`
    Zero,
}

/// One byte further into a query, or `None` when the byte ends one.
fn step(progress: Progress, byte: u8) -> Option<Progress> {
    Some(match (progress, byte) {
        (_, ESC) => Progress::Escape,
        (Progress::Escape, b'[') => Progress::Csi,
        (Progress::Csi, b'>') => Progress::Marker,
        (Progress::Marker, b'0') => Progress::Zero,
        (Progress::Marker | Progress::Zero, b'q') => return None,
        _ => Progress::None,
    })
}

/// Finds queries in output that arrives in reads cut at any byte.
#[derive(Debug, Default)]
pub struct QueryScan {
    progress: Progress,
}

impl QueryScan {
    /// The offset just past the first query that ends in `bytes`, counting a start carried over
    /// from the reads before. Call again with the bytes after it for the next query. `None` once
    /// there is none, and then a query cut off at the end is carried to the next read.
    pub fn find(&mut self, bytes: &[u8]) -> Option<usize> {
        let mut from = 0;
        while self.progress != Progress::None {
            let &byte = bytes.get(from)?;
            from += 1;
            match step(self.progress, byte) {
                Some(progress) => self.progress = progress,
                None => {
                    self.progress = Progress::None;
                    return Some(from);
                }
            }
        }
        // No query is under way at `from`, so any query from here on has all of its bytes here.
        let mut at = from;
        while let Some(found) = memchr::memchr(b'>', &bytes[at..]) {
            let marker = at + found;
            if marker >= from + 2 && bytes[marker - 2..marker] == [ESC, b'['] {
                match (bytes.get(marker + 1), bytes.get(marker + 2)) {
                    (Some(b'q'), _) => return Some(marker + 2),
                    (Some(b'0'), Some(b'q')) => return Some(marker + 3),
                    _ => {}
                }
            }
            at = marker + 1;
        }
        let tail = bytes.len().saturating_sub(4).max(from);
        self.progress = bytes[tail..]
            .iter()
            .fold(Progress::None, |progress, &byte| {
                step(progress, byte).unwrap_or(Progress::None)
            });
        None
    }
}

/// The reply to the query, `DCS > | identity ST`. Only printable ASCII is kept, so the identity
/// can neither end the reply early nor carry a sequence of its own.
pub fn reply(identity: &str) -> String {
    let identity: String = identity
        .chars()
        .filter(|c| matches!(c, ' '..='~'))
        .collect();
    let identity = if identity.is_empty() {
        DEFAULT_IDENTITY
    } else {
        &identity
    };
    format!("\x1bP>|{identity}\x1b\\")
}

/// The reply before a program names the terminal.
pub fn default_reply() -> String {
    reply(DEFAULT_IDENTITY)
}
