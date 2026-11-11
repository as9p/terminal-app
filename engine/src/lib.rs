//! The terminal engine shared by every view: screen state now, input and drawing later.

mod atlas;
mod boxdraw;
mod font;
mod geometry;
mod keys;
mod links;
mod mouse;
mod screen;
mod theme;
mod view;

pub use atlas::{Atlas, AtlasSlot, GlyphKey};
pub use boxdraw::box_glyph;
pub use font::{FontStyle, Fonts, Glyph, GlyphKind};
pub use geometry::{CellPoint, CellSize, GridSize, Rect, Side, cell_at, fit};
pub use keys::{KeyCode, KeyEventKind, KeyInput, Mods, Platform, encode_key};
pub use links::url_at;
pub use mouse::{Button, MouseAction, encode_mouse};
pub use screen::{
    Attrs, Cell, CellWidth, Cursor, CursorShape, Damage, Modes, MouseTracking, SCROLLBACK_LINES,
    Screen, ScreenEvent, SelectKind,
};
pub use theme::{FONT_PX_DEFAULT, FONT_PX_MAX, FONT_PX_MIN, Palette, Rgb, Theme};
pub use view::{TerminalView, ViewEvent};
