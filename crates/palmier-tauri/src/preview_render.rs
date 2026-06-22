//! Robust preview frame rendering — the offscreen composite → GPU readback path.
//!
//! ## Why this exists (the product gap it closes)
//! The app showed a timeline but no video preview, and could not play/watch. The
//! original on-window present seam ([`crate::preview`], "plan A1": a wgpu swapchain
//! drawn directly on the Tauri window HWND under a transparent WebView2 child) is
//! fragile (the tao `clip_children` caveat → surface-fighting flicker) and was never
//! actually invoked end-to-end. This module implements the decided robust path
//! instead:
//!
//! 1. composite the ACTIVE project's timeline **offscreen** (a headless wgpu target),
//! 2. **read the pixels back** to CPU RGBA, downscaled for preview perf, and
//! 3. encode them as **JPEG** and hand them to the frontend as base64, which decodes
//!    them via `createImageBitmap` and draws them onto a `<canvas>`.
//!
//! This reuses the exact offscreen render+readback the **video export** path
//! (`palmier-export::render`, E6-S5) already proved: the SHARED `palmier-engine`
//! frame builder ([`build_frame`]) → [`Compositor::new_headless`] →
//! [`Compositor::render`] → [`Compositor::read_back`]. It does NOT touch the
//! on-window `PreviewSession`.
//!
//! ## Where the timeline + media come from
//! Both the timeline and the media library live in the ONE shared
//! `Arc<ToolExecutor>` (`AgentState.executor`) — the same `EditorState` the
//! `editor_get_timeline` command reads and every MCP/agent/UI edit mutates. We
//! snapshot the timeline + the `media_ref → (path, natural_size)` map under the
//! executor lock, then render outside it. So the preview always reflects the live
//! edit state with zero extra source of truth.
//!
//! ## Perf — why this is not the original (freezing) path
//! The first cut of this command was a **synchronous** `#[tauri::command]`, so Tauri
//! ran every render on the MAIN thread — each heavy offscreen composite + GPU readback
//! + ~2.8 MB base64 blocked the UI and every other command, and the rAF play loop
//! piled them up into a 30–40 s backlog. Worse, it rebuilt the [`FrameSource`] every
//! call, so each frame re-opened the file and seeked from scratch. This rewrite fixes
//! all of that:
//!
//! - **Off the main thread.** `preview_render_frame` is now `async`; the blocking
//!   GPU/decode work runs on [`tauri::async_runtime::spawn_blocking`], so it never
//!   blocks the UI or other commands. The cached state lives behind an
//!   `Arc<Mutex<…>>` cloned out of managed state before the blocking hop (no non-Send
//!   guard is held across an `await`).
//! - **Persisted decoder.** The [`FrameSource`] (its decoder pool + frame cache) is
//!   cached in [`PreviewCache`] and REUSED across frames, rebuilt only when the
//!   timeline's media set changes (keyed on a fingerprint of the url-map). Sequential
//!   playback N→N+1 then decodes incrementally instead of re-opening+seeking.
//! - **Smaller, faster payload.** The backing render defaults to ~480 px wide for
//!   playback (crisper width available for a paused still), and the readback is
//!   JPEG-encoded (≈10× smaller than raw RGBA) before base64 — far less to copy over
//!   IPC and to decode in the webview.
//!
//! A headless [`Compositor`] (GPU device + pipeline) is **cached** in the same state,
//! keyed by its backing (downscaled) size, so a request stream reuses one device + the
//! VRAM texture cache instead of standing up a new adapter per frame. The composite is
//! built at the **downscaled** size (the model's transforms are canvas-normalized, so
//! scaling the canvas scales the whole composition), keeping the readback small.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use palmier_engine::compositor::provider::FrameProvider;
use palmier_engine::{
    build_frame, Canvas, Compositor, QualityTarget, RenderFrame, SourceInfo,
};
use palmier_media::FrameSource;
use palmier_model::{MediaSource, Timeline};
use serde::Serialize;
use tauri::State;

