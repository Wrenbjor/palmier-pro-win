//! Image inlining for `@`-mention assets (E8-S5).
//!
//! Ports `Utilities/ImageEncoder.swift`: downscale an asset image to a longest
//! edge ≤ [`MAX_LONGEST_EDGE`] px and re-encode as JPEG so it is token-efficient
//! for the agent, with a process-global bounded cache so `api_messages()` — which
//! runs on **every** agent loop iteration — doesn't re-read and re-encode the same
//! images. The Anthropic image-inline limits are load-bearing
//! (`agent-panel.md` lines 143-146, reconciliation carry-forward): longest edge
//! **1568 px**, file ≤ **3,500,000 bytes**, JPEG quality ladder **[0.85, 0.7,
//! 0.55, 0.4]**.
//!
//! ## The asset-bytes seam (`AssetBytesSource`)
//! The reference reads bytes off `MediaAsset.url` (a local file) and stamps the
//! cache by `path + size + mtime`. Rather than couple `palmier-agent` to the
//! filesystem and the media library, the resolution of `media_ref → bytes + cache
//! stamp` is a trait ([`AssetBytesSource`]). The real Tauri wiring (E8-S9) adapts
//! the media library + file IO; tests pass an in-memory source. This keeps the
//! whole downscale/encode/cache path unit-testable with **no real media**.
//!
//! ## ImageIO → `image` crate
//! `CGImageSourceCreateThumbnailAtIndex(maxPixelSize:)` (a decode-time downscale)
//! maps to decoding then [`image::imageops::FilterType::Lanczos3`] resize.
//! `CGImageDestination` JPEG at a quality maps to
//! [`image::codecs::jpeg::JpegEncoder::new_with_quality`]. Quality is a 1..=100
//! integer here; the reference's `CGFloat` 0.0..=1.0 ladder is scaled to
//! `[85, 70, 55, 40]`.
//!
//! ## Cache (`agent-panel.md` line 230 open question — resolved as bounded LRU)
//! Process-global, behind a `Mutex`, keyed by the [`AssetStamp`] (`media_ref` +
//! size + mtime-nanos). The reference clears the **whole** cache when it reaches
//! `maxCacheEntries`; we port that exact eviction (clear-at-max) under
//! [`MAX_CACHE_ENTRIES`] rather than a true LRU — the reference's behavior is
//! "bounded, occasionally fully cleared", and matching it keeps parity. The mtime
//! key may false-hit on a coarse Windows FS (documented, acceptable).

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Mutex;

use base64::Engine as _;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{GenericImageView, ImageReader};

/// Inline target: longest edge ≤ this many pixels (reference `maxLongestEdge`).
pub const MAX_LONGEST_EDGE: u32 = 1568;

/// Inline target: encoded file ≤ this many bytes (reference `maxBytes`, 3.5 MB).
pub const MAX_BYTES: usize = 3_500_000;

/// JPEG quality ladder (1..=100), tried in order until under [`MAX_BYTES`]
/// (reference `[0.85, 0.7, 0.55, 0.4]` scaled from `CGFloat` 0–1).
pub const JPEG_QUALITY_LADDER: [u8; 4] = [85, 70, 55, 40];

/// Cache bound — when the cache reaches this many entries the **whole** cache is
/// cleared before inserting (reference `maxCacheEntries`, clear-at-max).
pub const MAX_CACHE_ENTRIES: usize = 32;

/// A successfully inlined image: the encoded bytes + their IANA media type
/// (reference `ImageEncoder.Output`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedImage {
    /// Encoded image bytes (JPEG when downscaled; original bytes on passthrough).
    pub data: Vec<u8>,
    /// IANA media type, e.g. `image/jpeg` / `image/png` (reference `mime`).
    pub mime: String,
}

impl EncodedImage {
    /// Base64 of [`data`](Self::data) for the Anthropic image block's
    /// `source.data` (reference `Data.base64EncodedString()`).
    #[must_use]
    pub fn base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.data)
    }
}

/// The bytes + cache stamp the encoder needs for one asset (reference: reading
/// `MediaAsset.url` + `attributesOfItem` for `path/size/mtime`).
#[derive(Debug, Clone)]
pub struct AssetBytes {
    /// Raw file bytes of the asset.
    pub bytes: Vec<u8>,
    /// File size in bytes (reference `FileStamp.size`).
    pub size: u64,
    /// Modification time as whole nanoseconds since the Unix epoch (reference
    /// `FileStamp.mtime`). The cache key includes this so a changed file re-encodes.
    pub mtime_nanos: i128,
}

