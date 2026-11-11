//! The screen library gate: everything on the day-one list, proved through our own `Screen` type.
//! If one of these cannot pass through the wrapper, the library does not stay.

use std::time::Duration;

use term_engine::{
    CellPoint, CellSize, CellWidth, Damage, GridSize, MouseTracking, Rect, Rgb, Screen,
    ScreenEvent, SelectKind, Side, Theme, fit,
};

fn screen(cols: u16, rows: u16) -> Screen {
    Screen::new(GridSize { cols, rows }, &Theme::builtin().palette)
}

fn at(col: u16, row: u16) -> CellPoint {
    CellPoint { col, row }
}

#[test]
fn scrollback_keeps_20000_lines() {
    let mut s = screen(80, 24);
    for n in 1..=25_000 {
        s.feed(format!("line {n}\r\n").as_bytes());
    }
    assert_eq!(s.history_len(), 20_000);
    s.scroll_to_top();
    assert_eq!(s.row_text(0), "line 4978");
    s.scroll_to_bottom();
    assert_eq!(s.row_text(22), "line 25000");
}

#[test]
fn rewraps_on_resize() {
    let mut s = screen(80, 24);
    s.feed("a".repeat(100).as_bytes());
    s.resize(GridSize { cols: 40, rows: 24 });
    s.scroll_to_top(); // the library may push the first row into the scrollback; either way the text starts at the top
    assert_eq!(
        [
            s.row_text(0).len(),
            s.row_text(1).len(),
            s.row_text(2).len()
        ],
        [40, 40, 20]
    );
    s.resize(GridSize {
        cols: 120,
        rows: 24,
    });
    s.scroll_to_top();
    assert_eq!((s.row_text(0).len(), s.row_text(1).as_str()), (100, ""));
}

#[test]
fn split_reads_give_the_same_screen() {
    let input = "\x1b[31mre\u{301}d \u{6F22}\u{5B57} \x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\\x1b[?2026h\x1b[2;1Hrow two\x1b[?2026l".as_bytes();
    let dump = |s: &Screen| (0..24).map(|r| s.row_text(r)).collect::<Vec<_>>();
    let mut whole = screen(80, 24);
    whole.feed(input);
    for cut in 1..input.len() {
        let mut s = screen(80, 24);
        s.feed(&input[..cut]);
        s.feed(&input[cut..]);
        assert_eq!(dump(&s), dump(&whole), "cut at {cut}");
        assert_eq!(s.cursor(), whole.cursor(), "cut at {cut}");
    }
}

#[test]
fn fit_never_returns_zero() {
    let cell = CellSize { w: 17, h: 31 };
    assert_eq!(
        fit(
            Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0
            },
            cell
        ),
        GridSize { cols: 1, rows: 1 }
    );
    assert_eq!(
        fit(
            Rect {
                x: 0,
                y: 0,
                w: 1700,
                h: 310
            },
            cell
        ),
        GridSize {
            cols: 100,
            rows: 10
        }
    );
}

#[test]
fn selects_by_character_word_and_line() {
    let mut s = screen(80, 24);
    s.feed(b"hello brave world");

    s.select(SelectKind::Char, at(0, 0), Side::Left);
    s.select_to(at(4, 0), Side::Right);
    assert_eq!(s.selection_text().as_deref(), Some("hello"));

    s.select(SelectKind::Word, at(7, 0), Side::Left);
    assert_eq!(s.selection_text().as_deref(), Some("brave"));

    s.select(SelectKind::Line, at(7, 0), Side::Left);
    let line = s.selection_text().expect("a line selection has text");
    assert_eq!(line.trim_end_matches('\n'), "hello brave world");
    assert!(s.is_selected(at(7, 0)));

    s.clear_selection();
    assert_eq!(s.selection_text(), None);
}

#[test]
fn text_around_joins_wrapped_rows() {
    let mut s = screen(80, 24);
    s.feed("a".repeat(100).as_bytes());
    assert_eq!(s.text_around(at(5, 1)), ("a".repeat(100), 85));
}

