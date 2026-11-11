//! Key and mouse encoding, one test per table in the stage A plan.

use term_engine::{
    Button, CellPoint, KeyCode, KeyEventKind, KeyInput, Modes, Mods, MouseAction, MouseTracking,
    Platform, encode_key, encode_mouse,
};

fn modes() -> Modes {
    Modes {
        bracketed_paste: false,
        app_cursor: false,
        app_keypad: false,
        mouse: MouseTracking::Off,
        sgr_mouse: false,
        alt_screen: false,
        alternate_scroll: false,
        focus_events: false,
        kitty_flags: 0,
    }
}

fn kitty(flags: u8) -> Modes {
    Modes {
        kitty_flags: flags,
        ..modes()
    }
}

fn mouse_modes(mouse: MouseTracking, sgr_mouse: bool) -> Modes {
    Modes {
        mouse,
        sgr_mouse,
        ..modes()
    }
}

fn key(code: KeyCode, mods: Mods) -> KeyInput {
    KeyInput {
        code,
        mods,
        text: None,
        kind: KeyEventKind::Press,
    }
}

fn text_key(code: KeyCode, mods: Mods, text: &str) -> KeyInput {
    KeyInput {
        text: Some(text.to_string()),
        ..key(code, mods)
    }
}

fn kind(mut input: KeyInput, kind: KeyEventKind) -> KeyInput {
    input.kind = kind;
    input
}

fn mac(input: &KeyInput, modes: &Modes) -> Option<Vec<u8>> {
    encode_key(input, modes, Platform::Mac)
}

fn other(input: &KeyInput, modes: &Modes) -> Option<Vec<u8>> {
    encode_key(input, modes, Platform::Other)
}

fn bytes(b: &[u8]) -> Option<Vec<u8>> {
    Some(b.to_vec())
}

fn at(col: u16, row: u16) -> CellPoint {
    CellPoint { col, row }
}

#[test]
fn legacy_keys() {
    use KeyCode::*;
    let m = modes();
    let none = Mods::empty();

    assert_eq!(mac(&text_key(Char('a'), none, "a"), &m), bytes(b"a"));
    assert_eq!(mac(&text_key(Char('a'), Mods::SHIFT, "A"), &m), bytes(b"A"));
    assert_eq!(
        mac(&text_key(Char('\u{E9}'), none, "\u{E9}"), &m),
        bytes(b"\xc3\xa9")
    );
    assert_eq!(mac(&key(Char('c'), Mods::CTRL), &m), bytes(b"\x03"));
    assert_eq!(mac(&key(Char(' '), Mods::CTRL), &m), bytes(b"\x00"));
    assert_eq!(mac(&key(Char('a'), Mods::ALT), &m), bytes(b"\x1ba"));
    assert_eq!(mac(&key(Enter, none), &m), bytes(b"\r"));
    assert_eq!(mac(&key(Enter, Mods::SHIFT), &m), bytes(b"\x1b\r"));
    assert_eq!(mac(&key(Enter, Mods::ALT), &m), bytes(b"\x1b\r"));
    assert_eq!(mac(&key(Tab, none), &m), bytes(b"\t"));
    assert_eq!(mac(&key(Tab, Mods::SHIFT), &m), bytes(b"\x1b[Z"));
    assert_eq!(mac(&key(Backspace, none), &m), bytes(b"\x7f"));
    assert_eq!(mac(&key(Backspace, Mods::CTRL), &m), bytes(b"\x08"));
    assert_eq!(mac(&key(Escape, none), &m), bytes(b"\x1b"));
    assert_eq!(mac(&key(Up, none), &m), bytes(b"\x1b[A"));
    let app_cursor = Modes {
        app_cursor: true,
        ..modes()
    };
    assert_eq!(mac(&key(Up, none), &app_cursor), bytes(b"\x1bOA"));
    assert_eq!(mac(&key(Up, Mods::SHIFT), &m), bytes(b"\x1b[1;2A"));
    assert_eq!(mac(&key(Right, Mods::CTRL), &m), bytes(b"\x1b[1;5C"));
    assert_eq!(mac(&key(Home, none), &m), bytes(b"\x1b[H"));
    assert_eq!(mac(&key(End, none), &m), bytes(b"\x1b[F"));
    assert_eq!(mac(&key(PageUp, none), &m), bytes(b"\x1b[5~"));
    assert_eq!(mac(&key(PageDown, none), &m), bytes(b"\x1b[6~"));
    assert_eq!(mac(&key(Delete, none), &m), bytes(b"\x1b[3~"));
    assert_eq!(mac(&key(Insert, none), &m), bytes(b"\x1b[2~"));
    assert_eq!(mac(&key(F(1), none), &m), bytes(b"\x1bOP"));
    assert_eq!(mac(&key(F(5), none), &m), bytes(b"\x1b[15~"));
    assert_eq!(mac(&key(F(12), none), &m), bytes(b"\x1b[24~"));

    let release = KeyEventKind::Release;
    assert_eq!(
        mac(&kind(text_key(Char('a'), none, "a"), release), &m),
        None
    );
    assert_eq!(mac(&kind(key(Enter, none), release), &m), None);
    assert_eq!(mac(&kind(key(Up, Mods::SHIFT), release), &m), None);
    assert_eq!(mac(&kind(key(Char('c'), Mods::CTRL), release), &m), None);
    assert_eq!(mac(&kind(key(Backspace, Mods::SUPER), release), &m), None);
}