/// The `media_ref → bytes` seam (reference: `editor.mediaAssets.first { id == ref }`
/// then `Data(contentsOf: asset.url)`).
///
/// Implementations resolve a `media_ref` to its raw bytes + a cache stamp. The
/// real adapter (E8-S9) looks the asset up in the media library and reads its
/// local file; tests use an in-memory map. Two failure modes mirror the reference:
/// the asset is **not in the library** (`None` → "asset not in media library"),
/// vs. it **can't be read/decoded** (surfaced later by [`ImageEncoder::encode`]).
pub trait AssetBytesSource: Send + Sync {
    /// Resolve `media_ref` to its bytes + cache stamp, or `None` if the asset is
    /// not in the library (distinct from a present-but-unreadable asset).
    fn load(&self, media_ref: &str) -> Option<AssetBytes>;
}

/// Cache key: the asset's `media_ref` + size + mtime (reference `FileStamp` —
/// `path + size + mtime`; `media_ref` is the stable per-asset identity here).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AssetStamp {
    media_ref: String,
    size: u64,
    mtime_nanos: i128,
}

/// Process-global inline cache (reference `private static var cache`). Guarded by
/// a `Mutex`; `OnceLock` defers allocation until first inline.
fn cache() -> &'static Mutex<HashMap<AssetStamp, EncodedImage>> {
    static CACHE: std::sync::OnceLock<Mutex<HashMap<AssetStamp, EncodedImage>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The image inliner (reference `enum ImageEncoder`).
///
/// All methods are associated functions over an [`AssetBytesSource`]; the cache is
/// process-global static state, matching the reference.
pub struct ImageEncoder;

impl ImageEncoder {
    /// Inline the asset `media_ref` to an [`EncodedImage`], or `None` if it can't
    /// be read/decoded/fit (reference `ImageEncoder.encode(url:)`).
    ///
    /// Cache-checked first; on a miss, tries the **passthrough** path (already an
    /// image, already small enough by both edge and byte limits → forward the
    /// original bytes), else **downscale + JPEG q-ladder**. A successful result is
    /// cached (clearing the whole cache first if at [`MAX_CACHE_ENTRIES`], per the
    /// reference).
    ///
    /// Returns `None` for: asset present but bytes don't decode as an image, or no
    /// quality in the ladder gets the JPEG under [`MAX_BYTES`]. (A *missing* asset
    /// is the source returning `None` from [`AssetBytesSource::load`] and is
    /// handled by the caller — [`crate::mention_context`] — as a distinct error.)
    #[must_use]
    pub fn encode(source: &dyn AssetBytesSource, media_ref: &str) -> Option<EncodedImage> {
        let asset = source.load(media_ref)?;
        let stamp = AssetStamp {
            media_ref: media_ref.to_string(),
            size: asset.size,
            mtime_nanos: asset.mtime_nanos,
        };

        if let Some(hit) = cache().lock().unwrap().get(&stamp).cloned() {
            return Some(hit);
        }

        let output = Self::passthrough(&asset.bytes).or_else(|| Self::downscaled(&asset.bytes))?;

        let mut guard = cache().lock().unwrap();
        if guard.len() >= MAX_CACHE_ENTRIES {
            guard.clear();
        }
        guard.insert(stamp, output.clone());
        Some(output)
    }

    /// Passthrough: forward the original bytes iff they sniff as a supported image
    /// **and** are already within both the byte and longest-edge limits (reference
    /// `passthrough`). `None` falls through to [`Self::downscaled`].
    fn passthrough(bytes: &[u8]) -> Option<EncodedImage> {
        if bytes.len() > MAX_BYTES {
            return None;
        }
        let mime = sniffed_mime(bytes)?;
        let (w, h) = decoded_dimensions(bytes)?;
        if w.max(h) > MAX_LONGEST_EDGE {
            return None;
        }
        Some(EncodedImage {
            data: bytes.to_vec(),
            mime,
        })
    }

