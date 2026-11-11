//! The view's input policy, one test per row of the stage A plan's Task 3 table.

use std::time::{Duration, Instant};

use term_engine::{
    Button, CellPoint, GridSize, KeyCode, KeyEventKind, KeyInput, Mods, MouseAction, Platform,
    Side, TerminalView, Theme, ViewEvent, url_at,
};

fn view() -> TerminalView {
    TerminalView::new(
        GridSize { cols: 80, rows: 24 },
        &Theme::builtin(),
        Platform::Mac,
    )
}

fn key(code: KeyCode, mods: Mods) -> KeyInput {
    KeyInput {
        code,
        mods,
        text: None,
        kind: KeyEventKind::Press,
    }
}

fn at(col: u16) -> CellPoint {
    CellPoint { col, row: 0 }
}

fn feed_lines(view: &mut TerminalView, from: usize, count: usize) {
    for n in from..from + count {
        view.feed(format!("line {n}\r\n").as_bytes());
    }
}

fn left_press(view: &mut TerminalView, col: u16, side: Side, mods: Mods, clicks: u8) {
    view.pointer(
        MouseAction::Press(Button::Left),
        at(col),
        side,
        mods,
        clicks,
    );
}

fn left_release(view: &mut TerminalView, col: u16, side: Side, mods: Mods, clicks: u8) {
    view.pointer(
        MouseAction::Release(Button::Left),
        at(col),
        side,
        mods,
        clicks,
    );
}

fn wheel(view: &mut TerminalView, lines: i32) {
    view.pointer(
        MouseAction::Wheel(lines),
        at(0),
        Side::Left,
        Mods::empty(),
        0,
    );
}

fn has_open_link(events: &[ViewEvent]) -> bool {
    events
        .iter()
        .any(|event| matches!(event, ViewEvent::OpenLink(_)))
}

#[test]
fn replies_go_out() {
    let mut view = view();
    view.feed(b"\x1b[6n");
    assert_eq!(view.take_output(), b"\x1b[1;1R");
}

#[test]
fn keys_go_out() {
    let mut view = view();
    let mut a = key(KeyCode::Char('a'), Mods::empty());
    a.text = Some("a".to_string());
    view.key(&a);
    assert_eq!(view.take_output(), b"a");
}

#[test]
fn output_while_scrolled_up_keeps_place() {
    let mut view = view();
    feed_lines(&mut view, 0, 100);
    wheel(&mut view, 10);
    let top = view.screen().row_text(0);
    feed_lines(&mut view, 100, 5);
    assert_eq!(view.screen().row_text(0), top);
    assert_eq!(view.screen().scroll_offset(), 15);

    let mut a = key(KeyCode::Char('a'), Mods::empty());
    a.text = Some("a".to_string());
    view.key(&a);
    assert_eq!(view.screen().scroll_offset(), 0);
}

#[test]
fn drag_selects_and_copies() {
    let mut view = view();
    view.feed(b"hello brave world");
    left_press(&mut view, 0, Side::Left, Mods::empty(), 1);
    view.pointer(
        MouseAction::Move(Some(Button::Left)),
        at(4),
        Side::Right,
        Mods::empty(),
        1,
    );
    left_release(&mut view, 4, Side::Right, Mods::empty(), 1);
    assert_eq!(
        view.take_events(),
        vec![ViewEvent::Copy("hello".to_string())]
    );
    assert_eq!(view.copy(), Some("hello".to_string()));
}

#[test]
fn double_click_selects_a_word() {
    let mut view = view();
    view.feed(b"hello brave world");
    left_press(&mut view, 7, Side::Left, Mods::empty(), 2);
    left_release(&mut view, 7, Side::Left, Mods::empty(), 2);
    assert_eq!(
        view.take_events(),
        vec![ViewEvent::Copy("brave".to_string())]
    );
}

#[test]
fn mouse_goes_to_the_program_when_it_asks() {
    let mut view = view();
    view.feed(b"\x1b[?1000h\x1b[?1006h");
    left_press(&mut view, 0, Side::Left, Mods::empty(), 1);
    assert_eq!(view.take_output(), b"\x1b[<0;1;1M");
    assert_eq!(view.copy(), None);
}

