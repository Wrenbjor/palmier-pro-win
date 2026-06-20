// Textured-quad compositor shader — E5-S8.
//
// One draw per layer. The vertex shader takes a per-vertex source-pixel corner +
// crop UV, maps the corner straight to clip space via the per-layer `clip` matrix
// (source-pixel → render-pixel → NDC, built CPU-side in quad.rs), and forwards the
// UV. The fragment shader samples the layer's premultiplied-RGBA texture and scales
// it by the layer opacity. Blending is PREMULTIPLIED_ALPHA (set on the pipeline), so
// the per-layer alpha already in the texture (premultiplied on upload) plus this
// opacity scalar composite correctly over the black-cleared target, bottom→top.

struct LayerUniform {
    // Column-major source-pixel → clip-space matrix (quad::layer_clip_matrix).
    clip: mat4x4<f32>,
    // Effective layer opacity in [0,1] (folds static × keyframe × fade).
    opacity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> layer: LayerUniform;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsIn {
    @location(0) corner: vec2<f32>, // source-pixel corner of the (cropped) quad
    @location(1) uv: vec2<f32>,     // texture coordinate for this corner
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.pos = layer.clip * vec4<f32>(in.corner, 0.0, 1.0);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Texture is premultiplied-alpha RGBA. Scaling a premultiplied color by a scalar
    // opacity keeps it premultiplied (both rgb and a scale by the same factor), so
    // the PREMULTIPLIED_ALPHA blend stays correct.
    let c = textureSample(tex, samp, in.uv);
    return c * layer.opacity;
}