#[test]
fn mac_line_keys() {
    use KeyCode::*;
    // The Mac rules hold whatever the kitty flags are.
    for flags in [0, 1, 3, 8, 31] {
        let m = kitty(flags);
        assert_eq!(mac(&key(Backspace, Mods::SUPER), &m), bytes(b"\x15"));
        assert_eq!(mac(&key(Left, Mods::SUPER), &m), bytes(b"\x01"));
        assert_eq!(mac(&key(Right, Mods::SUPER), &m), bytes(b"\x05"));
        assert_eq!(mac(&key(Delete, Mods::SUPER), &m), bytes(b"\x0b"));
        assert_eq!(mac(&key(Char('z'), Mods::SUPER), &m), bytes(b"\x1f"));
        assert_eq!(mac(&key(Left, Mods::ALT), &m), bytes(b"\x1bb"));
        assert_eq!(mac(&key(Right, Mods::ALT), &m), bytes(b"\x1bf"));
        assert_eq!(mac(&key(Backspace, Mods::ALT), &m), bytes(b"\x1b\x7f"));
        assert_eq!(mac(&key(Delete, Mods::ALT), &m), bytes(b"\x1bd"));
        assert_eq!(mac(&key(Char('t'), Mods::SUPER), &m), None);
    }
    let m = modes();
    assert_eq!(other(&key(Left, Mods::ALT), &m), bytes(b"\x1b[1;3D"));
    assert_eq!(other(&key(Left, Mods::SUPER), &m), None);
}

