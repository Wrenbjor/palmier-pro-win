//! Pure pixel conversion + premultiply for texture upload — E5-S8.
//!
//! `palmier-media` hands the engine **CPU-side** decoded planes
//! ([`DecodedFrame`](palmier_media::DecodedFrame)) in one of four layouts
//! ([`PixelLayout`](palmier_media::PixelLayout)): planar `Yuv420P`/`Yuv422P`/
//! `Yuv444P`, or interleaved `Rgba8`. The compositor uploads **one RGBA8 texture
//! per layer**, so this module normalizes any layout to a tightly-packed
//! `Rgba8Unorm` buffer the GPU can `write_texture` directly.
//!
//! Two reference-mandated invariants live here (preview-engine.md risk #3, #5):
//! - **Premultiplied alpha.** Straight-alpha sources must be premultiplied so alpha
//!   edges don't fringe under the compositor's premultiplied-alpha blend. We
//!   premultiply on upload (`r·a, g·a, b·a, a`) exactly when `has_alpha` is set —
//!   the codec/pixfmt flag only, never container capability. Opaque sources keep
//!   `a = 255` (premultiply is a no-op).
//! - **BT.709.** The YUV→RGB matrix is the BT.709 full→RGB conversion the reference
//!   forces everywhere (`ITU_R_709_2`).
//!
//! Doing the YUV→RGB + premultiply on the CPU here (rather than a 3-plane YUV
//! shader) keeps the GPU pipeline a single RGBA-sampling path — simpler, and the
//! conversion is a per-frame cost paid once on upload (the texture is then cached,
//! so a held frame converts once). A future optimization can move YUV sampling into
//! the shader; the texture-cache key + blend contract stay identical.

use palmier_media::{DecodedFrame, PixelLayout, Plane};

/// A tightly-packed `Rgba8Unorm` image ready for `Queue::write_texture`: `width *
/// height * 4` bytes, row-major, **premultiplied** alpha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` premultiplied RGBA bytes, no row padding.
    pub bytes: Vec<u8>,
}

impl RgbaImage {
    /// Bytes per row (`width * 4`) — the unpadded stride for `write_texture`.
    pub fn row_bytes(&self) -> u32 {
        self.width * 4
    }
}

/// Premultiply a single straight-alpha RGBA pixel in place (`c' = c·a/255`).
#[inline]
fn premul(r: u8, g: u8, b: u8, a: u8) -> [u8; 4] {
    if a == 255 {
        return [r, g, b, a];
    }
    let a16 = a as u16;
    // Round-to-nearest (`+127`) so a 50% alpha halves a channel symmetrically.
    let pm = |c: u8| (((c as u16) * a16 + 127) / 255) as u8;
    [pm(r), pm(g), pm(b), a]
}

/// BT.709 limited-range? No — the reference tags full-range BT.709 working space.
/// We use the **full-range** BT.709 Y'CbCr→R'G'B' matrix (Kr=0.2126, Kb=0.0722),
/// with chroma centered at 128. Inputs are 8-bit full range `[0,255]`.
#[inline]
fn yuv709_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let yf = y as f32;
    let uf = u as f32 - 128.0;
    let vf = v as f32 - 128.0;
    // BT.709 full-range inverse matrix.
    let r = yf + 1.5748 * vf;
    let g = yf - 0.1873 * uf - 0.4681 * vf;
    let b = yf + 1.8556 * uf;
    [
        r.clamp(0.0, 255.0) as u8,
        g.clamp(0.0, 255.0) as u8,
        b.clamp(0.0, 255.0) as u8,
    ]
}

/// Read a byte from a strided plane at `(x, y)`, clamping to the plane bounds (so
/// chroma upsampling near the right/bottom edge never reads out of range).
#[inline]
fn plane_at(p: &Plane, x: u32, y: u32) -> u8 {
    let xc = x.min(p.width.saturating_sub(1)) as usize;
    let yc = y.min(p.height.saturating_sub(1)) as usize;
    let idx = yc * p.stride + xc;
    p.bytes.get(idx).copied().unwrap_or(0)
}

