//! Per-view instance buffers, rebuilt row by row from the screen's damage and drawn in two
//! instanced draws: cell backgrounds, then glyphs from the shared glyph texture together with the
//! solid rectangles of text decorations and the cursor.

use bytemuck::Zeroable;

use crate::font::FontStyle;
use crate::geometry::{CellPoint, CellSize, GridSize, Rect};
use crate::gpu::{BgInstance, GlyphInstance, Gpu, ViewUniform, clear_color};
use crate::screen::{Attrs, Cell, CellWidth, CursorShape, Damage};
use crate::theme::Rgb;
use crate::view::TerminalView;

/// A frame whose own glyphs keep overflowing the glyph image gives up after this many passes
/// instead of looping; that takes more distinct glyphs on screen than a 4096 image holds.
const MAX_PASSES: usize = 3;

/// Glyph buffer slots per cell of a row: its glyph, an underline and a strikeout. A row never
/// needs more, because each of the three is at most one quad per cell and runs of cells merge.
const SLOTS_PER_CELL: usize = 3;
/// After the last row: the cursor's rectangles, at most the four sides of an outline. Being last
/// in the glyph pass, nothing drawn after can cover them.
const CURSOR_SLOTS: usize = 4;
/// `GlyphInstance::color[3]` of a solid rectangle (see the shader).
const SOLID: u8 = 128;

/// The cursor as last built. Any difference rebuilds the rows it left and entered, because a
/// cursor change alone reports no damage.
#[derive(Clone, Copy, PartialEq)]
struct CursorLook {
    at: CellPoint,
    shape: CursorShape,
    focused: bool,
}

/// Everything a row build needs besides the cells; the same for every row of a frame.
#[derive(Clone, Copy)]
struct Style {
    cell: CellSize,
    palette_bg: Rgb,
    cursor: Rgb,
    cursor_text: Rgb,
    selection_bg: Rgb,
    selection_fg: Rgb,
    cursor_look: Option<CursorLook>,
    /// Thickness of cursor bars, outlines and decorations: one pixel per unit of display scale.
    line: i64,
    baseline: i64,
}

#[derive(Clone, Copy, PartialEq)]
enum Line {
    Single,
    Double,
    Dotted,
    Dashed,
    Strike,
}

impl Line {
    /// Patterned lines start again at every cell's left edge, so only unbroken ones merge.
    fn joins(self) -> bool {
        matches!(self, Line::Single | Line::Double | Line::Strike)
    }
}

/// What the instance data was built from. Any difference rebuilds every row.
#[derive(Clone, Copy, PartialEq)]
struct Built {
    grid: GridSize,
    cell: CellSize,
    rect: Rect,
    generation: u64,
}

pub struct ViewDraw {
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    uniform_for: Option<(Rect, (u32, u32))>,
    bg_buffer: wgpu::Buffer,
    glyph_buffer: wgpu::Buffer,
    /// CPU copies of the two buffers. `bg[0]` fills the whole rectangle with the palette
    /// background; after it, both hold `cols` slots per row, unused slots zero-size.
    bg: Vec<BgInstance>,
    glyphs: Vec<GlyphInstance>,
    built: Option<Built>,
    dirty: Vec<bool>,
    cells: Vec<Cell>,
    /// Background and text colour of each cell of the row being built, after the selection and
    /// a block cursor; kept to reuse its allocation.
    colors: Vec<(Rgb, Rgb)>,
    cursor_built: Option<CursorLook>,
    /// The cursor slots changed since the last upload.
    cursor_changed: bool,
    /// `None` when the rectangle has no area inside the target.
    scissor: Option<[u32; 4]>,
    rows_rebuilt: usize,
}

impl ViewDraw {
    pub fn new(gpu: &Gpu) -> ViewDraw {
        let device = gpu.device();
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("view"),
            size: size_of::<ViewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("view"),
            layout: &gpu.view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        ViewDraw {
            uniform,
            bind_group,
            uniform_for: None,
            bg_buffer: instance_buffer::<BgInstance>(gpu, "backgrounds", 1),
            glyph_buffer: instance_buffer::<GlyphInstance>(gpu, "glyphs", 1),
            bg: Vec::new(),
            glyphs: Vec::new(),
            built: None,
            dirty: Vec::new(),
            cells: Vec::new(),
            colors: Vec::new(),
            cursor_built: None,
            cursor_changed: false,
            scissor: None,
            rows_rebuilt: 0,
        }
    }

