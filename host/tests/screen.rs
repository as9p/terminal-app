use term_host::{Screen, Size};

const SIZE: Size = Size { cols: 80, rows: 24 };
const SCROLLBACK: usize = 1000;

#[derive(Debug, PartialEq)]
struct RowState {
    wrapped: bool,
    cells: Vec<String>,
}

/// Everything a viewer could notice, read cell by cell so that two equal states cannot
/// come from a snapshot that merely repeats itself.
#[derive(Debug, PartialEq)]
struct State {
    scrollback: Vec<RowState>,
    visible: Vec<RowState>,
    cursor: (u16, u16),
    hide_cursor: bool,
    alternate: bool,
    application_keypad: bool,
    application_cursor: bool,
    bracketed_paste: bool,
    mouse: String,
    pen: String,
}

fn row_state(screen: &vt100::Screen, row: u16, cols: u16) -> RowState {
    let cells = (0..cols)
        .map(|col| {
            let cell = screen.cell(row, col).expect("cell in range");
            format!(
                "{:?} {:?} {:?} b{} i{} u{} r{} w{} c{}",
                cell.contents(),
                cell.fgcolor(),
                cell.bgcolor(),
                cell.bold(),
                cell.italic(),
                cell.underline(),
                cell.inverse(),
                cell.is_wide(),
                cell.is_wide_continuation(),
            )
        })
        .collect();
    RowState {
        wrapped: screen.row_wrapped(row),
        cells,
    }
}

fn state(screen: &mut Screen) -> State {
    let vt = screen.vt();
    let (rows, cols) = vt.size();
    let (rows, cols) = (rows.get(), cols.get());

    let mut scrollback = Vec::new();
    for back in (1..=vt.scrollback_len()).rev() {
        vt.set_scrollback(back);
        scrollback.push(row_state(vt, 0, cols));
    }
    vt.set_scrollback(0);

    State {
        scrollback,
        visible: (0..rows).map(|row| row_state(vt, row, cols)).collect(),
        cursor: vt.cursor_position(),
        hide_cursor: vt.hide_cursor(),
        alternate: vt.alternate_screen(),
        application_keypad: vt.application_keypad(),
        application_cursor: vt.application_cursor(),
        bracketed_paste: vt.bracketed_paste(),
        mouse: format!(
            "{:?} {:?} {:?}",
            vt.mouse_protocol_mode(),
            vt.mouse_protocol_encoding(),
            vt.cursor_style(),
        ),
        pen: format!(
            "{:?} {:?} {} {} {} {}",
            vt.fgcolor(),
            vt.bgcolor(),
            vt.bold(),
            vt.italic(),
            vt.underline(),
            vt.inverse(),
        ),
    }
}

fn assert_same(left: &State, right: &State) {
    assert_eq!(left.scrollback.len(), right.scrollback.len());
    for (i, (l, r)) in left.scrollback.iter().zip(&right.scrollback).enumerate() {
        assert_eq!(l, r, "scrollback row {i}");
    }
    for (i, (l, r)) in left.visible.iter().zip(&right.visible).enumerate() {
        assert_eq!(l, r, "visible row {i}");
    }
    assert_eq!(left, right);
}

/// The awkward rows: a line that wraps, a wide character that would straddle the right
/// edge, a wrapped row whose next row starts blank, a blank row with a background, and a
/// wide character that exactly fills the edge.
fn feed_edges(screen: &mut Screen) {
    screen.feed("w".repeat(200).as_bytes());
    screen.feed(b"\r\n");
    screen.feed("y".repeat(79).as_bytes());
    screen.feed("語\r\n".as_bytes());
    screen.feed("z".repeat(80).as_bytes());
    screen.feed(b"a\x08\x1b[X\x1b[5Cq\r\n");
    screen.feed(b"\x1b[48;5;21m\x1b[K\x1b[m\r\n");
    screen.feed("v".repeat(78).as_bytes());
    screen.feed("語語\r\n".as_bytes());
}

fn feed_main(screen: &mut Screen) {
    for i in 0..500u32 {
        let line = format!(
            "\x1b[38;5;{}mline {i} \x1b[38;2;{};{};{}mtruecolor \x1b[1mbold\x1b[22;3;4;7m mix\x1b[m 日本語\r\n",
            i % 256,
            i % 256,
            255 - i % 256,
            i * 7 % 256,
        );
        screen.feed(line.as_bytes());
    }
    feed_edges(screen);
    for i in 0..30 {
        screen.feed(format!("scroll {i}\r\n").as_bytes());
    }
    feed_edges(screen);
    screen.feed(b"\x1b[5;10Hmoved\x1b[2;3H\x1b[31mred\x1b[m\x1b[10;40H");
}

fn replay(snapshot: &[u8], size: Size) -> Screen {
    let mut screen = Screen::new(size, SCROLLBACK);
    screen.feed(snapshot);
    screen
}

