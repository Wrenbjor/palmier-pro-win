//! Video sprite-sheet thumbnail pipeline (story E4-S3) via **ffmpeg-next**.
//!
//! Port of `MediaVisualCache.generateVideoThumbnails` +
//! `saveThumbnails`/`loadThumbnails` (`Sources/PalmierPro/Timeline/MediaVisualCache.swift`).
//! The macOS reference drives `AVAssetImageGenerator`; we replace it with
//! `ffmpeg-next` seek + `swscale` to the **120×68** box (per
//! `docs/reference/media-panel.md` §"macOS/Apple APIs to replace").
//!
//! ## What this module owns
//! * [`extract_frame`] — single-frame seek+scale (`thumbnail(media_ref,
//!   source_seconds, max_size)` style; the Tauri "moment" command for Epic 11's
//!   search panel routes through this).
//! * [`generate_sprite_sheet`] — the full strip: compute the sampling times
//!   ([`super::times::video_thumbnail_times`]), extract each frame, assemble ONE
//!   JPEG sprite-sheet (≤ 50 cols, quality 0.75) + a `.thumbs.json` sidecar
//!   written **last** (the completion marker). Frames are published every **50**
//!   via an optional progress callback so the UI can render a partial strip
//!   without blocking import (the reference's progressive publish).
//! * [`VideoThumbnailCache`] — wires the above through the E4-S2 cache key
//!   (#16) + the **ungated** `VideoThumbnail` gate.
//!
//! ## Cache integration (#16)
//! On-disk layout under the `MediaVisualCache` dir mirrors the reference:
//! `<key>.thumbs.jpg` (sprite) + `<key>.thumbs.json` (sidecar). `<key>` is the
//! E4-S2 [`crate::cache::cache_key`] (`sha256(path|size|mtime).prefix16`), so a
//! source edit yields a new key and misses the stale entry.

use std::path::{Path, PathBuf};

use image::{ImageBuffer, Rgb, RgbImage};

use super::sprite::{sprite_grid, tile_origin, ThumbnailSidecar};
use super::times::{video_thumbnail_times, THUMB_MAX_HEIGHT, THUMB_MAX_WIDTH};
use crate::cache::{cache_key, CacheGates, CacheKind};

/// JPEG quality the reference encodes the sprite at
/// (`kCGImageDestinationLossyCompressionQuality: 0.75`). The `image` crate JPEG
/// encoder takes 1–100, so 0.75 → **75**.
pub const SPRITE_JPEG_QUALITY: u8 = 75;

/// Progressive-publish boundary: the reference republishes the partial strip
/// every `results.count % 50 == 0`. Callers pass a callback invoked at each
/// boundary with the frames decoded so far.
pub const PROGRESSIVE_PUBLISH_EVERY: usize = 50;

/// One extracted thumbnail frame: its source-seconds timestamp + the scaled RGB
/// pixels (already fit inside the 120×68 box).
#[derive(Debug, Clone)]
pub struct ThumbnailFrame {
    /// Source time in seconds this frame was sampled at.
    pub time: f64,
    /// The decoded, downscaled frame.
    pub image: RgbImage,
}

/// Errors the video thumbnail pipeline can surface.
#[derive(Debug)]
pub enum VideoThumbnailError {
    /// ffmpeg open/decode/scale failure.
    Ffmpeg(String),
    /// No decodable video stream in the container.
    NoVideoStream,
    /// I/O writing the sprite/sidecar (or reading cache).
    Io(String),
    /// Image encode/decode failure.
    Image(String),
}

impl std::fmt::Display for VideoThumbnailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoThumbnailError::Ffmpeg(m) => write!(f, "ffmpeg: {m}"),
            VideoThumbnailError::NoVideoStream => write!(f, "no video stream"),
            VideoThumbnailError::Io(m) => write!(f, "io: {m}"),
            VideoThumbnailError::Image(m) => write!(f, "image: {m}"),
        }
    }
}

impl std::error::Error for VideoThumbnailError {}

impl From<ffmpeg_next::Error> for VideoThumbnailError {
    fn from(e: ffmpeg_next::Error) -> Self {
        VideoThumbnailError::Ffmpeg(e.to_string())
    }
}

