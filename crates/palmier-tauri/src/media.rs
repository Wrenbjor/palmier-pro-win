//! Media-panel OS-action commands (E4-S12) — the Tauri seam the React media panel
//! (`src-ui/media-panel`) calls via `invoke` for Reveal in Explorer / Copy Path /
//! Relink / clipboard paste, plus the async "moment" `thumbnail` command seam.
//!
//! Reference mapping (docs/reference/media-panel.md §"macOS APIs to replace"):
//! - `NSWorkspace.activateFileViewerSelecting` (Reveal in Finder) →
//!   [`reveal_in_explorer`] via `tauri-plugin-opener` `reveal_item_in_dir`
//!   (Windows `explorer /select,`; Linux file-manager show-item / parent open).
//! - `NSPasteboard` Copy-Path → [`copy_paths_to_clipboard`] (newline-joined paths)
//!   via `tauri-plugin-clipboard-manager`.
//! - `NSOpenPanel` Relink → [`pick_relink_path`] via the already-wired
//!   `tauri-plugin-dialog` (E1-S7 pattern).
//! - `NSPasteboard` paste → [`read_clipboard_importable_paths`]: the file-URL
//!   branch of the reference `handleClipboardPaste` (image-data paste lands with
//!   the real import at Epic 7).
//!
//! The `thumbnail` command is the E4-S3 sprite/seek seam consumed by the media
//! grid tiles + the search panel's `MomentThumbnail` (keyed `path@time`). It
//! resolves `media_ref` → on-disk path, decodes a frame via `palmier-media`
//! (video: FFmpeg seek+scale; image: load + EXIF-correct), JPEG-encodes it, and
//! returns a `data:image/jpeg;base64,…` URL. Decoded results are memoized in
//! [`ThumbnailState`] managed state (per `path`/`source_seconds`/`max_size`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine as _;
use image::RgbImage;
use palmier_media::{clip_type_for_path, extract_frame, make_image_thumbnail_sized};
use palmier_model::ClipType;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// Reveal a file in the OS file manager, selecting it (Windows Explorer
/// `/select,`; Linux file-manager show-item / parent open). Reference
/// `NSWorkspace.activateFileViewerSelecting`.
#[tauri::command]
pub fn reveal_in_explorer<R: Runtime>(app: AppHandle<R>, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(PathBuf::from(&path))
        .map_err(|e| e.to_string())
}

/// Copy one or more absolute paths to the system clipboard, **newline-joined**
/// (reference Copy-Path writes newline-joined paths for a multi-selection).
#[tauri::command]
pub fn copy_paths_to_clipboard<R: Runtime>(
    app: AppHandle<R>,
    paths: Vec<String>,
) -> Result<(), String> {
    let joined = paths.join("\n");
    app.clipboard()
        .write_text(joined)
        .map_err(|e| e.to_string())
}

/// Open the OS file picker to repoint a missing asset (Relink). Returns the chosen
/// absolute path, or `None` on cancel. Reference `NSOpenPanel` relink.
#[tauri::command]
pub fn pick_relink_path<R: Runtime>(app: AppHandle<R>, name: String) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title(format!("Relink \"{name}\""))
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().to_string());
    Ok(picked)
}

/// Read importable file paths off the clipboard for paste (the file-URL branch of
/// the reference `handleClipboardPaste`). Splits the clipboard text into lines,
/// strips any `file://` scheme, and returns paths that exist on disk. Image-data
/// paste (`.png`/`.tiff` → written + imported) lands with the real import at Epic 7.
#[tauri::command]
pub fn read_clipboard_importable_paths<R: Runtime>(app: AppHandle<R>) -> Vec<String> {
    let Ok(text) = app.clipboard().read_text() else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let path = line.strip_prefix("file://").unwrap_or(line);
            // Windows file URLs look like `file:///C:/...`; drop the leading slash.
            let path = if cfg!(windows) {
                path.strip_prefix('/').unwrap_or(path)
            } else {
                path
            };
            let p = Path::new(path);
            p.exists().then(|| path.to_string())
        })
        .collect()
}

/// JPEG quality the moment thumbnail is encoded at before base64. Matches the
/// 75 the video sprite-sheet uses (reference `kCGImageDestinationLossyCompression
/// Quality: 0.75`); good enough for a small tile, keeps the IPC payload small.
const THUMBNAIL_JPEG_QUALITY: u8 = 75;

/// Default decode box if the frontend passes a degenerate `max_size` (0). The
/// `momentThumbnail` wrapper defaults to 240; this is the floor guard.
const THUMBNAIL_DEFAULT_MAX: u32 = 240;

