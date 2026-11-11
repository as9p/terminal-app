//! The built-in font, the cell size it gives, and one cell's text turned into a bitmap.

use std::sync::Arc;

use cosmic_text::fontdb::{Query, Source};
use cosmic_text::{
    Attrs, AttrsList, CacheKey, Family, FontSystem, ShapeLine, Shaping, Stretch, Style, SwashCache,
    SwashContent, SwashImage, Weight, Wrap,
};

use crate::boxdraw::box_glyph;
use crate::geometry::CellSize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontStyle {
    pub bold: bool,
    pub italic: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphKind {
    /// One coverage byte a pixel.
    Mask,
    /// Four bytes a pixel, RGBA with straight (not premultiplied) alpha.
    Color,
}

/// left and top place the bitmap relative to the top left corner of its cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub width: u32,
    pub height: u32,
    pub left: i32,
    pub top: i32,
    pub kind: GlyphKind,
    pub pixels: Vec<u8>,
}

const FAMILY: &str = "JetBrainsMono Nerd Font Mono";
const FACES: [&[u8]; 4] = [
    include_bytes!("../fonts/JetBrainsMonoNerdFontMono-Regular.ttf"),
    include_bytes!("../fonts/JetBrainsMonoNerdFontMono-Bold.ttf"),
    include_bytes!("../fonts/JetBrainsMonoNerdFontMono-Italic.ttf"),
    include_bytes!("../fonts/JetBrainsMonoNerdFontMono-BoldItalic.ttf"),
];
/// Today's app sets a line height of 1.1, which multiplies the font's own line height.
const LINE_HEIGHT_TENTHS: u32 = 11;
/// Room for float noise, so a font line height that is a whole number of pixels (25 px times
/// 1.32 em is 33.000001) does not round up to the next pixel.
const NOISE_PX: f64 = 1e-3;
/// Pixel sizes are kept in this range so a wild zoom or scale factor cannot ask for a bitmap
/// too large to allocate.
const MIN_PX: f64 = 1.0;
const MAX_PX: f64 = 512.0;

/// The regular face's metrics as fractions of the font size.
#[derive(Clone, Copy)]
struct Metrics {
    advance: f32,
    ascent: f32,
    descent: f32,
    /// Ascent plus descent plus line gap, worked out from the font units in double precision.
    line: f64,
}

/// JetBrains Mono's own values, used only if the built-in face somehow cannot be read.
const FALLBACK_METRICS: Metrics = Metrics {
    advance: 0.6,
    ascent: 1.02,
    descent: 0.3,
    line: 1.32,
};

pub struct Fonts {
    system: FontSystem,
    swash: SwashCache,
    metrics: Metrics,
}

impl Default for Fonts {
    fn default() -> Fonts {
        Fonts::new()
    }
}

impl Fonts {
    /// The four built-in faces, with the system fonts as fallback.
    pub fn new() -> Fonts {
        let faces = FACES.iter().map(|&bytes| Source::Binary(Arc::new(bytes)));
        let mut system = FontSystem::new_with_fonts(faces);
        // An installed copy of the same family could be a different version and would win ties
        // on lower face ids, so the built-in faces are the only ones left with that name.
        let installed: Vec<_> = system
            .db()
            .faces()
            .filter(|f| !matches!(f.source, Source::Binary(_)))
            .filter(|f| f.families.iter().any(|(name, _)| name == FAMILY))
            .map(|f| f.id)
            .collect();
        for id in installed {
            system.db_mut().remove_face(id);
        }
        let metrics = regular_metrics(&mut system).unwrap_or(FALLBACK_METRICS);
        Fonts {
            system,
            swash: SwashCache::new(),
            metrics,
        }
    }

    pub fn cell_size(&mut self, font_px: f32, scale: f64) -> CellSize {
        let px = physical_px(font_px, scale);
        let natural = (px * self.metrics.line - NOISE_PX).ceil().max(0.0) as u32;
        CellSize {
            w: ((px * f64::from(self.metrics.advance)).round() as u32).max(1),
            h: (natural.max(1) * LINE_HEIGHT_TENTHS / 10).max(1),
        }
    }

    /// Pixels from the top of a cell to the text baseline. Ascent and descent are centred in the
    /// row, so the extra space is shared evenly above and below.
    pub fn baseline(&mut self, font_px: f32, scale: f64) -> u32 {
        let cell = self.cell_size(font_px, scale);
        let px = physical_px(font_px, scale) as f32;
        let ascent = px * self.metrics.ascent;
        let descent = px * self.metrics.descent;
        ((cell.h as f32 - (ascent + descent)) / 2.0 + ascent)
            .round()
            .max(0.0) as u32
    }

