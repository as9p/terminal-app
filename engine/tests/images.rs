//! Six screens drawn on the GPU and compared with the images stored in `tests/images`, so a
//! change to how anything looks shows up as a failing test. `TERM_ENGINE_BLESS=1` stores the
//! current drawing instead; look at every stored file before committing it.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use term_engine::{
    CellPoint, Gpu, GridSize, Platform, Rect, Rgb, SelectKind, Side, TerminalView, Theme, ViewDraw,
    render_to_image,
};

const MAGENTA: Rgb = Rgb(255, 0, 255);
const WIDTH: u32 = 680;
const HEIGHT: u32 = 320;
/// A pixel differs when any channel is further off than this.
const CHANNEL_TOLERANCE: u8 = 16;
/// The share of differing pixels a drawing may have, in thousandths: 0.2 percent.
const DIFFERING_PER_MILLE: usize = 2;

/// The same shared setup as `draw.rs`: one `Gpu` per test binary, held under a lock, and
/// `None`, after saying so, when `TERM_ENGINE_NO_GPU=1` asks to skip drawing tests.
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

fn blessing() -> bool {
    std::env::var_os("TERM_ENGINE_BLESS").is_some_and(|v| v == "1")
}

fn stored_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/images")
        .join(format!("{name}.png"))
}

/// `target/image-diff`, found from the test program's own place in `target/<profile>/deps`, so
/// a moved target directory is followed.
fn diff_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("the test program has no path");
    exe.ancestors()
        .nth(3)
        .expect("the test program is not inside a target directory")
        .join("image-diff")
}

fn write_png(path: &Path, rgba: &[u8]) {
    let file =
        File::create(path).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
    let mut encoder = png::Encoder::new(BufWriter::new(file), WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .expect("writing the PNG header failed");
    writer
        .write_image_data(rgba)
        .expect("writing the PNG pixels failed");
}

/// Width, height and RGBA8 pixels of a stored image.
fn read_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut reader = png::Decoder::new(BufReader::new(file))
        .read_info()
        .map_err(|e| e.to_string())?;
    let size = reader
        .output_buffer_size()
        .ok_or("the image is too large to decode")?;
    let mut buf = vec![0; size];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if (info.color_type, info.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
        return Err(format!(
            "stored as {:?} {:?}, expected 8-bit RGBA",
            info.color_type, info.bit_depth
        ));
    }
    buf.truncate(info.buffer_size());
    Ok((info.width, info.height, buf))
}

/// Draws a 40 by 8 view, after `setup`, at scale 2 and compares it with the stored image.
fn check(name: &str, setup: impl FnOnce(&mut TerminalView)) {
    if blessing() && std::env::var_os("TERM_ENGINE_NO_GPU").is_some_and(|v| v == "1") {
        panic!("TERM_ENGINE_BLESS=1 needs a GPU, but TERM_ENGINE_NO_GPU=1 is set; nothing stored");
    }
    let Some(mut gpu) = gpu() else { return };
    let mut view = TerminalView::new(
        GridSize { cols: 40, rows: 8 },
        &Theme::builtin(),
        Platform::Mac,
    );
    setup(&mut view);
    let mut draw = ViewDraw::new(&gpu);
    let rect = Rect {
        x: 0,
        y: 0,
        w: WIDTH,
        h: HEIGHT,
    };
    let actual = render_to_image(&mut gpu, &mut draw, &mut view, rect, WIDTH, HEIGHT, MAGENTA);
    drop(gpu);
    assert_eq!(actual.len(), (WIDTH * HEIGHT * 4) as usize);

    let stored = stored_path(name);
    if blessing() {
        std::fs::create_dir_all(stored.parent().unwrap()).expect("cannot create tests/images");
        write_png(&stored, &actual);
        eprintln!("stored {}", stored.display());
        return;
    }
    let keep_actual = || {
        let dir = diff_dir();
        std::fs::create_dir_all(&dir).expect("cannot create target/image-diff");
        let path = dir.join(format!("{name}.actual.png"));
        write_png(&path, &actual);
        path
    };
    let (width, height, expected) = match read_png(&stored) {
        Ok(image) => image,
        Err(e) => panic!(
            "cannot read {}: {e}; TERM_ENGINE_BLESS=1 stores it (drawing kept in {})",
            stored.display(),
            keep_actual().display()
        ),
    };
    if (width, height) != (WIDTH, HEIGHT) || expected.len() != actual.len() {
        panic!(
            "{name}.png is {width} by {height}, the drawing is {WIDTH} by {HEIGHT} (drawing kept in {})",
            keep_actual().display()
        );
    }
    let differing = expected
        .chunks(4)
        .zip(actual.chunks(4))
        .filter(|(e, a)| {
            e.iter()
                .zip(a.iter())
                .any(|(e, a)| e.abs_diff(*a) > CHANNEL_TOLERANCE)
        })
        .count();
    let total = (WIDTH * HEIGHT) as usize;
    if differing * 1000 > total * DIFFERING_PER_MILLE {
        panic!(
            "{name}.png: {differing} of {total} pixels differ (drawing kept in {})",
            keep_actual().display()
        );
    }
}