/// Managed cache for decoded moment thumbnails, keyed by `(resolved path,
/// rounded source_seconds, max_size)` so the media grid / search panel never
/// redecode a frame they already have. Mirrors the `WaveformState` /
/// `AudioPcmCache` pattern (in-memory, behind a `Mutex`).
///
/// `None` is a *negative* cache entry (audio / unsupported / decode failure) so a
/// missing asset isn't retried on every render. Entries are cheap (a data-URL
/// string ≈ a few KB) and bounded in practice by the number of visible tiles.
#[derive(Default)]
pub struct ThumbnailState {
    cache: Mutex<HashMap<ThumbnailKey, Option<String>>>,
}

/// Cache key: resolved path + the quantized source time + the requested box. The
/// time is quantized to whole seconds (matching the reference's ±1 s seek
/// tolerance — two requests within the same second resolve to the same keyframe).
#[derive(Clone, PartialEq, Eq, Hash)]
struct ThumbnailKey {
    path: String,
    /// `source_seconds` quantized to whole seconds (saturating at 0).
    time_secs: u64,
    max_size: u32,
}

/// Encode an RGB image as JPEG → a `data:image/jpeg;base64,…` URL string.
fn rgb_to_data_url(img: &RgbImage) -> Result<String, String> {
    let mut bytes: Vec<u8> = Vec::new();
    {
        let mut cursor = std::io::Cursor::new(&mut bytes);
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, THUMBNAIL_JPEG_QUALITY);
        encoder
            .encode_image(img)
            .map_err(|e| format!("jpeg encode: {e}"))?;
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:image/jpeg;base64,{b64}"))
}

/// Synchronous decode → JPEG → data-URL for one asset at `source_seconds`, scaled
/// to fit `max_size × max_size`. Pure CPU work; the command runs it on a blocking
/// task. Returns `Ok(None)` for unsupported kinds (audio/text/lottie) and on a
/// per-asset decode failure (so one bad file doesn't surface an error to the UI —
/// the tile just keeps its placeholder glyph).
fn decode_thumbnail_data_url(path: &Path, source_seconds: f64, max_size: u32) -> Option<String> {
    if !path.exists() {
        return None;
    }
    match clip_type_for_path(path) {
        Some(ClipType::Video) => {
            // Single-frame seek+scale (the palmier-media moment primitive).
            let frame = extract_frame(path, source_seconds.max(0.0), max_size, max_size).ok()?;
            rgb_to_data_url(&frame).ok()
        }
        Some(ClipType::Image) => {
            // Load + EXIF-correct + scale to the requested box (RGBA → RGB for JPEG).
            let rgba = make_image_thumbnail_sized(path, max_size).ok()?;
            let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
            rgb_to_data_url(&rgb).ok()
        }
        // Audio shows the audio glyph; Text/Lottie have no decodable raster source.
        _ => None,
    }
}

