//! Key presses to the bytes a program in the terminal expects. With the kitty keyboard protocol off
//! the encodings are xterm's; with it on they follow the kitty keyboard protocol specification.

use bitflags::bitflags;

use crate::screen::Modes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

bitflags! {
    /// The bit values are the kitty protocol's, so a modifier parameter is just 1 plus the bits.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Mods: u8 {
        const SHIFT = 1;
        const ALT = 2;
        const CTRL = 4;
        const SUPER = 8;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    Press,
    Repeat,
    Release,
}

/// `code` is the key without Shift applied (Char('a') for Shift+A). `text` is what the keyboard layout produced, if anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    pub code: KeyCode,
    pub mods: Mods,
    pub text: Option<String>,
    pub kind: KeyEventKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Other,
}

const ESC: u8 = 0x1b;

// The kitty progressive enhancement flags.
const DISAMBIGUATE: u8 = 1;
const REPORT_EVENTS: u8 = 2;
const REPORT_ALTERNATES: u8 = 4;
const REPORT_ALL_KEYS: u8 = 8;
const REPORT_TEXT: u8 = 16;

pub fn encode_key(key: &KeyInput, modes: &Modes, platform: Platform) -> Option<Vec<u8>> {
    if platform == Platform::Mac {
        // Mac line editing shortcuts beat any protocol, because users expect them in every program.
        if let Some(bytes) = mac_line_key(key) {
            return (key.kind != KeyEventKind::Release).then(|| bytes.to_vec());
        }
        // Every other Cmd combination is a window shortcut, not input.
        if key.mods.contains(Mods::SUPER) {
            return None;
        }
    }
    if modes.kitty_flags & 0x1f == 0 {
        legacy(key, modes.app_cursor)
    } else {
        kitty(key, modes)
    }
}

/// Copied from the existing app's behaviour, not from any specification.
fn mac_line_key(key: &KeyInput) -> Option<&'static [u8]> {
    use KeyCode::*;
    if key.mods == Mods::SUPER {
        match key.code {
            Backspace => Some(b"\x15"),
            Left => Some(b"\x01"),
            Right => Some(b"\x05"),
            Delete => Some(b"\x0b"),
            Char('z') => Some(b"\x1f"),
            _ => None,
        }
    } else if key.mods == Mods::ALT {
        match key.code {
            Left => Some(b"\x1bb"),
            Right => Some(b"\x1bf"),
            Backspace => Some(b"\x1b\x7f"),
            Delete => Some(b"\x1bd"),
            _ => None,
        }
    } else {
        None
    }
}

fn legacy(key: &KeyInput, app_cursor: bool) -> Option<Vec<u8>> {
    // Legacy encodings have no way to say "released", and nothing to send for Super.
    if key.kind == KeyEventKind::Release || key.mods.contains(Mods::SUPER) {
        return None;
    }
    legacy_bytes(key, app_cursor)
}

fn legacy_bytes(key: &KeyInput, app_cursor: bool) -> Option<Vec<u8>> {
    use KeyCode::*;
    let mods = key.mods;
    let alt = mods.contains(Mods::ALT);
    let ctrl = mods.contains(Mods::CTRL);
    let shift = mods.contains(Mods::SHIFT);
    let mut out = Vec::new();
    match key.code {
        Char(c) => {
            if alt {
                out.push(ESC);
            }
            match ctrl.then(|| ctrl_byte(c)).flatten() {
                Some(byte) => out.push(byte),
                // Alt without Shift sends the bare key: the text may hold what Option would have typed.
                None if alt && !shift => push_char(&mut out, c),
                None => out.extend_from_slice(typed(key, c).as_bytes()),
            }
        }
        Enter => {
            // Shift+Enter as Escape then Return is the existing app's choice, so programs can tell it apart.
            if alt || mods == Mods::SHIFT {
                out.push(ESC);
            }
            out.push(b'\r');
        }
        Escape => {
            if alt {
                out.push(ESC);
            }
            out.push(ESC);
        }
        Backspace => {
            if alt {
                out.push(ESC);
            }
            out.push(if ctrl { 0x08 } else { 0x7f });
        }
        Tab => {
            if alt {
                out.push(ESC);
            }
            out.extend_from_slice(if shift { b"\x1b[Z" } else { b"\t" });
        }
        code => {
            let param = 1 + u32::from(mods.bits());
            let func = functional(code)?;
            if param == 1 {
                match (code, func) {
                    // F1 to F4 use SS3 when unmodified; F3 has no CSI letter form because CSI R is a cursor report.
                    (F(3), _) => return Some(b"\x1bOR".to_vec()),
                    (F(_), Func::Letter(l)) => return Some(vec![ESC, b'O', l]),
                    (_, Func::Letter(l)) if app_cursor => return Some(vec![ESC, b'O', l]),
                    _ => {}
                }
            }
            out = func.csi(param, None);
        }
    }
    Some(out)
}

