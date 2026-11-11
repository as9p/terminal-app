# Native engine stage A implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust terminal engine library and a one-window plain terminal built on it, on Mac, that pass the design's day-one list and its Mac speed targets.

**Architecture:** Two new crates in the existing workspace. `engine/` (crate `term-engine`) holds screen state behind our own `Screen` type (the only file that names `alacritty_terminal`), key and mouse encoding, a `TerminalView` that owns input policy, and GPU drawing of a cell grid with `wgpu` from one shared glyph texture. `plain/` (crate `term-plain`, which builds the program `snowflake`) is a winit window with one shell on a pty, plus a windowless mode used by tests and recordings. Nothing in `host/` or `client/` changes.

**Tech Stack:** Rust (workspace resolver 3, rustc 1.99). `alacritty_terminal` 0.26, `wgpu` 23, `winit` 0.30, `cosmic-text` 0.12 (font loading, fallback, shaping, rasterizing through its swash cache), `etagere` 0.2, `portable-pty` 0.9, `arboard` 3, `bitflags` 2, `bytemuck` 1, `pollster` 0.4; `png` as a dev dependency. The wgpu, winit, cosmic-text and portable-pty versions are the ones the measured prototype locked.

**Spec:** `docs/superpowers/specs/2026-11-11-native-renderer-design.md`. It adds to `DESIGN.md` at the repo root. Executors read both.

## Global Constraints

- The name. The plain terminal is Snowflake (Alex, 2026-11-11): the program file is `snowflake`, the window title is `Snowflake`, and `TERM_PROGRAM` is `snowflake`. The crate names (`term-engine`, `term-plain`), the folders and the `TERM_PLAIN_*` test switches keep their names. `plain/Cargo.toml` sets the program name with `[[bin]] name = "snowflake"`, `path = "src/main.rs"`.
- One engine. `term-plain` contains no terminal logic: screen state, input encoding, selection, scrolling rules and drawing all come from `term-engine`.
- The engine knows nothing about shells, hosts, sockets, sidebars, tabs or windows. `engine/Cargo.toml` must not depend on `winit`, `portable-pty`, `wry`, `tokio`, `arboard` or any socket crate.
- Only `engine/src/screen.rs` may name `alacritty_terminal` or `vte`. Every other file uses the types in this plan, so a different screen library can be swapped in.
- Rows and columns are set by the owner of a view. The engine only answers "how many fit in this rectangle".
- A grid that does not match its rectangle is anchored top left, the rest is filled with the background colour, and nothing is drawn outside the rectangle.
- Draw only when something changed, never on a timer. The cursor does not blink.
- Reading shell output happens off the drawing thread. Writes to the shell never block the drawing thread.
- Scrollback is 20,000 lines per terminal.
- Theme, copied from `client/src/theme.ts`: foreground `#CDD6F4`, background `#16141F`, cursor `#F5E0DC`, cursor text `#1E1E2E`, selection background `#F5E0DC`, selection text `#1E1E2E`; colours 0 to 15 in order `#45475A #F38BA8 #A6E3A1 #F9E2AF #89B4FA #F5C2E7 #94E2D5 #BAC2DE #585B70 #F38BA8 #A6E3A1 #F9E2AF #89B4FA #F5C2E7 #94E2D5 #A6ADC8`. Bold text does not switch to the bright colours.
- Font: JetBrainsMono Nerd Font Mono, 14 px, line height 1.1, built into the engine. Text zoom moves in 1 px steps between 8 and 40 px and resets to 14.
- A resize refits the grid once, 100 ms after the size stops changing. Until then the old grid stays where it is.
- Licence: no code copied from kitty, cmux, Zed or any GPL or AGPL project, and none from Alacritty's application crate (key handling is written from the public protocol documents). New dependencies must be MIT, Apache-2.0, BSD, Zlib or ISC. A patched copy of a library keeps its licence notice.
- `host/` and `client/` are not edited. Never run the `term-host` binary, never touch port 47211 or `/Users/x/Library/Application Support/terminal-app`.
- Automated tests never open a window. Windows are opened only by the scripts in `tools/` (through the speed runner, in a sitting) and by the one four-second check in Task 8 Step 4, which the controller announces to Alex first.
- From Task 1 on, CI runs the engine's windowless tests on Windows as well as Mac, and from Task 7 on it checks that `term-plain` compiles on Windows. Nothing is tuned for Windows in this stage.
- Writing rules for code comments, docs and commit messages: no em-dashes, no emoji characters (write them as `\u{...}` escapes in tests), sentence case, plain words.
- Git: run git as plain commands from the repo root, stage files by name, never `git add -A`, never `git stash`. Each commit message ends with the `Co-Authored-By` line for the model that wrote it.

## Review Focus

Five conditions the spec implies but does not spell out, most likely first. Each is pinned by a named test in the task that owns the code.

1. Output arrives while the user has scrolled up: the view stays on the same lines, and the next key press returns to the bottom. Task 3, `output_while_scrolled_up_keeps_place`.
2. The window moves between a Retina and a non-Retina screen: glyphs are redrawn at the new scale and the grid is refitted, with no stretched text. Task 5, `scale_change_redraws_glyphs`; Task 8, `scale_change_refits`.
3. A rectangle of zero or a few pixels (minimised window, a sliver during resize): no panic, the grid never goes below 1 by 1, and no surface is configured at size zero. Task 1, `fit_never_returns_zero`; Task 5, `zero_size_rect_draws_nothing`; Task 8, `hidden_or_empty_window_never_presents`.
4. An escape sequence or a multi-byte character split across two reads gives the same screen as when it arrives whole. Task 1, `split_reads_give_the_same_screen`.
5. A paste that is very large or that contains the paste-end marker: the marker is removed, and 5 MB reaches the shell whole without freezing the window. Task 3, `paste_strips_the_end_marker`; Task 7, `five_megabyte_write_arrives_whole`.

## File structure

```
engine/                       crate term-engine (library)
  Cargo.toml
  fonts/                      four JetBrainsMono Nerd Font Mono faces and their licence
  src/lib.rs                  public re-exports only
  src/geometry.rs             GridSize, Rect, CellSize, CellPoint, fit, cell_at
  src/theme.rs                Rgb, Palette, Theme, zoom limits
  src/screen.rs               Screen: the wrapper around the screen library
  src/keys.rs                 key press to bytes
  src/mouse.rs                mouse action to bytes
  src/links.rs                plain URL detection
  src/view.rs                 TerminalView: screen plus input policy
  src/recording.rs            the recording file format
  src/font.rs                 font loading, cell size, glyph rasterizing
  src/boxdraw.rs              box and block characters drawn from the cell size
  src/atlas.rs                packing glyphs into one texture
  src/gpu.rs                  Gpu: shared device, pipelines, glyph texture
  src/draw.rs                 ViewDraw: per-view buffers; render_to_image
  src/shader.wgsl
  tests/gate.rs  tests/memory.rs  tests/keys.rs  tests/view.rs
  tests/glyphs.rs  tests/draw.rs  tests/images.rs  tests/images/*.png  tests/replay.rs
plain/                        crate term-plain (binary snowflake)
  Cargo.toml
  src/main.rs                 arguments, then windowless mode or the window
  src/shell.rs                pty, reader thread, writer thread
  src/session.rs              one Shell tied to one TerminalView
  src/headless.rs             the windowless mode
  src/input.rs                winit events to engine types, settle timer, click counter
  src/window.rs               the winit window and frame loop
  tests/headless.rs
  README.md
tools/keylat/bin-window.sh  tools/keylat/test-bin.sh
tools/speed/throughput-bin.sh  tools/speed/plain-compare.sh  tools/speed/plain-targets.sh
tools/speed/idle-frames.sh  tools/speed/configs/  tools/plain/record-claude.sh
.github/workflows/engine.yml
```

## Shared types

Every task uses these exactly as written. Task 1 creates them.

```rust
// engine/src/geometry.rs  (all sizes in physical pixels)
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct GridSize { pub cols: u16, pub rows: u16 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct Rect { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct CellSize { pub w: u32, pub h: u32 }
/// row 0 is the top visible row, whatever the scroll position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct CellPoint { pub col: u16, pub row: u16 }
/// Which half of a cell the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Side { Left, Right }
pub fn fit(rect: Rect, cell: CellSize) -> GridSize;            // floor, never below 1 by 1
pub fn cell_at(rect: Rect, cell: CellSize, grid: GridSize, x: f64, y: f64) -> (CellPoint, Side);  // clamped into the grid

// engine/src/theme.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct Rgb(pub u8, pub u8, pub u8);
#[derive(Clone, Debug)] pub struct Palette { pub fg: Rgb, pub bg: Rgb, pub cursor: Rgb, pub cursor_text: Rgb,
    pub selection_bg: Rgb, pub selection_fg: Rgb, pub ansi: [Rgb; 16] }
#[derive(Clone, Debug)] pub struct Theme { pub palette: Palette, pub font_px: f32, pub line_height: f32 }
impl Theme { pub fn builtin() -> Theme }                       // the values in Global Constraints
pub const FONT_PX_DEFAULT: f32 = 14.0; pub const FONT_PX_MIN: f32 = 8.0; pub const FONT_PX_MAX: f32 = 40.0;
```

---

### Task 1: the screen library gate (go or no-go)

Dispatch: opus, effort xhigh. This task decides whether `alacritty_terminal` stays. No drawing code exists yet, on purpose.

**Files:**
- Create: `engine/Cargo.toml`, `engine/src/lib.rs`, `engine/src/geometry.rs`, `engine/src/theme.rs`, `engine/src/screen.rs`
- Create: `engine/tests/gate.rs`, `engine/tests/memory.rs`
- Create: `.github/workflows/engine.yml`
- Modify: `Cargo.toml` (root): `members = ["host", "client/src-tauri", "engine"]`

**Interfaces:**
- Consumes: nothing.
- Produces: the shared types above, and:

