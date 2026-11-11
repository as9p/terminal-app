//! Colours and text size, copied from `client/src/theme.ts` so both apps look the same.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Debug)]
pub struct Palette {
    pub fg: Rgb,
    pub bg: Rgb,
    pub cursor: Rgb,
    pub cursor_text: Rgb,
    pub selection_bg: Rgb,
    pub selection_fg: Rgb,
    /// Colours 0 to 15. Bold text does not switch to the bright half.
    pub ansi: [Rgb; 16],
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub palette: Palette,
}

pub const FONT_PX_DEFAULT: f32 = 14.0;
pub const FONT_PX_MIN: f32 = 8.0;
pub const FONT_PX_MAX: f32 = 40.0;

const fn hex(v: u32) -> Rgb {
    Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

impl Theme {
    pub fn builtin() -> Theme {
        Theme {
            palette: Palette {
                fg: hex(0xCDD6F4),
                bg: hex(0x16141F),
                cursor: hex(0xF5E0DC),
                cursor_text: hex(0x1E1E2E),
                selection_bg: hex(0xF5E0DC),
                selection_fg: hex(0x1E1E2E),
                ansi: [
                    hex(0x45475A),
                    hex(0xF38BA8),
                    hex(0xA6E3A1),
                    hex(0xF9E2AF),
                    hex(0x89B4FA),
                    hex(0xF5C2E7),
                    hex(0x94E2D5),
                    hex(0xBAC2DE),
                    hex(0x585B70),
                    hex(0xF38BA8),
                    hex(0xA6E3A1),
                    hex(0xF9E2AF),
                    hex(0x89B4FA),
                    hex(0xF5C2E7),
                    hex(0x94E2D5),
                    hex(0xA6ADC8),
                ],
            },
        }
    }
}