    /// Rebuilds only what changed since the last call, uploads it, and calls view.mark_drawn().
    pub fn prepare(
        &mut self,
        gpu: &mut Gpu,
        view: &mut TerminalView,
        rect: Rect,
        target: (u32, u32),
    ) {
        let damage = view.screen_mut().take_damage();
        let grid = view.grid();
        let (cols, rows) = (usize::from(grid.cols), usize::from(grid.rows));
        let cell = gpu.cell_size();
        let palette = &view.theme().palette;
        let style = Style {
            cell,
            palette_bg: palette.bg,
            cursor: palette.cursor,
            cursor_text: palette.cursor_text,
            selection_bg: palette.selection_bg,
            selection_fg: palette.selection_fg,
            cursor_look: view.screen().cursor().map(|c| CursorLook {
                at: c.at,
                shape: c.shape,
                focused: view.focused(),
            }),
            line: gpu.scale().round().max(1.0) as i64,
            baseline: i64::from(gpu.baseline()),
        };
        if self.built.is_none_or(|b| b.grid != grid) {
            self.resize(gpu, cols, rows);
        }
        let mut generation = gpu.glyph_generation();
        let current = Built {
            grid,
            cell,
            rect,
            generation,
        };
        let mut full = self.built != Some(current) || damage == Damage::Full;
        if full {
            self.dirty.fill(true);
        } else if let Damage::Rows(list) = &damage {
            for &row in list {
                if let Some(dirty) = self.dirty.get_mut(usize::from(row)) {
                    *dirty = true;
                }
            }
        }
        let cursor_moved = style.cursor_look != self.cursor_built;
        if cursor_moved {
            for look in [self.cursor_built, style.cursor_look].into_iter().flatten() {
                if let Some(dirty) = self.dirty.get_mut(usize::from(look.at.row)) {
                    *dirty = true;
                }
            }
            self.cursor_built = style.cursor_look;
        }
        if full || cursor_moved {
            // Building the cursor's row fills them again; a hidden cursor leaves them empty.
            let tail = self.glyphs.len() - CURSOR_SLOTS;
            self.glyphs[tail..].fill(GlyphInstance::zeroed());
            self.cursor_changed = true;
        }
        for _ in 0..MAX_PASSES {
            generation = gpu.glyph_generation();
            for row in 0..rows {
                if self.dirty[row] {
                    self.build_row(gpu, view, row, cols, &style);
                }
            }
            if gpu.glyph_generation() == generation {
                break;
            }
            // The glyph image started again while this frame was built: slots written earlier
            // in this pass, and in every row not rebuilt, point into the old image.
            full = true;
            self.dirty.fill(true);
        }
        if full {
            self.bg[0] = BgInstance {
                pos: [0, 0],
                size: [clamp_px(rect.w.into()), clamp_px(rect.h.into())],
                color: rgba(style.palette_bg, 255),
            };
        }
        self.upload(gpu, full, cols);
        gpu.upload_glyphs();
        self.rows_rebuilt = self.dirty.iter().filter(|&&d| d).count();
        self.dirty.fill(false);
        self.cursor_changed = false;
        // A frame that never settled keeps an older generation, so the next prepare tries again.
        self.built = Some(Built {
            generation,
            ..current
        });

        if self.uniform_for != Some((rect, target)) {
            let uniform = ViewUniform {
                target_size: [target.0.max(1) as f32, target.1.max(1) as f32],
                origin: [rect.x as f32, rect.y as f32],
            };
            gpu.queue()
                .write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
            self.uniform_for = Some((rect, target));
        }
        self.scissor = scissor(rect, target);
        view.mark_drawn();
    }

