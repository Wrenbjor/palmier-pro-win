//! Image thumbnail pipeline (story E4-S5) — EXIF-aware, gated at 4.
//!
//! Port of `MediaVisualCache.makeImageThumbnail`
//! (`Sources/PalmierPro/Timeline/MediaVisualCache.swift`), which uses ImageIO
//! `CGImageSourceCreateThumbnailAtIndex` with `maxPixelSize: 120` and the
//! `kCGImageSourceCreateThumbnailWithTransform: true` flag (so EXIF orientation
//! is baked into the output). We replace ImageIO with the `image` crate + a
//! manual EXIF-orientation transform (`docs/reference/media-panel.md`
//! §"Image thumbnail" / §"macOS/Apple APIs to replace").
//!
//! ## Cache integration (#16)
//! Thumbnails are cached under the E4-S2 [`crate::cache::cache_key`]
//! (`sha256(path|size|mtime).prefix16`) as `<key>.thumb.jpg`, behind the
//! [`crate::cache::CacheKind::ImageThumbnail`] gate (**4 concurrent**).

use std::path::{Path, PathBuf};

use image::{DynamicImage, RgbaImage};

use crate::cache::{cache_key, CacheGates, CacheKind};

/// Max pixel size of the longer thumbnail edge (`kCGImageSourceThumbnailMaxPixelSize:
/// 120`). The image is scaled so its longest side is ≤ this, aspect preserved.
pub const IMAGE_THUMB_MAX_PIXEL: u32 = 120;

/// Errors the image thumbnail pipeline can surface.
#[derive(Debug)]
pub enum ImageThumbnailError {
    /// Decode/encode failure from the `image` crate.
    Image(String),
    /// I/O reading the source or writing the cache.
    Io(String),
}

impl std::fmt::Display for ImageThumbnailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageThumbnailError::Image(m) => write!(f, "image: {m}"),
            ImageThumbnailError::Io(m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for ImageThumbnailError {}

/// EXIF orientation (1–8) per the TIFF/EXIF spec. 1 is upright; 2–8 are the
/// flips/rotations. Used to apply the same transform ImageIO's
/// `…WithTransform` flag bakes in.
fn read_exif_orientation(path: &Path) -> u16 {
    let Ok(file) = std::fs::File::open(path) else {
        return 1;
    };
    let mut reader = std::io::BufReader::new(file);
    let exif_reader = exif::Reader::new();
    let Ok(exif) = exif_reader.read_from_container(&mut reader) else {
        return 1;
    };
    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .map(|v| v as u16)
        .unwrap_or(1)
}

/// Apply an EXIF orientation (1–8) to `img`, returning an upright image. Mirrors
/// the standard EXIF orientation table:
/// 1=none, 2=flip-h, 3=rot180, 4=flip-v, 5=transpose, 6=rot90cw, 7=transverse,
/// 8=rot270cw. Public + pure so the transform is unit-tested directly.
pub fn apply_exif_orientation(img: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img, // 1 or unknown → as-is
    }
}

/// Generate an EXIF-corrected, ≤ [`IMAGE_THUMB_MAX_PIXEL`] thumbnail for the
/// image at `path`, returned as RGBA pixels (the form handed to the webview).
///
/// Decodes, applies the EXIF orientation, then resizes so the longest edge is
/// ≤ 120 px (aspect preserved; never upscales). Pure CPU work; the cached entry
/// point [`ImageThumbnailCache::generate`] runs it on a blocking pool under the
/// 4-wide gate.
pub fn make_image_thumbnail(path: &Path) -> Result<RgbaImage, ImageThumbnailError> {
    let orientation = read_exif_orientation(path);
    let img =
        image::open(path).map_err(|e| ImageThumbnailError::Image(format!("decode: {e}")))?;
    let upright = apply_exif_orientation(img, orientation);
    // `thumbnail` preserves aspect ratio, fitting within the box, and never
    // upscales beyond the source — matching ImageIO's maxPixelSize semantics.
    let thumb = upright.thumbnail(IMAGE_THUMB_MAX_PIXEL, IMAGE_THUMB_MAX_PIXEL);
    Ok(thumb.to_rgba8())
}

/// Cache path for an image-thumbnail `key` (`<key>.thumb.jpg`).
fn thumb_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.thumb.jpg"))
}