/// Scale `(src_w, src_h)` to fit inside the `max_w × max_h` box, preserving
/// aspect ratio (the reference `AVAssetImageGenerator.maximumSize` semantics).
/// Never upscales past the source; clamps every dimension to ≥ 1.
pub(crate) fn fit_within(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (1, 1);
    }
    let scale = f64::min(
        max_w as f64 / src_w as f64,
        max_h as f64 / src_h as f64,
    )
    .min(1.0); // don't upscale
    let w = ((src_w as f64 * scale).round() as u32).max(1);
    let h = ((src_h as f64 * scale).round() as u32).max(1);
    (w, h)
}

/// Sprite + sidecar paths for a cache `key` in `dir` (`<key>.thumbs.jpg` /
/// `<key>.thumbs.json`).
fn sprite_paths(dir: &Path, key: &str) -> (PathBuf, PathBuf) {
    (
        dir.join(format!("{key}.thumbs.jpg")),
        dir.join(format!("{key}.thumbs.json")),
    )
}

// -- ffmpeg-backed extraction ------------------------------------------------

/// Ensure ffmpeg is initialized exactly once (idempotent; `ffmpeg::init` is
/// safe to call repeatedly but we keep it cheap).
fn ensure_ffmpeg_init() -> Result<(), VideoThumbnailError> {
    use std::sync::Once;
    static INIT: Once = Once::new();
    let mut err: Option<String> = None;
    INIT.call_once(|| {
        if let Err(e) = ffmpeg_next::init() {
            err = Some(e.to_string());
        }
    });
    match err {
        Some(e) => Err(VideoThumbnailError::Ffmpeg(e)),
        None => Ok(()),
    }
}

/// Extract a single frame at `source_seconds`, scaled to fit inside
/// `max_w × max_h`. This is the core of the `thumbnail(media_ref, source_seconds,
/// max_size)` Tauri command (Epic 11 moment thumbnails) — small, synchronous,
/// CPU-bound; callers run it on a blocking pool.
///
/// Seeks to `source_seconds` (ffmpeg seeks to the nearest keyframe at/under the
/// target, matching the reference's ±1.0 s tolerance window), then decodes the
/// first frame at/after the seek point and scales it to RGB24.
pub fn extract_frame(
    path: &Path,
    source_seconds: f64,
    max_w: u32,
    max_h: u32,
) -> Result<RgbImage, VideoThumbnailError> {
    extract_frame_timed(path, source_seconds, max_w, max_h).map(|f| f.image)
}

/// One decoded frame plus the **actual** presentation time it landed on.
///
/// `extract_frame` returns just the image; the frame **sampler** (Epic 11
/// `FrameSampler`) also needs the decoded frame's true PTS so it can drop
/// non-monotonic snaps — two nearby candidate times can seek to the *same*
/// keyframe, and the reference skips any frame whose `actualTime` is ≤ the
/// previous one. `time` is in source seconds, derived from the frame PTS ×
/// stream time-base; it falls back to the requested seek time if the decoder
/// reports no PTS.
#[derive(Debug, Clone)]
pub struct TimedFrame {
    /// Actual presentation time of the decoded frame, in source seconds.
    pub time: f64,
    /// The decoded, downscaled RGB frame.
    pub image: RgbImage,
}

