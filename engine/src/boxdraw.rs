//! Box drawing and block characters (U+2500 to U+259F), drawn from the cell size rather than
//! taken from a font, so lines in neighbouring cells always meet and blocks fill the cell exactly.

use crate::font::{Glyph, GlyphKind};
use crate::geometry::CellSize;

/// The arms of U+2500 to U+257F, four digits each in the order up, right, down, left:
/// 0 none, 1 light, 2 heavy, 3 double. Dashed lines and arcs still name their arms here; the
/// diagonals have none and are drawn by their own code.
const ARMS: &str = "\
    0101 0202 1010 2020 0101 0202 1010 2020 0101 0202 1010 2020 0110 0210 0120 0220 \
    0011 0012 0021 0022 1100 1200 2100 2200 1001 1002 2001 2002 1110 1210 2110 1120 \
    2120 2210 1220 2220 1011 1012 2011 1021 2021 2012 1022 2022 0111 0112 0211 0212 \
    0121 0122 0221 0222 1101 1102 1201 1202 2101 2102 2201 2202 1111 1112 1211 1212 \
    2111 1121 2121 2112 2211 1122 1221 2212 1222 2122 2221 2222 0101 0202 1010 2020 \
    0303 3030 0310 0130 0330 0013 0031 0033 1300 3100 3300 1003 3001 3003 1310 3130 \
    3330 1013 3031 3033 0313 0131 0333 1303 3101 3303 1313 3131 3333 0110 0011 1001 \
    1100 0000 0000 0000 0001 1000 0100 0010 0002 2000 0200 0020 0201 1020 0102 2010";

const NONE: u8 = 0;
const DOUBLE: u8 = 3;

/// The bitmap for a box or block character, exactly one cell, or `None` outside U+2500 to U+259F.
pub fn box_glyph(ch: char, cell: CellSize) -> Option<Glyph> {
    let code = ch as u32;
    if !(0x2500..=0x259F).contains(&code) {
        return None;
    }
    let mut c = Canvas::new(cell);
    match code {
        0x2580..=0x259F => blocks(&mut c, code),
        0x256D..=0x2570 => arc(&mut c, arms(code)),
        0x2571..=0x2573 => diagonals(&mut c, code),
        _ => match dashes(code) {
            Some(n) => dashed(&mut c, arms(code), n),
            None => lines(&mut c, arms(code)),
        },
    }
    Some(Glyph {
        width: c.w,
        height: c.h,
        left: 0,
        top: 0,
        kind: GlyphKind::Mask,
        pixels: c.px,
    })
}

fn arms(code: u32) -> [u8; 4] {
    let digits = ARMS
        .split_whitespace()
        .nth((code - 0x2500) as usize)
        .unwrap_or("0000");
    let mut arms = [NONE; 4];
    for (arm, digit) in arms.iter_mut().zip(digits.bytes()) {
        *arm = digit - b'0';
    }
    arms
}

fn dashes(code: u32) -> Option<u32> {
    match code {
        0x2504..=0x2507 => Some(3),
        0x2508..=0x250B => Some(4),
        0x254C..=0x254F => Some(2),
        _ => None,
    }
}

struct Canvas {
    w: u32,
    h: u32,
    px: Vec<u8>,
    light: u32,
    heavy: u32,
}

impl Canvas {
    fn new(cell: CellSize) -> Canvas {
        // 1 pixel at small sizes, 2 at a 17 by 31 cell; heavy is twice that.
        let light = ((cell.h as f32 / 16.0).round() as u32).max(1);
        Canvas {
            w: cell.w,
            h: cell.h,
            px: vec![0; cell.w as usize * cell.h as usize],
            light,
            heavy: light * 2,
        }
    }

    fn thickness(&self, style: u8) -> u32 {
        match style {
            1 => self.light,
            2 => self.heavy,
            _ => 0,
        }
    }

    fn put(&mut self, x: u32, y: u32, value: u8) {
        if x < self.w && y < self.h {
            let p = &mut self.px[(y * self.w + x) as usize];
            *p = (*p).max(value);
        }
    }

    /// Fills `x0..x1` by `y0..y1`, clipped to the cell.
    fn fill(&mut self, (x0, x1): (u32, u32), (y0, y1): (u32, u32), value: u8) {
        for y in y0..y1.min(self.h) {
            for x in x0..x1.min(self.w) {
                self.put(x, y, value);
            }
        }
    }