/// How a key without text is written as an escape code.
#[derive(Clone, Copy)]
enum Func {
    /// `CSI 1 ; mods letter`, with the `1 ;` left out when there are no modifiers.
    Letter(u8),
    /// `CSI number ; mods ~`.
    Tilde(u32),
    /// `CSI number ; mods u`.
    Unicode(u32),
}

impl Func {
    fn csi(self, mods: u32, event: Option<u8>) -> Vec<u8> {
        match self {
            Func::Letter(l) => {
                let number = if mods > 1 || event.is_some() { "1" } else { "" };
                csi(number, mods, event, None, l)
            }
            Func::Tilde(n) => csi(&n.to_string(), mods, event, None, b'~'),
            Func::Unicode(n) => csi(&n.to_string(), mods, event, None, b'u'),
        }
    }
}

fn functional(code: KeyCode) -> Option<Func> {
    use KeyCode::*;
    Some(match code {
        Char(c) => Func::Unicode(u32::from(c)),
        Enter => Func::Unicode(13),
        Tab => Func::Unicode(9),
        Backspace => Func::Unicode(127),
        Escape => Func::Unicode(27),
        Up => Func::Letter(b'A'),
        Down => Func::Letter(b'B'),
        Right => Func::Letter(b'C'),
        Left => Func::Letter(b'D'),
        Home => Func::Letter(b'H'),
        End => Func::Letter(b'F'),
        Insert => Func::Tilde(2),
        Delete => Func::Tilde(3),
        PageUp => Func::Tilde(5),
        PageDown => Func::Tilde(6),
        F(1) => Func::Letter(b'P'),
        F(2) => Func::Letter(b'Q'),
        F(3) => Func::Tilde(13),
        F(4) => Func::Letter(b'S'),
        F(5) => Func::Tilde(15),
        F(6) => Func::Tilde(17),
        F(7) => Func::Tilde(18),
        F(8) => Func::Tilde(19),
        F(9) => Func::Tilde(20),
        F(10) => Func::Tilde(21),
        F(11) => Func::Tilde(23),
        F(12) => Func::Tilde(24),
        // F13 to F35 have no xterm sequence, so they use the kitty numbers in every mode.
        F(n @ 13..=35) => Func::Unicode(57376 + u32::from(n) - 13),
        F(_) => return None,
    })
}

