//! Drawing a view's cell grid on the GPU into an offscreen image.

use std::sync::{Mutex, MutexGuard, OnceLock};

use term_engine::{
    CellSize, Gpu, GridSize, Platform, Rect, Rgb, TerminalView, Theme, ViewDraw, render_to_image,
};

const MAGENTA: Rgb = Rgb(255, 0, 255);
const BG: Rgb = Rgb(0x16, 0x14, 0x1F);
const FG: Rgb = Rgb(0xCD, 0xD6, 0xF4);
const RED: Rgb = Rgb(0xF3, 0x8B, 0xA8);
const CELL_W: u32 = 17;
const CELL_H: u32 = 40;
const FULL: Rect = Rect {
    x: 0,
    y: 0,
    w: 340,
    h: 160,
};

/// Creating a device and loading the system fonts is slow, so every test in this binary shares
/// one `Gpu`, and holding the lock keeps the tests from changing its text size under each other.
/// Returns `None`, after saying so, when `TERM_ENGINE_NO_GPU=1` asks to skip drawing tests.
fn gpu() -> Option<MutexGuard<'static, Gpu>> {
    if std::env::var_os("TERM_ENGINE_NO_GPU").is_some_and(|v| v == "1") {
        eprintln!("TERM_ENGINE_NO_GPU=1: skipping a drawing test, no GPU adapter is expected");
        return None;
    }
    static GPU: OnceLock<Mutex<Gpu>> = OnceLock::new();
    let mut gpu = GPU
        .get_or_init(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
            let gpu = pollster::block_on(Gpu::new(
                &instance,
                None,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            ))
            .expect("no GPU adapter; set TERM_ENGINE_NO_GPU=1 to skip the drawing tests");
            Mutex::new(gpu)
        })
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    gpu.set_text_size(14.0, 2.0);
    Some(gpu)
}

fn view() -> TerminalView {
    TerminalView::new(
        GridSize { cols: 20, rows: 4 },
        &Theme::builtin(),
        Platform::Mac,
    )
}

struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn render(
    gpu: &mut Gpu,
    draw: &mut ViewDraw,
    view: &mut TerminalView,
    rect: Rect,
    width: u32,
    height: u32,
) -> Image {
    let rgba = render_to_image(gpu, draw, view, rect, width, height, MAGENTA);
    assert_eq!(rgba.len(), (width * height * 4) as usize);
    Image {
        width,
        height,
        rgba,
    }
}

fn render_full(gpu: &mut Gpu, draw: &mut ViewDraw, view: &mut TerminalView) -> Image {
    render(gpu, draw, view, FULL, FULL.w, FULL.h)
}

fn px(img: &Image, x: u32, y: u32) -> Rgb {
    assert!(
        x < img.width && y < img.height,
        "({x}, {y}) is outside the image"
    );
    let i = ((y * img.width + x) * 4) as usize;
    Rgb(img.rgba[i], img.rgba[i + 1], img.rgba[i + 2])
}

/// Every pixel of cell (`col`, `row`) at the test's cell size.
fn cell_pixels(img: &Image, col: u32, row: u32) -> Vec<Rgb> {
    let mut out = Vec::new();
    for y in row * CELL_H..(row + 1) * CELL_H {
        for x in col * CELL_W..(col + 1) * CELL_W {
            out.push(px(img, x, y));
        }
    }
    out
}

#[test]
fn default_background_is_exact() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    let mut draw = ViewDraw::new(&gpu);
    let img = render_full(&mut gpu, &mut draw, &mut view);
    assert_eq!(px(&img, 5, 5), BG);
}

#[test]
fn coloured_background_is_exact() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    view.feed(b"\x1b[41m  \x1b[0m");
    let mut draw = ViewDraw::new(&gpu);
    let img = render_full(&mut gpu, &mut draw, &mut view);
    assert_eq!(px(&img, 8, 15), RED);
    assert_eq!(px(&img, 8 + 2 * CELL_W, 15), BG);
}