    /// `text` is one cell's base character plus its zero-width characters. `wide` cells are two
    /// cells across. The bitmap is cropped to its ink and never reaches outside the cell box;
    /// text with no ink at all (a space, an unknown control) gives a zero-size mask.
    pub fn rasterize(
        &mut self,
        text: &str,
        style: FontStyle,
        font_px: f32,
        scale: f64,
        wide: bool,
    ) -> Glyph {
        let cell = self.cell_size(font_px, scale);
        let mut chars = text.chars();
        if let (Some(ch), None) = (chars.next(), chars.next())
            && let Some(glyph) = box_glyph(ch, cell)
        {
            return glyph;
        }
        // A line break would start a second paragraph, which the shaper does not accept.
        let text: String = text
            .chars()
            .filter(|c| !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}'))
            .collect();
        if text.is_empty() {
            return empty();
        }

        let px = physical_px(font_px, scale) as f32;
        let attrs = Attrs::new()
            .family(Family::Name(FAMILY))
            .weight(if style.bold {
                Weight::BOLD
            } else {
                Weight::NORMAL
            })
            .style(if style.italic {
                Style::Italic
            } else {
                Style::Normal
            });
        let line = ShapeLine::new(
            &mut self.system,
            &text,
            &AttrsList::new(attrs),
            Shaping::Advanced,
            8,
        );
        let layout = line.layout(px, None, Wrap::None, None, None);

        let box_w = if wide { cell.w * 2 } else { cell.w };
        let baseline = self.baseline(font_px, scale) as i32;
        // A fallback glyph narrower than its box (a Han character in a wide cell) sits centred.
        let advance = layout.iter().map(|l| l.w).fold(0.0, f32::max);
        let shift = ((box_w as f32 - advance) / 2.0).max(0.0).round() as i32;

        let mut images = Vec::new();
        for g in layout.iter().flat_map(|l| l.glyphs.iter()) {
            // Whole pixels only, so one glyph always gives one bitmap.
            let x = (g.x + g.font_size * g.x_offset).round() as i32;
            let y = (g.y - g.font_size * g.y_offset).round() as i32;
            let (key, _, _) = CacheKey::new(
                g.font_id,
                g.glyph_id,
                g.font_size,
                (0.0, 0.0),
                g.cache_key_flags,
            );
            if let Some(image) = self.swash.get_image_uncached(&mut self.system, key) {
                let left = shift + x + image.placement.left;
                let top = baseline + y - image.placement.top;
                images.push((left, top, image));
            }
        }
        compose(&images, box_w, cell.h)
    }
}

fn physical_px(font_px: f32, scale: f64) -> f64 {
    let px = f64::from(font_px) * scale;
    if px.is_nan() {
        MIN_PX
    } else {
        px.clamp(MIN_PX, MAX_PX)
    }
}

fn regular_metrics(system: &mut FontSystem) -> Option<Metrics> {
    let id = system.db().query(&Query {
        families: &[Family::Name(FAMILY)],
        weight: Weight::NORMAL,
        stretch: Stretch::Normal,
        style: Style::Normal,
    })?;
    let font = system.get_font(id)?;
    let face = font.as_swash();
    let m = face.metrics(&[]);
    let em = f32::from(m.units_per_em.max(1));
    let advance = face
        .glyph_metrics(&[])
        .advance_width(face.charmap().map('0'));
    Some(Metrics {
        advance: advance / em,
        ascent: m.ascent / em,
        descent: m.descent / em,
        line: f64::from(m.ascent + m.descent + m.leading) / f64::from(em),
    })
}

fn empty() -> Glyph {
    Glyph {
        width: 0,
        height: 0,
        left: 0,
        top: 0,
        kind: GlyphKind::Mask,
        pixels: Vec::new(),
    }
}

/// Draws the placed images into a `w` by `h` box, clipping anything outside it, and crops the
/// result to its ink. One colour image makes the whole cell colour, with any mask drawn white.
fn compose(images: &[(i32, i32, SwashImage)], w: u32, h: u32) -> Glyph {
    let color = images
        .iter()
        .any(|(_, _, im)| im.content == SwashContent::Color);
    let bpp = if color { 4 } else { 1 };
    let mut canvas = vec![0u8; w as usize * h as usize * bpp];
    for (left, top, im) in images {
        let p = im.placement;
        for iy in 0..p.height {
            let cy = top + iy as i32;
            if cy < 0 || cy >= h as i32 {
                continue;
            }
            for ix in 0..p.width {
                let cx = left + ix as i32;
                if cx < 0 || cx >= w as i32 {
                    continue;
                }
                let src = (iy * p.width + ix) as usize;
                let dst = cy as usize * w as usize + cx as usize;
                match im.content {
                    SwashContent::Color => {
                        if let Some(rgba) = im.data.get(src * 4..src * 4 + 4) {
                            over(&mut canvas[dst * 4..dst * 4 + 4], rgba);
                        }
                    }
                    SwashContent::Mask => {
                        let a = im.data.get(src).copied().unwrap_or(0);
                        if color {
                            over(&mut canvas[dst * 4..dst * 4 + 4], &[255, 255, 255, a]);
                        } else {
                            canvas[dst] = canvas[dst].max(a);
                        }
                    }
                    // Rendering asks for alpha masks, so subpixel masks never arrive.
                    SwashContent::SubpixelMask => {}
                }
            }
        }
    }
    crop(&canvas, w, h, color)
}

/// Source over destination, both with straight alpha.
fn over(dst: &mut [u8], src: &[u8]) {
    let sa = u32::from(src[3]);
    if sa == 0 {
        return;
    }
    let da = u32::from(dst[3]) * (255 - sa) / 255;
    let out = sa + da;
    for i in 0..3 {
        dst[i] = ((u32::from(src[i]) * sa + u32::from(dst[i]) * da) / out) as u8;
    }
    dst[3] = out as u8;
}

fn crop(canvas: &[u8], w: u32, h: u32, color: bool) -> Glyph {
    let bpp = if color { 4 } else { 1 };
    let alpha = |x: u32, y: u32| canvas[(y * w + x) as usize * bpp + bpp - 1];
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if alpha(x, y) > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 >= x1 {
        return empty();
    }
    let mut pixels = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize * bpp);
    for y in y0..y1 {
        let row = (y * w) as usize * bpp;
        pixels.extend_from_slice(&canvas[row + x0 as usize * bpp..row + x1 as usize * bpp]);
    }
    Glyph {
        width: x1 - x0,
        height: y1 - y0,
        left: x0 as i32,
        top: y0 as i32,
        kind: if color {
            GlyphKind::Color
        } else {
            GlyphKind::Mask
        },
        pixels,
    }
}
