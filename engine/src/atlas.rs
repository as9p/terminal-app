//! Glyph bitmaps packed into one RGBA image, which the drawing side uploads as a texture.

use std::collections::HashMap;

use etagere::{AtlasAllocator, size2};

use crate::font::{FontStyle, Glyph, GlyphKind};
use crate::geometry::Rect;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub text: Box<str>,
    pub style: FontStyle,
    pub wide: bool,
}

/// Where a glyph sits in the image (`x`, `y`, `w`, `h`) and where it goes in its cell (`left`,
/// `top`, as in `Glyph`). A glyph with no ink, or too large for any image, has `w == 0, h == 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasSlot {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub left: i32,
    pub top: i32,
    pub color: bool,
}

const START_SIZE: u32 = 1024;
const MAX_SIZE: u32 = 4096;
/// Empty pixels kept to the right of and below each glyph, so sampling at one glyph's edge never
/// picks up its neighbour.
const GAP: u32 = 1;

/// One RGBA image; masks are stored as white with alpha, colour glyphs as their own RGBA.
pub struct Atlas {
    size: u32,
    packer: AtlasAllocator,
    slots: HashMap<GlyphKey, AtlasSlot>,
    pixels: Vec<u8>,
    generation: u64,
    dirty: Option<Rect>,
}

impl Default for Atlas {
    fn default() -> Atlas {
        Atlas::new()
    }
}

impl Atlas {
    /// Starts at 1024 by 1024, all transparent and with nothing dirty: a new texture of that
    /// size already matches it.
    pub fn new() -> Atlas {
        Atlas {
            size: START_SIZE,
            packer: AtlasAllocator::new(size2(START_SIZE as i32, START_SIZE as i32)),
            slots: HashMap::new(),
            pixels: vec![0; image_len(START_SIZE)],
            generation: 0,
            dirty: None,
        }
    }

    /// The slot for `key`, calling `make` only the first time. When the glyph does not fit, the
    /// image doubles its side up to 4096, and once full at 4096 it starts again empty. Either way
    /// `generation` changes and every earlier slot is gone.
    pub fn get_or_insert(&mut self, key: &GlyphKey, make: impl FnOnce() -> Glyph) -> AtlasSlot {
        if let Some(slot) = self.slots.get(key) {
            return *slot;
        }
        let slot = self.place(&make());
        self.slots.insert(key.clone(), slot);
        slot
    }

    /// Empties the image at its current size.
    pub fn clear(&mut self) {
        self.reset(self.size);
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Changes whenever earlier slots stop being valid: the image grew or was cleared.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// `size()` by `size()` RGBA pixels, row by row.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The region changed since the last call; the whole image after the generation changes.
    pub fn take_dirty(&mut self) -> Option<Rect> {
        self.dirty.take()
    }

    fn place(&mut self, glyph: &Glyph) -> AtlasSlot {
        let mut slot = AtlasSlot {
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            left: glyph.left,
            top: glyph.top,
            color: glyph.kind == GlyphKind::Color,
        };
        let (w, h) = (
            glyph.width.saturating_add(GAP),
            glyph.height.saturating_add(GAP),
        );
        if glyph.width == 0 || glyph.height == 0 || w > MAX_SIZE || h > MAX_SIZE {
            return slot;
        }
        let want = size2(w as i32, h as i32);
        let mut found = self.packer.allocate(want);
        while found.is_none() {
            let full = self.size >= MAX_SIZE;
            self.reset(if full { MAX_SIZE } else { self.size * 2 });
            found = self.packer.allocate(want);
            // An empty image at the largest size is the last chance; never loop past it.
            if full {
                break;
            }
        }
        let Some(found) = found else {
            return slot;
        };
        slot.x = found.rectangle.min.x as u32;
        slot.y = found.rectangle.min.y as u32;
        slot.w = glyph.width;
        slot.h = glyph.height;
        self.copy_in(glyph, &slot);
        let rect = Rect {
            x: slot.x,
            y: slot.y,
            w: slot.w,
            h: slot.h,
        };
        self.dirty = Some(self.dirty.map_or(rect, |d| union(d, rect)));
        slot
    }

    fn copy_in(&mut self, glyph: &Glyph, slot: &AtlasSlot) {
        for y in 0..slot.h {
            for x in 0..slot.w {
                let src = (y * glyph.width + x) as usize;
                let rgba = match glyph.kind {
                    GlyphKind::Mask => [255, 255, 255, glyph.pixels.get(src).copied().unwrap_or(0)],
                    GlyphKind::Color => match glyph.pixels.get(src * 4..src * 4 + 4) {
                        Some(p) => [p[0], p[1], p[2], p[3]],
                        None => [0; 4],
                    },
                };
                let dst = (((slot.y + y) * self.size + slot.x + x) * 4) as usize;
                self.pixels[dst..dst + 4].copy_from_slice(&rgba);
            }
        }
    }

    fn reset(&mut self, size: u32) {
        if size == self.size {
            self.packer.clear();
            self.pixels.fill(0);
        } else {
            self.size = size;
            self.packer = AtlasAllocator::new(size2(size as i32, size as i32));
            self.pixels = vec![0; image_len(size)];
        }
        self.slots.clear();
        self.generation += 1;
        self.dirty = Some(Rect {
            x: 0,
            y: 0,
            w: size,
            h: size,
        });
    }
}

fn image_len(size: u32) -> usize {
    size as usize * size as usize * 4
}

fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = (a.x + a.w).max(b.x + b.w);
    let bottom = (a.y + a.h).max(b.y + b.h);
    Rect {
        x,
        y,
        w: right - x,
        h: bottom - y,
    }
}
