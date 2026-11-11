//! The recording file: what a program wrote and when the grid changed, so a session can be
//! replayed without the program. The line "TERMREC1\n", then records of
//! [kind u8][ms u64 LE][len u32 LE][payload].

use std::io::{self, Read, Write};

use crate::geometry::GridSize;

const HEADER: &[u8] = b"TERMREC1\n";
const KIND_OUTPUT: u8 = 0;
const KIND_RESIZE: u8 = 1;
const HEAD_LEN: usize = 1 + 8 + 4;
/// A length above this is a damaged file, not a record worth allocating for.
const MAX_PAYLOAD: u32 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Output { ms: u64, bytes: Vec<u8> },
    Resize { ms: u64, grid: GridSize },
}

pub struct Recorder<W: Write> {
    out: W,
}

impl<W: Write> Recorder<W> {
    /// Starts with the grid so a replay knows the size before the first output.
    pub fn new(mut out: W, grid: GridSize) -> io::Result<Recorder<W>> {
        out.write_all(HEADER)?;
        let mut recorder = Recorder { out };
        recorder.resize(0, grid)?;
        Ok(recorder)
    }

    pub fn output(&mut self, ms: u64, bytes: &[u8]) -> io::Result<()> {
        let len = u32::try_from(bytes.len())
            .ok()
            .filter(|len| *len <= MAX_PAYLOAD)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "output too large for one record",
                )
            })?;
        self.head(KIND_OUTPUT, ms, len)?;
        self.out.write_all(bytes)
    }

    pub fn resize(&mut self, ms: u64, grid: GridSize) -> io::Result<()> {
        self.head(KIND_RESIZE, ms, 4)?;
        let mut payload = [0; 4];
        payload[..2].copy_from_slice(&grid.cols.to_le_bytes());
        payload[2..].copy_from_slice(&grid.rows.to_le_bytes());
        self.out.write_all(&payload)
    }

    /// For a buffered writer, so the caller can see a failure that drop would hide.
    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    fn head(&mut self, kind: u8, ms: u64, len: u32) -> io::Result<()> {
        let mut head = [0; HEAD_LEN];
        head[0] = kind;
        head[1..9].copy_from_slice(&ms.to_le_bytes());
        head[9..].copy_from_slice(&len.to_le_bytes());
        self.out.write_all(&head)
    }
}

/// A file cut off inside a record (a recorder that was killed) gives the records before the cut.
pub fn read_recording(mut input: impl Read) -> io::Result<Vec<Record>> {
    let invalid = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut header = [0; HEADER.len()];
    match input.read_exact(&mut header) {
        Ok(()) if header == HEADER => {}
        Ok(()) => return Err(invalid("not a recording: the header is missing")),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            return Err(invalid("not a recording: the header is missing"));
        }
        Err(e) => return Err(e),
    }
    let mut records = Vec::new();
    loop {
        let mut head = [0; HEAD_LEN];
        match input.read_exact(&mut head) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(records),
            Err(e) => return Err(e),
        }
        let kind = head[0];
        let ms = u64::from_le_bytes(head[1..9].try_into().unwrap());
        let len = u32::from_le_bytes(head[9..].try_into().unwrap());
        if kind != KIND_OUTPUT && kind != KIND_RESIZE {
            return Err(invalid(&format!("unknown record kind {kind}")));
        }
        if len > MAX_PAYLOAD {
            return Err(invalid(&format!("record of {len} bytes is too large")));
        }
        let mut payload = Vec::new();
        (&mut input)
            .take(u64::from(len))
            .read_to_end(&mut payload)?;
        if payload.len() < len as usize {
            return Ok(records);
        }
        records.push(match kind {
            KIND_OUTPUT => Record::Output { ms, bytes: payload },
            _ => {
                if len != 4 {
                    return Err(invalid(&format!("resize record of {len} bytes")));
                }
                let cols = u16::from_le_bytes([payload[0], payload[1]]);
                let rows = u16::from_le_bytes([payload[2], payload[3]]);
                Record::Resize {
                    ms,
                    grid: GridSize { cols, rows },
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(cols: u16, rows: u16) -> GridSize {
        GridSize { cols, rows }
    }

    #[test]
    fn recording_round_trips() {
        let mut recorder = Recorder::new(Vec::new(), grid(80, 24)).unwrap();
        recorder.output(5, b"hi").unwrap();
        recorder.resize(9, grid(100, 30)).unwrap();
        let bytes = recorder.out;
        let expected = vec![
            Record::Resize {
                ms: 0,
                grid: grid(80, 24),
            },
            Record::Output {
                ms: 5,
                bytes: b"hi".to_vec(),
            },
            Record::Resize {
                ms: 9,
                grid: grid(100, 30),
            },
        ];
        assert_eq!(read_recording(&bytes[..]).unwrap(), expected);

        let no_header = &bytes[HEADER.len()..];
        assert!(read_recording(no_header).is_err());

        // Cut inside the last record: its 13-byte head plus 4 payload bytes.
        let cut = &bytes[..bytes.len() - 2];
        assert_eq!(read_recording(cut).unwrap(), expected[..2]);
    }
}