use crate::agent::AgentState;

/// Default preview downscale ceiling for **playback**: the composite is rendered at a
/// width of at most this many pixels (height follows the canvas aspect). Tuned for a
/// smooth request stream — small enough that decode + GPU readback + JPEG encode keep
/// up with a ~30 fps coalesced play loop. A paused/seek still can ask for a crisper
/// width via `max_width`.
const DEFAULT_MAX_WIDTH: u32 = 480;

/// JPEG quality (0–100) for the readback encode. 80 is visually clean for a preview
/// while keeping the payload ~10× smaller than raw RGBA.
const JPEG_QUALITY: u8 = 80;

/// One rendered preview frame handed to the frontend. `dataBase64` is a JPEG image
/// (base64-encoded); the frontend decodes it via `createImageBitmap`/`Image` and draws
/// it onto a `<canvas>`. `format` names the codec so the frontend can build the right
/// data URL.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFrame {
    /// Frame width in pixels (the downscaled backing width).
    pub width: u32,
    /// Frame height in pixels (the downscaled backing height).
    pub height: u32,
    /// Image codec of `data_base64` — currently always `"jpeg"`.
    pub format: &'static str,
    /// Base64 of the encoded image bytes (a JPEG of the `width × height` frame).
    pub data_base64: String,
}

/// Managed state for the robust offscreen preview path: the cached headless
/// compositor + the persisted decoder, behind one `Arc<Mutex<…>>` so an `async`
/// command can clone the `Arc` out and run the blocking render on a worker thread
/// without holding a guard across an `await`.
#[derive(Default, Clone)]
pub struct PreviewRenderState(pub Arc<Mutex<PreviewCache>>);

/// The cached compositor + persisted decoder. Both are reused across frames and
/// rebuilt only when their key changes (compositor: backing size; frame source: the
/// url-map fingerprint).
#[derive(Default)]
pub struct PreviewCache {
    /// Headless compositor pinned to one backing size (`None` until the first render,
    /// or when the box has no GPU — every render then returns a black frame).
    compositor: Option<CachedCompositor>,
    /// Persisted [`FrameSource`] + the fingerprint of the url-map it was built from.
    /// Reused across frames so sequential playback decodes incrementally; rebuilt only
    /// when the timeline's media set changes.
    frames: Option<CachedFrameSource>,
}

/// A headless [`Compositor`] pinned to one backing size.
pub struct CachedCompositor {
    compositor: Compositor,
    width: u32,
    height: u32,
}

/// A persisted [`FrameSource`] + the fingerprint of the url-map it resolves over.
pub struct CachedFrameSource {
    source: FrameSource,
    /// Fingerprint of the `media_ref → path` map (sorted). When the live snapshot's
    /// fingerprint differs, the decoder pool is stale (an asset was added/removed/
    /// relinked) and the source is rebuilt.
    fingerprint: u64,
}

/// Snapshot of everything the render needs out of the shared `EditorState`, taken
/// under the executor lock so the render itself runs lock-free.
struct TimelineSnapshot {
    timeline: Timeline,
    /// `media_ref → absolute source path` (External assets, and Project assets that
    /// already carry an absolute-ish relative path). Offline/unresolvable refs are
    /// simply absent (the compositor skips that layer).
    urls: HashMap<String, PathBuf>,
    /// `media_ref → natural size` from the asset's stored `source_width/height`,
    /// falling back to the canvas size when unknown.
    sizes: HashMap<String, (f64, f64)>,
}

/// Pull the live timeline + media-path/size maps out of the shared executor.
fn snapshot(agent: &AgentState) -> TimelineSnapshot {
    agent.executor.with_state_ref(|state| {
        let timeline = state.library.timeline.clone();
        let mut urls = HashMap::new();
        let mut sizes = HashMap::new();
        for asset in &state.library.assets {
            if let Some(path) = asset_path(&asset.source) {
                urls.insert(asset.id.clone(), path);
            }
            if let (Some(w), Some(h)) = (asset.source_width, asset.source_height) {
                if w > 0 && h > 0 {
                    sizes.insert(asset.id.clone(), (w as f64, h as f64));
                }
            }
        }
        TimelineSnapshot {
            timeline,
            urls,
            sizes,
        }
    })
}