/// Like [`extract_frame`] but also reports the decoded frame's **actual**
/// presentation time (source seconds). Same single FFmpeg seek+decode+scale
/// path — this is the timed primitive `FrameSampler` (Epic 11) drives so it can
/// apply the "skip frames whose actualTime ≤ previous actualTime" rule from the
/// reference `FrameSampler.sample`.
pub fn extract_frame_timed(
    path: &Path,
    source_seconds: f64,
    max_w: u32,
    max_h: u32,
) -> Result<TimedFrame, VideoThumbnailError> {
    ensure_ffmpeg_init()?;

    let mut ictx = ffmpeg_next::format::input(&path)?;
    let stream_index = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .map(|s| s.index())
        .ok_or(VideoThumbnailError::NoVideoStream)?;

    // Decoder for the video stream.
    let (time_base, mut decoder) = {
        let stream = ictx
            .stream(stream_index)
            .ok_or(VideoThumbnailError::NoVideoStream)?;
        let ctx = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())?;
        let decoder = ctx.decoder().video()?;
        (stream.time_base(), decoder)
    };
    // Seconds-per-PTS-tick for this stream (0 if the container reports a
    // degenerate time-base — then we fall back to the requested time).
    let tb_secs = f64::from(time_base.numerator()) / f64::from(time_base.denominator().max(1));

    // Seek to the requested time. ffmpeg `seek` works in AV_TIME_BASE units
    // (microseconds) over the whole file; seek to slightly before the target so
    // the first decoded frame is at/after it (the ±1s tolerance is generous).
    let target_secs = source_seconds.max(0.0);
    let ts = (target_secs * f64::from(ffmpeg_next::ffi::AV_TIME_BASE)) as i64;
    // Range end = i64::MAX so we land on the first keyframe ≤ target.
    let _ = ictx.seek(ts, ..ts);
    decoder.flush();

    let mut scaler: Option<ffmpeg_next::software::scaling::Context> = None;
    let (dst_w, dst_h) = fit_within(decoder.width(), decoder.height(), max_w, max_h);

    // Resolve a decoded frame's actual time: prefer its PTS, else the requested
    // seek time (some streams don't carry PTS on every frame).
    let frame_time = |decoded: &ffmpeg_next::frame::Video| -> f64 {
        match decoded.timestamp() {
            Some(pts) if tb_secs > 0.0 => pts as f64 * tb_secs,
            _ => target_secs,
        }
    };

    let send_and_collect =
        |decoder: &mut ffmpeg_next::decoder::Video,
         scaler: &mut Option<ffmpeg_next::software::scaling::Context>|
         -> Result<Option<TimedFrame>, VideoThumbnailError> {
            let mut decoded = ffmpeg_next::frame::Video::empty();
            while decoder.receive_frame(&mut decoded).is_ok() {
                // Lazily build the scaler now that we know the source pixel format.
                if scaler.is_none() {
                    *scaler = Some(ffmpeg_next::software::scaling::Context::get(
                        decoded.format(),
                        decoded.width(),
                        decoded.height(),
                        ffmpeg_next::format::Pixel::RGB24,
                        dst_w,
                        dst_h,
                        ffmpeg_next::software::scaling::Flags::BILINEAR,
                    )?);
                }
                let sc = scaler.as_mut().unwrap();
                let mut rgb = ffmpeg_next::frame::Video::empty();
                sc.run(&decoded, &mut rgb)?;
                if let Some(img) = rgb_frame_to_image(&rgb, dst_w, dst_h) {
                    return Ok(Some(TimedFrame {
                        time: frame_time(&decoded),
                        image: img,
                    }));
                }
            }
            Ok(None)
        };

    for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index {
            continue;
        }
        decoder.send_packet(&packet)?;
        if let Some(frame) = send_and_collect(&mut decoder, &mut scaler)? {
            return Ok(frame);
        }
    }
    // Drain.
    decoder.send_eof()?;
    if let Some(frame) = send_and_collect(&mut decoder, &mut scaler)? {
        return Ok(frame);
    }

    Err(VideoThumbnailError::Ffmpeg(
        "no frame decoded at requested time".into(),
    ))
}

/// Copy an RGB24 ffmpeg frame (which has row padding / `stride`) into a tightly
/// packed [`RgbImage`].
fn rgb_frame_to_image(frame: &ffmpeg_next::frame::Video, w: u32, h: u32) -> Option<RgbImage> {
    if w == 0 || h == 0 {
        return None;
    }
    let stride = frame.stride(0);
    let data = frame.data(0);
    let row_bytes = (w * 3) as usize;
    let mut buf = Vec::with_capacity(row_bytes * h as usize);
    for y in 0..h as usize {
        let start = y * stride;
        let end = start + row_bytes;
        if end > data.len() {
            return None;
        }
        buf.extend_from_slice(&data[start..end]);
    }
    ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(w, h, buf)
}

