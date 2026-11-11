//! One terminal as the user meets it: the screen plus what keys, the mouse, paste and focus do.
//! Bytes for the shell and events for the window are queued here and taken by the owner.

use std::mem;
use std::time::Instant;

use crate::geometry::{CellPoint, GridSize, Side};
use crate::keys::{KeyCode, KeyEventKind, KeyInput, Mods, Platform, encode_key};
use crate::links::{is_web, url_at};
use crate::mouse::{Button, MouseAction, encode_mouse};
use crate::screen::{Modes, MouseTracking, Screen, ScreenEvent, SelectKind};
use crate::theme::Theme;

/// A wild wheel count must not queue unbounded arrow keys; this matches the mouse report cap.
const MAX_WHEEL_KEYS: u32 = 100;

const PASTE_START: &str = "\x1b[200~";
const PASTE_END: &str = "\x1b[201~";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewEvent {
    Title(String),
    Bell,
    Copy(String),
    OpenLink(String),
}

pub struct TerminalView {
    screen: Screen,
    theme: Theme,
    platform: Platform,
    output: Vec<u8>,
    events: Vec<ViewEvent>,
    focused: bool,
    /// A left press of ours started a selection, so moves extend it and the release copies it.
    selecting: bool,
    /// Output arrived since the last draw. It only counts once no synchronized update holds it.
    fed: bool,
    /// The selection, scroll position, focus or grid changed since the last draw.
    changed: bool,
}