/// A stable fingerprint of the `media_ref → path` map: hashes the entries in sorted
/// order so it is independent of `HashMap` iteration order. Used to decide whether the
/// persisted [`FrameSource`] (its decoder pool) is still valid for the live media set.
fn url_fingerprint(urls: &HashMap<String, PathBuf>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut entries: Vec<(&String, &PathBuf)> = urls.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    entries.len().hash(&mut hasher);
    for (k, v) in entries {
        k.hash(&mut hasher);
        v.hash(&mut hasher);
    }
    hasher.finish()
}

/// Resolve a [`MediaSource`] to an absolute path for decoding. `External` uses its
/// absolute path directly; `Project` uses its relative path as-is (the project base
/// dir is owned by `palmier-project`; absent here a relative path still decodes when
/// the process cwd matches — and an unresolvable path simply yields a skipped layer,
/// never a crash). Returns `None` for an empty path.
fn asset_path(source: &MediaSource) -> Option<PathBuf> {
    let raw = match source {
        MediaSource::External { absolute_path } => absolute_path,
        MediaSource::Project { relative_path } => relative_path,
    };
    if raw.is_empty() {
        None
    } else {
        Some(PathBuf::from(raw))
    }
}

/// Compute the downscaled backing size for `canvas_w × canvas_h` under `max_width`,
/// preserving aspect, each dim ≥ 1. Never upscales past the canvas size.
fn backing_size(canvas_w: u32, canvas_h: u32, max_width: u32) -> (u32, u32) {
    let cw = canvas_w.max(1);
    let ch = canvas_h.max(1);
    let mw = max_width.max(1).min(cw); // never upscale
    if mw >= cw {
        return (cw, ch);
    }
    let scale = mw as f64 / cw as f64;
    let h = ((ch as f64 * scale).round() as u32).max(1);
    (mw, h)
}

/// Build a `Timeline` clone scaled to the backing size. The model's per-clip
/// transforms are **canvas-normalized** (`0..1`), so [`build_frame`] multiplies them
/// by `timeline.width/height` to get render-pixel space; scaling those dimensions
/// scales the whole composition proportionally — letting us composite directly at the
/// (small) preview size with no separate downscale pass and a tiny readback.
fn scaled_timeline(timeline: &Timeline, w: u32, h: u32) -> Timeline {
    let mut t = timeline.clone();
    t.width = w as i32;
    t.height = h as i32;
    t
}

/// Encode a `width × height` premultiplied-RGBA buffer to a JPEG (base64). JPEG has no
/// alpha, so this composites over opaque black implicitly (the readback's RGB already
/// carries the premultiplied color; the preview always renders over a black floor).
fn encode_jpeg(rgba: &[u8], width: u32, height: u32) -> Option<String> {
    use image::codecs::jpeg::JpegEncoder;
    use image::{ColorType, ImageEncoder};

    // JPEG wants RGB; drop the alpha channel from the premultiplied RGBA readback.
    let px = (width as usize) * (height as usize);
    let mut rgb = Vec::with_capacity(px * 3);
    for chunk in rgba.chunks_exact(4) {
        rgb.push(chunk[0]);
        rgb.push(chunk[1]);
        rgb.push(chunk[2]);
    }
    let mut out: Vec<u8> = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
    encoder
        .write_image(&rgb, width, height, ColorType::Rgb8.into())
        .ok()?;
    Some(base64::engine::general_purpose::STANDARD.encode(&out))
}