/// Extract the full strip of frames for `path`, sampling at
/// [`video_thumbnail_times`]. `on_progress`, if given, is called every
/// [`PROGRESSIVE_PUBLISH_EVERY`] frames with the frames decoded so far (the
/// reference's progressive publish). Returns all frames in time order.
///
/// A frame that fails to extract (seek past EOF, corrupt GOP) is skipped rather
/// than aborting the strip — matching the reference, which only appends
/// `.success` results.
pub fn extract_strip(
    path: &Path,
    duration: f64,
    mut on_progress: impl FnMut(&[ThumbnailFrame]),
) -> Result<Vec<ThumbnailFrame>, VideoThumbnailError> {
    let times = video_thumbnail_times(duration);
    let mut frames: Vec<ThumbnailFrame> = Vec::with_capacity(times.len());
    for t in times {
        match extract_frame(path, t, THUMB_MAX_WIDTH, THUMB_MAX_HEIGHT) {
            Ok(image) => {
                frames.push(ThumbnailFrame { time: t, image });
                if !frames.is_empty() && frames.len().is_multiple_of(PROGRESSIVE_PUBLISH_EVERY) {
                    on_progress(&frames);
                }
            }
            Err(VideoThumbnailError::NoVideoStream) => {
                return Err(VideoThumbnailError::NoVideoStream)
            }
            // Per-frame extraction failures are skipped (reference appends only
            // successful results); keep going so a single bad GOP doesn't kill
            // the strip.
            Err(_) => continue,
        }
    }
    frames.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(frames)
}

// -- sprite assembly + sidecar ----------------------------------------------

/// Assemble `frames` into ONE sprite-sheet [`RgbImage`] (≤ 50 columns) + the
/// matching [`ThumbnailSidecar`]. Tile size is the first frame's dimensions; a
/// frame larger than the tile is cropped to fit, a smaller one is placed at the
/// cell origin (the reference uses a fixed tile box from the first frame).
///
/// Returns `None` for an empty `frames` (nothing to write).
pub fn assemble_sprite(frames: &[ThumbnailFrame]) -> Option<(RgbImage, ThumbnailSidecar)> {
    let first = frames.first()?;
    let tile_w = first.image.width();
    let tile_h = first.image.height();
    if tile_w == 0 || tile_h == 0 {
        return None;
    }
    let (columns, rows) = sprite_grid(frames.len());
    let sheet_w = columns as u32 * tile_w;
    let sheet_h = rows as u32 * tile_h;

    // Black background (the reference draws onto an opaque context).
    let mut sheet: RgbImage = ImageBuffer::from_pixel(sheet_w, sheet_h, Rgb([0, 0, 0]));
    for (i, frame) in frames.iter().enumerate() {
        let (ox, oy) = tile_origin(i, columns, tile_w, tile_h);
        // Copy the frame into its cell, clipping to the tile box.
        let fw = frame.image.width().min(tile_w);
        let fh = frame.image.height().min(tile_h);
        for y in 0..fh {
            for x in 0..fw {
                let px = *frame.image.get_pixel(x, y);
                sheet.put_pixel(ox + x, oy + y, px);
            }
        }
    }

    let sidecar = ThumbnailSidecar {
        tile_width: tile_w,
        tile_height: tile_h,
        columns: columns as u32,
        times: frames.iter().map(|f| f.time).collect(),
    };
    Some((sheet, sidecar))
}

/// Write the sprite JPEG then the JSON sidecar **last** (the completion marker).
/// On any failure the sidecar is not written, so a partial entry reads as
/// incomplete.
pub fn write_sprite(
    dir: &Path,
    key: &str,
    sheet: &RgbImage,
    sidecar: &ThumbnailSidecar,
) -> Result<(), VideoThumbnailError> {
    std::fs::create_dir_all(dir).map_err(|e| VideoThumbnailError::Io(e.to_string()))?;
    let (jpg_path, json_path) = sprite_paths(dir, key);

    // Encode the sprite as JPEG at quality 75 (= reference 0.75).
    let file =
        std::fs::File::create(&jpg_path).map_err(|e| VideoThumbnailError::Io(e.to_string()))?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, SPRITE_JPEG_QUALITY);
    encoder
        .encode_image(sheet)
        .map_err(|e| VideoThumbnailError::Image(e.to_string()))?;
    drop(writer);

    // Sidecar LAST = completion marker.
    let json = serde_json::to_vec(sidecar).map_err(|e| VideoThumbnailError::Io(e.to_string()))?;
    std::fs::write(&json_path, json).map_err(|e| VideoThumbnailError::Io(e.to_string()))?;
    Ok(())
}

