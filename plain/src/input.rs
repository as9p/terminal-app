//! What the window decides without a window: keys, zoom, the resize rule, click counts, wheel
//! lines and the grid. Pure, so it is tested here.

use std::time::{Duration, Instant};

use term_engine::{
    CellPoint, CellSize, FONT_PX_DEFAULT, FONT_PX_MAX, FONT_PX_MIN, GridSize, KeyCode,
    KeyEventKind, KeyInput, Mods, Rect, fit,
};
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// How long a resize must be still before the grid follows, so a drag reflows once, not per pixel.
const SETTLE: Duration = Duration::from_millis(100);
/// The longest gap between presses on one cell that still makes a double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

/// `None` for a key that means nothing to a terminal: a modifier alone, a dead key, an unknown key.
pub fn key_input(
    logical: &Key,
    text: Option<&str>,
    mods: ModifiersState,
    pressed: bool,
    repeat: bool,
) -> Option<KeyInput> {
    let code = match logical {
        // The code is the key without Shift, so `A` is `a`; a character without a single-character
        // lower case keeps its own.
        Key::Character(s) => {
            let c = s.chars().next()?;
            let mut lower = c.to_lowercase();
            KeyCode::Char(match (lower.next(), lower.next()) {
                (Some(l), None) => l,
                _ => c,
            })
        }
        Key::Named(named) => named_code(*named)?,
        _ => return None,
    };
    let kind = match (pressed, repeat) {
        (false, _) => KeyEventKind::Release,
        (true, true) => KeyEventKind::Repeat,
        (true, false) => KeyEventKind::Press,
    };
    Some(KeyInput {
        code,
        mods: key_mods(mods),
        // winit gives Enter "\r", Backspace "\x08" and the like as text; the encoder writes those
        // keys itself.
        text: text
            .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
            .map(str::to_string),
        kind,
    })
}

fn named_code(named: NamedKey) -> Option<KeyCode> {
    use NamedKey::*;
    Some(match named {
        Enter => KeyCode::Enter,
        Tab => KeyCode::Tab,
        Backspace => KeyCode::Backspace,
        Escape => KeyCode::Escape,
        Delete => KeyCode::Delete,
        Insert => KeyCode::Insert,
        ArrowLeft => KeyCode::Left,
        ArrowRight => KeyCode::Right,
        ArrowUp => KeyCode::Up,
        ArrowDown => KeyCode::Down,
        Home => KeyCode::Home,
        End => KeyCode::End,
        PageUp => KeyCode::PageUp,
        PageDown => KeyCode::PageDown,
        // winit names the space bar rather than giving it as a character.
        Space => KeyCode::Char(' '),
        F1 => KeyCode::F(1),
        F2 => KeyCode::F(2),
        F3 => KeyCode::F(3),
        F4 => KeyCode::F(4),
        F5 => KeyCode::F(5),
        F6 => KeyCode::F(6),
        F7 => KeyCode::F(7),
        F8 => KeyCode::F(8),
        F9 => KeyCode::F(9),
        F10 => KeyCode::F(10),
        F11 => KeyCode::F(11),
        F12 => KeyCode::F(12),
        F13 => KeyCode::F(13),
        F14 => KeyCode::F(14),
        F15 => KeyCode::F(15),
        F16 => KeyCode::F(16),
        F17 => KeyCode::F(17),
        F18 => KeyCode::F(18),
        F19 => KeyCode::F(19),
        F20 => KeyCode::F(20),
        F21 => KeyCode::F(21),
        F22 => KeyCode::F(22),
        F23 => KeyCode::F(23),
        F24 => KeyCode::F(24),
        F25 => KeyCode::F(25),
        F26 => KeyCode::F(26),
        F27 => KeyCode::F(27),
        F28 => KeyCode::F(28),
        F29 => KeyCode::F(29),
        F30 => KeyCode::F(30),
        F31 => KeyCode::F(31),
        F32 => KeyCode::F(32),
        F33 => KeyCode::F(33),
        F34 => KeyCode::F(34),
        F35 => KeyCode::F(35),
        _ => return None,
    })
}

