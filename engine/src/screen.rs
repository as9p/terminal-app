//! Screen state behind our own types. This is the only file that names the screen library, so a
//! different library can be swapped in without touching the rest of the engine.

use std::cell::OnceCell;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side as LibSide};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::cell::{Cell as LibCell, Flags};
use alacritty_terminal::term::color::COUNT as COLOR_COUNT;
use alacritty_terminal::term::{Config, MIN_COLUMNS, MIN_SCREEN_LINES, Term, TermDamage, TermMode};
use alacritty_terminal::vte::ansi::{
    ClearMode, Color, CursorShape as LibCursorShape, CursorStyle, Handler, NamedColor, Processor,
    Rgb as LibRgb,
};
use bitflags::bitflags;

use crate::geometry::{CellPoint, GridSize, Side};
use crate::theme::{Palette, Rgb};
use crate::xtversion::{self, QueryScan};

pub const SCROLLBACK_LINES: usize = 20_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScreenEvent {
    Title(String),
    Bell,
    ClipboardStore(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modes {
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub mouse: MouseTracking,
    pub sgr_mouse: bool,
    pub alt_screen: bool,
    pub alternate_scroll: bool,
    pub focus_events: bool,
    pub kitty_flags: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseTracking {
    Off,
    Click,
    Drag,
    Motion,
}

/// fg and bg are final: inverse, dim, hidden and colours the program redefined are already applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub ch: char,
    pub zerowidth: Vec<char>,
    pub fg: Rgb,
    pub bg: Rgb,
    pub attrs: Attrs,
    pub width: CellWidth,
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Attrs: u8 {
        const BOLD = 1;
        const ITALIC = 1 << 1;
        const UNDERLINE = 1 << 2;
        const DOUBLE_UNDERLINE = 1 << 3;
        const UNDERCURL = 1 << 4;
        const DOTTED_UNDERLINE = 1 << 5;
        const DASHED_UNDERLINE = 1 << 6;
        const STRIKEOUT = 1 << 7;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellWidth {
    Narrow,
    Wide,
    Spacer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub at: CellPoint,
    pub shape: CursorShape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Bar,
    Block,
    Underline,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Damage {
    None,
    Rows(Vec<u16>),
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectKind {
    Char,
    Word,
    Line,
}

const ATTRS: [(Flags, Attrs); 8] = [
    (Flags::BOLD, Attrs::BOLD),
    (Flags::ITALIC, Attrs::ITALIC),
    (Flags::UNDERLINE, Attrs::UNDERLINE),
    (Flags::DOUBLE_UNDERLINE, Attrs::DOUBLE_UNDERLINE),
    (Flags::UNDERCURL, Attrs::UNDERCURL),
    (Flags::DOTTED_UNDERLINE, Attrs::DOTTED_UNDERLINE),
    (Flags::DASHED_UNDERLINE, Attrs::DASHED_UNDERLINE),
    (Flags::STRIKEOUT, Attrs::STRIKEOUT),
];

const KITTY_FLAGS: [(TermMode, u8); 5] = [
    (TermMode::DISAMBIGUATE_ESC_CODES, 1),
    (TermMode::REPORT_EVENT_TYPES, 2),
    (TermMode::REPORT_ALTERNATE_KEYS, 4),
    (TermMode::REPORT_ALL_KEYS_AS_ESC, 8),
    (TermMode::REPORT_ASSOCIATED_TEXT, 16),
];

type ColorFormat = Arc<dyn Fn(LibRgb) -> String + Sync + Send>;
type SizeFormat = Arc<dyn Fn(WindowSize) -> String + Sync + Send>;

/// What the library sent while it read bytes, kept in order. Colour questions are answered after
/// the read, because only `Screen` knows the palette.
enum Message {
    Reply(String),
    Color(usize, ColorFormat),
    Size(SizeFormat),
    Event(ScreenEvent),
}

/// The library hands events to a listener it owns, through `&self`, so the inbox is shared. A
/// mutex rather than a `RefCell` keeps `Screen` free to move to another thread.
#[derive(Clone, Default)]
struct Listener(Arc<Mutex<Vec<Message>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let message = match event {
            Event::PtyWrite(text) => Message::Reply(text),
            Event::ColorRequest(index, format) => Message::Color(index, format),
            Event::TextAreaSizeRequest(format) => Message::Size(format),
            Event::Title(title) => Message::Event(ScreenEvent::Title(title)),
            Event::ResetTitle => Message::Event(ScreenEvent::Title(String::new())),
            Event::Bell => Message::Event(ScreenEvent::Bell),
            Event::ClipboardStore(_, text) => Message::Event(ScreenEvent::ClipboardStore(text)),
            // Clipboard reads stay off (the library's default), and the rest is about a window
            // or a child process the library does not own here.
            _ => return,
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message);
    }
}

struct Dims(GridSize);

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.0.rows)
    }

    fn columns(&self) -> usize {
        usize::from(self.0.cols)
    }
}

/// The library exports its minimum but does not enforce it, and a wide character needs two
/// columns, so smaller sizes are raised to it here.
fn usable(size: GridSize) -> GridSize {
    GridSize {
        cols: size.cols.max(MIN_COLUMNS as u16),
        rows: size.rows.max(MIN_SCREEN_LINES as u16),
    }
}

pub struct Screen {
    term: Term<Listener>,
    parser: Processor,
    inbox: Listener,
    /// Every colour a cell can name, from the theme. The program's own redefinitions live in
    /// the library and win over these.
    colors: [Rgb; COLOR_COUNT],
    replies: Vec<u8>,
    events: Vec<ScreenEvent>,
    /// Finds the XTVERSION queries the library ignores, and the reply naming the terminal.
    version_scan: QueryScan,
    version_reply: String,
    /// Set by output. The library always reports the cursor's row as damaged, even when nothing
    /// happened, so it is only asked when this is set.
    touched: bool,
    /// Set by selection, scrolling and resize, which the library does not track as damage.
    full_damage: bool,
    /// The selection's extent in grid lines. Working it out walks the grid for word and line
    /// selections, and drawing asks `is_selected` once per cell, so it is worked out on the first
    /// ask after a change and kept until the next. Filled lazily rather than at each change so a
    /// burst of output between two frames pays for it once.
    selection_range: OnceCell<Option<SelectionRange>>,
}

impl Screen {
    pub fn new(size: GridSize, palette: &Palette) -> Screen {
        let config = Config {
            scrolling_history: SCROLLBACK_LINES,
            kitty_keyboard: true,
            default_cursor_style: CursorStyle {
                shape: LibCursorShape::Beam,
                blinking: false,
            },
            ..Config::default()
        };
        let inbox = Listener::default();
        let term = Term::new(config, &Dims(usable(size)), inbox.clone());
        Screen {
            term,
            parser: Processor::new(),
            inbox,
            colors: color_table(palette),
            replies: Vec::new(),
            events: Vec::new(),
            version_scan: QueryScan::default(),
            version_reply: xtversion::default_reply(),
            touched: true,
            full_damage: true,
            selection_range: OnceCell::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        // The parser reads up to the end of each query before the reply goes in the inbox, so the
        // reply keeps its place among the library's own. Inside a synchronized update the reply
        // is sent at once all the same, while the library holds back the update's bytes, and so
        // its replies to earlier queries in it, until the update ends. That is fine: the asking
        // program is waiting on this reply, and the reply draws nothing.
        let mut rest = bytes;
        while let Some(end) = self.version_scan.find(rest) {
            self.parser.advance(&mut self.term, &rest[..end]);
            self.inbox
                .send_event(Event::PtyWrite(self.version_reply.clone()));
            rest = &rest[end..];
        }
        self.parser.advance(&mut self.term, rest);
        self.touched = true;
        self.forget_selection_range();
        self.collect();
    }

    /// Who the terminal says it is when a program asks (XTVERSION), such as `paw 1.0`.
    /// Anything outside printable ASCII is dropped, and an empty name falls back to the engine's.
    pub fn set_identity(&mut self, identity: &str) {
        self.version_reply = xtversion::reply(identity);
    }

    pub fn take_replies(&mut self) -> Vec<u8> {
        mem::take(&mut self.replies)
    }

    pub fn take_events(&mut self) -> Vec<ScreenEvent> {
        mem::take(&mut self.events)
    }

    pub fn size(&self) -> GridSize {
        let grid = self.term.grid();
        GridSize {
            cols: grid.columns() as u16,
            rows: grid.screen_lines() as u16,
        }
    }

    pub fn resize(&mut self, size: GridSize) {
        let size = usable(size);
        if size == self.size() {
            return;
        }
        self.term.resize(Dims(size));
        self.full_damage = true;
        self.forget_selection_range();
    }

    pub fn modes(&self) -> Modes {
        let mode = *self.term.mode();
        let mouse = if mode.contains(TermMode::MOUSE_MOTION) {
            MouseTracking::Motion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseTracking::Drag
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseTracking::Click
        } else {
            MouseTracking::Off
        };
        let kitty_flags = KITTY_FLAGS
            .iter()
            .filter(|(flag, _)| mode.contains(*flag))
            .fold(0, |bits, (_, bit)| bits | bit);
        Modes {
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            app_keypad: mode.contains(TermMode::APP_KEYPAD),
            mouse,
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            focus_events: mode.contains(TermMode::FOCUS_IN_OUT),
            kitty_flags,
        }
    }

    /// The parser holds back everything after `\x1b[?2026h` until the program ends the update,
    /// and nothing ends an update the program forgets unless the owner calls `end_sync_if_due`.
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    pub fn end_sync_if_due(&mut self, now: Instant) -> bool {
        match self.sync_deadline() {
            Some(deadline) if now >= deadline => {
                self.parser.stop_sync(&mut self.term);
                self.touched = true;
                self.forget_selection_range();
                self.collect();
                true
            }
            _ => false,
        }
    }

    pub fn history_len(&self) -> usize {
        self.term.grid().history_size()
    }

    pub fn clear_history(&mut self) {
        self.term.clear_screen(ClearMode::Saved);
        self.full_damage = true;
        self.forget_selection_range();
    }

    pub fn scroll_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn scroll_by(&mut self, lines: i32) {
        self.scroll(Scroll::Delta(lines));
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll(Scroll::Top);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll(Scroll::Bottom);
    }

    pub fn row_text(&self, row: u16) -> String {
        let Some(line) = self.line_of(row) else {
            return String::new();
        };
        let grid = self.term.grid();
        let mut text = String::new();
        for column in 0..grid.columns() {
            push_cell_text(&grid[line][Column(column)], &mut text);
        }
        text.truncate(text.trim_end_matches(' ').len());
        text
    }

    pub fn cells(&self, row: u16, out: &mut Vec<Cell>) {
        out.clear();
        let Some(line) = self.line_of(row) else {
            return;
        };
        let grid = self.term.grid();
        out.extend((0..grid.columns()).map(|column| self.cell(&grid[line][Column(column)])));
    }

    pub fn text_around(&self, at: CellPoint) -> (String, usize) {
        let grid = self.term.grid();
        let target = self.point(at);
        let wraps = |line: Line| {
            grid[line][grid.last_column()]
                .flags
                .contains(Flags::WRAPLINE)
        };

        let mut first = target.line;
        while first > grid.topmost_line() && wraps(Line(first.0 - 1)) {
            first = Line(first.0 - 1);
        }
        let mut last = target.line;
        while last < grid.bottommost_line() && wraps(last) {
            last = Line(last.0 + 1);
        }

        let mut text = String::new();
        let mut chars: usize = 0;
        let mut index = 0;
        for line in first.0..=last.0 {
            for column in 0..grid.columns() {
                let cell = &grid[Line(line)][Column(column)];
                if Line(line) == target.line && Column(column) == target.column {
                    // The right half of a wide character belongs to the character on its left.
                    index = if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                        chars.saturating_sub(1)
                    } else {
                        chars
                    };
                }
                let before = text.len();
                push_cell_text(cell, &mut text);
                chars += text[before..].chars().count();
            }
        }
        text.truncate(text.trim_end_matches(' ').len());
        (text, index)
    }

    pub fn cursor(&self) -> Option<Cursor> {
        if !self.term.mode().contains(TermMode::SHOW_CURSOR) {
            return None;
        }
        let shape = match self.term.cursor_style().shape {
            LibCursorShape::Beam => CursorShape::Bar,
            LibCursorShape::Block | LibCursorShape::HollowBlock => CursorShape::Block,
            LibCursorShape::Underline => CursorShape::Underline,
            LibCursorShape::Hidden => return None,
        };
        let grid = self.term.grid();
        let point = grid.cursor.point;
        let row = point.line.0 + grid.display_offset() as i32;
        if row >= grid.screen_lines() as i32 {
            return None;
        }
        Some(Cursor {
            at: CellPoint {
                col: point.column.0 as u16,
                row: row as u16,
            },
            shape,
        })
    }

    pub fn take_damage(&mut self) -> Damage {
        if !self.touched && !self.full_damage {
            return Damage::None;
        }
        let full = mem::take(&mut self.full_damage);
        self.touched = false;
        let damage = match self.term.damage() {
            TermDamage::Full => Damage::Full,
            _ if full => Damage::Full,
            // The library counts these lines from the top of the live screen and adds the scroll
            // offset itself, leaving out lines that fall below the view, so they are view rows
            // already. Output that scrolls the screen, and any change of the scroll offset,
            // arrive as `Full`.
            TermDamage::Partial(lines) => {
                let rows: Vec<u16> = lines.map(|bounds| bounds.line as u16).collect();
                if rows.is_empty() {
                    Damage::None
                } else {
                    Damage::Rows(rows)
                }
            }
        };
        self.term.reset_damage();
        damage
    }

    pub fn select(&mut self, kind: SelectKind, at: CellPoint, side: Side) {
        let ty = match kind {
            SelectKind::Char => SelectionType::Simple,
            SelectKind::Word => SelectionType::Semantic,
            SelectKind::Line => SelectionType::Lines,
        };
        self.term.selection = Some(Selection::new(ty, self.point(at), lib_side(side)));
        self.full_damage = true;
        self.forget_selection_range();
    }

    pub fn select_to(&mut self, at: CellPoint, side: Side) {
        let point = self.point(at);
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, lib_side(side));
            self.full_damage = true;
            self.forget_selection_range();
        }
    }

    pub fn select_all(&mut self) {
        let grid = self.term.grid();
        let start = Point::new(grid.topmost_line(), Column(0));
        let end = Point::new(grid.bottommost_line(), grid.last_column());
        let mut selection = Selection::new(SelectionType::Simple, start, LibSide::Left);
        selection.update(end, LibSide::Right);
        self.term.selection = Some(selection);
        self.full_damage = true;
        self.forget_selection_range();
    }

    pub fn clear_selection(&mut self) {
        if self.term.selection.take().is_some() {
            self.full_damage = true;
            self.forget_selection_range();
        }
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    pub fn is_selected(&self, at: CellPoint) -> bool {
        let point = self.point(at);
        self.selection_range
            .get_or_init(|| {
                self.term
                    .selection
                    .as_ref()
                    .and_then(|s| s.to_range(&self.term))
            })
            .as_ref()
            .is_some_and(|range| range.contains(point))
    }

    pub fn link_at(&self, at: CellPoint) -> Option<String> {
        let point = self.point(at);
        self.term.grid()[point]
            .hyperlink()
            .map(|link| link.uri().to_owned())
    }

    fn scroll(&mut self, scroll: Scroll) {
        let before = self.scroll_offset();
        self.term.scroll_display(scroll);
        if self.scroll_offset() != before {
            self.full_damage = true;
        }
        self.forget_selection_range();
    }

    /// Called wherever the selection, the grid's contents, the scroll position or the size can
    /// change. The library moves or drops a selection when output scrolls or erases under it.
    fn forget_selection_range(&mut self) {
        self.selection_range = OnceCell::new();
    }

    /// Moves what the library sent during the last read into the reply and event buffers.
    fn collect(&mut self) {
        let messages = mem::take(&mut *self.inbox.0.lock().unwrap_or_else(PoisonError::into_inner));
        for message in messages {
            match message {
                Message::Reply(text) => self.replies.extend_from_slice(text.as_bytes()),
                Message::Color(index, format) => {
                    // An index the table does not hold has no answer a program could use.
                    if index < COLOR_COUNT {
                        let Rgb(r, g, b) = self.indexed(index);
                        self.replies
                            .extend_from_slice(format(LibRgb { r, g, b }).as_bytes());
                    }
                }
                Message::Size(format) => {
                    // The screen does not know its cell size in pixels, so it says zero rather
                    // than leave a program waiting for an answer.
                    let GridSize { cols, rows } = self.size();
                    let size = WindowSize {
                        num_lines: rows,
                        num_cols: cols,
                        cell_width: 0,
                        cell_height: 0,
                    };
                    self.replies.extend_from_slice(format(size).as_bytes());
                }
                Message::Event(event) => self.events.push(event),
            }
        }
    }

    /// The grid line shown at viewport `row`, or `None` below the last row.
    fn line_of(&self, row: u16) -> Option<Line> {
        let grid = self.term.grid();
        (usize::from(row) < grid.screen_lines())
            .then(|| Line(i32::from(row) - grid.display_offset() as i32))
    }

    /// The grid point shown at `at`, clamped into the viewport.
    fn point(&self, at: CellPoint) -> Point {
        let grid = self.term.grid();
        let row = usize::from(at.row).min(grid.screen_lines() - 1);
        let column = usize::from(at.col).min(grid.columns() - 1);
        Point::new(
            Line(row as i32 - grid.display_offset() as i32),
            Column(column),
        )
    }

    fn cell(&self, cell: &LibCell) -> Cell {
        let flags = cell.flags;
        let mut fg = self.resolve(cell.fg);
        let mut bg = self.resolve(cell.bg);
        if flags.contains(Flags::DIM) {
            fg = dim(fg);
        }
        if flags.contains(Flags::INVERSE) {
            mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        let width = if flags.contains(Flags::WIDE_CHAR) {
            CellWidth::Wide
        } else if flags.contains(Flags::WIDE_CHAR_SPACER) {
            CellWidth::Spacer
        } else {
            CellWidth::Narrow
        };
        let attrs = ATTRS
            .iter()
            .filter(|(flag, _)| flags.contains(*flag))
            .fold(Attrs::empty(), |attrs, (_, attr)| attrs | *attr);
        Cell {
            ch: visible_char(cell.c),
            zerowidth: cell.zerowidth().map(<[char]>::to_vec).unwrap_or_default(),
            fg,
            bg,
            attrs,
            width,
        }
    }

    fn resolve(&self, color: Color) -> Rgb {
        match color {
            Color::Spec(LibRgb { r, g, b }) => Rgb(r, g, b),
            Color::Named(name) => self.indexed(name as usize),
            Color::Indexed(index) => self.indexed(usize::from(index)),
        }
    }

    /// A colour the program redefined (OSC 4, 10, 11 or 12) wins over the theme.
    fn indexed(&self, index: usize) -> Rgb {
        match self.term.colors()[index] {
            Some(LibRgb { r, g, b }) => Rgb(r, g, b),
            None => self.colors[index],
        }
    }
}

fn lib_side(side: Side) -> LibSide {
    match side {
        Side::Left => LibSide::Left,
        Side::Right => LibSide::Right,
    }
}

/// The library keeps a tab it printed as a `\t` in the cell so a copy can return it; on screen
/// it is a blank.
fn visible_char(c: char) -> char {
    if c == '\t' { ' ' } else { c }
}

/// The text one cell adds to a row. Spacers stand in for the right half of a wide character, or
/// for a wide character pushed to the next row, so they add nothing.
fn push_cell_text(cell: &LibCell, text: &mut String) {
    if cell
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        return;
    }
    text.push(visible_char(cell.c));
    if let Some(zerowidth) = cell.zerowidth() {
        text.extend(zerowidth);
    }
}

/// Dim text at two thirds of its colour's brightness.
fn dim(Rgb(r, g, b): Rgb) -> Rgb {
    let scale = |v: u8| (u16::from(v) * 2 / 3) as u8;
    Rgb(scale(r), scale(g), scale(b))
}

/// Every colour index the library can name: the theme's 16, the xterm 6 by 6 by 6 cube and grey
/// ramp, then the named foreground, background, cursor and dim colours.
fn color_table(palette: &Palette) -> [Rgb; COLOR_COUNT] {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let mut table = [palette.fg; COLOR_COUNT];
    table[..16].copy_from_slice(&palette.ansi);
    for (i, slot) in table[16..232].iter_mut().enumerate() {
        *slot = Rgb(LEVELS[i / 36], LEVELS[i / 6 % 6], LEVELS[i % 6]);
    }
    for (i, slot) in table[232..256].iter_mut().enumerate() {
        let v = 8 + 10 * i as u8;
        *slot = Rgb(v, v, v);
    }
    table[NamedColor::Foreground as usize] = palette.fg;
    table[NamedColor::Background as usize] = palette.bg;
    table[NamedColor::Cursor as usize] = palette.cursor;
    for i in 0..8 {
        table[NamedColor::DimBlack as usize + i] = dim(palette.ansi[i]);
    }
    table[NamedColor::BrightForeground as usize] = palette.fg;
    table[NamedColor::DimForeground as usize] = dim(palette.fg);
    table
}
