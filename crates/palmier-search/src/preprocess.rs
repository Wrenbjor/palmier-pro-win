//! Image preprocessing — reproduce the reference's SigLIP squash-resize.
//!
//! Ported from Spike S-3 (`spikes/s3-siglip2/src/preprocess.rs`), proven against the
//! macOS reference `VisualEmbedder.pixelBuffer`.
//!
//! Reference (`VisualEmbedder.pixelBuffer`, search.md "Image preproc"):
//!   - create a 256×256 BGRA buffer, **fill black first** (recycled buffer memory;
//!     alpha must blend over black, not garbage),
//!   - **squash-resize** the image into the full square (NO aspect crop), high
//!     interpolation, sRGB colour space.
//! The CoreML `ImageType` input then scales those 8-bit pixels to **[-1, 1]**
//! internally (per the model card). The ONNX `vision_model.onnx` instead wants a
//! pre-normalized `pixel_values [1,3,256,256]` f32 tensor, so we do the rescale +
//! mean/std-0.5 normalization here (preprocessor_config.json: rescale 1/255,
//! mean 0.5, std 0.5 → (x/255 - 0.5)/0.5 == x/127.5 - 1.0 == exactly [-1, 1]).
//!
//! Net: same geometry as the reference, same [-1,1] value range, emitted in the
//! channel-first RGB layout ONNX Runtime expects.

use anyhow::Result;
use image::{imageops::FilterType, DynamicImage, Rgba, RgbaImage};

use crate::spec::IMAGE_SIZE;

/// Squash-resize `img` into a black-filled `IMAGE_SIZE`×`IMAGE_SIZE` RGBA square,
/// no aspect crop. Mirrors the reference draw-into-square-over-black step.
///
/// `image`'s `resize_exact` ignores aspect ratio (it squashes to the exact target),
/// which is precisely the reference behavior. We then alpha-composite over an opaque
/// black background so any transparency blends to black like the reference's
/// `setFillColor(black); fill(); draw(image)`.
pub fn squash_to_square(img: &DynamicImage) -> RgbaImage {
    let edge = IMAGE_SIZE as u32;
    // FilterType::Triangle == bilinear, matching preprocessor_config.json resample=2
    // (PIL BILINEAR). (The reference uses CG "high" interpolation; bilinear is the
    // model-card-specified resample and the closest portable match — see the S-3
    // FINDINGS "preprocessing parity" for the residual difference.)
    let resized = img.resize_exact(edge, edge, FilterType::Triangle);
    let resized = resized.to_rgba8();

    let mut canvas = RgbaImage::from_pixel(edge, edge, Rgba([0, 0, 0, 255]));
    for (x, y, px) in resized.enumerate_pixels() {
        let a = px[3] as f32 / 255.0;
        // straight-alpha over black: out = src*a + black*(1-a) = src*a
        let r = (px[0] as f32 * a).round() as u8;
        let g = (px[1] as f32 * a).round() as u8;
        let b = (px[2] as f32 * a).round() as u8;
        canvas.put_pixel(x, y, Rgba([r, g, b, 255]));
    }
    canvas
}

/// Full preprocess: a decoded `DynamicImage` → ONNX `pixel_values` `[3*256*256]`
/// f32 in CHW order, values in [-1, 1].
///
/// Layout: channel-first (all R, then all G, then all B), row-major within each
/// channel — the standard ONNX `NCHW` convention with N folded out (N=1).
pub fn to_pixel_values(img: &DynamicImage) -> Vec<f32> {
    let square = squash_to_square(img);
    let n = IMAGE_SIZE * IMAGE_SIZE;
    let mut out = vec![0.0f32; 3 * n];
    for (i, px) in square.pixels().enumerate() {
        // (x/255 - 0.5)/0.5 == x/127.5 - 1.0
        out[i] = px[0] as f32 / 127.5 - 1.0; // R plane
        out[n + i] = px[1] as f32 / 127.5 - 1.0; // G plane
        out[2 * n + i] = px[2] as f32 / 127.5 - 1.0; // B plane
    }
    out
}

/// Convenience: preprocess an in-memory `RgbImage` frame (the type the sampler /
/// `palmier_media` decode path emits) directly to `pixel_values`, with no file I/O.
pub fn pixel_values_from_rgb(img: &image::RgbImage) -> Vec<f32> {
    to_pixel_values(&DynamicImage::ImageRgb8(img.clone()))
}

/// Convenience: load an image file (jpeg/png) and preprocess it.
pub fn pixel_values_from_path(path: &std::path::Path) -> Result<Vec<f32>> {
    let img = image::open(path)?;
    Ok(to_pixel_values(&img))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};

    #[test]
    fn squash_produces_exact_square_no_crop() {
        // A wide 400×100 image must squash (not crop) to 256×256.
        let src = DynamicImage::ImageRgb8(RgbImage::from_pixel(400, 100, image::Rgb([10, 20, 30])));
        let sq = squash_to_square(&src);
        assert_eq!(sq.dimensions(), (IMAGE_SIZE as u32, IMAGE_SIZE as u32));
    }

    #[test]
    fn pixel_values_are_in_minus_one_to_one_and_chw() {
        // Solid white → every channel maps to +1.0 exactly (255/127.5 - 1 == 1.0).
        let white =
            DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255])));
        let pv = to_pixel_values(&white);
        assert_eq!(pv.len(), 3 * IMAGE_SIZE * IMAGE_SIZE);
        for v in &pv {
            assert!((*v - 1.0).abs() < 1e-6, "white must normalize to +1.0, got {v}");
        }

        // Solid black → every channel -1.0 exactly (0/127.5 - 1 == -1.0).
        let black = DynamicImage::ImageRgb8(RgbImage::from_pixel(64, 64, image::Rgb([0, 0, 0])));
        let pv = to_pixel_values(&black);
        for v in &pv {
            assert!((*v + 1.0).abs() < 1e-6, "black must normalize to -1.0, got {v}");
        }
    }

    #[test]
    fn mid_grey_maps_near_zero() {
        // 127/128 grey straddles 0; 128 → ~+0.0039, 127 → ~-0.0039.
        let grey =
            DynamicImage::ImageRgb8(RgbImage::from_pixel(8, 8, image::Rgb([128, 128, 128])));
        let pv = to_pixel_values(&grey);
        for v in &pv {
            assert!(v.abs() < 0.01, "mid grey must be ~0, got {v}");
        }
    }

    #[test]
    fn rgb_frame_helper_matches_dynamic() {
        let rgb = RgbImage::from_pixel(100, 40, image::Rgb([200, 100, 50]));
        let a = pixel_values_from_rgb(&rgb);
        let b = to_pixel_values(&DynamicImage::ImageRgb8(rgb));
        assert_eq!(a, b);
    }
}
