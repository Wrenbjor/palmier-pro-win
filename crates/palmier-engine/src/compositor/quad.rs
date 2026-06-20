//! Pure quad geometry math for the wgpu compositor — E5-S8.
//!
//! A [`VisualLayer`](crate::VisualLayer) is drawn as one textured quad. The layer's
//! [`Mat3`] maps **source pixels → render pixels** (the reference `emitTransform`
//! base × clip affine, already sampled at the frame). To draw it with wgpu we need
//! two transforms expressed as GPU-friendly matrices:
//!
//! 1. **Position** (`layer_clip_matrix`): source-pixel → render-pixel ([`Mat3`]) →
//!    normalized device coordinates (NDC / clip space). The quad's local space is
//!    the source frame's pixel box `[0..natW] × [0..natH]`; this matrix takes a
//!    local corner all the way to clip space in the vertex shader.
//! 2. **Crop UV** (`crop_uv_rect`): the visible [`CropRect`] (in source pixels)
//!    converted to `[0, 1]` texture coordinates so the fragment shader samples only
//!    the cropped region.
//!
//! Everything here is **pure** (no wgpu types) so it unit-tests headlessly — the
//! GPU pass just uploads the results. The matrices are emitted column-major as
//! `[f32; 16]` (a `mat4x4<f32>` uniform) because WGSL has no `mat3x3` uniform with
//! convenient std140 packing; we lift the 2-D affine into the top-left of a 4×4.

use crate::composition::CropRect;
use crate::Mat3;

/// The render canvas the layers composite into (mirrors
/// [`Canvas`](crate::Canvas) but kept local so this pure module needs no preview
/// dep). Width/height in **render pixels**.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CanvasSize {
    pub width: f32,
    pub height: f32,
}

impl CanvasSize {
    pub fn new(width: u32, height: u32) -> Self {
        CanvasSize {
            width: width.max(1) as f32,
            height: height.max(1) as f32,
        }
    }
}

/// A `[0, 1]` texture-coordinate rectangle (the cropped sub-region of the source
/// texture the quad samples).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvRect {
    /// Left edge in `[0, 1]`.
    pub u0: f32,
    /// Top edge in `[0, 1]`.
    pub v0: f32,
    /// Right edge in `[0, 1]`.
    pub u1: f32,
    /// Bottom edge in `[0, 1]`.
    pub v1: f32,
}

/// Convert a [`CropRect`] (source pixels) + the source's natural size into a `[0, 1]`
/// UV rectangle, clamped to the texture. An identity crop (`CropRect::full`) yields
/// the whole `[0,1]×[0,1]` texture.
///
/// The crop is already expressed in the decoded frame's own pixel space (the build
/// applied `preferred_transform.inverted()`), so this is a straight divide by the
/// natural size + clamp.
pub fn crop_uv_rect(crop: CropRect, natural_w: f64, natural_h: f64) -> UvRect {
    let w = natural_w.max(1.0);
    let h = natural_h.max(1.0);
    let u0 = (crop.x / w).clamp(0.0, 1.0);
    let v0 = (crop.y / h).clamp(0.0, 1.0);
    let u1 = ((crop.x + crop.width) / w).clamp(0.0, 1.0);
    let v1 = ((crop.y + crop.height) / h).clamp(0.0, 1.0);
    UvRect {
        u0: u0 as f32,
        v0: v0 as f32,
        u1: u1.max(u0) as f32,
        v1: v1.max(v0) as f32,
    }
}