/// Convert any [`DecodedFrame`] to a tightly-packed, premultiplied [`RgbaImage`].
///
/// - `Rgba8`: copy row-by-row (dropping any stride padding) and premultiply when
///   `has_alpha`.
/// - planar YUV: BT.709 full-range YUV→RGB per luma pixel, nearest-neighbor chroma
///   upsampling for the sub-sampled planes; alpha forced opaque (YUV carries none).
///
/// Returns an empty image for a zero-size frame (the caller skips uploading it).
pub fn decoded_to_rgba(frame: &DecodedFrame) -> RgbaImage {
    let w = frame.width;
    let h = frame.height;
    if w == 0 || h == 0 {
        return RgbaImage { width: 0, height: 0, bytes: Vec::new() };
    }
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];

    match frame.layout {
        PixelLayout::Rgba8 => {
            let plane = &frame.planes[0];
            let row_src = plane.stride;
            let row_dst = (w as usize) * 4;
            for y in 0..h as usize {
                let src = &plane.bytes[y * row_src..y * row_src + (w as usize) * 4];
                let dst = &mut out[y * row_dst..y * row_dst + row_dst];
                if frame.has_alpha {
                    for x in 0..w as usize {
                        let s = &src[x * 4..x * 4 + 4];
                        dst[x * 4..x * 4 + 4].copy_from_slice(&premul(s[0], s[1], s[2], s[3]));
                    }
                } else {
                    // Opaque: copy RGB, force alpha 255 (ignore any garbage in the
                    // source alpha byte for non-alpha pixfmts).
                    for x in 0..w as usize {
                        let s = &src[x * 4..x * 4 + 4];
                        dst[x * 4..x * 4 + 4].copy_from_slice(&[s[0], s[1], s[2], 255]);
                    }
                }
            }
        }
        PixelLayout::Yuv420P | PixelLayout::Yuv422P | PixelLayout::Yuv444P => {
            let yp = &frame.planes[0];
            let up = &frame.planes[1];
            let vp = &frame.planes[2];
            // Chroma sub-sampling factors derived from the chroma plane dimensions
            // relative to luma (robust to 420/422/444 without branching on layout).
            let sx = (w / up.width.max(1)).max(1);
            let sy = (h / up.height.max(1)).max(1);
            let row_dst = (w as usize) * 4;
            for y in 0..h {
                for x in 0..w {
                    let yv = plane_at(yp, x, y);
                    let cu = plane_at(up, x / sx, y / sy);
                    let cv = plane_at(vp, x / sx, y / sy);
                    let [r, g, b] = yuv709_to_rgb(yv, cu, cv);
                    let i = (y as usize) * row_dst + (x as usize) * 4;
                    out[i] = r;
                    out[i + 1] = g;
                    out[i + 2] = b;
                    out[i + 3] = 255;
                }
            }
        }
    }

    RgbaImage { width: w, height: h, bytes: out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn rgba_frame(w: u32, h: u32, has_alpha: bool, px: &[u8]) -> DecodedFrame {
        DecodedFrame {
            layout: PixelLayout::Rgba8,
            width: w,
            height: h,
            has_alpha,
            planes: Arc::new(vec![Plane {
                bytes: px.to_vec(),
                stride: (w as usize) * 4,
                width: w,
                height: h,
            }]),
            source_frame: 0,
        }
    }

    #[test]
    fn premul_opaque_is_noop() {
        assert_eq!(premul(10, 20, 30, 255), [10, 20, 30, 255]);
    }

    #[test]
    fn premul_half_alpha_halves_channels() {
        // alpha 128 ≈ 50.2%: 200·128/255 ≈ 100 (round-nearest).
        let p = premul(200, 100, 50, 128);
        assert_eq!(p[3], 128);
        assert_eq!(p[0], 100);
        assert_eq!(p[1], 50);
        assert_eq!(p[2], 25);
        // Fully transparent → all channels zero.
        assert_eq!(premul(255, 255, 255, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn rgba_opaque_forces_alpha_255() {
        // 1×1, source alpha byte is garbage (7) but has_alpha=false → forced opaque.
        let f = rgba_frame(1, 1, false, &[10, 20, 30, 7]);
        let img = decoded_to_rgba(&f);
        assert_eq!(img.bytes, vec![10, 20, 30, 255]);
        assert_eq!(img.row_bytes(), 4);
    }

    #[test]
    fn rgba_alpha_premultiplies() {
        let f = rgba_frame(1, 1, true, &[200, 100, 50, 128]);
        let img = decoded_to_rgba(&f);
        assert_eq!(img.bytes, vec![100, 50, 25, 128]);
    }

    #[test]
    fn rgba_drops_stride_padding() {
        // 1px wide, stride 8 (4 padding bytes per row), 2 rows.
        let f = DecodedFrame {
            layout: PixelLayout::Rgba8,
            width: 1,
            height: 2,
            has_alpha: false,
            planes: Arc::new(vec![Plane {
                bytes: vec![1, 2, 3, 255, 0, 0, 0, 0, /*row1*/ 4, 5, 6, 255, 0, 0, 0, 0],
                stride: 8,
                width: 1,
                height: 2,
            }]),
            source_frame: 0,
        };
        let img = decoded_to_rgba(&f);
        assert_eq!(img.bytes, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    }

    #[test]
    fn yuv_gray_midpoint_is_neutral() {
        // Y=128, U=V=128 (neutral chroma) → mid gray, fully opaque.
        let yp = Plane { bytes: vec![128], stride: 1, width: 1, height: 1 };
        let up = Plane { bytes: vec![128], stride: 1, width: 1, height: 1 };
        let vp = Plane { bytes: vec![128], stride: 1, width: 1, height: 1 };
        let f = DecodedFrame {
            layout: PixelLayout::Yuv444P,
            width: 1,
            height: 1,
            has_alpha: false,
            planes: Arc::new(vec![yp, up, vp]),
            source_frame: 0,
        };
        let img = decoded_to_rgba(&f);
        assert_eq!(img.bytes[0], 128);
        assert_eq!(img.bytes[1], 128);
        assert_eq!(img.bytes[2], 128);
        assert_eq!(img.bytes[3], 255, "YUV always opaque");
    }

    #[test]
    fn yuv709_white_and_black() {
        // Y=255 neutral chroma → white-ish; Y=0 → black.
        assert_eq!(yuv709_to_rgb(255, 128, 128), [255, 255, 255]);
        assert_eq!(yuv709_to_rgb(0, 128, 128), [0, 0, 0]);
    }

    #[test]
    fn yuv420_chroma_upsamples_2x() {
        // 2×2 luma, 1×1 chroma (4:2:0). All luma 128, chroma neutral → 4 gray px.
        let yp = Plane { bytes: vec![128, 128, 128, 128], stride: 2, width: 2, height: 2 };
        let up = Plane { bytes: vec![128], stride: 1, width: 1, height: 1 };
        let vp = Plane { bytes: vec![128], stride: 1, width: 1, height: 1 };
        let f = DecodedFrame {
            layout: PixelLayout::Yuv420P,
            width: 2,
            height: 2,
            has_alpha: false,
            planes: Arc::new(vec![yp, up, vp]),
            source_frame: 0,
        };
        let img = decoded_to_rgba(&f);
        assert_eq!(img.bytes.len(), 2 * 2 * 4);
        // Every pixel mid-gray opaque.
        for px in img.bytes.chunks(4) {
            assert_eq!(px, &[128, 128, 128, 255]);
        }
    }

    #[test]
    fn zero_size_frame_is_empty() {
        let f = rgba_frame(0, 0, false, &[]);
        let img = decoded_to_rgba(&f);
        assert_eq!(img.width, 0);
        assert!(img.bytes.is_empty());
    }
}