```rust
// engine/src/screen.rs
pub const SCROLLBACK_LINES: usize = 20_000;
pub struct Screen { /* private */ }
impl Screen {
    pub fn new(size: GridSize, palette: &Palette) -> Screen;
    pub fn feed(&mut self, bytes: &[u8]);                       // output from the program
    pub fn take_replies(&mut self) -> Vec<u8>;                  // answers the program asked for, to send back
    pub fn take_events(&mut self) -> Vec<ScreenEvent>;
    pub fn size(&self) -> GridSize;
    pub fn resize(&mut self, size: GridSize);                   // rewraps lines
    pub fn modes(&self) -> Modes;
    pub fn sync_deadline(&self) -> Option<std::time::Instant>;  // Some while a synchronized update is open
    pub fn end_sync_if_due(&mut self, now: std::time::Instant) -> bool;
    pub fn history_len(&self) -> usize;
    pub fn clear_history(&mut self);
    pub fn scroll_offset(&self) -> usize;                       // 0 is the bottom
    pub fn scroll_by(&mut self, lines: i32);                    // positive is towards older lines
    pub fn scroll_to_top(&mut self);
    pub fn scroll_to_bottom(&mut self);
    pub fn row_text(&self, row: u16) -> String;                 // trailing blanks trimmed
    pub fn cells(&self, row: u16, out: &mut Vec<Cell>);         // clears out, then one Cell per column
    pub fn text_around(&self, at: CellPoint) -> (String, usize);// the whole wrapped line, and the char index of `at` in it
    pub fn cursor(&self) -> Option<Cursor>;                     // None when hidden or scrolled out of view
    pub fn take_damage(&mut self) -> Damage;                    // rows changed since the last call, by output, selection, scrolling or resize (Full is fine for the last three)
    pub fn select(&mut self, kind: SelectKind, at: CellPoint, side: Side);
    pub fn select_to(&mut self, at: CellPoint, side: Side);
    pub fn select_all(&mut self);
    pub fn clear_selection(&mut self);
    pub fn selection_text(&self) -> Option<String>;
    pub fn is_selected(&self, at: CellPoint) -> bool;
    pub fn link_at(&self, at: CellPoint) -> Option<String>;     // links the program marked (OSC 8)
}
pub enum ScreenEvent { Title(String), Bell, ClipboardStore(String) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modes { pub bracketed_paste: bool, pub app_cursor: bool, pub app_keypad: bool, pub mouse: MouseTracking,
    pub sgr_mouse: bool, pub alt_screen: bool, pub alternate_scroll: bool, pub focus_events: bool, pub kitty_flags: u8 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum MouseTracking { Off, Click, Drag, Motion }
/// fg and bg are final: inverse, dim, hidden and colours the program redefined are already applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell { pub ch: char, pub zerowidth: Vec<char>, pub fg: Rgb, pub bg: Rgb, pub attrs: Attrs, pub width: CellWidth }
bitflags! { pub struct Attrs: u8 { const BOLD; const ITALIC; const UNDERLINE; const DOUBLE_UNDERLINE; const UNDERCURL;
    const DOTTED_UNDERLINE; const DASHED_UNDERLINE; const STRIKEOUT; } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum CellWidth { Narrow, Wide, Spacer }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct Cursor { pub at: CellPoint, pub shape: CursorShape }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum CursorShape { Bar, Block, Underline }
#[derive(Clone, Debug, PartialEq, Eq)] pub enum Damage { None, Rows(Vec<u16>), Full }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum SelectKind { Char, Word, Line }
```

Facts about `alacritty_terminal` 0.26 that a reader collected from its source on the main branch (check them against the 0.26 docs before relying on one): `Term::new(config, &dimensions, listener)`; `Config { scrolling_history, kitty_keyboard, default_cursor_style, .. }` with `kitty_keyboard` off by default; bytes go in through `vte::ansi::Processor::advance(&mut term, bytes)` (the crate re-exports `vte`); answers to the program arrive as `Event::PtyWrite`, `ColorRequest`, `ClipboardLoad` and `TextAreaSizeRequest` on the `EventListener`; `term.damage()` and `term.reset_damage()`; `term.resize(size)` rewraps; `Selection::new`, `selection_to_string`; `Cell::hyperlink()`; mode flags in `TermMode`. Synchronized output (mode 2026) is buffered inside the vte `Processor`, which exposes `sync_timeout()` and `stop_sync(&mut term)`; nobody ends an overdue update unless the embedder does. `term.damage()` is believed to report the cursor's row every time it is asked, even when nothing happened, so `Screen` keeps its own "touched since the last `take_damage`" flag and answers `Damage::None` without asking the library when the flag is clear.

Settings to use: `scrolling_history = SCROLLBACK_LINES`, `kitty_keyboard = true`, default cursor style a bar that does not blink.

- [ ] **Step 1: Scaffold the crate and the workflow**

`engine/Cargo.toml` with `name = "term-engine"`, `edition = "2024"`, `license = "MIT"`. `.github/workflows/engine.yml` copies the shape of `.github/workflows/host.yml` (same matrix of `windows-latest` and `macos-latest`, same actions), triggers on `engine/**`, `plain/**`, `Cargo.toml`, `Cargo.lock` and itself, and runs `cargo test -p term-engine`.

- [ ] **Step 2: Write the gate tests in `engine/tests/gate.rs`**

```rust
fn screen(cols: u16, rows: u16) -> Screen { Screen::new(GridSize { cols, rows }, &Theme::builtin().palette) }
fn at(col: u16, row: u16) -> CellPoint { CellPoint { col, row } }

#[test]
fn scrollback_keeps_20000_lines() {
    let mut s = screen(80, 24);
    for n in 1..=25_000 { s.feed(format!("line {n}\r\n").as_bytes()); }
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
    s.scroll_to_top();   // the library may push the first row into the scrollback; either way the text starts at the top
    assert_eq!([s.row_text(0).len(), s.row_text(1).len(), s.row_text(2).len()], [40, 40, 20]);
    s.resize(GridSize { cols: 120, rows: 24 });
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
```

The remaining gate tests, one `#[test]` per row, on `screen(80, 24)` unless noted:

| Test | Feed | Assert |
|---|---|---|
| `fit_never_returns_zero` | none | `fit(Rect{x:0,y:0,w:0,h:0}, CellSize{w:17,h:31}) == GridSize{cols:1,rows:1}`; `fit(Rect{x:0,y:0,w:1700,h:310}, CellSize{w:17,h:31}) == GridSize{cols:100,rows:10}` |
| `selects_by_character_word_and_line` | `hello brave world` | `select(Char, at(0,0), Left)` then `select_to(at(4,0), Right)` gives `Some("hello")`; `select(Word, at(7,0), Left)` gives `Some("brave")`; `select(Line, at(7,0), Left)` gives text that equals `hello brave world` once a trailing newline is trimmed; `is_selected(at(7,0))` is true; `clear_selection()` gives `None` |
| `text_around_joins_wrapped_rows` | 100 times `a` | `text_around(at(5,1)) == ("a".repeat(100), 85)` |
| `reports_mouse_modes` | `\x1b[?1000h\x1b[?1006h`, then `\x1b[?1002h`, then `\x1b[?1003h`, then `\x1b[?1003l\x1b[?1002l\x1b[?1000l` | `modes().mouse` is `Click` with `sgr_mouse` true, then `Drag`, then `Motion`, then `Off` |
| `reports_paste_focus_and_screen_modes` | `\x1b[?2004h\x1b[?1004h\x1b[?1h\x1b[?1049h` | `bracketed_paste`, `focus_events`, `app_cursor`, `alt_screen` all true |
| `synchronized_output_holds_changes` | `\x1b[?2026hHELLO` | `row_text(0) == ""` and `sync_deadline().is_some()`; after feeding `\x1b[?2026l`, `row_text(0) == "HELLO"` and `sync_deadline().is_none()` |
| `overdue_sync_is_ended` | `\x1b[?2026hHELLO` | with `d = sync_deadline().unwrap()`: `end_sync_if_due(d - 1ms)` is false and row 0 is empty; `end_sync_if_due(d + 1ms)` is true and `row_text(0) == "HELLO"` |
| `marked_links_are_on_their_cells` | `\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ plain` | `link_at(at(1,0)) == Some("https://example.com")`; `link_at(at(6,0)) == None` |
| `wide_and_combined_characters_land_in_cells` | `a\u{6F22}b e\u{301}` | cells of row 0: index 1 is `ch == '\u{6F22}'` with `width == Wide`, index 2 is `Spacer`, index 3 is `'b'`, index 5 is `'e'` with `zerowidth == ['\u{301}']` |
| `kitty_keyboard_flags_follow_the_program` | `\x1b[?u` | `take_replies() == b"\x1b[?0u"`; after `\x1b[>1u`, `modes().kitty_flags == 1`; after `\x1b[<u`, `0` |
| `answers_the_queries_claude_code_sends` | each in turn | `\x1b[c` gives a reply that starts `\x1b[?` and ends `c`; `\x1b[6n` gives `\x1b[1;1R`; `\x1b[?2026$p` gives `\x1b[?2026;2$y`; `\x1b]11;?\x1b\\` gives a reply containing `rgb:1616/1414/1f1f` |
| `colours_are_final` | `\x1b[31mr\x1b[0m\x1b[7mi\x1b[0m\x1b[38;2;1;2;3mt` | cell 0 `fg == Rgb(0xF3,0x8B,0xA8)`, `bg == Rgb(0x16,0x14,0x1F)`; cell 1 `fg == Rgb(0x16,0x14,0x1F)`, `bg == Rgb(0xCD,0xD6,0xF4)`; cell 2 `fg == Rgb(1,2,3)` |
| `damage_names_only_changed_rows` | `x` after a first `take_damage()` | `take_damage() == Damage::Rows(vec![0])`; a second call gives `Damage::None`; after `select(Char, at(0,0), Left)` the next call is not `None`, and the one after it is `None` |
| `title_and_bell_are_events` | `\x1b]0;hi\x07\x07` | `take_events()` holds `Title("hi")` and `Bell` |

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p term-engine --test gate`
Expected: does not compile, `Screen` is not defined.

- [ ] **Step 4: Implement `geometry.rs`, `theme.rs` and `Screen` in `engine/src/screen.rs`**

`Screen` owns the `Term`, the `Processor` and a listener that collects replies and events into buffers. Colour requests are answered from the palette. `end_sync_if_due` calls `stop_sync`.

- [ ] **Step 5: Run the gate tests**

Run: `cargo test -p term-engine --test gate`
Expected: all pass. A test that cannot be made to pass through the wrapper is the gate speaking: do not weaken or delete it. Stop and report BLOCKED with the test name, what the library does instead, and whether a patch of under about 100 lines to the library would close it.

- [ ] **Step 6: Write and run the memory figure in `engine/tests/memory.rs`**

A counting `#[global_allocator]` (live bytes). Test `full_scrollback_memory_is_bounded`: one `Screen` at 120 by 40, fed 20,100 lines of 120 visible characters with a colour change on each line. With `cells = 120 * 20_040`: print `MEMORY one_terminal_bytes=<n> bytes_per_cell=<n / cells>`, assert `bytes_per_cell <= 32.0`. Then four such screens: assert the total is within 10% of four times the first figure.