#[test]
fn reports_mouse_modes() {
    let mut s = screen(80, 24);
    s.feed(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(s.modes().mouse, MouseTracking::Click);
    assert!(s.modes().sgr_mouse);
    s.feed(b"\x1b[?1002h");
    assert_eq!(s.modes().mouse, MouseTracking::Drag);
    s.feed(b"\x1b[?1003h");
    assert_eq!(s.modes().mouse, MouseTracking::Motion);
    s.feed(b"\x1b[?1003l\x1b[?1002l\x1b[?1000l");
    assert_eq!(s.modes().mouse, MouseTracking::Off);
}

#[test]
fn reports_paste_focus_and_screen_modes() {
    let mut s = screen(80, 24);
    s.feed(b"\x1b[?2004h\x1b[?1004h\x1b[?1h\x1b[?1049h");
    let m = s.modes();
    assert!(m.bracketed_paste, "{m:?}");
    assert!(m.focus_events, "{m:?}");
    assert!(m.app_cursor, "{m:?}");
    assert!(m.alt_screen, "{m:?}");
}

#[test]
fn synchronized_output_holds_changes() {
    let mut s = screen(80, 24);
    s.feed(b"\x1b[?2026hHELLO");
    assert_eq!(s.row_text(0), "");
    assert!(s.sync_deadline().is_some());
    s.feed(b"\x1b[?2026l");
    assert_eq!(s.row_text(0), "HELLO");
    assert!(s.sync_deadline().is_none());
}

#[test]
fn overdue_sync_is_ended() {
    let mut s = screen(80, 24);
    s.feed(b"\x1b[?2026hHELLO");
    let d = s.sync_deadline().expect("a synchronized update is open");
    let ms = Duration::from_millis(1);
    assert!(!s.end_sync_if_due(d - ms));
    assert_eq!(s.row_text(0), "");
    assert!(s.end_sync_if_due(d + ms));
    assert_eq!(s.row_text(0), "HELLO");
}

#[test]
fn marked_links_are_on_their_cells() {
    let mut s = screen(80, 24);
    s.feed(b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ plain");
    assert_eq!(s.link_at(at(1, 0)).as_deref(), Some("https://example.com"));
    assert_eq!(s.link_at(at(6, 0)), None);
}

#[test]
fn wide_and_combined_characters_land_in_cells() {
    let mut s = screen(80, 24);
    s.feed("a\u{6F22}b e\u{301}".as_bytes());
    let mut cells = Vec::new();
    s.cells(0, &mut cells);
    assert_eq!(cells.len(), 80);
    assert_eq!((cells[1].ch, cells[1].width), ('\u{6F22}', CellWidth::Wide));
    assert_eq!(cells[2].width, CellWidth::Spacer);
    assert_eq!(cells[3].ch, 'b');
    assert_eq!(
        (cells[5].ch, cells[5].zerowidth.as_slice()),
        ('e', &['\u{301}'][..])
    );
}

#[test]
fn kitty_keyboard_flags_follow_the_program() {
    let mut s = screen(80, 24);
    s.feed(b"\x1b[?u");
    assert_eq!(s.take_replies(), b"\x1b[?0u");
    s.feed(b"\x1b[>1u");
    assert_eq!(s.modes().kitty_flags, 1);
    s.feed(b"\x1b[<u");
    assert_eq!(s.modes().kitty_flags, 0);
}

#[test]
fn answers_the_queries_claude_code_sends() {
    let mut s = screen(80, 24);

    s.feed(b"\x1b[c");
    let attributes = s.take_replies();
    assert!(
        attributes.starts_with(b"\x1b[?") && attributes.ends_with(b"c"),
        "{:?}",
        String::from_utf8_lossy(&attributes)
    );

    s.feed(b"\x1b[6n");
    assert_eq!(s.take_replies(), b"\x1b[1;1R");

    s.feed(b"\x1b[?2026$p");
    assert_eq!(s.take_replies(), b"\x1b[?2026;2$y");

    s.feed(b"\x1b]11;?\x1b\\");
    let background = String::from_utf8(s.take_replies()).expect("the reply is text");
    assert!(background.contains("rgb:1616/1414/1f1f"), "{background:?}");
}

#[test]
fn colours_are_final() {
    let mut s = screen(80, 24);
    s.feed(b"\x1b[31mr\x1b[0m\x1b[7mi\x1b[0m\x1b[38;2;1;2;3mt");
    let mut cells = Vec::new();
    s.cells(0, &mut cells);
    assert_eq!(cells[0].fg, Rgb(0xF3, 0x8B, 0xA8));
    assert_eq!(cells[0].bg, Rgb(0x16, 0x14, 0x1F));
    assert_eq!(cells[1].fg, Rgb(0x16, 0x14, 0x1F));
    assert_eq!(cells[1].bg, Rgb(0xCD, 0xD6, 0xF4));
    assert_eq!(cells[2].fg, Rgb(1, 2, 3));
}

#[test]
fn damage_names_only_changed_rows() {
    let mut s = screen(80, 24);
    s.take_damage();
    s.feed(b"x");
    assert_eq!(s.take_damage(), Damage::Rows(vec![0]));
    assert_eq!(s.take_damage(), Damage::None);
    s.select(SelectKind::Char, at(0, 0), Side::Left);
    assert_ne!(s.take_damage(), Damage::None);
    assert_eq!(s.take_damage(), Damage::None);
}

/// The characters on every cell `is_selected` names, in reading order.
fn selected_text(s: &Screen) -> Option<String> {
    let text: String = (0..24u16)
        .flat_map(|row| (0..80u16).map(move |col| at(col, row)))
        .filter(|p| s.is_selected(*p))
        .map(|p| {
            s.row_text(p.row)
                .chars()
                .nth(usize::from(p.col))
                .unwrap_or(' ')
        })
        .collect();
    (!text.is_empty()).then_some(text)
}

#[test]
fn selection_survives_scrolling_and_output() {
    let mut s = screen(80, 24);
    for n in 1..=30 {
        s.feed(format!("row{n} brave\r\n").as_bytes());
    }
    // Rows 0 to 22 show row8 to row30, so row 5 shows row13.
    s.select(SelectKind::Word, at(0, 5), Side::Left);
    assert_eq!(s.selection_text().as_deref(), Some("row13"));
    assert!(s.is_selected(at(2, 5)));
    assert_eq!(selected_text(&s), s.selection_text());

    // One line up, row13 is on row 6 and row 5 shows row12.
    s.scroll_by(1);
    assert_eq!(s.row_text(5), "row12 brave");
    assert!(!s.is_selected(at(2, 5)));
    assert!(s.is_selected(at(2, 6)));
    assert_eq!(selected_text(&s), s.selection_text());

    // Output that scrolls the screen moves the selection with its text, and while scrolled up
    // the view stays on the same lines, so row13 is still selected on row 6.
    s.feed(b"row31 brave\r\n");
    assert_eq!(s.row_text(6), "row13 brave");
    assert_eq!(s.selection_text().as_deref(), Some("row13"));
    assert!(s.is_selected(at(2, 6)));
    assert!(!s.is_selected(at(2, 7)));
    assert_eq!(selected_text(&s), s.selection_text());
}

/// Feeds `bytes` and checks the damage is `Full`, or rows in view whose text really changed.
fn damage_from(s: &mut Screen, bytes: &[u8]) -> Damage {
    let before: Vec<String> = (0..24).map(|r| s.row_text(r)).collect();
    s.feed(bytes);
    let damage = s.take_damage();
    if let Damage::Rows(rows) = &damage {
        for &row in rows {
            assert!(row < 24, "{damage:?}");
            assert_ne!(
                s.row_text(row),
                before[usize::from(row)],
                "row {row} in {damage:?}"
            );
        }
    }
    damage
}

#[test]
fn damage_while_scrolled_up_is_in_view_rows() {
    let mut s = screen(80, 24);
    for n in 1..=100 {
        s.feed(format!("line {n}\r\n").as_bytes());
    }
    s.take_damage();
    s.scroll_by(10);
    s.take_damage();
    damage_from(&mut s, b"line 101\r\n");

    // A change on a line still in view names its row counted from the top of the view.
    let offset = s.scroll_offset() as u16;
    assert_eq!(
        damage_from(&mut s, b"\x1b[5;1Hchanged"),
        Damage::Rows(vec![4 + offset])
    );
}

#[test]
fn title_and_bell_are_events() {
    let mut s = screen(80, 24);
    s.feed(b"\x1b]0;hi\x07\x07");
    let events = s.take_events();
    assert!(
        events.contains(&ScreenEvent::Title("hi".to_owned())),
        "{events:?}"
    );
    assert!(events.contains(&ScreenEvent::Bell), "{events:?}");
}