    pub fn draw(&self, gpu: &Gpu, pass: &mut wgpu::RenderPass<'_>) {
        // wgpu rejects a scissor of zero area, and there is nothing to see anyway.
        let (Some([x, y, w, h]), Some(built)) = (self.scissor, self.built) else {
            return;
        };
        pass.set_scissor_rect(x, y, w, h);
        pass.set_pipeline(&gpu.bg_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.bg_buffer.slice(..));
        pass.draw(0..4, 0..self.bg.len() as u32);
        // Another view's prepare reset the glyph image after this one was built; its slots would
        // show other glyphs, so text waits for this view's next prepare.
        if built.generation != gpu.glyph_generation() {
            return;
        }
        pass.set_pipeline(&gpu.glyph_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_bind_group(1, &gpu.texture_bind_group, &[]);
        pass.set_vertex_buffer(0, self.glyph_buffer.slice(..));
        pass.draw(0..4, 0..self.glyphs.len() as u32);
    }

    /// Rows whose instance data the last prepare rebuilt.
    pub fn rows_rebuilt(&self) -> usize {
        self.rows_rebuilt
    }

    fn resize(&mut self, gpu: &Gpu, cols: usize, rows: usize) {
        let slots = cols * rows;
        self.bg = vec![BgInstance::zeroed(); 1 + slots];
        self.glyphs = vec![GlyphInstance::zeroed(); slots * SLOTS_PER_CELL + CURSOR_SLOTS];
        self.bg_buffer = instance_buffer::<BgInstance>(gpu, "backgrounds", self.bg.len());
        self.glyph_buffer = instance_buffer::<GlyphInstance>(gpu, "glyphs", self.glyphs.len());
        self.dirty = vec![false; rows];
    }

    fn build_row(
        &mut self,
        gpu: &mut Gpu,
        view: &TerminalView,
        row: usize,
        cols: usize,
        style: &Style,
    ) {
        let screen = view.screen();
        screen.cells(row as u16, &mut self.cells);
        let cells = &self.cells[..self.cells.len().min(cols)];
        let cell = style.cell;
        let top = i64::from(cell.h) * row as i64;
        let x_of = |col: usize| i64::from(cell.w) * col as i64;
        // The right half of a wide character takes its colours and decorations from the left.
        let spacer_of_wide = |col: usize| {
            col > 0
                && cells[col].width == CellWidth::Spacer
                && cells[col - 1].width == CellWidth::Wide
        };
        let cursor_here = style
            .cursor_look
            .filter(|c| usize::from(c.at.row) == row && usize::from(c.at.col) < cells.len());

        let block = cursor_here
            .filter(|c| c.focused && c.shape == CursorShape::Block)
            .map(|c| usize::from(c.at.col));
        let selected = |col: usize| {
            screen.is_selected(CellPoint {
                col: col as u16,
                row: row as u16,
            })
        };
        self.colors.clear();
        for (col, c) in cells.iter().enumerate() {
            // The cursor wins over the selection. A cursor on the right half of a wide character
            // should not happen; if it does, it covers that half only.
            let colors = if block == Some(col) {
                (style.cursor, style.cursor_text)
            } else if spacer_of_wide(col) {
                self.colors[col - 1]
            } else if selected(col) || (c.width == CellWidth::Wide && selected(col + 1)) {
                (style.selection_bg, style.selection_fg)
            } else {
                (c.bg, c.fg)
            };
            self.colors.push(colors);
        }

        // Runs of one colour make one quad; the palette background is already under them.
        let bg_row = &mut self.bg[1 + row * cols..1 + (row + 1) * cols];
        let mut used = 0;
        let mut run: Option<(usize, Rgb)> = None;
        for col in 0..=cells.len() {
            let color = self
                .colors
                .get(col)
                .map(|c| c.0)
                .filter(|&c| c != style.palette_bg);
            if color == run.map(|(_, c)| c) {
                continue;
            }
            if let Some((start, run_color)) = run {
                bg_row[used] = BgInstance {
                    pos: [clamp_px(x_of(start)), clamp_px(top)],
                    size: [clamp_px(x_of(col - start)), clamp_px(cell.h.into())],
                    color: rgba(run_color, 255),
                };
                used += 1;
            }
            run = color.map(|c| (col, c));
        }
        bg_row[used..].fill(BgInstance::zeroed());

        let row_slots = SLOTS_PER_CELL * cols;
        let glyph_row = &mut self.glyphs[row * row_slots..(row + 1) * row_slots];
        let mut used = 0;
        for (col, c) in cells.iter().enumerate() {
            // A spacer is the right half of a wide character, drawn by its left half.
            if c.width == CellWidth::Spacer || (c.ch == ' ' && c.zerowidth.is_empty()) {
                continue;
            }
            let face = FontStyle {
                bold: c.attrs.contains(Attrs::BOLD),
                italic: c.attrs.contains(Attrs::ITALIC),
            };
            let slot = gpu.glyph(c.ch, &c.zerowidth, face, c.width == CellWidth::Wide);
            if slot.w == 0 || slot.h == 0 {
                continue;
            }
            glyph_row[used] = GlyphInstance {
                pos: [
                    clamp_px(x_of(col) + i64::from(slot.left)),
                    clamp_px(top + i64::from(slot.top)),
                ],
                size: [clamp_px(slot.w.into()), clamp_px(slot.h.into())],
                slot: [clamp_px(slot.x.into()), clamp_px(slot.y.into())],
                color: rgba(self.colors[col].1, if slot.color { 0 } else { 255 }),
            };
            used += 1;
        }

        // After the glyphs, so a descender's transparent edge cannot hide part of a line.
        for strike in [false, true] {
            let mut run: Option<(usize, Line, Rgb)> = None;
            for col in 0..=cells.len() {
                let key = cells.get(col).and_then(|c| {
                    let attrs = if spacer_of_wide(col) {
                        cells[col - 1].attrs
                    } else {
                        c.attrs
                    };
                    let line = if strike {
                        attrs.contains(Attrs::STRIKEOUT).then_some(Line::Strike)
                    } else {
                        underline_of(attrs)
                    };
                    line.map(|line| (line, self.colors[col].1))
                });
                if let (Some((_, line, color)), Some(next)) = (run, key)
                    && line.joins()
                    && (line, color) == next
                {
                    continue;
                }
                if let Some((start, line, color)) = run {
                    glyph_row[used] =
                        line_quad(line, color, x_of(start), x_of(col - start), top, style);
                    used += 1;
                }
                run = key.map(|(line, color)| (col, line, color));
            }
        }
        glyph_row[used..].fill(GlyphInstance::zeroed());

        if let Some(look) = cursor_here {
            let col = usize::from(look.at.col);
            let wide = cells[col].width == CellWidth::Wide;
            let (x, w, h) = (x_of(col), x_of(if wide { 2 } else { 1 }), i64::from(cell.h));
            let t = style.line.min(w).min(h);
            let paint = |x, y, w, h| solid(x, y, w, h, style.cursor, [0, 0]);
            let mut quads = [GlyphInstance::zeroed(); CURSOR_SLOTS];
            if !look.focused {
                let side = (h - 2 * t).max(0);
                quads = [
                    paint(x, top, w, t),
                    paint(x, top + h - t, w, t),
                    paint(x, top + t, t, side),
                    paint(x + w - t, top + t, t, side),
                ];
            } else {
                match look.shape {
                    CursorShape::Bar => quads[0] = paint(x, top, t, h),
                    CursorShape::Underline => quads[0] = paint(x, top + h - t, w, t),
                    // Drawn by the cell colours above.
                    CursorShape::Block => {}
                }
            }
            let tail = self.glyphs.len() - CURSOR_SLOTS;
            self.glyphs[tail..].copy_from_slice(&quads);
            self.cursor_changed = true;
        }
    }

    /// Everything after a full rebuild, otherwise one write per run of adjacent dirty rows.
    fn upload(&self, gpu: &Gpu, full: bool, cols: usize) {
        let queue = gpu.queue();
        if full {
            queue.write_buffer(&self.bg_buffer, 0, bytemuck::cast_slice(&self.bg));
            queue.write_buffer(&self.glyph_buffer, 0, bytemuck::cast_slice(&self.glyphs));
            return;
        }
        let rows = self.dirty.len();
        let mut row = 0;
        while row < rows {
            if !self.dirty[row] {
                row += 1;
                continue;
            }
            let first = row;
            while row < rows && self.dirty[row] {
                row += 1;
            }
            let (bg_from, bg_to) = (1 + first * cols, 1 + row * cols);
            queue.write_buffer(
                &self.bg_buffer,
                (bg_from * size_of::<BgInstance>()) as u64,
                bytemuck::cast_slice(&self.bg[bg_from..bg_to]),
            );
            let row_slots = SLOTS_PER_CELL * cols;
            let (glyph_from, glyph_to) = (first * row_slots, row * row_slots);
            queue.write_buffer(
                &self.glyph_buffer,
                (glyph_from * size_of::<GlyphInstance>()) as u64,
                bytemuck::cast_slice(&self.glyphs[glyph_from..glyph_to]),
            );
        }
        if self.cursor_changed {
            let tail = self.glyphs.len() - CURSOR_SLOTS;
            queue.write_buffer(
                &self.glyph_buffer,
                (tail * size_of::<GlyphInstance>()) as u64,
                bytemuck::cast_slice(&self.glyphs[tail..]),
            );
        }
    }
}

/// Test helper: draws one view into a new `width` by `height` image cleared to `clear`, returns RGBA8 rows.
pub fn render_to_image(
    gpu: &mut Gpu,
    draw: &mut ViewDraw,
    view: &mut TerminalView,
    rect: Rect,
    width: u32,
    height: u32,
    clear: Rgb,
) -> Vec<u8> {
    draw.prepare(gpu, view, rect, (width, height));
    // wgpu rejects a texture with no pixels.
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let device = gpu.device();
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("image"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let target = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("image"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color(clear)),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        draw.draw(gpu, &mut pass);
    }
    // Buffer copies need each row padded to a multiple of 256 bytes.
    let row_bytes = width * 4;
    let padded = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("image readback"),
        size: u64::from(padded) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &readback,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        size,
    );
    gpu.queue().submit([encoder.finish()]);
    let slice = readback.slice(..);
    let (done, wait) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = done.send(result);
    });
    device.poll(wgpu::Maintain::Wait);
    wait.recv()
        .expect("the readback callback never ran")
        .expect("reading the image back failed");
    let mut out = Vec::with_capacity((row_bytes * height) as usize);
    {
        let data = slice.get_mapped_range();
        for row in data.chunks(padded as usize) {
            out.extend_from_slice(&row[..row_bytes as usize]);
        }
    }
    readback.unmap();
    out
}