#[test]
fn snapshot_round_trip() {
    let mut original = Screen::new(SIZE, SCROLLBACK);
    feed_main(&mut original);
    original.feed(b"\x1b[?1002h\x1b[?1006h\x1b[?2004h\x1b[?25l\x1b[?1h\x1b=\x1b[4 q");
    original.feed(b"\x1b[1;38;5;9m");

    let before = state(&mut original);
    assert!(before.scrollback.len() >= 500, "the data has to scroll");
    assert!(before.scrollback.iter().any(|row| row.wrapped));
    assert!(before.visible.iter().any(|row| row.wrapped));
    assert!(
        before
            .scrollback
            .iter()
            .any(|row| row.cells.iter().any(|cell| cell.contains("Rgb")))
    );
    assert!(
        before
            .scrollback
            .iter()
            .any(|row| row.cells.iter().any(|cell| cell.contains("\"本\"")))
    );
    assert_eq!(before.cursor, (9, 39));
    assert!(before.bracketed_paste && before.hide_cursor);

    let snapshot = original.snapshot();
    assert!(snapshot.starts_with(b"\x1bc"));
    assert_same(&before, &state(&mut original));
    assert_eq!(snapshot, original.snapshot(), "snapshot changed the screen");

    let mut copy = replay(&snapshot, SIZE);
    assert_same(&before, &state(&mut copy));
    assert_eq!(snapshot, copy.snapshot());

    // RIS at the front has to clear whatever the receiver already shows.
    let mut dirty = Screen::new(SIZE, SCROLLBACK);
    for i in 0..40 {
        dirty.feed(format!("junk {i}\r\n").as_bytes());
    }
    dirty.feed(b"\x1b[?1049h\x1b[?1000h\x1b[41mjunk");
    dirty.feed(&snapshot);
    assert_same(&before, &state(&mut dirty));

    // Both must keep behaving alike, not just look alike.
    let more = b"\x1b[m\x1b[24;1Hafter\r\nmore\r\n";
    original.feed(more);
    copy.feed(more);
    assert_same(&state(&mut original), &state(&mut copy));
}

#[test]
fn snapshot_keeps_a_cursor_waiting_to_wrap() {
    let size = Size { cols: 20, rows: 5 };
    let mut original = Screen::new(size, SCROLLBACK);
    original.feed(b"\x1b[5;1H\x1b[32m");
    original.feed("e".repeat(20).as_bytes());

    let mut copy = replay(&original.snapshot(), size);
    assert_same(&state(&mut original), &state(&mut copy));

    original.feed(b"N");
    copy.feed(b"N");
    let after = state(&mut original);
    assert_eq!(after.scrollback.len(), 1);
    assert_same(&after, &state(&mut copy));
}

#[test]
fn alt_screen_round_trip() {
    let mut original = Screen::new(SIZE, SCROLLBACK);
    feed_main(&mut original);
    original.feed(b"\x1b[7;7H\x1b[?1049h");
    original.feed(b"\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[?25l\x1b[?1h\x1b=");
    original.feed(
        "\x1b[1;1H\x1b[1;38;2;1;2;3mfull screen\x1b[3;5H\x1b[48;5;200m語語\x1b[12;30H\x1b[35m"
            .as_bytes(),
    );

    let before = state(&mut original);
    assert!(before.alternate);
    assert!(before.bracketed_paste && before.hide_cursor);
    assert_eq!(before.cursor, (11, 29));

    let snapshot = original.snapshot();
    assert!(snapshot.starts_with(b"\x1bc"));
    assert_same(&before, &state(&mut original));
    assert_eq!(snapshot, original.snapshot(), "snapshot changed the screen");

    let mut copy = replay(&snapshot, SIZE);
    assert_same(&before, &state(&mut copy));

    // The hidden main screen can only be seen by leaving the full-screen program.
    original.feed(b"\x1b[?1049l");
    copy.feed(b"\x1b[?1049l");
    let main = state(&mut original);
    assert!(!main.alternate);
    assert_eq!(main.cursor, (6, 6));
    assert!(main.scrollback.len() >= 500);
    assert!(
        main.visible
            .iter()
            .any(|row| row.cells.iter().any(|cell| cell.contains("\"m\"")))
    );
    assert_same(&main, &state(&mut copy));
}

#[test]
fn snapshot_between_the_halves_of_a_sequence() {
    let mut original = Screen::new(SIZE, SCROLLBACK);
    original.feed(b"main\x1b[?1049halt\x1b[3");

    let before = state(&mut original);
    let snapshot = original.snapshot();
    assert_same(&before, &state(&mut original));

    // The bytes after the snapshot go to both, so the copy has to be waiting too.
    let mut copy = replay(&snapshot, SIZE);
    original.feed("1mred 語".as_bytes());
    copy.feed("1mred 語".as_bytes());
    let after = state(&mut original);
    assert_eq!(after.pen, state(&mut copy).pen);
    assert_same(&after, &state(&mut copy));
    assert!(after.pen.contains("Idx(1)"), "pen was {}", after.pen);

    original.feed(&"語".as_bytes()[..1]);
    let snapshot = original.snapshot();
    let mut copy = replay(&snapshot, SIZE);
    original.feed(&"語".as_bytes()[1..]);
    copy.feed(&"語".as_bytes()[1..]);
    assert_same(&state(&mut original), &state(&mut copy));
}