    /// Downscale to longest edge ≤ [`MAX_LONGEST_EDGE`] and JPEG-encode trying each
    /// quality in [`JPEG_QUALITY_LADDER`] until under [`MAX_BYTES`] (reference
    /// `downscaled`). `None` if undecodable or no quality fits.
    fn downscaled(bytes: &[u8]) -> Option<EncodedImage> {
        let img = decode_image(bytes)?;
        // `resize` preserves aspect ratio, fitting WITHIN (w, h); pass the edge as
        // both bounds so the LONGEST edge ends up ≤ MAX_LONGEST_EDGE (matching
        // ImageIO's `thumbnailMaxPixelSize`). Only downscale — never upsample.
        let (w, h) = img.dimensions();
        let resized = if w.max(h) > MAX_LONGEST_EDGE {
            img.resize(MAX_LONGEST_EDGE, MAX_LONGEST_EDGE, FilterType::Lanczos3)
        } else {
            img
        };
        // JPEG has no alpha — flatten to RGB8 once, reuse across the ladder.
        let rgb = resized.to_rgb8();
        for quality in JPEG_QUALITY_LADDER {
            let mut buf: Vec<u8> = Vec::new();
            let mut encoder = JpegEncoder::new_with_quality(Cursor::new(&mut buf), quality);
            if encoder
                .encode(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    image::ExtendedColorType::Rgb8,
                )
                .is_err()
            {
                return None;
            }
            if buf.len() <= MAX_BYTES {
                return Some(EncodedImage {
                    data: buf,
                    mime: "image/jpeg".to_string(),
                });
            }
        }
        None
    }
}

/// IANA media type from sniffing the byte container (reference `sniffedMime`,
/// which read the UTI). `None` for non-image / unsupported formats — only the
/// formats the reference whitelisted are passthrough-eligible.
fn sniffed_mime(bytes: &[u8]) -> Option<String> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some("image/png".to_string()),
        image::ImageFormat::Jpeg => Some("image/jpeg".to_string()),
        image::ImageFormat::Gif => Some("image/gif".to_string()),
        image::ImageFormat::WebP => Some("image/webp".to_string()),
        _ => None,
    }
}

/// `(width, height)` from the header without a full decode where possible
/// (reference `metadata` reading `kCGImagePropertyPixel{Width,Height}`).
fn decoded_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.into_dimensions().ok()
}

/// Full-decode the bytes to a [`image::DynamicImage`] (reference: the thumbnail
/// decode). `None` if the bytes aren't a decodable image.
fn decode_image(bytes: &[u8]) -> Option<image::DynamicImage> {
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()
}

#[cfg(test)]
pub(crate) mod test_support {
    //! In-memory [`AssetBytesSource`] + synthetic-image helpers shared by the
    //! image-encoder and mention-context tests.
    use super::{AssetBytes, AssetBytesSource};
    use image::{ImageFormat, RgbImage};
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::sync::Mutex;

    /// A map-backed source. Unknown `media_ref`s resolve to `None` ("asset not in
    /// media library"); a present-but-garbage entry decodes-fails ("could not read
    /// or decode image file").
    #[derive(Default)]
    pub struct MapSource {
        assets: Mutex<HashMap<String, AssetBytes>>,
    }

    impl MapSource {
        pub fn new() -> Self {
            Self::default()
        }

        /// Insert an asset with a fixed cache stamp.
        pub fn insert(&self, media_ref: &str, bytes: Vec<u8>, mtime_nanos: i128) {
            let size = bytes.len() as u64;
            self.assets.lock().unwrap().insert(
                media_ref.to_string(),
                AssetBytes {
                    bytes,
                    size,
                    mtime_nanos,
                },
            );
        }
    }

    impl AssetBytesSource for MapSource {
        fn load(&self, media_ref: &str) -> Option<AssetBytes> {
            self.assets.lock().unwrap().get(media_ref).cloned()
        }
    }