#[test]
fn text_is_drawn_in_its_colour() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    view.feed("\x1b[38;2;255;255;255m\u{2588}".as_bytes());
    let mut draw = ViewDraw::new(&gpu);
    let img = render_full(&mut gpu, &mut draw, &mut view);
    assert_eq!(px(&img, 8, 15), Rgb(255, 255, 255));
}

#[test]
fn letters_leave_ink() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    view.feed(b"A");
    let mut draw = ViewDraw::new(&gpu);
    let img = render_full(&mut gpu, &mut draw, &mut view);
    assert!(cell_pixels(&img, 0, 0).iter().any(|&p| p != BG));
    assert!(cell_pixels(&img, 5, 0).iter().all(|&p| p == BG));
}

#[test]
fn box_lines_join_across_cells() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    view.feed("\u{2500}\u{2500}\u{2500}".as_bytes());
    let mut draw = ViewDraw::new(&gpu);
    let img = render_full(&mut gpu, &mut draw, &mut view);
    let joined = (19..=21).any(|y| (0..=50).all(|x| px(&img, x, y) == FG));
    assert!(joined, "no row between 19 and 21 is one unbroken line");
}

#[test]
fn grid_smaller_than_rect_is_padded() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    let mut draw = ViewDraw::new(&gpu);
    let rect = Rect {
        x: 0,
        y: 0,
        w: 400,
        h: 200,
    };
    let img = render(&mut gpu, &mut draw, &mut view, rect, 400, 200);
    assert_eq!(px(&img, 390, 190), BG);
}

#[test]
fn grid_larger_than_rect_is_clipped() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    let mut bytes = b"\x1b[41m".to_vec();
    bytes.extend([b' '; 80]);
    view.feed(&bytes);
    let mut draw = ViewDraw::new(&gpu);
    let rect = Rect {
        x: 10,
        y: 10,
        w: 100,
        h: 50,
    };
    let img = render(&mut gpu, &mut draw, &mut view, rect, 340, 160);
    assert_eq!(px(&img, 5, 5), MAGENTA);
    assert_eq!(px(&img, 111, 30), MAGENTA);
    assert_eq!(px(&img, 30, 61), MAGENTA);
    assert_eq!(px(&img, 20, 20), RED);
}

#[test]
fn zero_size_rect_draws_nothing() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    view.feed(b"\x1b[41mhello");
    let mut draw = ViewDraw::new(&gpu);
    let rect = Rect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };
    let img = render(&mut gpu, &mut draw, &mut view, rect, 340, 160);
    assert!(img.rgba.chunks(4).all(|p| p[..3] == [255, 0, 255]));
}

#[test]
fn scale_change_redraws_glyphs() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    view.feed(b"A\x1b[3;1H");
    let mut draw = ViewDraw::new(&gpu);
    render_full(&mut gpu, &mut draw, &mut view);
    gpu.set_text_size(14.0, 1.0);
    let rect = Rect {
        x: 0,
        y: 0,
        w: 160,
        h: 80,
    };
    let img = render(&mut gpu, &mut draw, &mut view, rect, 160, 80);
    assert_eq!(gpu.cell_size(), CellSize { w: 8, h: 20 });
    let mut first = Vec::new();
    let mut rest = Vec::new();
    for y in 0..20 {
        for x in 0..img.width {
            if x < 8 {
                first.push(px(&img, x, y))
            } else {
                rest.push(px(&img, x, y))
            }
        }
    }
    assert!(
        first.iter().any(|&p| p != BG),
        "no ink in the first cell at scale 1"
    );
    assert!(
        rest.iter().all(|&p| p == BG),
        "ink outside the first cell at scale 1"
    );
}

#[test]
fn second_frame_rebuilds_only_changed_rows() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    let mut draw = ViewDraw::new(&gpu);
    render_full(&mut gpu, &mut draw, &mut view);
    view.feed(b"x");
    draw.prepare(&mut gpu, &mut view, FULL, (FULL.w, FULL.h));
    assert_eq!(draw.rows_rebuilt(), 1);
}