fn kitty(key: &KeyInput, modes: &Modes) -> Option<Vec<u8>> {
    use KeyCode::*;
    let flags = modes.kitty_flags;
    let report_events = flags & REPORT_EVENTS != 0;
    let all_keys = flags & REPORT_ALL_KEYS != 0;
    // Reporting every key as an escape code implies disambiguation.
    let disambiguate = all_keys || flags & DISAMBIGUATE != 0;
    let release = key.kind == KeyEventKind::Release;
    if release && !report_events {
        return None;
    }

    if !all_keys {
        if let Char(c) = key.code
            && is_text_key(key)
            && !release
        {
            return Some(typed(key, c).into_bytes());
        }
        // Left as legacy bytes, with no release, so a shell stays usable when a program exits
        // without turning the protocol off.
        if matches!(key.code, Enter | Tab | Backspace) && key.mods.is_empty() {
            return if release {
                None
            } else {
                legacy_bytes(key, modes.app_cursor)
            };
        }
        // Repeats sent as legacy bytes look like presses, which is all legacy bytes can say.
        if !disambiguate && !release && !key.mods.contains(Mods::SUPER) {
            return legacy_bytes(key, modes.app_cursor);
        }
    }

    let mods = 1 + u32::from(key.mods.bits());
    let event = match key.kind {
        _ if !report_events => None,
        KeyEventKind::Press => None,
        KeyEventKind::Repeat => Some(2),
        KeyEventKind::Release => Some(3),
    };
    let Char(c) = key.code else {
        return Some(functional(key.code)?.csi(mods, event));
    };

    let mut number = u32::from(c).to_string();
    if flags & REPORT_ALTERNATES != 0
        && key.mods.contains(Mods::SHIFT)
        && let Some(shifted) = shifted_key(key, c)
    {
        number.push_str(&format!(":{}", u32::from(shifted)));
    }
    // Flag 16 only means something with flag 8, and a release types nothing.
    let text = (all_keys && flags & REPORT_TEXT != 0 && !release && is_text_key(key))
        .then(|| printable_text(key))
        .flatten()
        .map(|t| {
            t.chars()
                .map(|ch| u32::from(ch).to_string())
                .collect::<Vec<_>>()
                .join(":")
        });
    Some(csi(&number, mods, event, text.as_deref(), b'u'))
}

/// `CSI number ; mods:event ; text end`, leaving out trailing fields that hold their defaults.
fn csi(number: &str, mods: u32, event: Option<u8>, text: Option<&str>, end: u8) -> Vec<u8> {
    let mods_field = match event {
        Some(e) => format!("{mods}:{e}"),
        None if mods > 1 => mods.to_string(),
        None => String::new(),
    };
    let mut s = format!("\x1b[{number}");
    if !mods_field.is_empty() || text.is_some() {
        s.push(';');
        s.push_str(&mods_field);
    }
    if let Some(text) = text {
        s.push(';');
        s.push_str(text);
    }
    let mut out = s.into_bytes();
    out.push(end);
    out
}

/// A key that types its text: nothing held but Shift.
fn is_text_key(key: &KeyInput) -> bool {
    matches!(key.code, KeyCode::Char(_))
        && !key.mods.intersects(Mods::ALT | Mods::CTRL | Mods::SUPER)
}

/// The layout's text when it is something a program can be sent as typed text.
fn printable_text(key: &KeyInput) -> Option<&str> {
    key.text
        .as_deref()
        .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
}

/// What the key types, falling back to the key itself when the layout gave no usable text.
fn typed(key: &KeyInput, c: char) -> String {
    match printable_text(key) {
        Some(text) => text.to_string(),
        None if key.mods.contains(Mods::SHIFT) => c.to_uppercase().collect(),
        None => c.to_string(),
    }
}

/// The shifted key for kitty's alternate key report: the layout's text when it is one character,
/// else the upper case form, and nothing when Shift does not change the key.
fn shifted_key(key: &KeyInput, c: char) -> Option<char> {
    let from_text = printable_text(key).and_then(|t| {
        let mut chars = t.chars();
        let first = chars.next()?;
        chars.next().is_none().then_some(first)
    });
    let shifted = from_text.or_else(|| {
        let mut upper = c.to_uppercase();
        let first = upper.next()?;
        upper.next().is_none().then_some(first)
    })?;
    (shifted != c).then_some(shifted)
}

/// The C0 byte Ctrl turns an ASCII key into, from the common xterm table. Keys not listed are left
/// as they are.
fn ctrl_byte(c: char) -> Option<u8> {
    Some(match c {
        ' ' | '2' | '@' => 0,
        'a'..='z' => c as u8 - b'a' + 1,
        '3' | '[' => 27,
        '4' | '\\' => 28,
        '5' | ']' => 29,
        '6' | '^' | '~' => 30,
        '7' | '/' | '_' => 31,
        '8' | '?' => 127,
        _ => return None,
    })
}

fn push_char(out: &mut Vec<u8>, c: char) {
    let mut buf = [0; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
}
