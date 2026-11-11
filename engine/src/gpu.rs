//! The device, the two pipelines and the glyph texture that every view draws through.

use std::collections::HashMap;

use crate::atlas::{Atlas, AtlasSlot, GlyphKey};
use crate::font::{FontStyle, Fonts};
use crate::geometry::CellSize;
use crate::theme::{FONT_PX_DEFAULT, Rgb};

#[derive(Debug)]
pub enum GpuError {
    NoAdapter,
    Device(String),
}

/// One cell background run: `pos` and `size` in pixels from the top left of the view's rectangle.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct BgInstance {
    pub pos: [u16; 2],
    pub size: [u16; 2],
    pub color: [u8; 4],
}

/// One glyph quad; `slot` is its top left corner in the glyph texture. `color[3]` is 255 for a
/// mask glyph and 0 for a colour glyph. A solid rectangle (a cursor or a text decoration) has
/// `color[3]` 128 and carries its dash pattern in `slot` instead (see the shader).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GlyphInstance {
    pub pos: [u16; 2],
    pub size: [u16; 2],
    pub slot: [u16; 2],
    pub color: [u8; 4],
}

/// The view's place in its target, as the shader reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ViewUniform {
    pub target_size: [f32; 2],
    pub origin: [f32; 2],
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pub(crate) view_layout: wgpu::BindGroupLayout,
    pub(crate) bg_pipeline: wgpu::RenderPipeline,
    pub(crate) glyph_pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    texture: wgpu::Texture,
    pub(crate) texture_bind_group: wgpu::BindGroup,
    fonts: Fonts,
    atlas: Atlas,
    font_px: f32,
    scale: f64,
    cell: CellSize,
    /// Pixels from the top of a cell to the text baseline, where underlines hang from.
    baseline: u32,
    /// Slots of single characters, so the common cell skips building a `GlyphKey`, which
    /// allocates. Valid for `quick_generation` only.
    quick: HashMap<(char, FontStyle, bool), AtlasSlot>,
    quick_generation: u64,
    text: String,
}