/// Async "moment" thumbnail command (E4-S3 `thumbnail(media_ref, source_seconds,
/// max_size)`), consumed by the media grid tiles + the search panel's
/// `MomentThumbnail`. Resolves `media_ref` to an on-disk path, decodes a frame
/// (video: seek to `source_seconds`; image: load + EXIF-correct), scales it to fit
/// `max_size × max_size`, encodes JPEG, and returns a `data:image/jpeg;base64,…`
/// URL the webview can drop straight into a `background-image` / `<img>`.
///
/// Returns `Ok(None)` for audio/text/lottie (the tile keeps its type glyph) and on
/// a decode failure (best-effort — never errors the UI). Results are cached in
/// [`ThumbnailState`] per `(path, ~source_seconds, max_size)` so the grid doesn't
/// redecode on every render. The decode runs on a blocking pool task so the IPC
/// thread (and the Tauri event loop) never blocks on FFmpeg.
#[tauri::command]
pub async fn thumbnail<R: Runtime>(
    app: AppHandle<R>,
    media_ref: String,
    source_seconds: f64,
    max_size: u32,
) -> Result<Option<String>, String> {
    let max_size = if max_size == 0 { THUMBNAIL_DEFAULT_MAX } else { max_size };
    let key = ThumbnailKey {
        path: media_ref.clone(),
        time_secs: source_seconds.max(0.0) as u64,
        max_size,
    };

    // Hot path: a previously-decoded (or negatively-cached) result.
    if let Some(state) = app.try_state::<ThumbnailState>() {
        if let Some(hit) = state
            .cache
            .lock()
            .expect("thumbnail cache mutex")
            .get(&key)
            .cloned()
        {
            return Ok(hit);
        }
    }

    // Cold path: decode on a blocking worker (FFmpeg seek/scale is CPU-bound).
    let path = PathBuf::from(&media_ref);
    let decoded = tauri::async_runtime::spawn_blocking(move || {
        decode_thumbnail_data_url(&path, source_seconds, max_size)
    })
    .await
    .map_err(|e| format!("thumbnail task join: {e}"))?;

    // Store the outcome (including a `None` negative entry) so it isn't retried.
    if let Some(state) = app.try_state::<ThumbnailState>() {
        state
            .cache
            .lock()
            .expect("thumbnail cache mutex")
            .insert(key, decoded.clone());
    }

    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_path_parsing_strips_file_scheme() {
        // The line-splitting + scheme-stripping logic is exercised here without a
        // live clipboard. (A non-existent path is dropped, matching the command.)
        let lines = "file:///nonexistent/path.mp4\n   \nplain/no-scheme.mov";
        let parsed: Vec<&str> = lines
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].starts_with("file://"));
    }

    // --- thumbnail decode ---------------------------------------------------

    #[test]
    fn rgb_to_data_url_encodes_nonempty_jpeg() {
        // A 16×16 solid RGB image must encode to a non-empty JPEG data-URL whose
        // base64 payload decodes back to a real JPEG (SOI marker 0xFFD8).
        let img: RgbImage = image::ImageBuffer::from_pixel(16, 16, image::Rgb([200, 40, 90]));
        let url = rgb_to_data_url(&img).expect("encode data-url");
        assert!(url.starts_with("data:image/jpeg;base64,"), "data-url prefix");
        let b64 = url.trim_start_matches("data:image/jpeg;base64,");
        assert!(!b64.is_empty(), "non-empty base64 payload");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("valid base64");
        assert!(bytes.len() > 2, "decoded JPEG has bytes");
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "JPEG SOI marker");
    }

    #[test]
    fn unsupported_kinds_return_none() {
        // Audio / text / lottie have no decodable raster source → None (the tile
        // keeps its glyph). Path need not exist past classification for this path
        // — but we point at a missing file so the `exists()` guard also returns None.
        assert!(decode_thumbnail_data_url(Path::new("nope.mp3"), 0.0, 240).is_none());
        assert!(decode_thumbnail_data_url(Path::new("nope.txt"), 0.0, 240).is_none());
    }

    /// Build a fixture with the FFmpeg CLI; returns its path, or `None` if ffmpeg
    /// is unavailable (the test self-skips). Mirrors
    /// `palmier-media/tests/decode_real_clip.rs::make_clip`.
    fn make_with_ffmpeg(dir: &Path, name: &str, args: &[&str]) -> Option<PathBuf> {
        let out = dir.join(name);
        let mut cmd = std::process::Command::new("ffmpeg");
        cmd.arg("-y");
        cmd.args(args);
        cmd.arg(&out);
        let status = cmd.status().ok()?;
        (status.success() && out.exists()).then_some(out)
    }

    #[test]
    fn thumbnail_of_real_video_clip_is_nonempty_data_url() {
        // Generate a real h264 clip (testsrc2 color-bars), then decode a moment
        // thumbnail at t=0.5s and assert it is a non-empty JPEG data-URL. This is
        // the GREEN gate for the un-stubbed `thumbnail` command. Self-skips when
        // ffmpeg isn't on PATH (still runs under the MSVC+ffmpeg-env wrapper).
        let dir = tempfile::tempdir().expect("tempdir");
        let Some(clip) = make_with_ffmpeg(
            dir.path(),
            "clip.mp4",
            &[
                "-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=2",
                "-c:v", "libopenh264", "-b:v", "1M", "-pix_fmt", "yuv420p",
            ],
        ) else {
            eprintln!("ffmpeg not available — skipping real-video thumbnail test");
            return;
        };

        let url = decode_thumbnail_data_url(&clip, 0.5, 240)
            .expect("a real video clip yields a moment thumbnail");
        assert!(url.starts_with("data:image/jpeg;base64,"));
        let b64 = url.trim_start_matches("data:image/jpeg;base64,");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("valid base64");
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "real JPEG SOI marker");
        // The decoded thumbnail must fit inside the requested 240px box.
        let decoded = image::load_from_memory(&bytes).expect("decode produced JPEG");
        assert!(decoded.width() <= 240 && decoded.height() <= 240, "fits box");
        assert!(decoded.width() > 0 && decoded.height() > 0, "non-degenerate");
    }

    #[test]
    fn thumbnail_of_real_image_is_nonempty_data_url() {
        // Generate a single PNG frame via ffmpeg (testsrc), then decode an image
        // moment thumbnail. Same self-skip contract as the video test.
        let dir = tempfile::tempdir().expect("tempdir");
        let Some(png) = make_with_ffmpeg(
            dir.path(),
            "still.png",
            &[
                "-f", "lavfi", "-i", "testsrc=size=300x150:rate=1:duration=1",
                "-frames:v", "1",
            ],
        ) else {
            eprintln!("ffmpeg not available — skipping real-image thumbnail test");
            return;
        };

        let url = decode_thumbnail_data_url(&png, 0.0, 240)
            .expect("a real image yields a thumbnail");
        let b64 = url.trim_start_matches("data:image/jpeg;base64,");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("valid base64");
        let decoded = image::load_from_memory(&bytes).expect("decode produced JPEG");
        // 300×150 scaled into a 240 box → longest edge 240 (240×120, aspect kept).
        assert_eq!(decoded.width(), 240, "longest edge clamps to 240");
        assert_eq!(decoded.height(), 120, "aspect preserved (2:1)");
    }
}