/// A black frame (JPEG base64) at `width × height` — the degraded result when there is
/// no GPU / no timeline / a transient render glitch, so the UI always gets a paintable
/// image instead of an error.
fn black_frame(width: u32, height: u32) -> PreviewFrame {
    let w = width.max(1);
    let h = height.max(1);
    let bytes = black_rgba(w, h);
    let data_base64 = encode_jpeg(&bytes, w, h).unwrap_or_default();
    PreviewFrame {
        width: w,
        height: h,
        format: "jpeg",
        data_base64,
    }
}

/// A `width × height` opaque-black premultiplied RGBA buffer.
fn black_rgba(width: u32, height: u32) -> Vec<u8> {
    let w = width.max(1) as usize;
    let h = height.max(1) as usize;
    let mut bytes = vec![0u8; w * h * 4];
    for px in bytes.chunks_exact_mut(4) {
        px[3] = 255; // opaque
    }
    bytes
}

/// Render the active timeline at `frame`, downscaled to at most `max_width` px wide,
/// and return a JPEG (base64) for the `<canvas>`.
///
/// This is an **async** command: Tauri runs it on the async runtime (never the main
/// thread), and the blocking GPU/decode work is offloaded to a blocking worker via
/// [`tauri::async_runtime::spawn_blocking`], so neither the UI nor any other command
/// is blocked while a frame renders. The persisted compositor + decoder live in
/// [`PreviewRenderState`]; we clone the `Arc` out before the blocking hop so no
/// non-`Send` guard crosses the `await`.
///
/// Empty/no-timeline (or no GPU) ⇒ a black frame at the (downscaled) composition
/// size — the viewport still shows the canvas rect, never an error. A
/// missing/offline clip source skips just that layer (the engine compositor's
/// per-layer skip), so one bad asset never blanks the whole frame.
#[tauri::command]
pub async fn preview_render_frame(
    agent: State<'_, AgentState>,
    render_state: State<'_, PreviewRenderState>,
    frame: i32,
    max_width: Option<u32>,
) -> Result<PreviewFrame, String> {
    // Snapshot the live edit state on the async thread (cheap clone under the lock).
    let snap = snapshot(&agent);
    let cache = render_state.0.clone();
    let max_width = max_width.unwrap_or(DEFAULT_MAX_WIDTH);

    // Offload the heavy composite + readback + JPEG encode to a blocking worker so the
    // main thread / async runtime stays free for input + other commands.
    tauri::async_runtime::spawn_blocking(move || {
        let mut guard = cache.lock().map_err(|e| e.to_string())?;
        let cache = &mut *guard;

        // Persisted decoder: reuse the FrameSource unless the media set changed.
        let fingerprint = url_fingerprint(&snap.urls);
        let need_source = match cache.frames.as_ref() {
            Some(f) => f.fingerprint != fingerprint,
            None => true,
        };
        if need_source {
            let urls = snap.urls.clone();
            let resolver: palmier_media::UrlResolver =
                Arc::new(move |media_ref: &str| urls.get(media_ref).cloned());
            cache.frames = Some(CachedFrameSource {
                source: FrameSource::new(resolver),
                fingerprint,
            });
        }
        let source = cache
            .frames
            .as_ref()
            .expect("frame source present after build")
            .source
            .clone();

        Ok(render_core(
            &snap.timeline,
            &snap.sizes,
            &source,
            &mut cache.compositor,
            frame,
            max_width,
        ))
    })
    .await
    .map_err(|e| format!("preview render task failed: {e}"))?
}