impl Gpu {
    /// One per process. Every view in every window draws through it. `target` must be an sRGB
    /// format: the shader hands it linear colours.
    pub async fn new(
        instance: &wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
        target: wgpu::TextureFormat,
    ) -> Result<Gpu, GpuError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                // A terminal never needs the discrete GPU, and waking it costs battery.
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: surface,
            })
            .await
            .ok_or(GpuError::NoAdapter)?;
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("term-engine"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await
            .map_err(|e| GpuError::Device(e.to_string()))?;

        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));
        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("view"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<ViewUniform>() as u64),
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("glyph texture"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let bg_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("backgrounds"),
            bind_group_layouts: &[&view_layout],
            push_constant_ranges: &[],
        });
        let glyph_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("glyphs"),
            bind_group_layouts: &[&view_layout, &texture_layout],
            push_constant_ranges: &[],
        });
        let bg_attributes = wgpu::vertex_attr_array![0 => Uint16x2, 1 => Uint16x2, 2 => Unorm8x4];
        let glyph_attributes =
            wgpu::vertex_attr_array![0 => Uint16x2, 1 => Uint16x2, 2 => Uint16x2, 3 => Unorm8x4];
        let bg_pipeline = pipeline(
            &device,
            &bg_layout,
            &shader,
            ("bg_vs", "bg_fs"),
            wgpu::VertexBufferLayout {
                array_stride: size_of::<BgInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &bg_attributes,
            },
            target,
            None,
        );
        let glyph_pipeline = pipeline(
            &device,
            &glyph_layout,
            &shader,
            ("glyph_vs", "glyph_fs"),
            wgpu::VertexBufferLayout {
                array_stride: size_of::<GlyphInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &glyph_attributes,
            },
            target,
            // Straight alpha for both kinds: mask coverage and colour glyph alpha.
            Some(wgpu::BlendState::ALPHA_BLENDING),
        );

        let atlas = Atlas::new();
        let (texture, texture_bind_group) = glyph_texture(&device, &texture_layout, atlas.size());
        let mut fonts = Fonts::new();
        let cell = fonts.cell_size(FONT_PX_DEFAULT, 1.0);
        let baseline = fonts.baseline(FONT_PX_DEFAULT, 1.0);
        Ok(Gpu {
            device,
            queue,
            view_layout,
            bg_pipeline,
            glyph_pipeline,
            texture_layout,
            texture,
            texture_bind_group,
            fonts,
            atlas,
            font_px: FONT_PX_DEFAULT,
            scale: 1.0,
            cell,
            baseline,
            quick: HashMap::new(),
            quick_generation: 0,
            text: String::new(),
        })
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// Zoom or a new display scale; clears the glyph texture. The size already set does nothing,
    /// so a repeated scale event does not throw away every glyph.
    pub fn set_text_size(&mut self, font_px: f32, scale: f64) {
        if font_px == self.font_px && scale == self.scale {
            return;
        }
        self.font_px = font_px;
        self.scale = scale;
        self.cell = self.fonts.cell_size(font_px, scale);
        self.baseline = self.fonts.baseline(font_px, scale);
        self.atlas.clear();
    }

    pub(crate) fn scale(&self) -> f64 {
        self.scale
    }

    pub(crate) fn baseline(&self) -> u32 {
        self.baseline
    }

    pub fn cell_size(&self) -> CellSize {
        self.cell
    }

    pub fn glyph_count(&self) -> usize {
        self.atlas.len()
    }

    /// Changes whenever slots handed out before stop being valid. A view drawn before the change
    /// must be prepared again before it is drawn, even when nothing in it changed.
    pub fn glyph_generation(&self) -> u64 {
        self.atlas.generation()
    }

    /// Where `ch` plus `zerowidth` sits in the glyph texture.
    pub(crate) fn glyph(
        &mut self,
        ch: char,
        zerowidth: &[char],
        style: FontStyle,
        wide: bool,
    ) -> AtlasSlot {
        self.forget_old_quick_slots();
        if zerowidth.is_empty()
            && let Some(slot) = self.quick.get(&(ch, style, wide))
        {
            return *slot;
        }
        self.text.clear();
        self.text.push(ch);
        self.text.extend(zerowidth);
        let key = GlyphKey {
            text: self.text.as_str().into(),
            style,
            wide,
        };
        let (fonts, text, font_px, scale) = (&mut self.fonts, &self.text, self.font_px, self.scale);
        let slot = self
            .atlas
            .get_or_insert(&key, || fonts.rasterize(text, style, font_px, scale, wide));
        self.forget_old_quick_slots();
        if zerowidth.is_empty() {
            self.quick.insert((ch, style, wide), slot);
        }
        slot
    }

    fn forget_old_quick_slots(&mut self) {
        if self.quick_generation != self.atlas.generation() {
            self.quick.clear();
            self.quick_generation = self.atlas.generation();
        }
    }

    /// Brings the GPU texture up to date with the glyph image: a new texture when the image
    /// changed size, otherwise only the region changed since the last upload.
    pub(crate) fn upload_glyphs(&mut self) {
        let size = self.atlas.size();
        if self.texture.width() != size {
            let (texture, bind_group) = glyph_texture(&self.device, &self.texture_layout, size);
            self.texture = texture;
            self.texture_bind_group = bind_group;
        }
        let Some(dirty) = self.atlas.take_dirty() else {
            return;
        };
        if dirty.w == 0 || dirty.h == 0 {
            return;
        }
        let row_bytes = size as usize * 4;
        let start = dirty.y as usize * row_bytes + dirty.x as usize * 4;
        let end = start + (dirty.h as usize - 1) * row_bytes + dirty.w as usize * 4;
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: dirty.x,
                    y: dirty.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &self.atlas.pixels()[start..end],
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: dirty.w,
                height: dirty.h,
                depth_or_array_layers: 1,
            },
        );
    }
}

/// A clear colour for an sRGB target, matching `srgb_to_linear` in the shader, so a window's
/// clear and a cell's background give the same byte for the same `Rgb`.
pub fn clear_color(Rgb(r, g, b): Rgb) -> wgpu::Color {
    let linear = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    wgpu::Color {
        r: linear(r),
        g: linear(g),
        b: linear(b),
        a: 1.0,
    }
}

fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    (vs, fs): (&str, &str),
    buffer: wgpu::VertexBufferLayout<'_>,
    target: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(vs),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some(vs),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[buffer],
        },
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..wgpu::PrimitiveState::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview: None,
        cache: None,
    })
}

/// Plain `Rgba8Unorm`: the shader decodes colour glyph texels itself, in the same function as
/// cell colours, and mask alpha is coverage, never a colour.
fn glyph_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    size: u32,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("glyphs"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("glyphs"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });
    (texture, bind_group)
}