#[test]
fn shift_keeps_the_mouse_for_selecting() {
    let mut view = view();
    // Something to select, since a selection of blank cells is empty and copies nothing.
    view.feed(b"hello brave world");
    view.feed(b"\x1b[?1000h\x1b[?1006h");
    left_press(&mut view, 0, Side::Left, Mods::SHIFT, 1);
    view.pointer(
        MouseAction::Move(Some(Button::Left)),
        at(4),
        Side::Right,
        Mods::SHIFT,
        1,
    );
    left_release(&mut view, 4, Side::Right, Mods::SHIFT, 1);
    assert!(view.take_output().is_empty());
    assert!(view.copy().is_some());
}

#[test]
fn cmd_click_opens_a_plain_link() {
    let mut view = view();
    view.feed(b"see https://example.com/a?b=1, ok");
    left_press(&mut view, 10, Side::Left, Mods::SUPER, 1);
    assert_eq!(
        view.take_events(),
        vec![ViewEvent::OpenLink("https://example.com/a?b=1".to_string())]
    );
}

#[test]
fn cmd_click_opens_a_marked_link() {
    let mut view = view();
    view.feed(b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\");
    left_press(&mut view, 1, Side::Left, Mods::SUPER, 1);
    assert_eq!(
        view.take_events(),
        vec![ViewEvent::OpenLink("https://example.com".to_string())]
    );
}

#[test]
fn only_web_links_open() {
    let mut view = view();
    view.feed(b"\x1b]8;;file:///etc/passwd\x1b\\x\x1b]8;;\x1b\\");
    left_press(&mut view, 0, Side::Left, Mods::SUPER, 1);
    assert!(!has_open_link(&view.take_events()));
}

#[test]
fn url_edges() {
    assert_eq!(
        url_at("(https://example.com/x).", 5),
        Some("https://example.com/x".to_string())
    );
    assert_eq!(url_at("no link here", 3), None);
}

#[test]
fn wheel_scrolls_scrollback() {
    let mut view = view();
    feed_lines(&mut view, 0, 100);
    wheel(&mut view, 3);
    assert_eq!(view.screen().scroll_offset(), 3);
    assert!(view.take_output().is_empty());
}

#[test]
fn wheel_becomes_arrows_in_a_full_screen_program() {
    let mut view = view();
    view.feed(b"\x1b[?1049h\x1b[?1007h");
    wheel(&mut view, 2);
    assert_eq!(view.take_output(), b"\x1b[A\x1b[A");
}

#[test]
fn paste_is_bracketed_when_asked() {
    let mut view = view();
    view.feed(b"\x1b[?2004h");
    view.paste("a\nb");
    assert_eq!(view.take_output(), b"\x1b[200~a\rb\x1b[201~");
}

#[test]
fn paste_strips_the_end_marker() {
    let mut view = view();
    view.feed(b"\x1b[?2004h");
    view.paste("x\x1b[201~rm -rf /\n");
    assert_eq!(view.take_output(), b"\x1b[200~xrm -rf /\r\x1b[201~");
}

#[test]
fn plain_paste_turns_newlines_into_returns() {
    let mut view = view();
    view.paste("a\r\nb\n");
    assert_eq!(view.take_output(), b"a\rb\r");
}

#[test]
fn cmd_k_clears_scrollback() {
    let mut view = view();
    feed_lines(&mut view, 0, 100);
    view.key(&key(KeyCode::Char('k'), Mods::SUPER));
    assert_eq!(view.screen().history_len(), 0);
    assert_eq!(view.take_output(), b"\x0c");
}

#[test]
fn cmd_a_selects_all() {
    let mut view = view();
    view.feed(b"one\r\ntwo");
    view.key(&key(KeyCode::Char('a'), Mods::SUPER));
    let copied = view.copy().expect("a selection");
    assert!(copied.contains("one"));
    assert!(copied.contains("two"));
    assert!(view.take_output().is_empty());
}

#[test]
fn focus_is_reported_when_asked() {
    let mut view = view();
    view.feed(b"\x1b[?1004h");
    view.set_focused(false);
    view.set_focused(true);
    assert_eq!(view.take_output(), b"\x1b[O\x1b[I");
}

#[test]
fn cmd_scroll_keys_send_nothing() {
    let mut view = view();
    feed_lines(&mut view, 0, 100);
    view.key(&key(KeyCode::Home, Mods::SUPER));
    assert_eq!(view.screen().scroll_offset(), view.screen().history_len());
    assert!(view.take_output().is_empty());
    view.key(&key(KeyCode::End, Mods::SUPER));
    assert_eq!(view.screen().scroll_offset(), 0);
    assert!(view.take_output().is_empty());
}

#[test]
fn idle_needs_no_draw() {
    let mut view = view();
    view.feed(b"x");
    assert!(view.needs_draw());
    view.mark_drawn();
    view.take_output();
    view.tick(Instant::now());
    assert!(!view.needs_draw());
}

#[test]
fn open_sync_needs_no_draw() {
    let mut view = view();
    view.mark_drawn();
    view.feed(b"\x1b[?2026hHELLO");
    assert!(!view.needs_draw());
    let deadline = view
        .tick(Instant::now())
        .expect("an open sync has a deadline");
    view.tick(deadline + Duration::from_millis(1));
    assert!(view.needs_draw());
}

#[test]
fn host_replay_then_resize() {
    let mut view = view();
    for n in 0..30 {
        let digit = char::from(b'0' + (n % 10) as u8);
        let line: String = std::iter::repeat_n(digit, 100).collect();
        view.feed(format!("{line}\r\n").as_bytes());
    }
    let grid = GridSize {
        cols: 100,
        rows: 30,
    };
    view.set_grid(grid);
    assert_eq!(view.grid(), grid);
    assert!(view.needs_draw());
    assert_eq!(view.screen().row_text(0).chars().count(), 100);
}

#[test]
fn paste_returns_to_the_bottom() {
    let mut view = view();
    feed_lines(&mut view, 0, 100);
    wheel(&mut view, 10);
    view.paste("x");
    assert_eq!(view.screen().scroll_offset(), 0);
}

#[test]
fn cmd_page_keys_move_one_page() {
    let mut view = view();
    feed_lines(&mut view, 0, 100);
    view.key(&key(KeyCode::Up, Mods::SUPER));
    assert_eq!(view.screen().scroll_offset(), 23);
    assert!(view.take_output().is_empty());
    view.key(&key(KeyCode::Down, Mods::SUPER));
    assert_eq!(view.screen().scroll_offset(), 0);
    assert!(view.take_output().is_empty());
}

#[test]
fn a_forwarded_pointer_ends_a_drag() {
    let mut view = view();
    view.feed(b"hello brave world");
    view.feed(b"\x1b[?1000h\x1b[?1006h");
    left_press(&mut view, 0, Side::Left, Mods::SHIFT, 1);
    view.pointer(
        MouseAction::Move(Some(Button::Left)),
        at(4),
        Side::Right,
        Mods::empty(),
        1,
    );
    view.feed(b"\x1b[?1000l");
    let before = view.copy();
    view.pointer(
        MouseAction::Move(Some(Button::Left)),
        at(8),
        Side::Right,
        Mods::empty(),
        1,
    );
    assert_eq!(view.copy(), before);
}

#[test]
fn cmd_k_while_scrolled_up_returns_to_the_bottom() {
    let mut view = view();
    feed_lines(&mut view, 0, 100);
    wheel(&mut view, 10);
    view.key(&key(KeyCode::Char('k'), Mods::SUPER));
    assert_eq!(view.screen().scroll_offset(), 0);
    assert_eq!(view.screen().history_len(), 0);
}

#[test]
fn a_click_clears_the_old_selection_and_copies_nothing() {
    let mut view = view();
    view.feed(b"hello brave world");
    left_press(&mut view, 0, Side::Left, Mods::empty(), 1);
    view.pointer(
        MouseAction::Move(Some(Button::Left)),
        at(4),
        Side::Right,
        Mods::empty(),
        1,
    );
    left_release(&mut view, 4, Side::Right, Mods::empty(), 1);
    view.take_events();
    left_press(&mut view, 2, Side::Left, Mods::empty(), 1);
    left_release(&mut view, 2, Side::Left, Mods::empty(), 1);
    assert!(
        !view
            .take_events()
            .iter()
            .any(|event| matches!(event, ViewEvent::Copy(_)))
    );
    assert_eq!(view.copy(), None);
}

#[test]
fn a_nested_end_marker_cannot_survive() {
    let mut view = view();
    view.feed(b"\x1b[?2004h");
    view.paste("a\x1b[20\x1b[201~1~b");
    assert_eq!(view.take_output(), b"\x1b[200~ab\x1b[201~");
}