/// Cache-integrated image thumbnail generator. Wires [`make_image_thumbnail`]
/// through the E4-S2 cache key (#16) + the **4-wide**
/// [`CacheKind::ImageThumbnail`] gate, persisting each thumbnail as a JPEG.
#[derive(Clone)]
pub struct ImageThumbnailCache {
    dir: PathBuf,
    gates: CacheGates<Option<PathBuf>>,
}

impl ImageThumbnailCache {
    /// Build a cache rooted at `dir` (typically [`crate::cache::media_visual_cache_dir`]).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        ImageThumbnailCache {
            dir: dir.into(),
            gates: CacheGates::new(),
        }
    }

    /// Cache directory this instance writes under.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Generate (or reuse) the thumbnail for `path`, returning its on-disk JPEG
    /// path. Gated at 4 concurrent; concurrent same-key requests share one job.
    /// A cache hit (file already present) skips decode entirely.
    pub async fn generate(&self, path: &Path) -> Result<PathBuf, ImageThumbnailError> {
        let Some(key) = cache_key(path) else {
            // No stable key → generate to a deterministic temp path uncached.
            return self.generate_uncached(path).await;
        };
        let out = thumb_path(&self.dir, &key);
        if out.exists() {
            return Ok(out);
        }

        let dir = self.dir.clone();
        let path = path.to_path_buf();
        let produced = self
            .gates
            .run(CacheKind::ImageThumbnail, &key, || {
                let dir = dir.clone();
                let key = key.clone();
                let path = path.clone();
                async move {
                    tokio::task::spawn_blocking(move || {
                        write_thumbnail(&dir, &key, &path).ok()
                    })
                    .await
                    .ok()
                    .flatten()
                }
            })
            .await;

        produced.ok_or_else(|| {
            ImageThumbnailError::Image("thumbnail generation failed".into())
        })
    }

    async fn generate_uncached(&self, path: &Path) -> Result<PathBuf, ImageThumbnailError> {
        let dir = self.dir.clone();
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || write_thumbnail(&dir, "uncached", &path))
            .await
            .map_err(|e| ImageThumbnailError::Io(e.to_string()))?
    }
}

