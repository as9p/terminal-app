//! Fonts, glyph bitmaps, box characters and the glyph atlas, all on the CPU.

use std::sync::{Mutex, MutexGuard, OnceLock};

use term_engine::{
    Atlas, AtlasSlot, CellSize, FontStyle, Fonts, Glyph, GlyphKey, GlyphKind, Rect, box_glyph,
};

const REGULAR: FontStyle = FontStyle {
    bold: false,
    italic: false,
};
const BOLD: FontStyle = FontStyle {
    bold: true,
    italic: false,
};
const ITALIC: FontStyle = FontStyle {
    bold: false,
    italic: true,
};
const CELL: CellSize = CellSize { w: 17, h: 31 };

/// Loading the system fonts is slow, so every test in this binary shares one `Fonts`.
fn fonts() -> MutexGuard<'static, Fonts> {
    static FONTS: OnceLock<Mutex<Fonts>> = OnceLock::new();
    FONTS
        .get_or_init(|| Mutex::new(Fonts::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn ink(g: &Glyph) -> usize {
    match g.kind {
        GlyphKind::Mask => g.pixels.iter().filter(|&&a| a > 127).count(),
        GlyphKind::Color => g
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 127)
            .count(),
    }
}

fn glyph(text: &str, style: FontStyle, wide: bool) -> Glyph {
    fonts().rasterize(text, style, 14.0, 2.0, wide)
}

fn boxed(ch: char) -> Glyph {
    box_glyph(ch, CELL).expect("a box character")
}

fn at(g: &Glyph, x: u32, y: u32) -> u8 {
    g.pixels[(y * g.width + x) as usize]
}

fn ink_rows_in_column(g: &Glyph, x: u32) -> Vec<u32> {
    (0..g.height).filter(|&y| at(g, x, y) > 127).collect()
}

fn ink_columns_in_row(g: &Glyph, y: u32) -> Vec<u32> {
    (0..g.width).filter(|&x| at(g, x, y) > 127).collect()
}

/// A 20 by 40 mask whose values differ per pixel and per glyph, so a misplaced copy shows.
fn test_glyph(seed: usize) -> Glyph {
    let (w, h) = (20u32, 40u32);
    let pixels = (0..w * h)
        .map(|i| ((i as usize + seed) % 255 + 1) as u8)
        .collect();
    Glyph {
        width: w,
        height: h,
        left: 1,
        top: 2,
        kind: GlyphKind::Mask,
        pixels,
    }
}

fn key(i: usize) -> GlyphKey {
    GlyphKey {
        text: format!("k{i}").into(),
        style: REGULAR,
        wide: false,
    }
}

fn inside(slot: &AtlasSlot, size: u32) -> bool {
    slot.x + slot.w <= size && slot.y + slot.h <= size
}

#[test]
fn cell_size_follows_the_rule() {
    let mut f = fonts();
    assert_eq!(f.cell_size(14.0, 2.0), CellSize { w: 17, h: 40 });
    assert_eq!(f.cell_size(14.0, 1.0), CellSize { w: 8, h: 20 });
    assert_eq!(f.cell_size(15.0, 2.0).h, 44);
    // 25 px times 1.32 em is 33 pixels up to float noise, which must not round up to 34.
    assert_eq!(f.cell_size(25.0, 1.0).h, 36);
}

#[test]
fn plain_letter_fits_its_cell() {
    let g = glyph("A", REGULAR, false);
    assert_eq!(g.kind, GlyphKind::Mask);
    assert!(ink(&g) > 0);
    assert!(g.left >= 0);
    assert!(g.left + g.width as i32 <= 17);
    assert!(g.top >= 0);
    assert!(g.top + g.height as i32 <= 40);
}

#[test]
fn descenders_are_not_clipped() {
    let g = glyph("g", REGULAR, false);
    assert!(
        g.top + g.height as i32 <= 39,
        "g ends at {}",
        g.top + g.height as i32
    );
    let accented = glyph("\u{C1}", REGULAR, false);
    assert!(accented.top >= 1, "A acute starts at {}", accented.top);
}

#[test]
fn bold_has_more_ink() {
    assert!(ink(&glyph("A", BOLD, false)) > ink(&glyph("A", REGULAR, false)));
}

#[test]
fn italic_differs_from_regular() {
    assert_ne!(
        glyph("A", ITALIC, false).pixels,
        glyph("A", REGULAR, false).pixels
    );
}

#[cfg(target_os = "macos")]
#[test]
fn emoji_is_in_colour() {
    let g = glyph("\u{1F600}", REGULAR, true);
    assert_eq!(g.kind, GlyphKind::Color);
    assert!(g.pixels.as_chunks::<4>().0.iter().any(|p| p[0] != p[2]));
}

#[cfg(target_os = "macos")]
#[test]
fn han_character_falls_back_to_a_system_font() {
    let g = glyph("\u{6F22}", REGULAR, true);
    assert!(ink(&g) > 0);
    assert!(g.left + g.width as i32 <= 34);
}

#[test]
fn combining_mark_adds_ink_above() {
    assert!(glyph("e\u{301}", REGULAR, false).top < glyph("e", REGULAR, false).top);
}

#[test]
fn nerd_font_symbol_comes_from_the_built_in_face() {
    assert!(ink(&glyph("\u{E0B0}", REGULAR, false)) > 0);
}

#[test]
fn unknown_character_does_not_panic() {
    let g = glyph("\u{10FFFD}", REGULAR, false);
    assert!(g.left + g.width as i32 <= 17);
}

#[test]
fn horizontal_line_reaches_both_edges() {
    let g = boxed('\u{2500}');
    let full: Vec<u32> = (0..g.height)
        .filter(|&y| (0..g.width).all(|x| at(&g, x, y) == 255))
        .collect();
    assert_eq!(g.width, 17);
    assert!(
        full.iter().any(|&y| y.abs_diff(31 / 2) <= 1),
        "full rows {full:?}"
    );
}

#[test]
fn vertical_line_reaches_both_edges() {
    let g = boxed('\u{2502}');
    assert!((0..g.width).any(|x| (0..g.height).all(|y| at(&g, x, y) == 255)));
}

#[test]
fn corners_meet_the_lines() {
    let horizontal = boxed('\u{2500}');
    let vertical = boxed('\u{2502}');
    for ch in ['\u{250C}', '\u{256D}'] {
        let g = boxed(ch);
        let right = g.width - 1;
        let bottom = g.height - 1;
        assert_eq!(
            ink_rows_in_column(&g, right),
            ink_rows_in_column(&horizontal, right),
            "{ch}"
        );
        assert_eq!(
            ink_columns_in_row(&g, bottom),
            ink_columns_in_row(&vertical, bottom),
            "{ch}"
        );
        assert!(!ink_rows_in_column(&g, right).is_empty(), "{ch}");
    }
}

#[test]
fn full_and_half_blocks_are_exact() {
    assert!(boxed('\u{2588}').pixels.iter().all(|&a| a == 255));
    let upper = boxed('\u{2580}');
    for y in 0..31 {
        let want = if y < 15 { 255 } else { 0 };
        assert!((0..17).all(|x| at(&upper, x, y) == want), "row {y}");
    }
    let shade = boxed('\u{2591}');
    assert!(!shade.pixels.iter().all(|&a| a == 0));
    assert!(!shade.pixels.iter().all(|&a| a == 255));
}

#[test]
fn every_box_character_is_drawn() {
    for ch in '\u{2500}'..='\u{259F}' {
        let g = box_glyph(ch, CELL).unwrap_or_else(|| panic!("no glyph for {:X}", ch as u32));
        assert_eq!((g.width, g.height), (17, 31), "{:X}", ch as u32);
        assert_eq!(g.kind, GlyphKind::Mask);
    }
    assert!(box_glyph('A', CELL).is_none());
}

#[test]
fn same_key_same_slot() {
    let mut atlas = Atlas::new();
    let mut calls = 0;
    let a = atlas.get_or_insert(&key(1), || {
        calls += 1;
        test_glyph(1)
    });
    let b = atlas.get_or_insert(&key(1), || {
        calls += 1;
        test_glyph(1)
    });
    assert_eq!(a, b);
    assert_eq!(calls, 1);
}

#[test]
fn slots_do_not_overlap() {
    let mut atlas = Atlas::new();
    let slots: Vec<AtlasSlot> = (0..200)
        .map(|i| atlas.get_or_insert(&key(i), || test_glyph(i)))
        .collect();
    let size = atlas.size();
    for (i, a) in slots.iter().enumerate() {
        assert!(inside(a, size), "{a:?}");
        assert_eq!((a.w, a.h), (20, 40));
        for b in &slots[i + 1..] {
            let apart =
                a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y;
            assert!(apart, "{a:?} overlaps {b:?}");
        }
    }
}

#[test]
fn a_full_atlas_grows_then_clears() {
    let mut atlas = Atlas::new();
    let mut changes = 0;
    let mut last = atlas.generation();
    let mut slot = None;
    for i in 0..30_000 {
        slot = Some(atlas.get_or_insert(&key(i), || test_glyph(i)));
        assert!(atlas.size() <= 4096);
        if atlas.generation() != last {
            changes += 1;
            last = atlas.generation();
        }
    }
    assert!(changes >= 2, "generation changed {changes} times");
    let slot = slot.unwrap();
    let size = atlas.size();
    assert!(inside(&slot, size));
    let want = test_glyph(29_999);
    let pixels = atlas.pixels();
    assert_eq!(pixels.len(), (size * size * 4) as usize);
    for y in 0..slot.h {
        for x in 0..slot.w {
            let at = (((slot.y + y) * size + slot.x + x) * 4) as usize;
            let alpha = want.pixels[(y * want.width + x) as usize];
            assert_eq!(
                &pixels[at..at + 4],
                &[255, 255, 255, alpha],
                "pixel {x},{y}"
            );
        }
    }
}

#[test]
fn dirty_region_covers_new_glyphs() {
    let mut atlas = Atlas::new();
    let slot = atlas.get_or_insert(&key(7), || test_glyph(7));
    let Rect { x, y, w, h } = atlas.take_dirty().expect("a dirty region");
    assert!(x <= slot.x && y <= slot.y);
    assert!(slot.x + slot.w <= x + w && slot.y + slot.h <= y + h);
    assert_eq!(atlas.take_dirty(), None);
}

#[test]
fn baseline_is_where_a_capital_letter_ends() {
    let mut fonts = fonts();
    for (scale, want) in [(2.0, 30), (1.0, 15)] {
        let h = fonts.rasterize("H", REGULAR, 14.0, scale, false);
        assert_eq!(fonts.baseline(14.0, scale), want, "scale {scale}");
        // Underlines hang from this row, so it has to be the row the letters stand on.
        assert_eq!((h.top + h.height as i32) as u32, want, "scale {scale}");
    }
}