impl TerminalView {
    pub fn new(grid: GridSize, theme: &Theme, platform: Platform) -> TerminalView {
        TerminalView {
            screen: Screen::new(grid, &theme.palette),
            theme: theme.clone(),
            platform,
            output: Vec::new(),
            events: Vec::new(),
            focused: true,
            selecting: false,
            fed: false,
            // Nothing has been drawn yet.
            changed: true,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.screen.feed(bytes);
        self.fed = true;
        self.collect();
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        mem::take(&mut self.output)
    }

    pub fn take_events(&mut self) -> Vec<ViewEvent> {
        mem::take(&mut self.events)
    }

    pub fn key(&mut self, key: &KeyInput) {
        if self.platform == Platform::Mac
            && key.kind != KeyEventKind::Release
            && key.mods == Mods::SUPER
            && self.mac_key(key.code)
        {
            return;
        }
        let modes = self.screen.modes();
        if let Some(bytes) = encode_key(key, &modes, self.platform).filter(|b| !b.is_empty()) {
            self.scroll_view(Screen::scroll_to_bottom);
            self.output.extend_from_slice(&bytes);
        }
    }

    pub fn pointer(
        &mut self,
        action: MouseAction,
        at: CellPoint,
        side: Side,
        mods: Mods,
        clicks: u8,
    ) {
        let modes = self.screen.modes();
        // Shift is the way to select text while a program has the mouse.
        if modes.mouse != MouseTracking::Off && !mods.contains(Mods::SHIFT) {
            // The program has the pointer now, so a drag of ours that lost Shift is over and a
            // later move must not extend it.
            self.selecting = false;
            if let Some(bytes) = encode_mouse(action, at, mods, &modes) {
                self.output.extend_from_slice(&bytes);
            }
            return;
        }
        match action {
            MouseAction::Press(Button::Left) => {
                if mods.contains(self.link_modifier())
                    && let Some(url) = self.link_at(at)
                {
                    self.events.push(ViewEvent::OpenLink(url));
                    self.selecting = false;
                    return;
                }
                let kind = match clicks {
                    0 | 1 => SelectKind::Char,
                    2 => SelectKind::Word,
                    _ => SelectKind::Line,
                };
                self.screen.select(kind, at, side);
                self.selecting = true;
                self.changed = true;
            }
            MouseAction::Move(Some(Button::Left)) if self.selecting => {
                self.screen.select_to(at, side);
                self.changed = true;
            }
            MouseAction::Release(Button::Left) if self.selecting => {
                self.selecting = false;
                // Selecting copies in today's app; a click that selected nothing clears the old
                // selection instead.
                match self.copy() {
                    Some(text) => self.events.push(ViewEvent::Copy(text)),
                    None => {
                        self.screen.clear_selection();
                        self.changed = true;
                    }
                }
            }
            MouseAction::Wheel(lines) => self.wheel(lines, &modes),
            _ => {}
        }
    }

    pub fn paste(&mut self, text: &str) {
        self.scroll_view(Screen::scroll_to_bottom);
        let mut text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.screen.modes().bracketed_paste {
            // Removing one marker can join the pieces around it into another, so repeat until
            // none is left; otherwise pasted text could end the paste and run as typed commands.
            while text.contains(PASTE_END) {
                text = text.replace(PASTE_END, "");
            }
            self.output.extend_from_slice(PASTE_START.as_bytes());
            self.output.extend_from_slice(text.as_bytes());
            self.output.extend_from_slice(PASTE_END.as_bytes());
        } else {
            self.output.extend_from_slice(text.as_bytes());
        }
    }

    /// The selected text, or `None` when nothing or only empty space is selected.
    pub fn copy(&self) -> Option<String> {
        self.screen.selection_text().filter(|text| !text.is_empty())
    }

    pub fn set_focused(&mut self, focused: bool) {
        if focused == self.focused {
            return;
        }
        self.focused = focused;
        self.changed = true;
        if self.screen.modes().focus_events {
            self.output
                .extend_from_slice(if focused { b"\x1b[I" } else { b"\x1b[O" });
        }
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    /// What the screen holds, which can differ from what was asked for: it has a minimum size.
    pub fn grid(&self) -> GridSize {
        self.screen.size()
    }

    pub fn set_grid(&mut self, grid: GridSize) {
        let before = self.screen.size();
        self.screen.resize(grid);
        if self.screen.size() != before {
            self.changed = true;
        }
    }

    /// Ends a synchronized update the program left open too long. Returns when to call again.
    pub fn tick(&mut self, now: Instant) -> Option<Instant> {
        if self.screen.end_sync_if_due(now) {
            // Forced, because the held output may have opened another update straight away.
            self.changed = true;
            self.collect();
        }
        self.screen.sync_deadline()
    }

    pub fn needs_draw(&self) -> bool {
        self.changed || (self.fed && self.screen.sync_deadline().is_none())
    }

    pub fn mark_drawn(&mut self) {
        self.changed = false;
        self.fed = false;
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn screen_mut(&mut self) -> &mut Screen {
        &mut self.screen
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The Cmd keys the view handles itself, as `cmdKey` in `client/src/terminal.ts` does.
    /// Returns false for a key that is not one of them.
    fn mac_key(&mut self, code: KeyCode) -> bool {
        // A full-screen program owns the screen, so there is no scrollback to move or clear.
        let alternate = self.screen.modes().alt_screen;
        // xterm.js moves a page by one row less than the screen, keeping a line of context.
        let page = i32::from(self.screen.size().rows.saturating_sub(1).max(1));
        match code {
            KeyCode::Char('k') => {
                if !alternate {
                    self.screen.clear_history();
                    self.changed = true;
                }
                // Asks the shell to redraw its prompt at the top of the cleared screen.
                self.output.push(0x0c);
            }
            KeyCode::Char('a') => {
                self.screen.select_all();
                self.changed = true;
            }
            KeyCode::Up if !alternate => self.scroll_view(|s| s.scroll_by(page)),
            KeyCode::Down if !alternate => self.scroll_view(|s| s.scroll_by(-page)),
            KeyCode::Home if !alternate => self.scroll_view(Screen::scroll_to_top),
            KeyCode::End if !alternate => self.scroll_view(Screen::scroll_to_bottom),
            KeyCode::Up | KeyCode::Down | KeyCode::Home | KeyCode::End => {}
            _ => return false,
        }
        true
    }

    fn wheel(&mut self, lines: i32, modes: &Modes) {
        if lines == 0 {
            return;
        }
        if modes.alt_screen && modes.alternate_scroll {
            let code = if lines > 0 {
                KeyCode::Up
            } else {
                KeyCode::Down
            };
            let arrow = KeyInput {
                code,
                mods: Mods::empty(),
                text: None,
                kind: KeyEventKind::Press,
            };
            // Through the key encoder, so a program in application cursor mode gets its form.
            if let Some(bytes) = encode_key(&arrow, modes, self.platform) {
                let count = lines.unsigned_abs().min(MAX_WHEEL_KEYS) as usize;
                self.output.extend_from_slice(&bytes.repeat(count));
            }
        } else {
            // The library adds the count to its offset as an i32, so a wild count is cut to
            // what could move the view at all before it can overflow.
            let limit = i32::try_from(self.screen.history_len()).unwrap_or(i32::MAX);
            let lines = lines.clamp(-limit, limit);
            self.scroll_view(|s| s.scroll_by(lines));
        }
    }

    fn scroll_view(&mut self, scroll: impl FnOnce(&mut Screen)) {
        let before = self.screen.scroll_offset();
        scroll(&mut self.screen);
        if self.screen.scroll_offset() != before {
            self.changed = true;
        }
    }

    fn link_modifier(&self) -> Mods {
        match self.platform {
            Platform::Mac => Mods::SUPER,
            Platform::Other => Mods::CTRL,
        }
    }

    /// A link the program marked wins over a URL found in the text, and only web links count.
    fn link_at(&self, at: CellPoint) -> Option<String> {
        self.screen
            .link_at(at)
            .or_else(|| {
                let (text, index) = self.screen.text_around(at);
                url_at(&text, index)
            })
            .filter(|url| is_web(url))
    }

    /// Moves the screen's replies and events into the view's queues.
    fn collect(&mut self) {
        self.output.extend(self.screen.take_replies());
        self.events.extend(
            self.screen
                .take_events()
                .into_iter()
                .map(|event| match event {
                    ScreenEvent::Title(title) => ViewEvent::Title(title),
                    ScreenEvent::Bell => ViewEvent::Bell,
                    ScreenEvent::ClipboardStore(text) => ViewEvent::Copy(text),
                }),
        );
    }
}
