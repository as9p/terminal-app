// Cell backgrounds and glyphs. Every position is in whole pixels from the top left of the view's
// rectangle, so quads land on pixel edges and each glyph texel maps to exactly one pixel.

struct View {
    target_size: vec2<f32>,
    origin: vec2<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var glyphs: texture_2d<f32>;

// The one place colours change space. The target is an sRGB format, so the hardware encodes
// what the shader returns and blends in linear space; decoding the 8-bit sRGB values exactly
// here is what lets a palette byte come back out unchanged.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

// Triangle strip corners 0 to 3: (0,0) (1,0) (0,1) (1,1).
fn corner(index: u32) -> vec2<f32> {
    return vec2<f32>(f32(index & 1u), f32(index >> 1u));
}

fn to_clip(px: vec2<f32>) -> vec4<f32> {
    let ndc = (view.origin + px) / view.target_size * 2.0 - 1.0;
    return vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
}

struct BgOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
}

@vertex
fn bg_vs(
    @builtin(vertex_index) index: u32,
    @location(0) pos: vec2<u32>,
    @location(1) size: vec2<u32>,
    @location(2) color: vec4<f32>,
) -> BgOut {
    var out: BgOut;
    out.position = to_clip(vec2<f32>(pos) + corner(index) * vec2<f32>(size));
    out.color = vec4<f32>(srgb_to_linear(color.rgb), 1.0);
    return out;
}

@fragment
fn bg_fs(in: BgOut) -> @location(0) vec4<f32> {
    return in.color;
}

struct GlyphOut {
    @builtin(position) position: vec4<f32>,
    @location(0) texel: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
}

// `color.a` is 1 for a mask glyph, tinted with `color.rgb`, and 0 for a colour glyph, drawn
// with its own texels.
@vertex
fn glyph_vs(
    @builtin(vertex_index) index: u32,
    @location(0) pos: vec2<u32>,
    @location(1) size: vec2<u32>,
    @location(2) slot: vec2<u32>,
    @location(3) color: vec4<f32>,
) -> GlyphOut {
    let offset = corner(index) * vec2<f32>(size);
    var out: GlyphOut;
    out.position = to_clip(vec2<f32>(pos) + offset);
    out.texel = vec2<f32>(slot) + offset;
    out.color = vec4<f32>(srgb_to_linear(color.rgb), color.a);
    return out;
}

@fragment
fn glyph_fs(in: GlyphOut) -> @location(0) vec4<f32> {
    // Read without filtering: a pixel centre sits half a texel into its texel.
    let texel = textureLoad(glyphs, vec2<i32>(floor(in.texel)), 0);
    if in.color.a > 0.5 {
        return vec4<f32>(in.color.rgb, texel.a);
    }
    return vec4<f32>(srgb_to_linear(texel.rgb), texel.a);
}