Run: `cargo test -p term-engine --test memory -- --nocapture`
Expected: PASS, and a `MEMORY` line. Put that line in the report: forty times `one_terminal_bytes` is the figure stage B plans around (about 2.3 GB by arithmetic if every one of 40 terminals has filled its scrollback at 120 columns).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock engine/Cargo.toml engine/src engine/tests/gate.rs engine/tests/memory.rs .github/workflows/engine.yml
git commit -m "feat(engine): screen state behind our own type, with the library gate tests"
```

**Gate rule for the controller.** All gate tests green and `bytes_per_cell <= 32` means go. Otherwise: a small gap is closed by a patched copy of the library kept in this repo with its licence notice; a large gap means repeating this task with `termwiz`, then with Ghostty's library only if it builds on Windows. Tasks 2 to 11 do not start until this rule says go.

---

### Task 2: keys and mouse to bytes

Dispatch: opus, effort high.

**Files:**
- Create: `engine/src/keys.rs`, `engine/src/mouse.rs`, `engine/tests/keys.rs`
- Modify: `engine/src/lib.rs`

**Interfaces:**
- Consumes: `Modes`, `MouseTracking`, `CellPoint`.
- Produces:

```rust
// engine/src/keys.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyCode { Char(char), Enter, Tab, Backspace, Escape, Delete, Insert, Left, Right, Up, Down, Home, End, PageUp, PageDown, F(u8) }
bitflags! { pub struct Mods: u8 { const SHIFT = 1; const ALT = 2; const CTRL = 4; const SUPER = 8; } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum KeyEventKind { Press, Repeat, Release }
/// `code` is the key without Shift applied (Char('a') for Shift+A). `text` is what the keyboard layout produced, if anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput { pub code: KeyCode, pub mods: Mods, pub text: Option<String>, pub kind: KeyEventKind }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Platform { Mac, Other }
pub fn encode_key(key: &KeyInput, modes: &Modes, platform: Platform) -> Option<Vec<u8>>;

// engine/src/mouse.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Button { Left, Middle, Right }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction { Press(Button), Release(Button), Move(Option<Button>), Wheel(i32) }   // Wheel: lines, positive is up
pub fn encode_mouse(action: MouseAction, at: CellPoint, mods: Mods, modes: &Modes) -> Option<Vec<u8>>;
```

`Mods::ALT` means "Option acting as Meta". On Mac the window sets it only for the left Option key; the right Option key arrives as plain `text`.

- [ ] **Step 1: Write the failing tests in `engine/tests/keys.rs`**

One test per table, each row an `assert_eq!` on `encode_key` or `encode_mouse`. `kind` is `Press` and `platform` is `Mac` unless a row says otherwise.

`legacy_keys` (`kitty_flags == 0`):

| Key | Mods | Bytes |
|---|---|---|
| `Char('a')`, text `a` | none | `a` |
| `Char('a')`, text `A` | SHIFT | `A` |
| `Char('\u{E9}')`, text `\u{E9}` | none | `\xc3\xa9` |
| `Char('c')` | CTRL | `\x03` |
| `Char(' ')` | CTRL | `\x00` |
| `Char('a')` | ALT | `\x1ba` |
| `Enter` | none | `\r` |
| `Enter` | SHIFT | `\x1b\r` |
| `Enter` | ALT | `\x1b\r` |
| `Tab` | none, then SHIFT | `\t`, then `\x1b[Z` |
| `Backspace` | none, then CTRL | `\x7f`, then `\x08` |
| `Escape` | none | `\x1b` |
| `Up` | none | `\x1b[A`, and `\x1bOA` when `app_cursor` |
| `Up` | SHIFT | `\x1b[1;2A` |
| `Right` | CTRL | `\x1b[1;5C` |
| `Home`, `End` | none | `\x1b[H`, `\x1b[F` |
| `PageUp`, `PageDown` | none | `\x1b[5~`, `\x1b[6~` |
| `Delete`, `Insert` | none | `\x1b[3~`, `\x1b[2~` |
| `F(1)`, `F(5)`, `F(12)` | none | `\x1bOP`, `\x1b[15~`, `\x1b[24~` |
| any key, `kind: Release` | any | `None` |

`mac_line_keys` (these win over everything else, whatever `kitty_flags` is, when exactly one of SUPER or ALT is held and neither CTRL nor SHIFT):

| Key | Mods | Bytes |
|---|---|---|
| `Backspace` | SUPER | `\x15` |
| `Left`, `Right` | SUPER | `\x01`, `\x05` |
| `Delete` | SUPER | `\x0b` |
| `Char('z')` | SUPER | `\x1f` |
| `Left`, `Right` | ALT | `\x1bb`, `\x1bf` |
| `Backspace` | ALT | `\x1b\x7f` |
| `Delete` | ALT | `\x1bd` |
| `Char('t')` | SUPER | `None` (other Cmd keys belong to the window) |
| `Left` with `Platform::Other` | ALT | `\x1b[1;3D` |
| `Left` with `Platform::Other` | SUPER | `None` |

`kitty_keys` (the kitty keyboard protocol; `Platform::Other` so the Mac rules stay out of the way):

| `kitty_flags` | Key | Mods | Bytes |
|---|---|---|---|
| 1 | `Enter` | SHIFT | `\x1b[13;2u` |
| 1 | `Enter`, `Tab`, `Backspace` | none | `\r`, `\t`, `\x7f` |
| 1 | `Escape` | none | `\x1b[27u` |
| 1 | `Char('c')` | CTRL | `\x1b[99;5u` |
| 1 | `Char('a')` | ALT | `\x1b[97;3u` |
| 1 | `Char('a')` | CTRL and SHIFT | `\x1b[97;6u` |
| 1 | `Tab` | SHIFT | `\x1b[9;2u` |
| 1 | `Backspace` | CTRL | `\x1b[127;5u` |
| 1 | `Char('a')` text `a`; `Char('a')` text `A` with SHIFT | | `a`; `A` |
| 1 | `Up`; `Up` with CTRL | | `\x1b[A`; `\x1b[1;5A` |
| 3 | `Escape`, `kind: Release` | none | `\x1b[27;1:3u` |
| 3 | `Enter`, `kind: Repeat` | SHIFT | `\x1b[13;2:2u` |
| 3 | `Char('a')`, `kind: Release` | none | `\x1b[97;1:3u` |
| 3 | `Enter`, `kind: Release` | none | `None` (Enter, Tab and Backspace report no release unless flag 8 is set) |
| 8 | `Char('a')` text `a`; `Enter` | none | `\x1b[97u`; `\x1b[13u` |
| 12 | `Char('a')` text `A` | SHIFT | `\x1b[97:65;2u` |
| 24 | `Char('a')` text `A` | SHIFT | `\x1b[97;2;65u` |

If a row here disagrees with the kitty keyboard protocol document (`https://sw.kovidgoyal.net/kitty/keyboard-protocol/`, a specification, not code), the document wins: stop and report the row instead of picking.

`mouse_reports`, at `CellPoint { col: 0, row: 0 }` unless noted:

| Modes | Action | Mods | Bytes |
|---|---|---|---|
| `mouse: Off` | `Press(Left)` | none | `None` |
| `Click`, `sgr_mouse` | `Press(Left)`, then `Release(Left)` | none | `\x1b[<0;1;1M`, then `\x1b[<0;1;1m` |
| `Click`, `sgr_mouse` | `Press(Middle)`, `Press(Right)` | none | `\x1b[<1;1;1M`, `\x1b[<2;1;1M` |
| `Click`, `sgr_mouse` | `Press(Left)` | CTRL | `\x1b[<16;1;1M` |
| `Click`, `sgr_mouse` | `Wheel(1)`, `Wheel(-1)` | none | `\x1b[<64;1;1M`, `\x1b[<65;1;1M` |
| `Click`, `sgr_mouse` | `Wheel(3)` | none | `\x1b[<64;1;1M` three times |
| `Click`, `sgr_mouse` | `Move(Some(Left))` | none | `None` |
| `Drag`, `sgr_mouse` | `Move(Some(Left))` at col 2, row 1 | none | `\x1b[<32;3;2M` |
| `Drag`, `sgr_mouse` | `Move(None)` | none | `None` |
| `Motion`, `sgr_mouse` | `Move(None)` at col 2, row 1 | none | `\x1b[<35;3;2M` |
| `Click`, not `sgr_mouse` | `Press(Left)` | none | `\x1b[M` then the bytes 32, 33, 33 |
| `Click`, not `sgr_mouse` | `Press(Left)` at col 300 | none | `None` |

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p term-engine --test keys`
Expected: does not compile, `encode_key` is not defined.

- [ ] **Step 3: Implement `encode_key` and `encode_mouse`**

Written from the xterm control sequences document and the kitty keyboard protocol document. All five kitty flags are honoured, not only the rows tested.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p term-engine --test keys`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add engine/src/keys.rs engine/src/mouse.rs engine/src/lib.rs engine/tests/keys.rs
git commit -m "feat(engine): key and mouse encoding"
```

---

### Task 3: the view (input policy, no drawing)

Dispatch: opus, effort high.

**Files:**
- Create: `engine/src/view.rs`, `engine/src/links.rs`, `engine/tests/view.rs`
- Modify: `engine/src/lib.rs`

**Interfaces:**
- Consumes: `Screen` and its types (Task 1), `encode_key`, `encode_mouse`, `KeyInput`, `Mods`, `MouseAction`, `Button` (Task 2).
- Produces:

```rust
// engine/src/view.rs
pub struct TerminalView { /* private */ }
#[derive(Clone, Debug, PartialEq, Eq)] pub enum ViewEvent { Title(String), Bell, Copy(String), OpenLink(String) }
impl TerminalView {
    pub fn new(grid: GridSize, theme: &Theme, platform: Platform) -> TerminalView;
    pub fn feed(&mut self, bytes: &[u8]);                          // output from the shell
    pub fn take_output(&mut self) -> Vec<u8>;                      // bytes to write to the shell
    pub fn take_events(&mut self) -> Vec<ViewEvent>;
    pub fn key(&mut self, key: &KeyInput);
    pub fn pointer(&mut self, action: MouseAction, at: CellPoint, side: Side, mods: Mods, clicks: u8);
    pub fn paste(&mut self, text: &str);
    pub fn copy(&self) -> Option<String>;
    pub fn set_focused(&mut self, focused: bool);
    pub fn focused(&self) -> bool;
    pub fn grid(&self) -> GridSize;
    pub fn set_grid(&mut self, grid: GridSize);
    pub fn tick(&mut self, now: std::time::Instant) -> Option<std::time::Instant>;  // ends an overdue sync; when to call again
    pub fn needs_draw(&self) -> bool;
    pub fn mark_drawn(&mut self);
    pub fn screen(&self) -> &Screen;
    pub fn screen_mut(&mut self) -> &mut Screen;                    // for drawing, which takes the damage
    pub fn theme(&self) -> &Theme;
}
// engine/src/links.rs
pub fn url_at(text: &str, char_index: usize) -> Option<String>;     // an http or https URL covering that character
```

Rules the view owns:
- `key`: on `Platform::Mac`, the keys that are ours come first: Cmd+K clears the scrollback (not in the alternate screen) and queues `\x0c`; Cmd+A selects all; Cmd+Up, Cmd+Down, Cmd+Home and Cmd+End scroll exactly as `cmdKey` in `client/src/terminal.ts` does (read it, do not edit it) and queue nothing. Every other key goes through `encode_key`, and when that yields bytes the view scrolls to the bottom and queues them.
- `pointer` when `modes().mouse != Off` and Shift is not held: queue `encode_mouse`'s bytes. Otherwise it is ours: a left press starts a selection (`clicks` 1 by character, 2 by word, 3 by line), a move with the left button held extends it, and the release emits `ViewEvent::Copy(text)` when the selection is not empty, because selecting copies in today's app.
- A left press with Cmd held on Mac (Ctrl elsewhere) over a link emits `OpenLink(url)` and starts no selection. A link is a marked link from `link_at`, else `url_at` over `text_around`. Only `http://` and `https://` are ever emitted.
- `Wheel(n)` when the mouse is not reported: in the alternate screen with `alternate_scroll`, queue the Up or Down key `|n|` times; otherwise scroll the scrollback by `n` lines.
- `paste`: line endings become `\r`. With `bracketed_paste`, every `\x1b[201~` inside the text is removed and the text is wrapped in `\x1b[200~` and `\x1b[201~`.
- `set_focused`: queues `\x1b[I` or `\x1b[O` when `focus_events` is on.
- `needs_draw` is true only when what would be drawn has changed: output was fed since `mark_drawn` and no synchronized update is open, or the selection, scroll position, focus or grid changed.

- [ ] **Step 1: Write the failing tests in `engine/tests/view.rs`**

All on `TerminalView::new(GridSize { cols: 80, rows: 24 }, &Theme::builtin(), Platform::Mac)` unless noted.

| Test | Do | Assert |
|---|---|---|
| `replies_go_out` | `feed(b"\x1b[6n")` | `take_output() == b"\x1b[1;1R"` |
| `keys_go_out` | `key` of `Char('a')` with text `a` | `take_output() == b"a"` |
| `output_while_scrolled_up_keeps_place` | feed 100 numbered lines; `pointer(Wheel(10), ..)`; remember `row_text(0)`; feed 5 more lines | `row_text(0)` unchanged and `scroll_offset() == 15`; after a key press `scroll_offset() == 0` |
| `drag_selects_and_copies` | feed `hello brave world`; press left at col 0 side Left, move to col 4 side Right, release | `take_events() == [Copy("hello")]` and `copy() == Some("hello")` |
| `double_click_selects_a_word` | press and release left at col 7 with `clicks: 2` | `take_events() == [Copy("brave")]` |
| `mouse_goes_to_the_program_when_it_asks` | feed `\x1b[?1000h\x1b[?1006h`; press left at col 0 | `take_output() == b"\x1b[<0;1;1M"`, no selection |
| `shift_keeps_the_mouse_for_selecting` | same modes; press with SHIFT, move, release | `take_output()` is empty and `copy()` is `Some` |
| `cmd_click_opens_a_plain_link` | feed `see https://example.com/a?b=1, ok`; press left with SUPER at col 10 | `take_events() == [OpenLink("https://example.com/a?b=1")]` |
| `cmd_click_opens_a_marked_link` | feed `\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\`; press left with SUPER at col 1 | `[OpenLink("https://example.com")]` |
| `only_web_links_open` | feed `\x1b]8;;file:///etc/passwd\x1b\\x\x1b]8;;\x1b\\`; press left with SUPER at col 0 | no `OpenLink` event |
| `url_edges` | `url_at("(https://example.com/x).", 5)`; `url_at("no link here", 3)` | `Some("https://example.com/x")`; `None` |
| `wheel_scrolls_scrollback` | feed 100 lines; `Wheel(3)` | `scroll_offset() == 3`, `take_output()` empty |
| `wheel_becomes_arrows_in_a_full_screen_program` | feed `\x1b[?1049h\x1b[?1007h`; `Wheel(2)` | `take_output() == b"\x1b[A\x1b[A"` |
| `paste_is_bracketed_when_asked` | feed `\x1b[?2004h`; `paste("a\nb")` | `take_output() == b"\x1b[200~a\rb\x1b[201~"` |
| `paste_strips_the_end_marker` | feed `\x1b[?2004h`; `paste("x\x1b[201~rm -rf /\n")` | `take_output() == b"\x1b[200~xrm -rf /\r\x1b[201~"` |
| `plain_paste_turns_newlines_into_returns` | `paste("a\r\nb\n")` | `take_output() == b"a\rb\r"` |
| `cmd_k_clears_scrollback` | feed 100 lines; key `Char('k')` with SUPER | `screen().history_len() == 0`, `take_output() == b"\x0c"` |
| `cmd_a_selects_all` | feed `one\r\ntwo`; key `Char('a')` with SUPER | `copy()` contains `one` and `two`; `take_output()` empty |
| `focus_is_reported_when_asked` | feed `\x1b[?1004h`; `set_focused(false)`; `set_focused(true)` | `take_output() == b"\x1b[O\x1b[I"` |
| `cmd_scroll_keys_send_nothing` | feed 100 lines; key `Home` with SUPER; then key `End` with SUPER | after Home `screen().scroll_offset() == screen().history_len()`; after End it is 0; `take_output()` empty both times. If `cmdKey` does something else for these keys, `cmdKey` wins: report it |
| `idle_needs_no_draw` | feed `x` (`needs_draw()` is true); `mark_drawn()`; `take_output()`; `tick(now)` | `needs_draw()` false |
| `open_sync_needs_no_draw` | `mark_drawn()`; feed `\x1b[?2026hHELLO` | `needs_draw()` false and `tick(now)` returns `Some(deadline)`; `tick(deadline + 1ms)` makes `needs_draw()` true |
| `host_replay_then_resize` | view at 80 by 24; feed 30 lines of 100 characters; `set_grid(GridSize { cols: 100, rows: 30 })` | no panic, `grid()` is the new size, `needs_draw()` true, row 0 is 100 characters long |

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p term-engine --test view`
Expected: does not compile, `TerminalView` is not defined.

- [ ] **Step 3: Implement `TerminalView` and `url_at`**

A URL ends before closing brackets and trailing punctuation (`.`, `,`, `;`, `:`, `!`, `?`, `)`, `]`, `>`, quotes) that have no opener inside the URL.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p term-engine --test view`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add engine/src/view.rs engine/src/links.rs engine/src/lib.rs engine/tests/view.rs
git commit -m "feat(engine): terminal view with selection, scrolling, paste and link rules"
```

---

### Task 4: fonts, glyphs and the glyph texture packing (no GPU)

Dispatch: opus, effort high.

**Files:**
- Create: `engine/fonts/JetBrainsMonoNerdFontMono-Regular.ttf`, `-Bold.ttf`, `-Italic.ttf`, `-BoldItalic.ttf`, `engine/fonts/LICENSE.md`
- Create: `engine/src/font.rs`, `engine/src/boxdraw.rs`, `engine/src/atlas.rs`, `engine/tests/glyphs.rs`
- Modify: `engine/Cargo.toml`, `engine/src/lib.rs`

**Interfaces:**
- Consumes: `CellSize`.
- Produces:

```rust
// engine/src/font.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)] pub struct FontStyle { pub bold: bool, pub italic: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum GlyphKind { Mask, Color }   // Mask: 1 byte a pixel. Color: RGBA.
/// left and top place the bitmap relative to the top left corner of its cell.
pub struct Glyph { pub width: u32, pub height: u32, pub left: i32, pub top: i32, pub kind: GlyphKind, pub pixels: Vec<u8> }
pub struct Fonts { /* private */ }
impl Fonts {
    pub fn new() -> Fonts;                                          // the four built-in faces, system fonts as fallback
    pub fn cell_size(&mut self, font_px: f32, scale: f64) -> CellSize;
    /// `text` is one cell's base character plus its zero-width characters. `wide` cells are two cells across.
    pub fn rasterize(&mut self, text: &str, style: FontStyle, font_px: f32, scale: f64, wide: bool) -> Glyph;
}
// engine/src/boxdraw.rs
pub fn box_glyph(ch: char, cell: CellSize) -> Option<Glyph>;        // Some for U+2500..=U+259F, always exactly one cell, Mask
// engine/src/atlas.rs
#[derive(Clone, Debug, PartialEq, Eq, Hash)] pub struct GlyphKey { pub text: Box<str>, pub style: FontStyle, pub wide: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasSlot { pub x: u32, pub y: u32, pub w: u32, pub h: u32, pub left: i32, pub top: i32, pub color: bool }
pub struct Atlas { /* private */ }                                  // one RGBA image; masks are stored as white with alpha
impl Atlas {
    pub fn new() -> Atlas;                                          // starts at 1024 by 1024
    pub fn get_or_insert(&mut self, key: &GlyphKey, make: impl FnOnce() -> Glyph) -> AtlasSlot;
    pub fn clear(&mut self);
    pub fn size(&self) -> u32;
    pub fn len(&self) -> usize;
    pub fn generation(&self) -> u64;                                // changes whenever earlier slots stop being valid
    pub fn pixels(&self) -> &[u8];
    pub fn take_dirty(&mut self) -> Option<Rect>;                   // the region changed since the last call
}
```

Rules:
- Cell size: `h = round(font_px * scale * 1.1)`, `w = round(advance of one character at font_px * scale)`.
- When a glyph does not fit, the atlas doubles its side, up to 4096. Full at 4096, it clears itself. Either way `generation` changes and callers ask again for every glyph they still need.
- The fonts are copied from `/Users/x/Library/Fonts/` (same four file names). `LICENSE.md` names the source (JetBrains Mono, SIL Open Font License 1.1; patched by the Nerd Fonts project) and carries the OFL text. If the licence text cannot be confirmed from the font's own name table or a copy on disk, report it instead of writing one from memory.

- [ ] **Step 1: Write the failing tests in `engine/tests/glyphs.rs`**

`ink(g)` is the count of pixels whose alpha (or mask value) is above 127.

| Test | Assert |
|---|---|
| `cell_size_follows_the_rule` | `cell_size(14.0, 2.0) == CellSize { w: 17, h: 31 }`; `cell_size(14.0, 1.0) == CellSize { w: 8, h: 15 }`; `cell_size(15.0, 2.0).h == 33` |
| `plain_letter_fits_its_cell` | `A` regular at 14 px, scale 2: `Mask`, `ink > 0`, `left >= 0`, `left + width <= 17`, `top >= 0`, `top + height <= 31` |
| `bold_has_more_ink` | `ink` of bold `A` is greater than `ink` of regular `A` |
| `italic_differs_from_regular` | pixels of italic `A` differ from regular `A` |
| `emoji_is_in_colour` | `\u{1F600}` with `wide: true`: `Color`, and some pixel has `r != b` |
| `han_character_falls_back_to_a_system_font` | `\u{6F22}` with `wide: true`: `ink > 0`, `left + width <= 34` |
| `combining_mark_adds_ink_above` | `e\u{301}` has a smaller `top` than `e` |
| `nerd_font_symbol_comes_from_the_built_in_face` | `\u{E0B0}`: `ink > 0` |
| `unknown_character_does_not_panic` | `\u{10FFFD}` returns a `Glyph` |
| `horizontal_line_reaches_both_edges` | `box_glyph('\u{2500}', CellSize{w:17,h:31})`: some row has all 17 pixels at 255, and that row is `31 / 2` give or take 1 |
| `vertical_line_reaches_both_edges` | `'\u{2502}'`: some column has all 31 pixels at 255 |
| `corners_meet_the_lines` | for `'\u{250C}'` and the rounded `'\u{256D}'`: the right edge column has ink on the same rows as `'\u{2500}'`, and the bottom edge row has ink on the same columns as `'\u{2502}'` |
| `full_and_half_blocks_are_exact` | `'\u{2588}'` is all 255; `'\u{2580}'` is 255 on rows 0 to 14 and 0 from row 15 down (the split is `h / 2`); `'\u{2591}'` is neither all 0 nor all 255 |
| `every_box_character_is_drawn` | `box_glyph` is `Some` with `width == 17, height == 31` for every `ch` in `'\u{2500}'..='\u{259F}'`; `box_glyph('A', ..)` is `None` |
| `same_key_same_slot` | two `get_or_insert` calls with one key return equal slots and call `make` once |
| `slots_do_not_overlap` | 200 distinct keys with 20 by 40 glyphs: all slots inside `size()`, no two rectangles intersect |
| `a_full_atlas_grows_then_clears` | 30,000 distinct keys with 20 by 40 glyphs: `size() <= 4096` throughout, `generation()` changed at least twice, the slot returned last lies inside `size()`, and its pixels in `pixels()` are the glyph's |
| `dirty_region_covers_new_glyphs` | after one insert, `take_dirty()` is a `Rect` containing the slot; a second call gives `None` |

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p term-engine --test glyphs`
Expected: does not compile.

- [ ] **Step 3: Implement `font.rs`, `boxdraw.rs`, `atlas.rs`**

`Fonts` loads the four faces with `include_bytes!` into a cosmic-text `FontSystem` that also has the system fonts, shapes one cell's text as a single run so fallback and combining marks are handled by the library, and rasterizes through its swash cache. Box characters never come from a font.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p term-engine --test glyphs`
Expected: PASS on Mac. On the Windows CI runner the two fallback tests (`emoji_is_in_colour`, `han_character_falls_back_to_a_system_font`) are marked `#[cfg(target_os = "macos")]`.

- [ ] **Step 5: Commit**

```bash
git add engine/fonts engine/src/font.rs engine/src/boxdraw.rs engine/src/atlas.rs engine/src/lib.rs engine/Cargo.toml Cargo.lock engine/tests/glyphs.rs
git commit -m "feat(engine): built-in font, glyph rasterizing, box characters and glyph packing"
```

---

### Task 5: draw a grid on the GPU

Dispatch: opus, effort xhigh.

**Files:**
- Create: `engine/src/gpu.rs`, `engine/src/draw.rs`, `engine/src/shader.wgsl`, `engine/tests/draw.rs`
- Modify: `engine/Cargo.toml`, `engine/src/lib.rs`, `.github/workflows/engine.yml`

**Interfaces:**
- Consumes: `TerminalView`, `Screen::cells`, `Screen::take_damage` (Tasks 1, 3); `Fonts`, `Atlas`, `GlyphKey`, `box_glyph` (Task 4).
- Produces:

```rust
// engine/src/gpu.rs
pub struct Gpu { /* private: device, queue, pipelines, glyph texture, Fonts, Atlas */ }
#[derive(Debug)] pub enum GpuError { NoAdapter, Device(String) }
impl Gpu {
    /// One per process. Every view in every window draws through it.
    pub async fn new(instance: &wgpu::Instance, surface: Option<&wgpu::Surface<'_>>, target: wgpu::TextureFormat) -> Result<Gpu, GpuError>;
    pub fn device(&self) -> &wgpu::Device;
    pub fn queue(&self) -> &wgpu::Queue;
    pub fn set_text_size(&mut self, font_px: f32, scale: f64);     // zoom or a new display scale; clears the glyph texture
    pub fn cell_size(&self) -> CellSize;
    pub fn glyph_count(&self) -> usize;
}
// engine/src/draw.rs
pub struct ViewDraw { /* private: instance buffers and the rows they were built from */ }
impl ViewDraw {
    pub fn new(gpu: &Gpu) -> ViewDraw;
    /// Rebuilds only what changed since the last call, uploads it, and calls view.mark_drawn().
    pub fn prepare(&mut self, gpu: &mut Gpu, view: &mut TerminalView, rect: Rect, target: (u32, u32));
    pub fn draw(&self, gpu: &Gpu, pass: &mut wgpu::RenderPass<'_>);
    pub fn rows_rebuilt(&self) -> usize;                            // by the last prepare
}
/// Test helper: draws one view into a new `width` by `height` image cleared to `clear`, returns RGBA8 rows.
pub fn render_to_image(gpu: &mut Gpu, draw: &mut ViewDraw, view: &mut TerminalView, rect: Rect, width: u32, height: u32, clear: Rgb) -> Vec<u8>;
```

Rules:
- A frame is two instanced draws per view: cell backgrounds, then glyphs from the one glyph texture. The scissor is the view's rectangle. The area of the rectangle not covered by the grid is filled with the palette background.
- Glyph quads sit on whole pixels and sample without filtering.
- This task draws backgrounds, text in its colours, and box characters. Cursor, selection and text decorations are Task 6.

- [ ] **Step 1: Write the failing tests in `engine/tests/draw.rs`**

Shared setup: `Gpu::new(&instance, None, Rgba8UnormSrgb)` with `pollster::block_on`, `set_text_size(14.0, 2.0)` (cell 17 by 31), a view of 20 by 4, clear colour magenta `Rgb(255, 0, 255)`. `px(img, x, y)` reads one pixel as `Rgb`.

| Test | Do | Assert |
|---|---|---|
| `default_background_is_exact` | empty view, rect `0,0,340,124`, image 340 by 124 | `px(5, 5) == Rgb(0x16, 0x14, 0x1F)` |
| `coloured_background_is_exact` | feed `\x1b[41m  \x1b[0m` | `px(8, 15) == Rgb(0xF3, 0x8B, 0xA8)`; `px(8 + 2 * 17, 15) == Rgb(0x16, 0x14, 0x1F)` |
| `text_is_drawn_in_its_colour` | feed `\x1b[38;2;255;255;255m\u{2588}` | `px(8, 15) == Rgb(255, 255, 255)` |
| `letters_leave_ink` | feed `A` | some pixel in cell (0,0) differs from the background; every pixel in cell (5,0) equals it |
| `box_lines_join_across_cells` | feed `\u{2500}\u{2500}\u{2500}` | some row y between 14 and 16 has every pixel from x 0 to x 50 equal to `Rgb(0xCD, 0xD6, 0xF4)` |
| `grid_smaller_than_rect_is_padded` | rect `0,0,400,200`, image 400 by 200 | `px(390, 190) == Rgb(0x16, 0x14, 0x1F)` |
| `grid_larger_than_rect_is_clipped` | feed `\x1b[41m` and 80 spaces; rect `10,10,100,50`, image 340 by 124 | `px(5, 5)`, `px(111, 30)` and `px(30, 61)` are magenta; `px(20, 20) == Rgb(0xF3, 0x8B, 0xA8)` |
| `zero_size_rect_draws_nothing` | rect `0,0,0,0` | no panic; every pixel is magenta |
| `scale_change_redraws_glyphs` | feed `A\x1b[3;1H`; render; `set_text_size(14.0, 1.0)`; render with rect `0,0,160,60`, image 160 by 60 | `cell_size() == CellSize { w: 8, h: 15 }`; some pixel with `x < 8, y < 15` differs from the background; every pixel with `x >= 8, y < 15` is the background |
| `second_frame_rebuilds_only_changed_rows` | render; feed `x`; `prepare` | `rows_rebuilt() == 1` |
| `unchanged_view_rebuilds_nothing` | render; `prepare` again | `rows_rebuilt() == 0` |
| `two_views_share_the_glyph_texture` | two views with their own `ViewDraw`, both fed `hello`; render both | `glyph_count()` after the second equals the count after the first |

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p term-engine --test draw`
Expected: does not compile.

- [ ] **Step 3: Implement `gpu.rs`, `draw.rs`, `shader.wgsl`**

Colours are written so that an 8-bit palette value comes out of an sRGB target unchanged (the exact-colour tests pin this). When the atlas `generation` changes, every row is rebuilt.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p term-engine --test draw`
Expected: PASS on this Mac.

- [ ] **Step 5: Split the CI run**

In `engine.yml`: Windows runs `cargo test -p term-engine --test gate --test keys --test view --test glyphs --lib`; Mac runs `cargo test -p term-engine`. If the Mac runner has no GPU adapter, the drawing tests there are skipped by setting `TERM_ENGINE_NO_GPU=1` in the workflow only, which makes the shared setup return early with a printed notice; say so in the report.

- [ ] **Step 6: Commit**

```bash
git add engine/src/gpu.rs engine/src/draw.rs engine/src/shader.wgsl engine/src/lib.rs engine/Cargo.toml Cargo.lock engine/tests/draw.rs .github/workflows/engine.yml
git commit -m "feat(engine): draw the cell grid with wgpu from one glyph texture"
```

---

### Task 6: cursor, selection, decorations and the stored images

Dispatch: opus, effort high.

**Files:**
- Modify: `engine/src/draw.rs`, `engine/src/shader.wgsl`, `engine/tests/draw.rs`
- Create: `engine/tests/images.rs`, `engine/tests/images/*.png` (six files)

**Interfaces:**
- Consumes: Task 5's `Gpu`, `ViewDraw`, `render_to_image`; `Screen::cursor`, `Screen::is_selected`, `TerminalView::focused`.
- Produces: no new names. `ViewDraw::prepare` now also draws the cursor, the selection and the decorations.

Rules:
- Cursor `Bar`: a column `max(1, round(scale))` pixels wide at the left of the cell, in the cursor colour. `Block`: the cell filled with the cursor colour and its glyph in the cursor text colour. `Underline`: a bar of the same thickness at the bottom of the cell. When the view is not focused, every shape is drawn as a one-pixel-thick (times scale) outline of the cell.
- Selected cells: background `#F5E0DC`, glyph `#1E1E2E`. Colour emoji keep their own colours.
- Underline, double underline, strikeout, dotted and dashed underline are rectangles in the cell's text colour; an undercurl may be drawn as a dotted underline.

- [ ] **Step 1: Add the failing tests to `engine/tests/draw.rs`**

Same setup as Task 5 (cell 17 by 31, scale 2).

| Test | Do | Assert |
|---|---|---|
| `cursor_is_a_bar` | empty focused view | `px(0, 15)` and `px(1, 15)` are `Rgb(0xF5, 0xE0, 0xDC)`; `px(4, 15)` is the background |
| `block_cursor_inverts_its_cell` | feed `\x1b[2 q` | `px(8, 2) == Rgb(0xF5, 0xE0, 0xDC)` |
| `unfocused_cursor_is_an_outline` | `set_focused(false)` | `px(0, 15)` and `px(16, 15)` are the cursor colour; `px(8, 15)` is the background |
| `hidden_cursor_is_not_drawn` | feed `\x1b[?25l` | `px(0, 15)` is the background |
| `selection_uses_its_colours` | feed `\u{2588} \x1b[3;1H`; `screen_mut().select(Char, cell 0, Left)` then `select_to(cell 1, Right)` | `px(8, 15) == Rgb(0x1E, 0x1E, 0x2E)` (the block glyph in the selection text colour); `px(17 + 8, 15) == Rgb(0xF5, 0xE0, 0xDC)` (the empty cell in the selection background) |
| `underline_is_drawn_below_the_text` | feed `\x1b[4m \x1b[0m\x1b[3;1H` | some row in the bottom quarter of cell (0,0) is all `Rgb(0xCD, 0xD6, 0xF4)`; the top half of the cell is all background |
| `strikeout_crosses_the_middle` | feed `\x1b[9m \x1b[0m\x1b[3;1H` | some row within 4 pixels of y 15 is all foreground |
| `moving_the_cursor_rebuilds_two_rows` | render; feed `\x1b[3;1H`; `prepare` | `rows_rebuilt() <= 2` |

- [ ] **Step 2: Run to see them fail, implement, run to see them pass**

Run: `cargo test -p term-engine --test draw`
Expected before: the new tests FAIL. After: PASS.

- [ ] **Step 3: Write `engine/tests/images.rs` (stored images)**

Six screens, each a view of 40 by 8 drawn at scale 2 into an image of 680 by 248:

| File | Feed |
|---|---|
| `colours.png` | the 16 colours as backgrounds and as text, a row of 256-colour blocks, a row of true-colour blocks |
| `styles.png` | regular, bold, italic, bold italic, each underline style, strikeout, dim, inverse |
| `boxes.png` | a single-line box with rounded corners around text, a double-line box, a heavy box, half blocks and shades |
| `wide.png` | Han characters, a combining mark, colour emoji, Nerd Font symbols `\u{E0B0}` `\u{E0A0}` |
| `cursor.png` | three lines of text with a block cursor (`\x1b[2 q`) on a letter in the middle of line 2; the bar cursor is already in the other images |
| `selection.png` | three lines with a selection spanning line ends |

Each test renders, then compares with `engine/tests/images/<name>.png`: it fails if more than 0.2% of pixels differ by more than 16 in any channel, and on failure writes `target/image-diff/<name>.actual.png`. With `TERM_ENGINE_BLESS=1` it writes the stored file instead. `emoji` and Han content make `wide.png` Mac-only; mark that test `#[cfg(target_os = "macos")]`.

- [ ] **Step 4: Bless, look, run**

Run: `TERM_ENGINE_BLESS=1 cargo test -p term-engine --test images`, then `cargo test -p term-engine --test images`.
Expected: PASS. Before committing, open each of the six PNG files with the Read tool and check it against its row above: the right content, no gaps in box lines, no clipped glyphs, emoji in colour. A wrong image is a bug to fix, not a baseline to store.

- [ ] **Step 5: Commit**

```bash
git add engine/src/draw.rs engine/src/shader.wgsl engine/tests/draw.rs engine/tests/images.rs engine/tests/images
git commit -m "feat(engine): cursor, selection and text decorations, with stored image checks"
```

---

### Task 7: the shell and the windowless mode

Dispatch: opus, effort high.

**Files:**
- Create: `plain/Cargo.toml`, `plain/src/main.rs`, `plain/src/shell.rs`, `plain/src/session.rs`, `plain/src/headless.rs`, `plain/tests/headless.rs`
- Create: `engine/src/recording.rs`
- Modify: `engine/src/lib.rs`, `Cargo.toml` (root: add `"plain"` to `members`), `.github/workflows/engine.yml`

**Interfaces:**
- Consumes: `TerminalView`, `GridSize`, `CellSize`, `Theme`, `Platform`.
- Produces:

```rust
// engine/src/recording.rs   file: the line "TERMREC1\n", then records of [kind u8][ms u64 LE][len u32 LE][payload]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record { Output { ms: u64, bytes: Vec<u8> }, Resize { ms: u64, grid: GridSize } }   // kinds 0 and 1; Resize payload is cols u16 LE, rows u16 LE
pub struct Recorder<W: std::io::Write> { /* private */ }
impl<W: std::io::Write> Recorder<W> {
    pub fn new(out: W, grid: GridSize) -> std::io::Result<Recorder<W>>;   // writes the header and a Resize at ms 0
    pub fn output(&mut self, ms: u64, bytes: &[u8]) -> std::io::Result<()>;
    pub fn resize(&mut self, ms: u64, grid: GridSize) -> std::io::Result<()>;
}
pub fn read_recording(input: impl std::io::Read) -> std::io::Result<Vec<Record>>;

// plain/src/shell.rs
pub enum ShellEvent { Output(Vec<u8>), Exited(u32) }
pub struct Shell { /* private */ }
impl Shell {
    /// `command` None runs `$SHELL -l` (or `/bin/zsh -l`). `on_event` is called on the reader thread.
    pub fn spawn(command: Option<Vec<String>>, cwd: Option<std::path::PathBuf>, grid: GridSize,
                 on_event: Box<dyn Fn(ShellEvent) + Send>) -> std::io::Result<Shell>;
    pub fn write(&self, bytes: Vec<u8>);                 // queued to a writer thread; returns at once, also after exit
    pub fn resize(&self, grid: GridSize, cell: CellSize);
}
// plain/src/session.rs
pub struct Session { pub view: TerminalView, /* private: Shell, optional Recorder, start time */ }
impl Session {
    pub fn new(shell: Shell, grid: GridSize, record: Option<std::path::PathBuf>) -> std::io::Result<Session>;
    pub fn output(&mut self, bytes: &[u8]);              // record, feed the view, then flush
    pub fn flush(&mut self);                             // view.take_output() to the shell
    pub fn set_grid(&mut self, grid: GridSize, cell: CellSize);   // view, shell and recording together
}
```

The shell's environment: `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=snowflake`, `TERM_PROGRAM_VERSION` the crate version. The reader thread reads up to 64 KiB at a time.

Command line: `snowflake [--headless --cols N --rows N [--keys FILE] [--timeout SECONDS]] [--record FILE] [--cwd DIR] [-- COMMAND ARGS...]`. In `--headless` mode there is no window: output feeds a `Session`, each line of the keys file (`<milliseconds since start> <text>`, with `\r`, `\n`, `\t`, `\\` and `\xNN` escapes) is written to the shell as raw bytes at its time, and when the command exits the final screen is printed (one `row_text` per line) and the process exits with the command's exit code. A timeout kills the command and exits 124. Without `--headless`, this task prints `window mode is not built yet` and exits 2.

- [ ] **Step 1: Write the failing tests**

In `engine/src/recording.rs` (unit test) `recording_round_trips`: a `Recorder` at 80 by 24 given `output(5, b"hi")` and `resize(9, 100 by 30)` reads back as `[Resize{0, 80x24}, Output{5, "hi"}, Resize{9, 100x30}]`; `read_recording` of bytes without the header is an error; a file cut off mid-record yields the whole records before the cut.

In `plain/tests/headless.rs`, each test runs `env!("CARGO_BIN_EXE_snowflake")` with `--headless --cols 80 --rows 24`:

| Test | Arguments after the common ones | Assert |
|---|---|---|
| `prints_the_final_screen` | `-- /bin/sh -c 'printf "hello\r\nworld"'` | stdout line 1 is `hello`, line 2 is `world`; exit code 0 |
| `exit_code_is_the_commands` | `-- /bin/sh -c 'exit 7'` | exit code 7 |
| `queries_are_answered_through_the_pty` | `-- /bin/zsh -c 'printf "\e[6n"; read -rs -d R reply; print -r -- "got ${reply#*\[}"'` | stdout contains `got 1;1` |
| `keys_file_is_typed` | `--keys <file with "300 hi\r" and "600 \x04">` `-- /bin/cat` | stdout has `hi` on two lines (the echo and cat's copy); exit code 0 |
| `timeout_ends_a_stuck_command` | `--timeout 1 -- /bin/cat` | exit code 124 within 3 seconds |
| `recording_holds_the_output` | `--record <temp file> -- /bin/sh -c 'printf hello'` | `read_recording` gives a first `Resize` of 80 by 24 and `Output` records whose bytes joined contain `hello` |
| `environment_names_the_terminal` | `-- /bin/sh -c 'printf "%s %s" "$TERM" "$TERM_PROGRAM"'` | stdout line 1 is `xterm-256color snowflake` |
| `five_megabyte_write_arrives_whole` | keys file: one line at 300 ms holding 65,536 lines of 79 `x` plus `\r` (5,242,880 bytes), then `\x04` at a time after it; `--timeout 30 -- /bin/sh -c 'stty -echo; wc -c'` | stdout contains `5242880` |

Unit test in `plain/src/shell.rs`, `write_after_exit_does_not_panic`: spawn `/usr/bin/true`, wait for `Exited`, call `write(vec![b'x'; 1024])` and `resize(..)`; no panic and `write` returns within 100 ms.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p term-plain`
Expected: does not compile.

- [ ] **Step 3: Implement `recording.rs`, `shell.rs`, `session.rs`, `headless.rs`, `main.rs`**

- [ ] **Step 4: Run the tests**

Run: `cargo test -p term-engine --lib recording && cargo test -p term-plain`
Expected: PASS. If the 5 MB test is unreliable because of how the pty treats input lines, keep the 5,242,880 byte count and the 30 second limit and change only the receiving command; say what changed in the report.

- [ ] **Step 5: CI**

`engine.yml`: Mac also runs `cargo test -p term-plain`; Windows also runs `cargo check -p term-plain`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock plain/Cargo.toml plain/src plain/tests/headless.rs engine/src/recording.rs engine/src/lib.rs .github/workflows/engine.yml
git commit -m "feat(plain): shell on a pty with a windowless mode, and the recording format"
```

---

### Task 8: the window

Dispatch: opus, effort xhigh.

**Files:**
- Create: `plain/src/window.rs`, `plain/src/input.rs`
- Modify: `plain/src/main.rs`, `plain/Cargo.toml`

**Interfaces:**
- Consumes: `Session`, `Shell`, `ShellEvent` (Task 7); `Gpu`, `ViewDraw` (Tasks 5, 6); `fit`, `cell_at`, `KeyInput`, `KeyCode`, `Mods`, `KeyEventKind`, `MouseAction`, `Button`, `ViewEvent`, `FONT_PX_*`.
- Produces:

```rust
// plain/src/input.rs  (pure, so it is tested without a window)
pub fn key_input(logical: &winit::keyboard::Key, text: Option<&str>, mods: winit::keyboard::ModifiersState,
                 pressed: bool, repeat: bool) -> Option<KeyInput>;
#[derive(Clone, Copy, Debug, PartialEq)] pub enum WindowKey { Quit, Close, Copy, Paste, ZoomIn, ZoomOut, ZoomReset }
pub fn window_key(input: &KeyInput) -> Option<WindowKey>;            // Cmd+Q, Cmd+W, Cmd+C, Cmd+V, Cmd+= and Cmd+Shift+=, Cmd+-, Cmd+0
pub fn zoomed(font_px: f32, key: WindowKey) -> f32;                  // 1 px steps, clamped to 8..=40, reset to 14
pub struct Settle { /* private */ }                                  // the 100 ms resize rule
impl Settle {
    pub fn new() -> Settle;
    pub fn changed(&mut self, now: std::time::Instant);
    pub fn due(&mut self, now: std::time::Instant) -> bool;          // true once, 100 ms after the last change
    pub fn deadline(&self) -> Option<std::time::Instant>;
}
pub struct Clicks { /* private */ }
impl Clicks {
    pub fn new() -> Clicks;
    pub fn press(&mut self, at: CellPoint, now: std::time::Instant) -> u8;   // 1, 2, 3, then 1 again; same cell within 400 ms
}
pub struct WheelLines { /* private */ }
impl WheelLines {
    pub fn new() -> WheelLines;
    pub fn pixels(&mut self, dy: f64, cell_h: u32) -> i32;           // whole lines, remainder kept for the next call
}
pub fn grid_for(size_px: (u32, u32), cell: CellSize) -> GridSize;    // the window's only way to a grid: `fit` over the whole surface
pub fn may_present(visible: bool, size_px: (u32, u32)) -> bool;      // false when hidden or when either side is 0
```

What the window does. Each line is a decision, not a suggestion:
- Size 1280 by 800 logical pixels, the system's standard title bar, title `Snowflake` until the shell sets one. Option is Meta for the left key only (`OptionAsAlt::OnlyLeft`); the composing pop-up is off.
- One `Gpu`, one `ViewDraw`, one `Session`. Surface present mode from `TERM_PLAIN_PRESENT` (`fifo` or `immediate`, default `fifo`), `desired_maximum_frame_latency: 1`.
- The reader thread's events reach the event loop through an `EventLoopProxy`. On a wake: take everything waiting, feed it, and ask for a redraw only if `view.needs_draw()`. While output keeps arriving, at most 8 ms of it is fed before a frame is drawn, so a flood never starves the screen or the keyboard.
- A redraw calls `prepare` and `draw` once and presents. Nothing else presents. Every configure and every present is guarded by `may_present`, so while the window is fully hidden or minimised nothing is drawn, and a surface is never configured with a zero width or height.
- Keys: `window_key` first; everything else goes to `view.key` and then `session.flush()`. Copy does nothing without a selection. Paste reads the clipboard and calls `view.paste`.
- `ViewEvent::Copy` writes the clipboard, `OpenLink` runs `/usr/bin/open <url>`, `Title` sets the window title, `Bell` is ignored.
- Mouse: positions go through `cell_at`; `Clicks` supplies the count; trackpad pixels go through `WheelLines`, wheel notches count as 3 lines.
- Resize: the surface follows at once and the old grid stays (the engine pads or clips). `Settle` then triggers one `fit` and `session.set_grid`. The first frame and a zoom fit at once.
- Zoom: `gpu.set_text_size(new_px, scale)`, then fit. A display scale change does the same with the new scale.
- `view.tick` is called before each frame and its answer becomes the event loop's wake time, together with `Settle::deadline`. With neither, the loop waits for events and uses no CPU.
- The shell exiting closes the window and ends the process with the shell's exit code.
- With `TERM_PLAIN_STATS=<file>` set, the file is rewritten after every presented frame with `frames=<count>`.
- `--record FILE` works here as in the windowless mode.

- [ ] **Step 1: Write the failing unit tests in `plain/src/input.rs`**

| Test | Assert |
|---|---|
| `letters_carry_their_text` | `key_input(Character("a"), Some("a"), none, true, false)` is `Char('a')`, text `a`, `Press` |
| `shift_keeps_the_base_key` | `key_input(Character("A"), Some("A"), SHIFT, ..)` has `code == Char('a')`, text `A`, `Mods::SHIFT` |
| `named_keys_map` | `Enter`, `Tab`, `Backspace`, `Escape`, `ArrowLeft`, `Home`, `PageDown`, `F5`, `Delete` map to their `KeyCode`s; `Named(Shift)` alone gives `None` |
| `release_and_repeat_are_marked` | `pressed: false` gives `Release`; `repeat: true` gives `Repeat` |
| `command_is_super` | `ModifiersState::SUPER` gives `Mods::SUPER`; `ALT` gives `Mods::ALT` |
| `window_keys` | Cmd+Q `Quit`, Cmd+W `Close`, Cmd+C `Copy`, Cmd+V `Paste`, Cmd+= `ZoomIn`, Cmd+Shift+= `ZoomIn`, Cmd+- `ZoomOut`, Cmd+0 `ZoomReset`; Cmd+K and plain `q` give `None` |
| `zoom_steps_and_limits` | `zoomed(14.0, ZoomIn) == 15.0`; `zoomed(40.0, ZoomIn) == 40.0`; `zoomed(8.0, ZoomOut) == 8.0`; `zoomed(23.0, ZoomReset) == 14.0` |
| `settle_fires_once_after_100_ms` | `changed(t)`; `due(t + 99ms)` false; `changed(t + 50ms)`; `due(t + 149ms)` false; `due(t + 150ms)` true; `due(t + 300ms)` false |
| `clicks_count_to_three` | presses on one cell at `t`, `t+100ms`, `t+200ms`, `t+300ms` give 1, 2, 3, 1; a press 401 ms after the last gives 1; a press on another cell gives 1 |
| `trackpad_pixels_become_lines` | with `cell_h` 31: `pixels(20.0)` is 0, `pixels(20.0)` is 1, `pixels(-71.0)` is -2 |
| `scale_change_refits` | the 1280 by 800 point window at scale 2: `grid_for((2560, 1600), CellSize { w: 17, h: 31 }) == GridSize { cols: 150, rows: 51 }`; the same window at scale 1: `grid_for((1280, 800), CellSize { w: 8, h: 15 }) == GridSize { cols: 160, rows: 53 }` |
| `hidden_or_empty_window_never_presents` | `may_present(false, (800, 600))`, `may_present(true, (0, 600))` and `may_present(true, (800, 0))` are false; `may_present(true, (1, 1))` is true; `grid_for((0, 0), ..) == GridSize { cols: 1, rows: 1 }` |

- [ ] **Step 2: Run to see them fail, implement `input.rs`, run to see them pass**

Run: `cargo test -p term-plain input`
Expected before: does not compile. After: PASS.

- [ ] **Step 3: Implement `window.rs` and wire `main.rs`**

`wry`'s `examples/wgpu.rs` and winit 0.30's `ApplicationHandler` are the shape to follow for surface setup.

- [ ] **Step 4: Check the real window once**

A window appears on Alex's screen for four seconds each time this runs, and takes the keyboard while it is up. The controller tells Alex before dispatching this task; run it as few times as the fix needs.

Build with `cargo build --release -p term-plain`. Start `target/release/snowflake -- /bin/sh -c 'printf "\033[31mred\033[0m \342\224\200\342\224\200 ok\n"; sleep 4'` in the background, find its window id with `aerospace list-windows --monitor all --pid <pid> --format '%{window-id}'`, capture it with `screencapture -x -l <id> <scratch file>.png`, and read the image: dark background, `red` in the theme's red, a joined line, a bar cursor. The process exits by itself after 4 seconds; confirm it has. If the capture is refused, say so in the report and leave this check to the hand list. Do not type into the window and do not run a shell of your own in it.

- [ ] **Step 5: Run everything**

Run: `cargo test -p term-engine && cargo test -p term-plain`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add plain/src/window.rs plain/src/input.rs plain/src/main.rs plain/Cargo.toml Cargo.lock
git commit -m "feat(plain): the window, with zoom, settle-then-refit resize, copy, paste and links"
```

---

### Task 9: replay of a recorded Claude Code session

Dispatch: opus, effort high.

**Files:**
- Create: `engine/tests/replay.rs`, `tools/plain/record-claude.sh`
- Modify: `.gitignore` (add `engine/tests/recordings/*.local.rec`)

**Interfaces:**
- Consumes: `read_recording`, `Record` (Task 7), `TerminalView` (Task 3), `snowflake --headless --record --keys` (Task 7).
- Produces: `engine/tests/recordings/claude.local.rec` on this computer only. It is never committed, because the session's first screen can show the account and folder names.

- [ ] **Step 1: Write the replay tests in `engine/tests/replay.rs`**

A helper `replay(records: &[Record], merged: bool) -> Report`: a view at the first `Resize`'s grid; each `Output` is fed (when `merged`, neighbouring `Output` records are joined into one feed), each `Resize` calls `set_grid`. After every feed it calls `tick(start + ms)`, counts `frames` (times `needs_draw()` was true, each followed by `mark_drawn()`), counts `drawn_during_sync` (times `needs_draw()` was true while `screen().sync_deadline()` was `Some`), notes `saw_sync` and the largest `kitty_flags` seen. `Report` also holds the final rows.

- `synthetic_full_screen_redraw_never_shows_half_a_frame` (always runs): records built in the test: enter the alternate screen; then one synchronized update that clears the screen and paints three rows, cut into five `Output` records in the middle of escape sequences. Assert `drawn_during_sync == 0`, `saw_sync`, the three rows are in the final rows, and `replay(.., true)` gives the same final rows as `replay(.., false)`.
- `claude_session_replays_clean` (`#[ignore]`, reads `engine/tests/recordings/claude.local.rec`, fails with a clear message if the file is missing): assert no panic, `drawn_during_sync == 0`, `saw_sync` is true, merged and unmerged final rows are equal, and the final rows are not all empty. Print `REPLAY frames=<n> kitty_flags=<n> records=<n>`.

- [ ] **Step 2: Run the synthetic test**

Run: `cargo test -p term-engine --test replay`
Expected: PASS (the engine already holds drawing during a synchronized update; a failure here is an engine bug to fix in `view.rs`).

Then add `--test replay` to the Windows line in `.github/workflows/engine.yml` (the synthetic test needs no GPU), and stage that file in Step 5.

- [ ] **Step 3: Write `tools/plain/record-claude.sh`**

It makes a temporary empty folder, writes a keys file (`4000 \r`, `9000 say hi in five words\r`, `30000 /exit\r`), and runs `target/release/snowflake --headless --cols 120 --rows 40 --timeout 45 --cwd <folder> --keys <file> --record engine/tests/recordings/claude.local.rec -- claude`, then removes the folder. It prints the size of the recording.

- [ ] **Step 4: Make the recording and run the real replay**

Run: `zsh tools/plain/record-claude.sh`, then `cargo test -p term-engine --test replay -- --ignored --nocapture`.
Expected: PASS and a `REPLAY` line. The script sends one short prompt with Alex's own Claude Code login. If Claude Code refuses to start from inside an agent session, do not work around it: report the script path so Alex can run it with the `!` prefix, and leave the ignored test for the controller. If `saw_sync` is false, that is a finding (Claude Code did not get the answer it probes for): report it with the `REPLAY` line, do not delete the assertion.

- [ ] **Step 5: Commit**

```bash
git add engine/tests/replay.rs tools/plain/record-claude.sh .gitignore .github/workflows/engine.yml
git commit -m "test(engine): replay recorded sessions and check nothing draws mid-update"
```

---

### Task 10: timing scripts for any terminal program

Dispatch: opus, effort high.

**Files:**
- Create: `tools/keylat/bin-window.sh`, `tools/keylat/test-bin.sh`, `tools/speed/throughput-bin.sh`, `tools/speed/plain-compare.sh`, `tools/speed/plain-targets.sh`, `tools/speed/idle-frames.sh`, `tools/speed/configs/alacritty.toml`, `tools/speed/configs/ghostty.conf`, `tools/speed/configs/summary-fixture-pass.txt`, `tools/speed/configs/summary-fixture-fail.txt`
- Modify: `tools/speed/run-sitting.sh`, `tools/speed/README.md`

**Interfaces:**
- Consumes: the `snowflake` release binary and `TERM_PLAIN_PRESENT`, `TERM_PLAIN_STATS` (Task 8); the existing `tools/keylat/app-window.sh`, `tools/keylat/test-app.sh`, `tools/keylat/cat-shell.sh`, `tools/speed/throughput.sh`, `tools/speed/throughput-shell.sh`, `tools/speed/builds-compare.sh`, `tools/speed/run-sitting.sh`.
- Produces: `TA_JOB=plain-compare.sh zsh tools/speed/run-sitting.sh <name>` writes `tools/keylat/results/<name>/summary.txt`; `zsh tools/speed/plain-targets.sh <summary.txt>` prints one `PASS` or `FAIL` line per rule and exits non-zero on any `FAIL`.

Read the existing scripts first; the new ones follow their shape, their paths and their way of handing a job to the runner (`builds-compare.sh` is the closest example, and the README explains the runner). Do not run anything that takes the keyboard: only syntax-check (`zsh -n`) and the dry checks below. The sitting itself is Task 11.

Decisions:
- `bin-window.sh` is sourced. `open_bin <label> <program run inside the terminal>` starts one of five targets and sets `BIN_PID` and `BIN_WINDOW`; `close_bin` ends it. Targets: `plain-fifo` and `plain-immediate` (`target/release/snowflake -- <program>` with `TERM_PLAIN_PRESENT` set), `kitty`, `ghostty`, `alacritty`.
- Every target opens at 1280 by 800 points, font size 14, scrollback 20,000 lines (or the nearest setting it has), set by command-line overrides or by the two files in `tools/speed/configs/`. No script reads or writes any terminal's own config folder. Find each program's flags from its own `--help` output on this computer; do not guess a flag. kitty's existing overrides in `throughput.sh` are the model, plus `-o scrollback_lines=20000`.
- A target whose program is not installed is skipped with a line `<label>: not installed` in the summary; the sitting goes on.
- `test-bin.sh <label> <result file> <raw csv>` is `test-app.sh` with `open_bin`. `throughput-bin.sh <label> <runs>` is `throughput.sh` with `open_bin` and prints `<label> <MB> <seconds> <MB/s>` lines in the existing format.
- `plain-compare.sh <results folder>`: three rounds, each round one typing test per installed target in turn; then three output runs per target; then `idle-frames.sh`; then `summary.txt` in the existing format (`<label> typing: samples=.. median=.. p95=.. over_30ms=..`, the same for backspace, per-run lines, the `missed=` lines, the output lines, and the idle line).
- `idle-frames.sh`: opens `plain-fifo` on `cat-shell.sh` with `TERM_PLAIN_STATS` pointing at a temp file, waits 2 seconds, reads `frames`, waits 5 more seconds, reads it again, closes. Prints `idle frames in 5 s: <difference>`.
- `plain-targets.sh <summary.txt> [plain label, default plain-fifo]` applies the spec's rules to the chosen plain label against the other installed terminals: typing median and backspace median no more than 1.0 ms above the lowest of the others; typing and backspace p95 no more than 2.0 ms above the lowest p95 of the others; the median of the three output runs at least kitty's; `idle frames in 5 s` at most 1 (one stray redraw asked for by the system is allowed, a steady trickle is not). It also prints `ratio to kitty: typing <x.xx> output <x.xx>`.
- `run-sitting.sh` carries any new variable in its job line the way the existing ones are carried.

- [ ] **Step 1: Write a fixture and the failing check for `plain-targets.sh`**

Create `tools/speed/configs/summary-fixture-pass.txt` and `summary-fixture-fail.txt` (hand-written summaries in the format above; in the fail file `plain-fifo typing: median` is 2.0 ms above kitty's and the idle line says 4).
Run: `zsh tools/speed/plain-targets.sh tools/speed/configs/summary-fixture-pass.txt; echo $?` and the same for the fail file.
Expected before the script exists: an error. After Step 2: exit 0 with only `PASS` lines; exit 1 with `FAIL` on the typing median line and the idle line.

- [ ] **Step 2: Write the scripts and the README section**

The README gains a section "Comparing the plain terminal with other terminals": what gets installed, the one command, how to read the summary, and that numbers from different sittings are never compared.

- [ ] **Step 3: Dry checks**

Run: `for f in tools/keylat/bin-window.sh tools/keylat/test-bin.sh tools/speed/throughput-bin.sh tools/speed/plain-compare.sh tools/speed/plain-targets.sh tools/speed/idle-frames.sh; do zsh -n $f || echo "syntax: $f"; done` and the two fixture runs from Step 1.
Expected: no `syntax:` lines; fixtures behave as in Step 1.

- [ ] **Step 4: Commit**

```bash
git add tools/keylat/bin-window.sh tools/keylat/test-bin.sh tools/speed/throughput-bin.sh tools/speed/plain-compare.sh tools/speed/plain-targets.sh tools/speed/idle-frames.sh tools/speed/configs tools/speed/run-sitting.sh tools/speed/README.md
git commit -m "test: time the plain terminal against kitty, Ghostty and Alacritty in one sitting"
```

---

### Task 11: the speed sitting and the stage gate

Run by the controller, with builders only for fixes. This task ends stage A.

**Files:**
- Create: `plain/README.md`
- Modify: `DESIGN.md` (the dated results under "What changed on 2026-11-11"), `docs/superpowers/specs/2026-11-11-native-renderer-design.md` (status line; the present mode decision under "Frame timing"), `plain/src/window.rs` (only if the default present mode changes)

**Interfaces:**
- Consumes: everything above.
- Produces: `tools/keylat/results/stage-a-<n>/summary.txt` and a PASS from `plain-targets.sh`.

- [ ] **Step 1: Install the two missing terminals**

`brew install --cask ghostty`, and Alacritty by `brew install --cask alacritty` or, if Homebrew no longer offers it, `cargo install alacritty --locked`. Open each once by hand-free command (`open -a`) to confirm it starts; a first-run system prompt that needs a click is left for Alex and that terminal is skipped in this sitting.

- [ ] **Step 2: Build and run the full local gate**

Run: `cargo build --release -p term-plain && cargo test -p term-engine && cargo test -p term-plain && cargo test -p term-engine --test replay -- --ignored --nocapture`
Expected: all PASS.

- [ ] **Step 3: First sitting**

Tell Alex "hands off the keyboard for about 12 minutes" by push notification and in chat, then run `TA_JOB=plain-compare.sh zsh tools/speed/run-sitting.sh stage-a-1`.
Expected: `summary.txt` with typing and backspace lines for `plain-fifo`, `plain-immediate` and every installed terminal, nine or more output lines, and the idle line. A round flagged `missed` above 0 or `workspace changed` is rerun, not used.

- [ ] **Step 4: Decide the present mode**

`plain-immediate` becomes the default only if its typing median is at least 0.5 ms below `plain-fifo`'s in this sitting; otherwise `fifo` stays, since it cannot tear. Record the two numbers and the decision in the spec under "Frame timing". If the default changes, change it in `window.rs`, rebuild, and use the new default's label from here on.

- [ ] **Step 5: Check the targets**

Run: `zsh tools/speed/plain-targets.sh tools/keylat/results/stage-a-1/summary.txt <default label>`
Expected: every line `PASS`. On a `FAIL`: dispatch a debugger (opus) with the summary and the failing rule to find where the time goes, then an implementer for the fix, then a new sitting `stage-a-2`, and so on. Stage A does not end on a `FAIL` unless Alex rules that it may.

- [ ] **Step 6: Write the docs**

`plain/README.md`: what the plain terminal is and is not, how to build and run it, its keys, and the one command that reruns the comparison. `DESIGN.md`: a table of this sitting's numbers and the `ratio to kitty` line, which later stages must not fall behind. Spec status line: stage A built.

- [ ] **Step 7: Commit**

```bash
git add plain/README.md DESIGN.md docs/superpowers/specs/2026-11-11-native-renderer-design.md tools/keylat/results/stage-a-1
git commit -m "docs: stage A speed sitting and how to run the plain terminal"
```

- [ ] **Step 8: Hand the plain terminal to Alex**

Alex runs `target/release/snowflake` and checks, in Claude Code: Shift+Enter makes a new line; a long answer scrolls and the wheel scrolls back; select, copy, paste; Cmd+click on a link; Cmd+= and Cmd+0; dragging the window edge; the look of text, boxes and emoji against today's app. Stage A is done when the targets pass and Alex says the hand check is fine.

---

## Decisions made in this plan for Alex to accept or change

- The two crates are named `term-engine` (folder `engine/`) and `term-plain` (folder `plain/`). The program they build is called `snowflake` and its window says Snowflake, after Alex's note that the fast terminal is Snowflake. The crate names are inside names only and are cheap to change later.
- The font is built into the engine (four font files, about 10 MB added to the repo and the program), so Mac, Windows and the tests all draw the same glyphs.
- The plain terminal keeps the system's normal title bar. The frameless window is stage B work.
- The plain terminal does not remember its zoom between runs, the cursor never blinks, and only `http` and `https` links open on Cmd+click. Links are not underlined when the pointer is over them yet.
- Task 8 opens a real window for four seconds, a few times, so the builder can see that drawing works before the speed sitting. It takes the keyboard while it is up; Alex is told first.
- Shift+Enter sends the same bytes as today's app (Escape then Return) unless the program has turned on the kitty keyboard protocol, in which case it sends that protocol's Shift+Enter.
- Selecting text copies it, as in today's app.
- The comparison installs Ghostty and Alacritty on this Mac.
- Memory: by arithmetic, 40 terminals that have each filled 20,000 lines at 120 columns hold about 2.3 GB of screen state. Task 1 measures the real figure. It does not affect stage A (one terminal, about 58 MB at most); what to do about it is a stage B decision.
- CI also runs the engine's windowless tests on Windows from the start, as a cheap guard for stage C.