/// Read a cached strip back: requires BOTH the sidecar (valid) and the sprite.
/// A sprite present without a valid sidecar reads as **incomplete** → `None`
/// (parity with `loadThumbnails`, which keys completeness on the sidecar).
pub fn read_cached_strip(dir: &Path, key: &str) -> Option<Vec<ThumbnailFrame>> {
    let (jpg_path, json_path) = sprite_paths(dir, key);
    let json = std::fs::read(&json_path).ok()?;
    let sidecar: ThumbnailSidecar = serde_json::from_slice(&json).ok()?;
    if !sidecar.is_valid() {
        return None;
    }
    let sheet = image::open(&jpg_path).ok()?.to_rgb8();
    let columns = sidecar.columns as usize;
    let tile_w = sidecar.tile_width;
    let tile_h = sidecar.tile_height;
    let rows = sidecar.rows();
    if sheet.width() < columns as u32 * tile_w || sheet.height() < rows * tile_h {
        return None;
    }
    let mut out = Vec::with_capacity(sidecar.times.len());
    for (i, &t) in sidecar.times.iter().enumerate() {
        let (ox, oy) = tile_origin(i, columns, tile_w, tile_h);
        let tile = image::imageops::crop_imm(&sheet, ox, oy, tile_w, tile_h).to_image();
        out.push(ThumbnailFrame { time: t, image: tile });
    }
    Some(out)
}

/// Cache-integrated video sprite-sheet generator. Wires [`extract_strip`] +
/// [`assemble_sprite`] + [`write_sprite`] through the E4-S2 cache key (#16) and
/// the **ungated** [`CacheKind::VideoThumbnail`] gate (dedup only — no
/// concurrency cap, per ruling #16).
#[derive(Clone)]
pub struct VideoThumbnailCache {
    dir: PathBuf,
    gates: CacheGates<CacheOutcome>,
}

/// Cheap, cloneable outcome stored in the in-flight dedup map — whether the job
/// produced a complete cache entry. The frames themselves live on disk; callers
/// re-read via [`read_cached_strip`] (the reference likewise re-loads from disk).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheOutcome {
    /// A complete sprite + sidecar was written (or already present).
    Cached,
    /// No frames could be extracted (e.g. empty/zero-duration clip).
    Empty,
}

impl VideoThumbnailCache {
    /// Build a cache rooted at `dir` (typically [`crate::cache::media_visual_cache_dir`]).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        VideoThumbnailCache {
            dir: dir.into(),
            gates: CacheGates::new(),
        }
    }

    /// Cache directory this instance writes under.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Generate (or reuse) the sprite-sheet for `path` with the given `duration`,
    /// returning the strip frames. Ungated (video-thumb) but deduped: concurrent
    /// requests for the same key share one extraction job. `on_progress` fires
    /// every 50 frames during a fresh extraction.
    ///
    /// Cache hit (sidecar present ⇒ complete) skips extraction entirely.
    ///
    /// `on_progress` is `'static + Send` because extraction runs on a blocking
    /// pool task (the partial strips it receives are owned slices, so it can't
    /// borrow caller state — forward results out via a channel/`Arc` if needed).
    pub async fn generate(
        &self,
        path: &Path,
        duration: f64,
        on_progress: impl FnMut(&[ThumbnailFrame]) + Send + 'static,
    ) -> Result<Vec<ThumbnailFrame>, VideoThumbnailError> {
        // No cache key (missing file / unreadable stat) ⇒ extract uncached.
        let Some(key) = cache_key(path) else {
            return generate_uncached(&self.dir, path, duration, on_progress, /*write*/ false);
        };

        // Fast path: already complete on disk.
        if let Some(cached) = read_cached_strip(&self.dir, &key) {
            return Ok(cached);
        }

        let dir = self.dir.clone();
        let path = path.to_path_buf();
        // The ungated video-thumb gate still dedups same-key requests. The
        // closure runs only for the leader; followers await its outcome and then
        // re-read the strip from disk.
        let outcome = {
            // `on_progress` isn't Clone, so only the leader gets it; we move it in.
            let mut on_progress = Some(on_progress);
            self.gates
                .run(CacheKind::VideoThumbnail, &key, || {
                    let dir = dir.clone();
                    let key = key.clone();
                    let path = path.clone();
                    let cb = on_progress.take();
                    async move {
                        tokio::task::spawn_blocking(move || {
                            let mut noop = |_: &[ThumbnailFrame]| {};
                            let frames = match cb {
                                Some(cb) => extract_strip_boxed(&path, duration, cb),
                                None => extract_strip(&path, duration, &mut noop),
                            };
                            match frames {
                                Ok(frames) if !frames.is_empty() => {
                                    match assemble_sprite(&frames) {
                                        Some((sheet, sidecar)) => {
                                            match write_sprite(&dir, &key, &sheet, &sidecar) {
                                                Ok(()) => CacheOutcome::Cached,
                                                Err(_) => CacheOutcome::Empty,
                                            }
                                        }
                                        None => CacheOutcome::Empty,
                                    }
                                }
                                _ => CacheOutcome::Empty,
                            }
                        })
                        .await
                        .unwrap_or(CacheOutcome::Empty)
                    }
                })
                .await
        };

        match outcome {
            CacheOutcome::Cached => Ok(read_cached_strip(&self.dir, &key).unwrap_or_default()),
            CacheOutcome::Empty => Ok(Vec::new()),
        }
    }
}