    /// Fills a span along one axis and across the other, so one routine serves both directions.
    fn fill_along(&mut self, horizontal: bool, along: (u32, u32), across: (u32, u32)) {
        if horizontal {
            self.fill(along, across, 255);
        } else {
            self.fill(across, along, 255);
        }
    }

    /// Sets each pixel to the coverage `cover` gives its centre, for shapes that are not
    /// axis aligned.
    fn paint(&mut self, cover: impl Fn(f32, f32) -> f32) {
        for y in 0..self.h {
            for x in 0..self.w {
                let c = cover(x as f32 + 0.5, y as f32 + 0.5).clamp(0.0, 1.0);
                self.put(x, y, (c * 255.0).round() as u8);
            }
        }
    }
}

/// A band of `thick` pixels centred in `len`. Every cell of one size puts it in the same place,
/// which is what makes neighbouring cells join.
fn band(len: u32, thick: u32) -> (u32, u32) {
    let thick = thick.min(len);
    let start = (len - thick) / 2;
    (start, start + thick)
}

/// The two strokes of a double line across `len`: stroke, gap and stroke of one light width each.
fn double(len: u32, light: u32) -> ((u32, u32), (u32, u32)) {
    let t = light.min(len / 3).max(1);
    let start = len.saturating_sub(3 * t) / 2;
    ((start, start + t), (start + 2 * t, start + 3 * t))
}

fn lines(c: &mut Canvas, [up, right, down, left]: [u8; 4]) {
    axis(c, true, (left, right), (up, down));
    axis(c, false, (up, down), (left, right));
}

/// Draws the two arms on one axis. `arms` are the negative and positive side (left and right, or
/// up and down) and `perp` the arms on the other axis, which decide where each stroke stops so
/// that corners, tees and crossings close without gaps or overshoot.
fn axis(c: &mut Canvas, horizontal: bool, arms: (u8, u8), perp: (u8, u8)) {
    let (len, across_len) = if horizontal { (c.w, c.h) } else { (c.h, c.w) };
    let mid = len / 2;
    let perp_single = c.thickness(perp.0).max(c.thickness(perp.1));
    let perp_double = perp.0 == DOUBLE || perp.1 == DOUBLE;
    // Where the perpendicular strokes sit along this axis.
    let single = band(len, perp_single);
    let (lo, hi) = double(len, c.light);

    for (style, positive, opposite) in [(arms.0, false, arms.1), (arms.1, true, arms.0)] {
        let span = |stop: u32| if positive { (stop, len) } else { (0, stop) };
        // The near edge of a perpendicular stroke when this arm comes from the positive side.
        let edge = |(start, end): (u32, u32)| if positive { start } else { end };
        match style {
            NONE => {}
            DOUBLE => {
                let (near_lo, near_hi) = double(across_len, c.light);
                // Each stroke turns into the perpendicular stroke on its own side when there is
                // one, else into the far stroke of the perpendicular, else runs to the middle.
                for (stroke, near, far) in [(near_lo, perp.0, perp.1), (near_hi, perp.1, perp.0)] {
                    let inner = if positive { hi } else { lo };
                    let outer = if positive { lo } else { hi };
                    let stop = if near != NONE {
                        edge(if near == DOUBLE { inner } else { single })
                    } else if far != NONE {
                        edge(if far == DOUBLE { outer } else { single })
                    } else {
                        mid
                    };
                    c.fill_along(horizontal, span(stop), stroke);
                }
            }
            _ => {
                let across = band(across_len, c.thickness(style));
                // A single stroke ending at a double line stops at its near stroke; one that
                // carries on through the other side crosses it.
                let stop = if perp_double && opposite == NONE {
                    edge(if positive { hi } else { lo })
                } else if perp_double || perp_single == 0 {
                    mid
                } else {
                    edge(single)
                };
                c.fill_along(horizontal, span(stop), across);
            }
        }
    }
}

fn dashed(c: &mut Canvas, arms: [u8; 4], n: u32) {
    let horizontal = arms[1] != NONE;
    let style = if horizontal { arms[1] } else { arms[0] };
    let (len, across_len) = if horizontal { (c.w, c.h) } else { (c.h, c.w) };
    let across = band(across_len, c.thickness(style));
    // The gap is split over both ends of each dash so the pattern repeats evenly across cells.
    let gap = (len / n / 3).max(1);
    for i in 0..n {
        let start = i * len / n + gap / 2;
        let end = ((i + 1) * len / n).saturating_sub(gap - gap / 2);
        if start < end {
            c.fill_along(horizontal, (start, end), across);
        }
    }
}