/// Decode → orient → resize → encode the thumbnail to `<dir>/<key>.thumb.jpg`,
/// returning its path. The synchronous core run under the gate's blocking task.
fn write_thumbnail(dir: &Path, key: &str, path: &Path) -> Result<PathBuf, ImageThumbnailError> {
    std::fs::create_dir_all(dir).map_err(|e| ImageThumbnailError::Io(e.to_string()))?;
    let rgba = make_image_thumbnail(path)?;
    let out = thumb_path(dir, key);
    // Encode as JPEG (RGB; JPEG has no alpha). Convert RGBA→RGB first.
    let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();
    let file = std::fs::File::create(&out).map_err(|e| ImageThumbnailError::Io(e.to_string()))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 85);
    encoder
        .encode_image(&rgb)
        .map_err(|e| ImageThumbnailError::Image(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    /// Build an `w × h` image where pixel (0,0) is a unique marker color so we can
    /// detect rotations/flips by where the marker lands after the transform.
    fn marked(w: u32, h: u32) -> DynamicImage {
        let mut img: RgbaImage = ImageBuffer::from_pixel(w, h, Rgba([0, 0, 0, 255]));
        img.put_pixel(0, 0, Rgba([255, 0, 0, 255])); // top-left red marker
        DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn orientation_6_rotates_90cw_and_swaps_dims() {
        // 4×2 landscape, orientation 6 (rotate 90° CW) → 2×4 portrait.
        let img = marked(4, 2);
        let out = apply_exif_orientation(img, 6);
        assert_eq!((out.width(), out.height()), (2, 4), "rot90 swaps W/H");
    }

    #[test]
    fn orientation_8_rotates_270cw_and_swaps_dims() {
        let img = marked(4, 2);
        let out = apply_exif_orientation(img, 8);
        assert_eq!((out.width(), out.height()), (2, 4));
    }

    #[test]
    fn orientation_1_and_3_keep_dims() {
        let img = marked(4, 2);
        assert_eq!((apply_exif_orientation(img.clone(), 1).width(), apply_exif_orientation(img.clone(), 1).height()), (4, 2));
        // 180° keeps dimensions.
        let r = apply_exif_orientation(img, 3);
        assert_eq!((r.width(), r.height()), (4, 2));
    }

    #[test]
    fn make_thumbnail_landscape_fits_box_no_upscale() {
        use std::io::Write;
        // 300×150 landscape PNG → longest edge clamped to 120 (→ 120×60).
        let img: RgbaImage = ImageBuffer::from_pixel(300, 150, Rgba([20, 40, 60, 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        let mut tmp = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        tmp.write_all(&bytes).unwrap();
        tmp.flush().unwrap();

        let thumb = make_image_thumbnail(tmp.path()).unwrap();
        assert!(thumb.width() <= IMAGE_THUMB_MAX_PIXEL && thumb.height() <= IMAGE_THUMB_MAX_PIXEL);
        assert_eq!(thumb.width(), 120, "longest edge clamps to 120");
        assert_eq!(thumb.height(), 60, "aspect preserved (300:150 = 2:1)");
    }

    #[test]
    fn make_thumbnail_portrait_with_exif_orientation_is_upright_and_small() {
        use std::io::Write;
        // A landscape-stored 240×120 JPEG tagged Orientation=6 (rotate 90° CW)
        // must come out PORTRAIT (taller than wide) and ≤ 120px — the EXIF-aware
        // correction.
        let jpeg = make_jpeg_with_orientation(240, 120, 6);
        let mut tmp = tempfile::Builder::new().suffix(".jpg").tempfile().unwrap();
        tmp.write_all(&jpeg).unwrap();
        tmp.flush().unwrap();

        let thumb = make_image_thumbnail(tmp.path()).unwrap();
        assert!(
            thumb.height() > thumb.width(),
            "orientation-6 landscape source must render upright portrait, got {}×{}",
            thumb.width(),
            thumb.height()
        );
        assert!(thumb.width() <= 120 && thumb.height() <= 120);
    }

    // --- EXIF JPEG fixture builder (mirrors metadata.rs's helper) -----------

    fn make_jpeg_with_orientation(w: u32, h: u32, orientation: u16) -> Vec<u8> {
        use image::{ImageFormat, Rgb};
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(w, h, Rgb([120, 60, 200]));
        let mut base: Vec<u8> = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut base), ImageFormat::Jpeg)
            .unwrap();
        let app1 = build_exif_app1(orientation);
        let mut out = Vec::with_capacity(base.len() + app1.len());
        out.extend_from_slice(&base[..2]); // SOI
        out.extend_from_slice(&app1);
        out.extend_from_slice(&base[2..]);
        out
    }

    fn build_exif_app1(orientation: u16) -> Vec<u8> {
        let mut tiff: Vec<u8> = Vec::new();
        tiff.extend_from_slice(b"MM");
        tiff.extend_from_slice(&0x002A_u16.to_be_bytes());
        tiff.extend_from_slice(&0x0000_0008_u32.to_be_bytes());
        tiff.extend_from_slice(&0x0001_u16.to_be_bytes());
        tiff.extend_from_slice(&0x0112_u16.to_be_bytes()); // Orientation
        tiff.extend_from_slice(&0x0003_u16.to_be_bytes()); // SHORT
        tiff.extend_from_slice(&0x0000_0001_u32.to_be_bytes());
        tiff.extend_from_slice(&orientation.to_be_bytes());
        tiff.extend_from_slice(&[0x00, 0x00]);
        tiff.extend_from_slice(&0x0000_0000_u32.to_be_bytes());
        let mut payload: Vec<u8> = Vec::new();
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(&tiff);
        let mut app1: Vec<u8> = Vec::new();
        app1.extend_from_slice(&[0xFF, 0xE1]);
        let len = (payload.len() + 2) as u16;
        app1.extend_from_slice(&len.to_be_bytes());
        app1.extend_from_slice(&payload);
        app1
    }

    #[tokio::test]
    async fn cache_generate_writes_and_reuses_jpeg() {
        use std::io::Write;
        let cache_dir = tempfile::tempdir().unwrap();
        let img: RgbaImage = ImageBuffer::from_pixel(200, 100, Rgba([10, 200, 30, 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        let mut src = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        src.write_all(&bytes).unwrap();
        src.flush().unwrap();

        let cache = ImageThumbnailCache::new(cache_dir.path());
        let out1 = cache.generate(src.path()).await.unwrap();
        assert!(out1.exists(), "thumbnail jpeg written");
        // Second call hits the cache (same path returned).
        let out2 = cache.generate(src.path()).await.unwrap();
        assert_eq!(out1, out2);
    }
}
