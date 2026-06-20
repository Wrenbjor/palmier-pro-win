// Text + solid-quad shader for the wgpu text pass — E5-S9.
//
// Two entry-point pairs share one pipeline-per-mode (selected CPU-side):
//  * `vs_quad`/`fs_solid` — a solid-color quad in NDC (background fill, border
//    edges, shadow box). Color is a premultiplied RGBA uniform.
//  * `vs_glyph`/`fs_glyph` — a glyph quad sampling the R8 coverage atlas; the
//    coverage modulates a premultiplied tint color (glyph fill / shadow color).
//
// All geometry arrives already in NDC (CPU builds the source-pixel→NDC transform
// in text.rs, same convention as the composite pass), so the vertex shaders are
// pass-through. Blending is PREMULTIPLIED_ALPHA (set on the pipeline).

struct QuadUniform {
    // Premultiplied RGBA color (rgb already × a), all in [0,1].
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> quad: QuadUniform;

struct VsIn {
    @location(0) pos: vec2<f32>, // NDC position
    @location(1) uv: vec2<f32>,  // atlas UV (glyphs) / unused (solid)
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_quad(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(in.pos, 0.0, 1.0);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_solid(in: VsOut) -> @location(0) vec4<f32> {
    // Already-premultiplied solid color.
    return quad.color;
}

@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_samp: sampler;

@fragment
fn fs_glyph(in: VsOut) -> @location(0) vec4<f32> {
    // R8 coverage in the red channel; modulate the premultiplied tint by coverage.
    let coverage = textureSample(atlas, atlas_samp, in.uv).r;
    return quad.color * coverage;
}