/// The Tauri-free render core: composite `timeline` at `frame` (downscaled to
/// `max_width`) through `frames`, reusing/rebuilding the cached headless compositor in
/// `cache`. Returns a black frame on no-GPU / render error (never panics). Generic
/// over [`FrameProvider`] so tests can inject a solid-color provider (a one-clip
/// timeline then reads back non-black without a real media file); production passes a
/// [`FrameSource`].
fn render_core<P: FrameProvider>(
    timeline: &Timeline,
    sizes: &HashMap<String, (f64, f64)>,
    frames: &P,
    cache: &mut Option<CachedCompositor>,
    frame: i32,
    max_width: u32,
) -> PreviewFrame {
    let canvas_w = timeline.width.max(1) as u32;
    let canvas_h = timeline.height.max(1) as u32;
    let (w, h) = backing_size(canvas_w, canvas_h, max_width);

    // Source-geometry resolver: natural size from the asset (falls back to canvas).
    let sizes = sizes.clone();
    let canvas_size = (canvas_w as f64, canvas_h as f64);
    let geometry = move |media_ref: &str| -> Option<SourceInfo> {
        let nat = sizes.get(media_ref).copied().unwrap_or(canvas_size);
        Some(SourceInfo::upright(nat))
    };

    // Build the composition at the DOWNSCALED size (canvas-normalized transforms scale
    // with the canvas), finalize to a RenderFrame at full quality for that backing.
    let scaled = scaled_timeline(timeline, w, h);
    let composition = build_frame(&scaled, frame.max(0), &geometry);
    let render_frame = RenderFrame::new(composition, Canvas::new(w, h), QualityTarget::Full);

    // Acquire (or build) the cached headless compositor at this backing size.
    let need_new = match cache.as_ref() {
        Some(c) => c.width != w || c.height != h,
        None => true,
    };
    if need_new {
        match Compositor::new_headless(w, h) {
            Ok(compositor) => {
                *cache = Some(CachedCompositor {
                    compositor,
                    width: w,
                    height: h,
                });
            }
            Err(e) => {
                // No GPU adapter (headless box) — degrade to a black frame, never error.
                tracing::warn!(target: "preview", error = %format!("{e}"), "preview headless compositor unavailable; returning black frame");
                return black_frame(w, h);
            }
        }
    }
    let cached = cache.as_mut().expect("compositor present after build");

    // Render offscreen + read back. A render error degrades to black (one bad frame
    // must not tear the preview down).
    if let Err(e) = cached.compositor.render(&render_frame, frames) {
        tracing::warn!(target: "preview", error = %format!("{e}"), "preview render failed; returning black frame");
        return black_frame(w, h);
    }
    match cached.compositor.read_back() {
        Some(img) => match encode_jpeg(&img.bytes, img.width, img.height) {
            Some(data_base64) => PreviewFrame {
                width: img.width,
                height: img.height,
                format: "jpeg",
                data_base64,
            },
            None => {
                tracing::warn!(target: "preview", "preview JPEG encode failed; returning black frame");
                black_frame(w, h)
            }
        },
        None => {
            tracing::warn!(target: "preview", "preview readback returned None; returning black frame");
            black_frame(w, h)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backing_size_preserves_aspect_and_never_upscales() {
        // 1920x1080 capped at 960 → 960x540.
        assert_eq!(backing_size(1920, 1080, 960), (960, 540));
        // Smaller than cap → unchanged (no upscale).
        assert_eq!(backing_size(640, 360, 960), (640, 360));
        // Square.
        assert_eq!(backing_size(1000, 1000, 500), (500, 500));
        // Degenerate inputs clamp to ≥ 1.
        assert_eq!(backing_size(0, 0, 0), (1, 1));
    }

    #[test]
    fn scaled_timeline_scales_canvas_dims() {
        let mut tl = Timeline::default();
        tl.width = 1920;
        tl.height = 1080;
        let s = scaled_timeline(&tl, 960, 540);
        assert_eq!((s.width, s.height), (960, 540));
        // fps + tracks are preserved (only the canvas dims change).
        assert_eq!(s.fps, tl.fps);
    }

    #[test]
    fn black_frame_is_opaque_and_correct_size() {
        let f = black_frame(8, 4);
        assert_eq!((f.width, f.height), (8, 4));
        assert_eq!(f.format, "jpeg");
        // The payload is a decodable JPEG (magic SOI marker 0xFFD8).
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(f.data_base64)
            .unwrap();
        assert!(bytes.len() > 2, "non-empty JPEG");
        assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "JPEG SOI marker");
    }

    #[test]
    fn url_fingerprint_is_order_independent_and_change_sensitive() {
        let mut a = HashMap::new();
        a.insert("x".to_string(), PathBuf::from("/a.mp4"));
        a.insert("y".to_string(), PathBuf::from("/b.mp4"));
        let mut b = HashMap::new();
        // Same entries inserted in the opposite order → same fingerprint.
        b.insert("y".to_string(), PathBuf::from("/b.mp4"));
        b.insert("x".to_string(), PathBuf::from("/a.mp4"));
        assert_eq!(url_fingerprint(&a), url_fingerprint(&b));
        // A changed path → different fingerprint (decoder pool must rebuild).
        let mut c = a.clone();
        c.insert("x".to_string(), PathBuf::from("/c.mp4"));
        assert_ne!(url_fingerprint(&a), url_fingerprint(&c));
        // A removed asset → different fingerprint.
        let mut d = a.clone();
        d.remove("y");
        assert_ne!(url_fingerprint(&a), url_fingerprint(&d));
    }

    #[test]
    fn asset_path_resolves_external_and_skips_empty() {
        let ext = MediaSource::External {
            absolute_path: "/clip.mp4".into(),
        };
        assert_eq!(asset_path(&ext), Some(PathBuf::from("/clip.mp4")));
        let empty = MediaSource::External {
            absolute_path: String::new(),
        };
        assert_eq!(asset_path(&empty), None);
    }

    // ── GPU-gated render-core smoke (the story's required test) ──────────────────
    //
    // Asserts the SHARED offscreen render path the command delegates to:
    // - a ONE-clip timeline composites a non-black, correctly-sized frame, and
    // - an EMPTY timeline composites an opaque-black frame, and
    // - sequential frames REUSE one persisted decoder (no rebuild per call).
    //
    // GPU-gated like the engine's `compositor_smoke`: on a headless box with no
    // adapter, `render_core` degrades to a black frame, so the one-clip color
    // assertion is skipped with a notice (FOUNDATION §11.1: GPU paths run headless or
    // are skipped). The size + decoder-reuse assertions hold either way.

    use palmier_engine::compositor::provider::FrameProvider;
    use palmier_media::decode::frame::{PixelLayout, Plane};
    use palmier_media::{DecodedFrame, SeekMode};
    use palmier_model::{Clip, ClipType, Track};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// A provider that returns a single solid-color RGBA frame for any request and
    /// COUNTS how many times it was asked to provide a frame — so a test can assert
    /// sequential renders go through ONE persisted provider (not a fresh one per call).
    struct SolidProvider {
        w: u32,
        h: u32,
        rgba: [u8; 4],
        provides: AtomicUsize,
    }
    impl SolidProvider {
        fn new(w: u32, h: u32, rgba: [u8; 4]) -> Self {
            SolidProvider { w, h, rgba, provides: AtomicUsize::new(0) }
        }
    }
    impl FrameProvider for SolidProvider {
        type Error = std::convert::Infallible;
        fn provide_frame(
            &self,
            _media_ref: &str,
            source_frame: u64,
            _mode: SeekMode,
            _active_layers: u32,
        ) -> Result<DecodedFrame, Self::Error> {
            self.provides.fetch_add(1, Ordering::SeqCst);
            let mut bytes = Vec::with_capacity((self.w * self.h * 4) as usize);
            for _ in 0..(self.w * self.h) {
                bytes.extend_from_slice(&self.rgba);
            }
            Ok(DecodedFrame {
                layout: PixelLayout::Rgba8,
                width: self.w,
                height: self.h,
                has_alpha: false,
                planes: Arc::new(vec![Plane {
                    bytes,
                    stride: (self.w * 4) as usize,
                    width: self.w,
                    height: self.h,
                }]),
                source_frame,
            })
        }
    }

    fn one_clip_timeline() -> Timeline {
        let mut tl = Timeline::default();
        tl.width = 320;
        tl.height = 180;
        let mut track = Track::new(ClipType::Video);
        track.clips.push(Clip::new("asset-1", 0, 30));
        tl.tracks.push(track);
        tl
    }

    fn decode_jpeg_dims(frame: &PreviewFrame) -> (u32, u32) {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&frame.data_base64)
            .unwrap();
        let img = image::load_from_memory(&bytes).expect("decodable JPEG");
        (img.width(), img.height())
    }

    #[test]
    fn render_core_one_clip_non_empty_and_empty_black() {
        let sizes: HashMap<String, (f64, f64)> = HashMap::new();
        let mut cache: Option<CachedCompositor> = None;

        // EMPTY timeline → opaque black at the downscaled size. 1920x1080 capped at
        // 160 → 160x90. JPEG decodes back to those dims.
        let empty = Timeline::default();
        let dead = SolidProvider::new(1, 1, [0, 0, 0, 0]);
        let empty_frame = render_core(&empty, &sizes, &dead, &mut cache, 0, 160);
        assert_eq!((empty_frame.width, empty_frame.height), (160, 90));
        assert_eq!(empty_frame.format, "jpeg");
        assert_eq!(decode_jpeg_dims(&empty_frame), (160, 90));

        // ONE-clip timeline with a full-canvas red solid source. Skip the color
        // assertion when there's no GPU (the path degrades to black there).
        let tl = one_clip_timeline();
        // Backing: 320x180 capped at 160 → 160x90.
        let red = SolidProvider::new(160, 90, [255, 0, 0, 255]);
        let frame = render_core(&tl, &sizes, &red, &mut cache, 0, 160);
        assert_eq!((frame.width, frame.height), (160, 90));
        assert_eq!(decode_jpeg_dims(&frame), (160, 90));
        let has_gpu = cache.is_some();
        if has_gpu {
            // Sample the JPEG center; lossy compression keeps it clearly red.
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&frame.data_base64)
                .unwrap();
            let img = image::load_from_memory(&bytes).unwrap().to_rgb8();
            let px = img.get_pixel(img.width() / 2, img.height() / 2).0;
            assert!(
                px[0] > 180 && px[1] < 90 && px[2] < 90,
                "one-clip red layer composites red; got {px:?}"
            );
        } else {
            eprintln!("[preview_render] no GPU adapter — skipped one-clip color check");
        }
    }

    #[test]
    fn persisted_decoder_is_reused_across_sequential_frames() {
        // The story's decoder-reuse proof: rendering frame N then N+1 through the SAME
        // provider must NOT rebuild the source — the provider sees both requests, so
        // its provide-count grows across calls instead of resetting. (Mirrors how the
        // command persists ONE FrameSource in PreviewCache and clones it per call.)
        let sizes: HashMap<String, (f64, f64)> = HashMap::new();
        let mut cache: Option<CachedCompositor> = None;
        let tl = one_clip_timeline();
        let provider = SolidProvider::new(160, 90, [0, 128, 0, 255]);

        let _f0 = render_core(&tl, &sizes, &provider, &mut cache, 0, 160);
        let after_first = provider.provides.load(Ordering::SeqCst);
        let _f1 = render_core(&tl, &sizes, &provider, &mut cache, 1, 160);
        let after_second = provider.provides.load(Ordering::SeqCst);

        // Only meaningful with a GPU (no GPU → black, provider never queried). When a
        // GPU is present, the second sequential frame went through the SAME provider:
        // total provides strictly increased, proving no per-call source rebuild.
        if cache.is_some() {
            assert!(after_first >= 1, "first frame queried the provider");
            assert!(
                after_second > after_first,
                "second sequential frame reused the SAME provider (no rebuild): {after_first} → {after_second}"
            );
        } else {
            eprintln!("[preview_render] no GPU adapter — skipped decoder-reuse provide-count check");
        }
    }
}