/// The quad's local-space corners in **source pixels**, covering the cropped region
/// only — `(x, y)` pairs for the two triangles' 4 logical corners, top-left origin:
/// `[ (cx, cy), (cx+cw, cy), (cx, cy+ch), (cx+cw, cy+ch) ]`.
///
/// We restrict the quad to the crop rect so the affine maps the *visible* region
/// into render space exactly as the reference's crop+transform composition does
/// (crop is applied in source space, then the layer transform). Returned as f64
/// so the matrix multiply below stays in source-pixel precision.
pub fn crop_corners(crop: CropRect) -> [(f64, f64); 4] {
    let (x0, y0) = (crop.x, crop.y);
    let (x1, y1) = (crop.x + crop.width, crop.y + crop.height);
    [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
}

/// Build the column-major `[f32; 16]` `mat4x4<f32>` that maps a **source-pixel**
/// point `(x, y, 0, 1)` straight to clip space (NDC), composing:
///
/// `source-pixel → render-pixel` (the layer [`Mat3`]) then
/// `render-pixel → NDC` (`x' = 2x/W − 1`, `y' = 1 − 2y/H`, top-left origin → the
/// y-flip wgpu/NDC needs).
///
/// The vertex shader multiplies a source-pixel corner by this single matrix. Lifting
/// the 2-D affine into a 4×4 keeps the GPU uniform a plain `mat4x4<f32>`.
///
/// ## Convention bridge
/// [`Mat3`] follows Core Graphics' **row-vector** convention (`p' = p · M`), with
/// coefficients `a, b, c, d, tx, ty` meaning
/// `x' = a·x + c·y + tx`, `y' = b·x + d·y + ty`. WGSL multiplies **column-vectors**
/// (`p' = M · p`). So the 4×4 we hand the shader is the *transpose* of the row-form,
/// laid out column-major. We compute the composed `render→NDC ∘ source→render`
/// coefficients directly to avoid a separate matrix object.
pub fn layer_clip_matrix(transform: Mat3, canvas: CanvasSize) -> [f32; 16] {
    // Layer affine (source-px → render-px), row-vector form:
    //   rx = a·x + c·y + tx
    //   ry = b·x + d·y + ty
    let (a, b, c, d, tx, ty) = (
        transform.a,
        transform.b,
        transform.c,
        transform.d,
        transform.tx,
        transform.ty,
    );
    let (w, h) = (canvas.width as f64, canvas.height as f64);

    // render-px → NDC: nx = 2·rx/w − 1 ; ny = 1 − 2·ry/h.
    // Substitute rx, ry:
    //   nx = (2a/w)·x + (2c/w)·y + (2tx/w − 1)
    //   ny = (−2b/h)·x + (−2d/h)·y + (1 − 2ty/h)
    let m00 = (2.0 * a / w) as f32; // ∂nx/∂x
    let m01 = (-2.0 * b / h) as f32; // ∂ny/∂x
    let m10 = (2.0 * c / w) as f32; // ∂nx/∂y
    let m11 = (-2.0 * d / h) as f32; // ∂ny/∂y
    let m30 = (2.0 * tx / w - 1.0) as f32; // nx translation
    let m31 = (1.0 - 2.0 * ty / h) as f32; // ny translation

    // Column-major mat4x4<f32>. Column n is contiguous. We map (x, y, 0, 1):
    //   col0 acts on x, col1 on y, col2 on z (unused), col3 is translation.
    [
        // col0: x → (nx, ny, 0, 0)
        m00, m01, 0.0, 0.0, //
        // col1: y → (nx, ny, 0, 0)
        m10, m11, 0.0, 0.0, //
        // col2: z (unused; keep depth 0)
        0.0, 0.0, 1.0, 0.0, //
        // col3: translation + w=1
        m30, m31, 0.0, 1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mul_point(m: &[f32; 16], x: f32, y: f32) -> (f32, f32) {
        // Column-major mat4 · column-vector (x, y, 0, 1).
        let nx = m[0] * x + m[4] * y + m[8] * 0.0 + m[12];
        let ny = m[1] * x + m[5] * y + m[9] * 0.0 + m[13];
        (nx, ny)
    }

    #[test]
    fn identity_layer_maps_full_canvas_to_ndc() {
        // An identity transform on a 1920×1080 canvas: source pixel (0,0) is the
        // top-left → NDC (-1, +1); (1920, 1080) is bottom-right → NDC (+1, -1).
        let m = layer_clip_matrix(Mat3::IDENTITY, CanvasSize::new(1920, 1080));
        let (x, y) = mul_point(&m, 0.0, 0.0);
        assert!((x + 1.0).abs() < 1e-5, "top-left x: {x}");
        assert!((y - 1.0).abs() < 1e-5, "top-left y: {y}");
        let (x, y) = mul_point(&m, 1920.0, 1080.0);
        assert!((x - 1.0).abs() < 1e-5, "bot-right x: {x}");
        assert!((y + 1.0).abs() < 1e-5, "bot-right y: {y}");
        // Canvas center → NDC origin.
        let (x, y) = mul_point(&m, 960.0, 540.0);
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5, "center → origin: ({x},{y})");
    }

    #[test]
    fn translation_shifts_in_render_then_ndc() {
        // Translate the layer right by half the canvas width: source (0,0) lands at
        // render (960, 0) on a 1920-wide canvas → NDC x = 0.
        let m = layer_clip_matrix(Mat3::translation(960.0, 0.0), CanvasSize::new(1920, 1080));
        let (x, y) = mul_point(&m, 0.0, 0.0);
        assert!(x.abs() < 1e-5, "translated x → 0: {x}");
        assert!((y - 1.0).abs() < 1e-5, "y unchanged at top: {y}");
    }

    #[test]
    fn scale_affects_extent() {
        // Half-scale: source (1920,1080) maps to render (960,540) → NDC origin.
        let m = layer_clip_matrix(Mat3::scale(0.5, 0.5), CanvasSize::new(1920, 1080));
        let (x, y) = mul_point(&m, 1920.0, 1080.0);
        assert!(x.abs() < 1e-5 && y.abs() < 1e-5, "scaled corner → origin: ({x},{y})");
    }

    #[test]
    fn crop_uv_full_is_unit_square() {
        let uv = crop_uv_rect(CropRect::full(1920.0, 1080.0), 1920.0, 1080.0);
        assert_eq!(uv, UvRect { u0: 0.0, v0: 0.0, u1: 1.0, v1: 1.0 });
    }

    #[test]
    fn crop_uv_subrect_and_clamp() {
        // A centered half-size crop on a 100×100 source → UV [0.25,0.75].
        let crop = CropRect { x: 25.0, y: 25.0, width: 50.0, height: 50.0 };
        let uv = crop_uv_rect(crop, 100.0, 100.0);
        assert!((uv.u0 - 0.25).abs() < 1e-6);
        assert!((uv.v0 - 0.25).abs() < 1e-6);
        assert!((uv.u1 - 0.75).abs() < 1e-6);
        assert!((uv.v1 - 0.75).abs() < 1e-6);

        // Over-extent crop clamps to [0,1] (never samples outside the texture).
        let big = CropRect { x: -10.0, y: -10.0, width: 200.0, height: 200.0 };
        let uv = crop_uv_rect(big, 100.0, 100.0);
        assert_eq!(uv, UvRect { u0: 0.0, v0: 0.0, u1: 1.0, v1: 1.0 });
    }

    #[test]
    fn crop_corners_cover_rect() {
        let crop = CropRect { x: 10.0, y: 20.0, width: 30.0, height: 40.0 };
        let c = crop_corners(crop);
        assert_eq!(c[0], (10.0, 20.0));
        assert_eq!(c[3], (40.0, 60.0));
    }
}