fn at(col: u16, row: u16) -> CellPoint {
    CellPoint { col, row }
}

#[test]
fn colours() {
    check("colours", |view| {
        let mut s = String::new();
        // The 16 colours as backgrounds, then as text.
        for i in 0..16 {
            s += &format!("\x1b[{}m  ", if i < 8 { 40 + i } else { 100 + i - 8 });
        }
        s += "\x1b[0m\r\n";
        for i in 0..16 {
            s += &format!("\x1b[{}mAb", if i < 8 { 30 + i } else { 90 + i - 8 });
        }
        s += "\x1b[0m\r\n";
        // Every sixth of the 240 colours past the first 16.
        for i in 0..40 {
            s += &format!("\x1b[48;5;{}m ", 16 + i * 6);
        }
        s += "\x1b[0m\r\n";
        for i in 0..40 {
            let (r, g, b) = (i * 6, 255 - i * 6, 128);
            s += &format!("\x1b[48;2;{r};{g};{b}m ");
        }
        s += "\x1b[0m\x1b[6;1H";
        view.feed(s.as_bytes());
    });
}

#[test]
fn styles() {
    check("styles", |view| {
        view.feed(
            concat!(
                "regular \x1b[1mbold\x1b[0m \x1b[3mitalic\x1b[0m \x1b[1;3mbold italic\x1b[0m\r\n",
                "\x1b[4munderline\x1b[0m \x1b[4:2mdouble\x1b[0m \x1b[4:3mundercurl\x1b[0m\r\n",
                "\x1b[4:4mdotted\x1b[0m \x1b[4:5mdashed\x1b[0m \x1b[9mstrikeout\x1b[0m\r\n",
                "\x1b[2mdim\x1b[0m \x1b[7minverse\x1b[0m \x1b[4;9mboth lines\x1b[0m\r\n",
                "\x1b[31;4:3mcoloured curl\x1b[0m \x1b[32;4munderlined green\x1b[0m\r\n",
            )
            .as_bytes(),
        );
    });
}

#[test]
fn boxes() {
    check("boxes", |view| {
        view.feed(
            concat!(
                "\u{256D}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256E} ",
                "\u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2557} ",
                "\u{250F}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2513}\r\n",
                "\u{2502} text \u{2502} \u{2551} text \u{2551} \u{2503} text \u{2503}\r\n",
                "\u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{256F} ",
                "\u{255A}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255D} ",
                "\u{2517}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{251B}\r\n",
                "\r\n",
                "\u{2580}\u{2584}\u{258C}\u{2590} \u{2591}\u{2591}\u{2592}\u{2592}\u{2593}\u{2593}\u{2588}\u{2588}\r\n",
                "\u{2500}\u{253C}\u{2500}\u{2502}\u{251C}\u{2524}\u{252C}\u{2534}\r\n",
            )
            .as_bytes(),
        );
    });
}

#[cfg(target_os = "macos")]
#[test]
fn wide() {
    check("wide", |view| {
        view.feed(
            concat!(
                "\u{6F22}\u{5B57} \u{4E2D}\u{6587}|\r\n",
                "e\u{301} a\u{308} n\u{303} o\u{302}|\r\n",
                "\u{1F600}\u{1F680}\u{1F34E}|\r\n",
                "\u{E0B0} \u{E0A0} \x1b[44;30m main \x1b[0;34m\u{E0B0}\x1b[0m\r\n",
            )
            .as_bytes(),
        );
    });
}

#[test]
fn cursor() {
    check("cursor", |view| {
        view.feed(
            b"first line of text\r\nsecond line of text\r\nthird line of text\x1b[2;8H\x1b[2 q",
        );
    });
}

#[test]
fn selection() {
    check("selection", |view| {
        view.feed(b"first line of text\r\nsecond line\r\nthird line of text\x1b[6;1H");
        view.screen_mut()
            .select(SelectKind::Char, at(6, 0), Side::Left);
        view.screen_mut().select_to(at(4, 2), Side::Right);
    });
}