/// Monomorphization helper so the `FnMut` callback can be passed by value into
/// the leader closure above.
fn extract_strip_boxed(
    path: &Path,
    duration: f64,
    mut cb: impl FnMut(&[ThumbnailFrame]),
) -> Result<Vec<ThumbnailFrame>, VideoThumbnailError> {
    extract_strip(path, duration, &mut cb)
}

/// Extract a strip without (or optionally with) writing the cache — used when no
/// cache key is derivable.
fn generate_uncached(
    dir: &Path,
    path: &Path,
    duration: f64,
    on_progress: impl FnMut(&[ThumbnailFrame]),
    write: bool,
) -> Result<Vec<ThumbnailFrame>, VideoThumbnailError> {
    let frames = extract_strip_boxed(path, duration, on_progress)?;
    if write
        && !frames.is_empty()
        && let Some((sheet, sidecar)) = assemble_sprite(&frames)
    {
        // best-effort; a keyless path has nowhere stable to cache, so this is
        // only reachable when a caller forces `write`.
        let _ = write_sprite(dir, "uncached", &sheet, &sidecar);
    }
    Ok(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(time: f64, w: u32, h: u32, fill: u8) -> ThumbnailFrame {
        ThumbnailFrame {
            time,
            image: ImageBuffer::from_pixel(w, h, Rgb([fill, fill, fill])),
        }
    }

    #[test]
    fn fit_within_preserves_aspect_and_clamps_to_box() {
        // 1920×1080 into 120×68: width-bound (120/1920 < 68/1080).
        let (w, h) = fit_within(1920, 1080, 120, 68);
        assert_eq!(w, 120);
        assert_eq!(h, 68); // 1080 * (120/1920) = 67.5 → 68
        assert!(w <= 120 && h <= 68);
        // Portrait 1080×1920 into 120×68: height-bound.
        let (w, h) = fit_within(1080, 1920, 120, 68);
        assert_eq!(h, 68);
        assert!(w <= 120);
        // Never upscale a tiny source.
        assert_eq!(fit_within(40, 30, 120, 68), (40, 30));
        // Degenerate.
        assert_eq!(fit_within(0, 0, 120, 68), (1, 1));
    }

    #[test]
    fn assemble_sprite_lays_out_grid_and_sidecar() {
        // 51 frames of 120×68 → 50 cols × 2 rows; sidecar carries the times.
        let frames: Vec<ThumbnailFrame> = (0..51)
            .map(|i| frame(i as f64, 120, 68, (i % 256) as u8))
            .collect();
        let (sheet, sidecar) = assemble_sprite(&frames).unwrap();
        assert_eq!(sheet.width(), 50 * 120);
        assert_eq!(sheet.height(), 2 * 68);
        assert_eq!(sidecar.columns, 50);
        assert_eq!(sidecar.tile_width, 120);
        assert_eq!(sidecar.tile_height, 68);
        assert_eq!(sidecar.times.len(), 51);
        assert_eq!(sidecar.rows(), 2);
        // Empty input → nothing.
        assert!(assemble_sprite(&[]).is_none());
    }

    #[test]
    fn write_then_read_round_trips_strip_through_disk() {
        // No ffmpeg needed: assemble synthetic frames, write the sprite+sidecar,
        // read it back, and confirm count/times/tile-size survive the JPEG.
        let dir = tempfile::tempdir().unwrap();
        let frames: Vec<ThumbnailFrame> = (0..5)
            .map(|i| frame(i as f64 * 2.0, 120, 68, (i * 40) as u8))
            .collect();
        let (sheet, sidecar) = assemble_sprite(&frames).unwrap();
        write_sprite(dir.path(), "abc123", &sheet, &sidecar).unwrap();

        // Both files exist; sidecar is the completion marker.
        assert!(dir.path().join("abc123.thumbs.jpg").exists());
        assert!(dir.path().join("abc123.thumbs.json").exists());

        let read = read_cached_strip(dir.path(), "abc123").expect("complete entry reads back");
        assert_eq!(read.len(), 5);
        let times: Vec<f64> = read.iter().map(|f| f.time).collect();
        assert_eq!(times, vec![0.0, 2.0, 4.0, 6.0, 8.0]);
        assert!(read.iter().all(|f| f.image.width() == 120 && f.image.height() == 68));
    }

    // --- Real-ffmpeg integration (ignored: needs a committed video fixture) ---
    //
    // No video fixture is committed to this crate, so the end-to-end decode path
    // (ffmpeg open → seek → scale → sprite) is covered by these `#[ignore]`d
    // tests. Run against a real file by setting PALMIER_TEST_VIDEO to its path:
    //   PALMIER_TEST_VIDEO=C:\clip.mp4 cargo test -p palmier-media -- --ignored
    // The pure parts (times formula, fit_within, sprite layout, sidecar
    // round-trip, cache completeness) ARE covered by the non-ignored tests above.

    #[test]
    #[ignore = "needs a real video fixture via PALMIER_TEST_VIDEO"]
    fn extract_frame_from_real_video() {
        let Ok(path) = std::env::var("PALMIER_TEST_VIDEO") else {
            return;
        };
        let img = extract_frame(Path::new(&path), 0.0, THUMB_MAX_WIDTH, THUMB_MAX_HEIGHT)
            .expect("extract a frame at t=0");
        assert!(img.width() <= THUMB_MAX_WIDTH && img.height() <= THUMB_MAX_HEIGHT);
        assert!(img.width() > 0 && img.height() > 0);
    }

    #[tokio::test]
    #[ignore = "needs a real video fixture via PALMIER_TEST_VIDEO"]
    async fn generate_sprite_through_cache_from_real_video() {
        let Ok(path) = std::env::var("PALMIER_TEST_VIDEO") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let cache = VideoThumbnailCache::new(dir.path());
        // duration is read elsewhere (metadata); use a generous value so the
        // times formula produces several samples.
        let frames = cache
            .generate(Path::new(&path), 12.0, |_partial| {})
            .await
            .expect("generate strip");
        assert!(!frames.is_empty(), "real video yields at least one frame");
        // A complete entry must round-trip from disk (sidecar present).
        let key = cache_key(Path::new(&path)).unwrap();
        assert!(read_cached_strip(dir.path(), &key).is_some());
    }

    #[test]
    fn sidecar_absent_reads_as_incomplete() {
        // A sprite present but NO sidecar must read as incomplete (None).
        let dir = tempfile::tempdir().unwrap();
        let frames = vec![frame(0.0, 120, 68, 10)];
        let (sheet, sidecar) = assemble_sprite(&frames).unwrap();
        // Write only the JPEG, skip the sidecar.
        std::fs::create_dir_all(dir.path()).unwrap();
        let jpg = dir.path().join("k.thumbs.jpg");
        let f = std::fs::File::create(&jpg).unwrap();
        let mut w = std::io::BufWriter::new(f);
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, SPRITE_JPEG_QUALITY);
        enc.encode_image(&sheet).unwrap();
        drop(w);
        let _ = sidecar; // not written
        assert!(
            read_cached_strip(dir.path(), "k").is_none(),
            "no sidecar ⇒ incomplete"
        );
    }
}