#[test]
fn kitty_keys() {
    use KeyCode::*;
    let none = Mods::empty();
    let release = KeyEventKind::Release;
    let repeat = KeyEventKind::Repeat;

    let k1 = kitty(1);
    assert_eq!(other(&key(Enter, Mods::SHIFT), &k1), bytes(b"\x1b[13;2u"));
    assert_eq!(other(&key(Enter, none), &k1), bytes(b"\r"));
    assert_eq!(other(&key(Tab, none), &k1), bytes(b"\t"));
    assert_eq!(other(&key(Backspace, none), &k1), bytes(b"\x7f"));
    assert_eq!(other(&key(Escape, none), &k1), bytes(b"\x1b[27u"));
    assert_eq!(
        other(&key(Char('c'), Mods::CTRL), &k1),
        bytes(b"\x1b[99;5u")
    );
    assert_eq!(other(&key(Char('a'), Mods::ALT), &k1), bytes(b"\x1b[97;3u"));
    assert_eq!(
        other(&key(Char('a'), Mods::CTRL | Mods::SHIFT), &k1),
        bytes(b"\x1b[97;6u")
    );
    assert_eq!(other(&key(Tab, Mods::SHIFT), &k1), bytes(b"\x1b[9;2u"));
    assert_eq!(
        other(&key(Backspace, Mods::CTRL), &k1),
        bytes(b"\x1b[127;5u")
    );
    assert_eq!(other(&text_key(Char('a'), none, "a"), &k1), bytes(b"a"));
    assert_eq!(
        other(&text_key(Char('a'), Mods::SHIFT, "A"), &k1),
        bytes(b"A")
    );
    assert_eq!(other(&key(Up, none), &k1), bytes(b"\x1b[A"));
    assert_eq!(other(&key(Up, Mods::CTRL), &k1), bytes(b"\x1b[1;5A"));

    let k3 = kitty(3);
    assert_eq!(
        other(&kind(key(Escape, none), release), &k3),
        bytes(b"\x1b[27;1:3u")
    );
    assert_eq!(
        other(&kind(key(Enter, Mods::SHIFT), repeat), &k3),
        bytes(b"\x1b[13;2:2u")
    );
    assert_eq!(
        other(&kind(text_key(Char('a'), none, "a"), release), &k3),
        bytes(b"\x1b[97;1:3u")
    );
    assert_eq!(other(&kind(key(Enter, none), release), &k3), None);

    let k8 = kitty(8);
    assert_eq!(
        other(&text_key(Char('a'), none, "a"), &k8),
        bytes(b"\x1b[97u")
    );
    assert_eq!(other(&key(Enter, none), &k8), bytes(b"\x1b[13u"));

    assert_eq!(
        other(&text_key(Char('a'), Mods::SHIFT, "A"), &kitty(12)),
        bytes(b"\x1b[97:65;2u")
    );
    assert_eq!(
        other(&text_key(Char('a'), Mods::SHIFT, "A"), &kitty(24)),
        bytes(b"\x1b[97;2;65u")
    );
}

#[test]
fn mouse_reports() {
    use Button::*;
    use MouseAction::*;
    use MouseTracking::*;
    let none = Mods::empty();
    let origin = at(0, 0);

    assert_eq!(
        encode_mouse(Press(Left), origin, none, &mouse_modes(Off, true)),
        None
    );

    let click = mouse_modes(Click, true);
    assert_eq!(
        encode_mouse(Press(Left), origin, none, &click),
        bytes(b"\x1b[<0;1;1M")
    );
    assert_eq!(
        encode_mouse(Release(Left), origin, none, &click),
        bytes(b"\x1b[<0;1;1m")
    );
    assert_eq!(
        encode_mouse(Press(Middle), origin, none, &click),
        bytes(b"\x1b[<1;1;1M")
    );
    assert_eq!(
        encode_mouse(Press(Right), origin, none, &click),
        bytes(b"\x1b[<2;1;1M")
    );
    assert_eq!(
        encode_mouse(Press(Left), origin, Mods::CTRL, &click),
        bytes(b"\x1b[<16;1;1M")
    );
    assert_eq!(
        encode_mouse(Wheel(1), origin, none, &click),
        bytes(b"\x1b[<64;1;1M")
    );
    assert_eq!(
        encode_mouse(Wheel(-1), origin, none, &click),
        bytes(b"\x1b[<65;1;1M")
    );
    assert_eq!(
        encode_mouse(Wheel(3), origin, none, &click),
        bytes(b"\x1b[<64;1;1M\x1b[<64;1;1M\x1b[<64;1;1M")
    );
    assert_eq!(encode_mouse(Move(Some(Left)), origin, none, &click), None);

    let drag = mouse_modes(Drag, true);
    assert_eq!(
        encode_mouse(Move(Some(Left)), at(2, 1), none, &drag),
        bytes(b"\x1b[<32;3;2M")
    );
    assert_eq!(encode_mouse(Move(None), origin, none, &drag), None);

    assert_eq!(
        encode_mouse(Move(None), at(2, 1), none, &mouse_modes(Motion, true)),
        bytes(b"\x1b[<35;3;2M")
    );

    let x10 = mouse_modes(Click, false);
    assert_eq!(
        encode_mouse(Press(Left), origin, none, &x10),
        bytes(&[0x1b, b'[', b'M', 32, 33, 33])
    );
    assert_eq!(encode_mouse(Press(Left), at(300, 0), none, &x10), None);
}
