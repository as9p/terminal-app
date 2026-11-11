//! The terminal engine shared by every view: screen state now, input and drawing later.

mod geometry;
mod keys;
mod mouse;
mod screen;
mod theme;

pub use geometry::{CellPoint, CellSize, GridSize, Rect, Side, cell_at, fit};
pub use keys::{KeyCode, KeyEventKind, KeyInput, Mods, Platform, encode_key};
pub use mouse::{Button, MouseAction, encode_mouse};
pub use screen::{
    Attrs, Cell, CellWidth, Cursor, CursorShape, Damage, Modes, MouseTracking, SCROLLBACK_LINES,
    Screen, ScreenEvent, SelectKind,
};
pub use theme::{FONT_PX_DEFAULT, FONT_PX_MAX, FONT_PX_MIN, Palette, Rgb, Theme};