/// A rounded corner: a quarter circle of light width, with straight stubs reaching the two
/// edges exactly where the light lines do.
fn arc(c: &mut Canvas, [_, right, down, _]: [u8; 4]) {
    let t = c.light;
    let (x0, x1) = band(c.w, t);
    let (y0, y1) = band(c.h, t);
    let xc = (x0 + x1) as f32 / 2.0;
    let yc = (y0 + y1) as f32 / 2.0;
    let sx = if right != NONE { 1.0 } else { -1.0 };
    let sy = if down != NONE { 1.0 } else { -1.0 };
    let reach_x = if sx > 0.0 { c.w as f32 - xc } else { xc };
    let reach_y = if sy > 0.0 { c.h as f32 - yc } else { yc };
    // One pixel short of the nearer edge, so both edges see the plain straight line.
    let r = (reach_x.min(reach_y) - 1.0).max(0.0);
    let (ax, ay) = (xc + sx * r, yc + sy * r);

    if sx > 0.0 {
        c.fill((ax.floor() as u32, c.w), (y0, y1), 255);
    } else {
        c.fill((0, ax.ceil() as u32), (y0, y1), 255);
    }
    if sy > 0.0 {
        c.fill((x0, x1), (ay.floor() as u32, c.h), 255);
    } else {
        c.fill((x0, x1), (0, ay.ceil() as u32), 255);
    }
    let half = t as f32 / 2.0;
    c.paint(|x, y| {
        if (x - ax) * sx > 0.0 || (y - ay) * sy > 0.0 {
            return 0.0;
        }
        let d = ((x - ax).powi(2) + (y - ay).powi(2)).sqrt();
        half + 0.5 - (d - r).abs()
    });
}

fn diagonals(c: &mut Canvas, code: u32) {
    let (w, h) = (c.w as f32, c.h as f32);
    let length = (w * w + h * h).sqrt().max(1.0);
    let half = c.light as f32 / 2.0;
    // Distance from the corner to corner lines: top left to bottom right, and top right to
    // bottom left.
    let falling = move |x: f32, y: f32| (x * h - y * w).abs() / length;
    let rising = move |x: f32, y: f32| (x * h + y * w - w * h).abs() / length;
    let (fall, rise) = match code {
        0x2571 => (false, true),
        0x2572 => (true, false),
        _ => (true, true),
    };
    c.paint(|x, y| {
        let mut d = f32::INFINITY;
        if fall {
            d = d.min(falling(x, y));
        }
        if rise {
            d = d.min(rising(x, y));
        }
        half + 0.5 - d
    });
}

/// `k` eighths of `len`, rounded.
fn eighths(len: u32, k: u32) -> u32 {
    (len * k + 4) / 8
}

fn blocks(c: &mut Canvas, code: u32) {
    let (w, h) = (c.w, c.h);
    let (mx, my) = (w / 2, h / 2);
    match code {
        0x2580 => c.fill((0, w), (0, my), 255),
        0x2581..=0x2587 => c.fill((0, w), (h - eighths(h, code - 0x2580), h), 255),
        0x2588 => c.fill((0, w), (0, h), 255),
        // Left blocks are the complement of the matching right part, so the left half ends
        // exactly where the right half starts.
        0x2589..=0x258F => c.fill((0, w - eighths(w, 8 - (0x2590 - code))), (0, h), 255),
        0x2590 => c.fill((mx, w), (0, h), 255),
        // Shades are an even tint so they tile without a visible pattern seam.
        0x2591..=0x2593 => c.fill((0, w), (0, h), [64, 128, 191][(code - 0x2591) as usize]),
        0x2594 => c.fill((0, w), (0, eighths(h, 1)), 255),
        0x2595 => c.fill((w - eighths(w, 1), w), (0, h), 255),
        _ => {
            // Quadrant bits: 1 upper left, 2 upper right, 4 lower left, 8 lower right.
            let bits = [4u8, 8, 1, 13, 9, 7, 11, 2, 6, 14][(code - 0x2596) as usize];
            let quads = [
                ((0, mx), (0, my)),
                ((mx, w), (0, my)),
                ((0, mx), (my, h)),
                ((mx, w), (my, h)),
            ];
            for (i, (xs, ys)) in quads.into_iter().enumerate() {
                if bits & (1 << i) != 0 {
                    c.fill(xs, ys, 255);
                }
            }
        }
    }
}