fn instance_buffer<T>(gpu: &Gpu, label: &str, count: usize) -> wgpu::Buffer {
    gpu.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (count.max(1) * size_of::<T>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Instance positions are 16-bit. Anything past 65535 pixels is far outside any target, so
/// pinning it there only moves a quad the scissor already removes.
fn clamp_px(value: i64) -> u16 {
    value.clamp(0, i64::from(u16::MAX)) as u16
}

/// The one underline style a cell shows. Programs set one style at a time; should several flags
/// meet, the plainer style wins.
fn underline_of(attrs: Attrs) -> Option<Line> {
    if attrs.contains(Attrs::UNDERLINE) {
        Some(Line::Single)
    } else if attrs.contains(Attrs::DOUBLE_UNDERLINE) {
        Some(Line::Double)
    } else if attrs.intersects(Attrs::DOTTED_UNDERLINE | Attrs::UNDERCURL) {
        // A curl needs more than rectangles; dots keep it distinct from the other styles.
        Some(Line::Dotted)
    } else if attrs.contains(Attrs::DASHED_UNDERLINE) {
        Some(Line::Dashed)
    } else {
        None
    }
}

/// One decoration across `w` pixels from `x`, kept inside the cell's height.
fn line_quad(line: Line, color: Rgb, x: i64, w: i64, top: i64, style: &Style) -> GlyphInstance {
    let (t, cell_w, cell_h) = (style.line, i64::from(style.cell.w), i64::from(style.cell.h));
    let h = if line == Line::Double { 3 * t } else { t }.min(cell_h);
    let y = if line == Line::Strike {
        (cell_h - h) / 2
    } else {
        (style.baseline + t).min(cell_h - h)
    };
    let across = match line {
        Line::Dotted => pattern(t, 2 * t),
        Line::Dashed => {
            let on = (cell_w as f64 / 3.0).round().max(1.0) as i64;
            pattern(on, on + (cell_w as f64 / 6.0).round().max(1.0) as i64)
        }
        _ => 0,
    };
    let down = if line == Line::Double {
        pattern(t, 2 * t)
    } else {
        0
    };
    solid(x, top + y, w, h, color, [across, down])
}

/// A dash pattern for the shader; unbroken when it does not fit its 8-bit fields.
fn pattern(on: i64, period: i64) -> u16 {
    if on >= period || period > 255 {
        0
    } else {
        (on | period << 8) as u16
    }
}

fn solid(x: i64, y: i64, w: i64, h: i64, color: Rgb, pattern: [u16; 2]) -> GlyphInstance {
    GlyphInstance {
        pos: [clamp_px(x), clamp_px(y)],
        size: [clamp_px(w), clamp_px(h)],
        slot: pattern,
        color: rgba(color, SOLID),
    }
}

fn rgba(Rgb(r, g, b): Rgb, a: u8) -> [u8; 4] {
    [r, g, b, a]
}

/// The part of `rect` inside the target.
fn scissor(rect: Rect, (width, height): (u32, u32)) -> Option<[u32; 4]> {
    let (left, top) = (rect.x.min(width), rect.y.min(height));
    let right = rect.x.saturating_add(rect.w).min(width);
    let bottom = rect.y.saturating_add(rect.h).min(height);
    (right > left && bottom > top).then_some([left, top, right - left, bottom - top])
}