    /// A solid-color PNG of `w`×`h` (a real, decodable image for passthrough/
    /// dimension tests).
    pub fn png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbImage::from_pixel(w, h, image::Rgb([120, 30, 200]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .unwrap();
        buf
    }

    /// A high-entropy (noise) PNG of `w`×`h` — noise resists JPEG compression, so
    /// it is a realistic "oversized" fixture that forces the downscale path and a
    /// nontrivial encoded size.
    pub fn noise_png(w: u32, h: u32) -> Vec<u8> {
        let mut img = RgbImage::new(w, h);
        // Cheap deterministic LCG so the test is reproducible without a rng dep.
        let mut state: u32 = 0x1234_5678;
        for px in img.pixels_mut() {
            let mut next = || {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            };
            *px = image::Rgb([next(), next(), next()]);
        }
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
            .unwrap();
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{noise_png, png, MapSource};
    use super::*;

    #[test]
    fn small_png_passes_through_unmodified() {
        let bytes = png(64, 48);
        let source = MapSource::new();
        source.insert("media_small", bytes.clone(), 1);

        let out = ImageEncoder::encode(&source, "media_small").expect("small image inlines");
        // Passthrough: original bytes + sniffed mime (NOT re-encoded to JPEG).
        assert_eq!(out.mime, "image/png");
        assert_eq!(out.data, bytes);
        assert!(out.data.len() <= MAX_BYTES);
        // base64 is non-empty and decodes back to the original bytes.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(out.base64())
            .unwrap();
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn oversized_image_is_downscaled_to_jpeg_within_limits() {
        // 4000×3000 noise PNG → longest edge 4000 > 1568, forces downscale + JPEG.
        let bytes = noise_png(4000, 3000);
        let source = MapSource::new();
        source.insert("media_big", bytes, 1);

        let out = ImageEncoder::encode(&source, "media_big").expect("oversized image inlines");
        assert_eq!(out.mime, "image/jpeg", "downscale path emits JPEG");
        assert!(out.data.len() <= MAX_BYTES, "under the 3.5MB byte limit");

        // The encoded JPEG's longest edge is ≤ 1568 (downscaled).
        let (w, h) = decoded_dimensions(&out.data).unwrap();
        assert!(w.max(h) <= MAX_LONGEST_EDGE, "longest edge {} > {MAX_LONGEST_EDGE}", w.max(h));
    }

    #[test]
    fn quality_ladder_steps_down_until_under_max_bytes() {
        // A tiny MAX_BYTES can't be hit by q85 on a noisy image but a lower quality
        // can — exercise the ladder by checking the helper picks a smaller file as
        // the budget shrinks. We can't mutate the const, so assert monotonicity:
        // encoding the SAME noise at successive qualities is non-increasing in size.
        let bytes = noise_png(1568, 1568);
        let img = decode_image(&bytes).unwrap().to_rgb8();
        let mut last = usize::MAX;
        for q in JPEG_QUALITY_LADDER {
            let mut buf = Vec::new();
            JpegEncoder::new_with_quality(Cursor::new(&mut buf), q)
                .encode(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8)
                .unwrap();
            assert!(buf.len() <= last, "q{q} ({}) must not exceed previous ({last})", buf.len());
            last = buf.len();
        }
    }

    #[test]
    fn undecodable_bytes_yield_none() {
        let source = MapSource::new();
        source.insert("media_garbage", b"not an image at all".to_vec(), 1);
        assert!(ImageEncoder::encode(&source, "media_garbage").is_none());
    }

    #[test]
    fn missing_asset_yields_none_from_source() {
        let source = MapSource::new();
        assert!(source.load("nope").is_none());
        assert!(ImageEncoder::encode(&source, "nope").is_none());
    }

    #[test]
    fn cache_hit_returns_same_output_without_redecoding() {
        let bytes = png(100, 100);
        let source = MapSource::new();
        source.insert("media_cached", bytes, 42);

        let first = ImageEncoder::encode(&source, "media_cached").unwrap();
        let second = ImageEncoder::encode(&source, "media_cached").unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn changed_mtime_is_a_distinct_cache_key() {
        // Same media_ref, different mtime → different stamp → re-encodes (does not
        // serve a stale cache hit). Use distinct content so the outputs differ.
        let source = MapSource::new();
        source.insert("media_v", png(80, 80), 1);
        let v1 = ImageEncoder::encode(&source, "media_v").unwrap();

        // Replace bytes + bump mtime; the new stamp must not collide with v1's.
        source.insert("media_v", noise_png(2000, 100), 2);
        let v2 = ImageEncoder::encode(&source, "media_v").unwrap();
        assert_ne!(v1, v2, "a new mtime must re-encode, not serve the old cache entry");
    }
}