#[test]
fn unchanged_view_rebuilds_nothing() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    view.feed(b"hello");
    let mut draw = ViewDraw::new(&gpu);
    render_full(&mut gpu, &mut draw, &mut view);
    draw.prepare(&mut gpu, &mut view, FULL, (FULL.w, FULL.h));
    assert_eq!(draw.rows_rebuilt(), 0);
}

#[test]
fn two_views_share_the_glyph_texture() {
    let Some(mut gpu) = gpu() else { return };
    let (mut one, mut two) = (view(), view());
    one.feed(b"hello");
    two.feed(b"hello");
    let (mut draw_one, mut draw_two) = (ViewDraw::new(&gpu), ViewDraw::new(&gpu));
    render_full(&mut gpu, &mut draw_one, &mut one);
    let after_first = gpu.glyph_count();
    render_full(&mut gpu, &mut draw_two, &mut two);
    assert_eq!(gpu.glyph_count(), after_first);
}

#[test]
fn atlas_reset_mid_frame_keeps_earlier_rows_right() {
    let Some(mut gpu) = gpu() else { return };
    // At 320 physical pixels a cell is 192 by 465, so a few dozen distinct letters overflow the
    // first 1024 pixel glyph image while rows 1 to 3 are built, after row 0 already holds a slot.
    gpu.set_text_size(40.0, 8.0);
    let cell = gpu.cell_size();
    let rect = Rect {
        x: 0,
        y: 0,
        w: cell.w * 20,
        h: cell.h * 4,
    };
    let (width, height) = (cell.w * 2, cell.h);
    let mut big = view();
    big.feed(b"A\r\nBCDEFGHIJKLMNOPQRSTUVWXYZ0123456789bcdefghijklmnopqrstuvwxyz");
    let mut draw = ViewDraw::new(&gpu);
    let before = gpu.glyph_generation();
    let seen = render(&mut gpu, &mut draw, &mut big, rect, width, height);
    assert_ne!(
        gpu.glyph_generation(),
        before,
        "the glyph image never reset during the frame"
    );

    let mut alone = view();
    alone.feed(b"A");
    let mut draw_alone = ViewDraw::new(&gpu);
    let expected = render(&mut gpu, &mut draw_alone, &mut alone, rect, width, height);
    let cell_of = |img: &Image| -> Vec<Rgb> {
        (0..cell.h)
            .flat_map(|y| (0..cell.w).map(move |x| (x, y)))
            .map(|(x, y)| px(img, x, y))
            .collect()
    };
    assert!(
        cell_of(&expected).iter().any(|&p| p != BG),
        "the reference A left no ink"
    );
    assert!(
        cell_of(&seen) == cell_of(&expected),
        "row 0 drew from a slot of the old glyph image"
    );
}

#[test]
fn full_damage_rebuilds_every_row() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    for i in 0..10 {
        view.feed(format!("line {i}\r\n").as_bytes());
    }
    let mut draw = ViewDraw::new(&gpu);
    render_full(&mut gpu, &mut draw, &mut view);
    view.screen_mut().scroll_by(2);
    assert!(view.screen().scroll_offset() > 0, "the view did not scroll");
    draw.prepare(&mut gpu, &mut view, FULL, (FULL.w, FULL.h));
    assert_eq!(draw.rows_rebuilt(), 4);
}

#[cfg(target_os = "macos")]
#[test]
fn wide_character_spans_two_cells() {
    let Some(mut gpu) = gpu() else { return };
    let mut view = view();
    view.feed("\u{6F22}".as_bytes());
    let mut draw = ViewDraw::new(&gpu);
    let img = render_full(&mut gpu, &mut draw, &mut view);
    assert!(
        cell_pixels(&img, 0, 0).iter().any(|&p| p != BG),
        "no ink in cell (0,0)"
    );
    assert!(
        cell_pixels(&img, 1, 0).iter().any(|&p| p != BG),
        "no ink in cell (1,0)"
    );
    assert!(
        cell_pixels(&img, 2, 0).iter().all(|&p| p == BG),
        "ink in cell (2,0)"
    );
}