pub fn key_mods(state: ModifiersState) -> Mods {
    let mut mods = Mods::empty();
    mods.set(Mods::SHIFT, state.shift_key());
    mods.set(Mods::ALT, state.alt_key());
    mods.set(Mods::CTRL, state.control_key());
    mods.set(Mods::SUPER, state.super_key());
    mods
}

/// Only the left Option is Meta. The right one composes text (`å`, `€`) that is sent as it is,
/// so a character typed with it carries no Alt; keys without text keep Alt for word moves.
pub fn option_mods(logical: &Key, mods: ModifiersState, left_option: bool) -> ModifiersState {
    if mods.alt_key() && !left_option && matches!(logical, Key::Character(_)) {
        mods - ModifiersState::ALT
    } else {
        mods
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowKey {
    Quit,
    Close,
    Copy,
    Paste,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

/// The Command keys the window handles before the terminal sees them. A release is not one.
pub fn window_key(input: &KeyInput) -> Option<WindowKey> {
    if input.kind == KeyEventKind::Release {
        return None;
    }
    let KeyCode::Char(c) = input.code else {
        return None;
    };
    // Cmd+= and Cmd++ are one key on most layouts, so Shift may be held for zooming in.
    if c == '=' || c == '+' {
        return (input.mods == Mods::SUPER || input.mods == Mods::SUPER | Mods::SHIFT)
            .then_some(WindowKey::ZoomIn);
    }
    if input.mods != Mods::SUPER {
        return None;
    }
    Some(match c {
        'q' => WindowKey::Quit,
        'w' => WindowKey::Close,
        'c' => WindowKey::Copy,
        'v' => WindowKey::Paste,
        '-' => WindowKey::ZoomOut,
        '0' => WindowKey::ZoomReset,
        _ => return None,
    })
}

/// The text size after a zoom key; other keys leave it as it is.
pub fn zoomed(font_px: f32, key: WindowKey) -> f32 {
    match key {
        WindowKey::ZoomIn => (font_px + 1.0).clamp(FONT_PX_MIN, FONT_PX_MAX),
        WindowKey::ZoomOut => (font_px - 1.0).clamp(FONT_PX_MIN, FONT_PX_MAX),
        WindowKey::ZoomReset => FONT_PX_DEFAULT,
        _ => font_px,
    }
}

/// The grid follows a resize once the size has been still for 100 ms.
pub struct Settle {
    last: Option<Instant>,
}

impl Settle {
    pub fn new() -> Settle {
        Settle { last: None }
    }

    pub fn changed(&mut self, now: Instant) {
        self.last = Some(now);
    }

    /// True once per burst of changes, when the last of them is 100 ms old.
    pub fn due(&mut self, now: Instant) -> bool {
        let due = self.deadline().is_some_and(|deadline| now >= deadline);
        if due {
            self.last = None;
        }
        due
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.last.map(|last| last + SETTLE)
    }
}

/// Counts presses on one cell for double and triple clicks.
pub struct Clicks {
    last: Option<(CellPoint, Instant)>,
    count: u8,
}

impl Clicks {
    pub fn new() -> Clicks {
        Clicks {
            last: None,
            count: 0,
        }
    }

    /// 1, 2 or 3; a fourth quick press starts again at 1.
    pub fn press(&mut self, at: CellPoint, now: Instant) -> u8 {
        let quick = self.last.is_some_and(|(cell, then)| {
            cell == at && now.saturating_duration_since(then) <= MULTI_CLICK
        });
        self.count = if quick && self.count < 3 {
            self.count + 1
        } else {
            1
        };
        self.last = Some((at, now));
        self.count
    }
}

/// Trackpad pixels to whole lines; what is left over waits for the next event.
pub struct WheelLines {
    rest: f64,
}

impl WheelLines {
    pub fn new() -> WheelLines {
        WheelLines { rest: 0.0 }
    }

    /// Positive is up, as winit's and the engine's both are.
    pub fn pixels(&mut self, dy: f64, cell_h: u32) -> i32 {
        let cell_h = f64::from(cell_h.max(1));
        self.rest += dy;
        let lines = (self.rest / cell_h).trunc();
        self.rest -= lines * cell_h;
        // A wild delta is cut here; the engine caps what it does with it anyway.
        lines.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    }
}

/// The view fills the whole surface, so the grid is what fits in it.
pub fn grid_for(size_px: (u32, u32), cell: CellSize) -> GridSize {
    let (w, h) = size_px;
    fit(Rect { x: 0, y: 0, w, h }, cell)
}

/// A hidden window draws nothing, and a surface with a zero side cannot be configured.
pub fn may_present(visible: bool, size_px: (u32, u32)) -> bool {
    visible && size_px.0 > 0 && size_px.1 > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use term_engine::{CellPoint, CellSize, GridSize, KeyCode, KeyEventKind, Mods};
    use winit::keyboard::{Key, ModifiersState, NamedKey};

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    fn press(key: &Key, text: Option<&str>, mods: ModifiersState) -> Option<KeyInput> {
        key_input(key, text, mods, true, false)
    }

    fn cmd(s: &str) -> KeyInput {
        press(&ch(s), Some(s), ModifiersState::SUPER).unwrap()
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn letters_carry_their_text() {
        let key = press(&ch("a"), Some("a"), ModifiersState::empty()).unwrap();
        assert_eq!(key.code, KeyCode::Char('a'));
        assert_eq!(key.text.as_deref(), Some("a"));
        assert_eq!(key.mods, Mods::empty());
        assert_eq!(key.kind, KeyEventKind::Press);
    }

    #[test]
    fn shift_keeps_the_base_key() {
        let key = press(&ch("A"), Some("A"), ModifiersState::SHIFT).unwrap();
        assert_eq!(key.code, KeyCode::Char('a'));
        assert_eq!(key.text.as_deref(), Some("A"));
        assert_eq!(key.mods, Mods::SHIFT);
    }

    #[test]
    fn named_keys_map() {
        let cases = [
            (NamedKey::Enter, KeyCode::Enter),
            (NamedKey::Tab, KeyCode::Tab),
            (NamedKey::Backspace, KeyCode::Backspace),
            (NamedKey::Escape, KeyCode::Escape),
            (NamedKey::ArrowLeft, KeyCode::Left),
            (NamedKey::Home, KeyCode::Home),
            (NamedKey::PageDown, KeyCode::PageDown),
            (NamedKey::F5, KeyCode::F(5)),
            (NamedKey::Delete, KeyCode::Delete),
        ];
        for (named, code) in cases {
            let key = press(&Key::Named(named), None, ModifiersState::empty()).unwrap();
            assert_eq!(key.code, code, "{named:?}");
        }
        // winit's text for these is a control character, which is not text for a terminal.
        let enter = press(
            &Key::Named(NamedKey::Enter),
            Some("\r"),
            ModifiersState::empty(),
        );
        assert_eq!(enter.unwrap().text, None);
        assert_eq!(
            press(&Key::Named(NamedKey::Shift), None, ModifiersState::SHIFT),
            None
        );
    }

    #[test]
    fn release_and_repeat_are_marked() {
        let release = key_input(&ch("a"), None, ModifiersState::empty(), false, false).unwrap();
        assert_eq!(release.kind, KeyEventKind::Release);
        let repeat = key_input(&ch("a"), Some("a"), ModifiersState::empty(), true, true).unwrap();
        assert_eq!(repeat.kind, KeyEventKind::Repeat);
    }

    #[test]
    fn command_is_super() {
        let key = press(&ch("k"), Some("k"), ModifiersState::SUPER).unwrap();
        assert_eq!(key.mods, Mods::SUPER);
        let key = press(&ch("b"), Some("b"), ModifiersState::ALT).unwrap();
        assert_eq!(key.mods, Mods::ALT);
    }

    #[test]
    fn right_option_composes() {
        let alt = ModifiersState::ALT;
        // The left Option is Meta: winit already gave the key without the composition.
        assert_eq!(option_mods(&ch("b"), alt, true), alt);
        // The right Option typed `å`, which is sent as it is, not as Escape then `å`.
        assert_eq!(option_mods(&ch("å"), alt, false), ModifiersState::empty());
        // Option+Left still moves by a word with either key.
        let left = Key::Named(NamedKey::ArrowLeft);
        assert_eq!(option_mods(&left, alt, false), alt);
    }

    #[test]
    fn window_keys() {
        assert_eq!(window_key(&cmd("q")), Some(WindowKey::Quit));
        assert_eq!(window_key(&cmd("w")), Some(WindowKey::Close));
        assert_eq!(window_key(&cmd("c")), Some(WindowKey::Copy));
        assert_eq!(window_key(&cmd("v")), Some(WindowKey::Paste));
        assert_eq!(window_key(&cmd("=")), Some(WindowKey::ZoomIn));
        // With Command held winit reports the shifted character, `+` on a US layout.
        let shift_cmd = ModifiersState::SUPER | ModifiersState::SHIFT;
        for plus in ["=", "+"] {
            let key = press(&ch(plus), Some(plus), shift_cmd).unwrap();
            assert_eq!(window_key(&key), Some(WindowKey::ZoomIn), "{plus}");
        }
        assert_eq!(window_key(&cmd("-")), Some(WindowKey::ZoomOut));
        assert_eq!(window_key(&cmd("0")), Some(WindowKey::ZoomReset));
        assert_eq!(window_key(&cmd("k")), None);
        let q = press(&ch("q"), Some("q"), ModifiersState::empty()).unwrap();
        assert_eq!(window_key(&q), None);
    }

    #[test]
    fn zoom_steps_and_limits() {
        assert_eq!(zoomed(14.0, WindowKey::ZoomIn), 15.0);
        assert_eq!(zoomed(40.0, WindowKey::ZoomIn), 40.0);
        assert_eq!(zoomed(8.0, WindowKey::ZoomOut), 8.0);
        assert_eq!(zoomed(23.0, WindowKey::ZoomReset), 14.0);
    }

    #[test]
    fn settle_fires_once_after_100_ms() {
        let t = Instant::now();
        let mut settle = Settle::new();
        assert_eq!(settle.deadline(), None);
        settle.changed(t);
        assert!(!settle.due(t + ms(99)));
        settle.changed(t + ms(50));
        assert_eq!(settle.deadline(), Some(t + ms(150)));
        assert!(!settle.due(t + ms(149)));
        assert!(settle.due(t + ms(150)));
        assert!(!settle.due(t + ms(300)));
        assert_eq!(settle.deadline(), None);
    }

    #[test]
    fn clicks_count_to_three() {
        let t = Instant::now();
        let cell = CellPoint { col: 3, row: 2 };
        let mut clicks = Clicks::new();
        let counts: Vec<u8> = [0, 100, 200, 300]
            .into_iter()
            .map(|n| clicks.press(cell, t + ms(n)))
            .collect();
        assert_eq!(counts, [1, 2, 3, 1]);
        assert_eq!(clicks.press(cell, t + ms(701)), 1);
        assert_eq!(clicks.press(cell, t + ms(800)), 2);
        assert_eq!(clicks.press(CellPoint { col: 4, row: 2 }, t + ms(900)), 1);
    }

    #[test]
    fn trackpad_pixels_become_lines() {
        let mut wheel = WheelLines::new();
        assert_eq!(wheel.pixels(20.0, 31), 0);
        assert_eq!(wheel.pixels(20.0, 31), 1);
        assert_eq!(wheel.pixels(-71.0, 31), -2);
    }

    #[test]
    fn scale_change_refits() {
        assert_eq!(
            grid_for((2560, 1600), CellSize { w: 17, h: 40 }),
            GridSize {
                cols: 150,
                rows: 40
            }
        );
        assert_eq!(
            grid_for((1280, 800), CellSize { w: 8, h: 20 }),
            GridSize {
                cols: 160,
                rows: 40
            }
        );
    }

    #[test]
    fn hidden_or_empty_window_never_presents() {
        assert!(!may_present(false, (800, 600)));
        assert!(!may_present(true, (0, 600)));
        assert!(!may_present(true, (800, 0)));
        assert!(may_present(true, (1, 1)));
        assert_eq!(
            grid_for((0, 0), CellSize { w: 8, h: 20 }),
            GridSize { cols: 1, rows: 1 }
        );
    }
}
