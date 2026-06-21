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
//! 3. hand them to the frontend as base64, which paints them onto a `<canvas>`.
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
//! ## Perf
//! A headless [`Compositor`] (GPU device + pipeline) is **cached** in managed state,
//! keyed by its backing (downscaled) size, so a ~30 fps request stream reuses one
//! device + the VRAM texture cache instead of standing up a new adapter per frame.
//! The composite is built at the **downscaled** size (the model's transforms are
//! canvas-normalized, so scaling the canvas scales the whole composition), keeping
//! the readback small (≈960px wide by default).

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

/// Default preview downscale ceiling: the composite is rendered at a width of at most
/// this many pixels (height follows the canvas aspect). Keeps per-frame readback small
/// enough for a ~30 fps request stream while staying crisp in the viewport.
const DEFAULT_MAX_WIDTH: u32 = 960;

/// One rendered preview frame handed to the frontend. `rgba_base64` is the raw,
/// row-major, **premultiplied** RGBA8 buffer (`width * height * 4` bytes) base64-
/// encoded — the frontend decodes it straight into `ImageData` for a `<canvas>`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFrame {
    /// Frame width in pixels (the downscaled backing width).
    pub width: u32,
    /// Frame height in pixels (the downscaled backing height).
    pub height: u32,
    /// Base64 of the row-major RGBA8 pixels (`width * height * 4` bytes).
    pub rgba_base64: String,
}

/// The cached headless compositor + its backing size. Re-created only when the target
/// size changes (timeline resolution change / a different `max_width`). `None` until
/// the first render (or when the box has no GPU — every render then returns a black
/// frame produced on the CPU so the UI still shows the canvas rect).
#[derive(Default)]
pub struct PreviewRenderState(pub Mutex<Option<CachedCompositor>>);

/// A headless [`Compositor`] pinned to one backing size.
pub struct CachedCompositor {
    compositor: Compositor,
    width: u32,
    height: u32,
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

/// A black frame (base64) at `width × height` — the degraded result when there is no
/// GPU / no timeline / a transient render glitch, so the UI always gets a paintable
/// canvas instead of an error.
fn black_frame(width: u32, height: u32) -> PreviewFrame {
    let bytes = black_rgba(width, height);
    PreviewFrame {
        width,
        height,
        rgba_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
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
/// and return the RGBA pixels (base64) for the `<canvas>`.
///
/// Empty/no-timeline (or no GPU) ⇒ a black frame at the (downscaled) composition
/// size — the viewport still shows the canvas rect, never an error. A
/// missing/offline clip source skips just that layer (the engine compositor's
/// per-layer skip), so one bad asset never blanks the whole frame.
#[tauri::command]
pub fn preview_render_frame(
    agent: State<'_, AgentState>,
    render_state: State<'_, PreviewRenderState>,
    frame: i32,
    max_width: Option<u32>,
) -> Result<PreviewFrame, String> {
    let snap = snapshot(&agent);
    let urls = snap.urls.clone();
    let resolver: palmier_media::UrlResolver =
        Arc::new(move |media_ref: &str| urls.get(media_ref).cloned());
    let frame_source = FrameSource::new(resolver);
    let mut guard = render_state.0.lock().map_err(|e| e.to_string())?;
    Ok(render_core(
        &snap.timeline,
        &snap.sizes,
        &frame_source,
        &mut guard,
        frame,
        max_width.unwrap_or(DEFAULT_MAX_WIDTH),
    ))
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
        Some(img) => PreviewFrame {
            width: img.width,
            height: img.height,
            rgba_base64: base64::engine::general_purpose::STANDARD.encode(&img.bytes),
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
        let f = black_frame(4, 2);
        assert_eq!((f.width, f.height), (4, 2));
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(f.rgba_base64)
            .unwrap();
        assert_eq!(bytes.len(), 4 * 2 * 4);
        // Every pixel opaque black (RGB 0, A 255).
        for px in bytes.chunks_exact(4) {
            assert_eq!(px, &[0, 0, 0, 255]);
        }
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
    // - an EMPTY timeline composites an opaque-black frame.
    //
    // GPU-gated like the engine's `compositor_smoke`: on a headless box with no
    // adapter, `render_core` degrades to a black frame, so the one-clip assertion is
    // skipped with a notice (FOUNDATION §11.1: GPU paths run headless or are skipped).
    // The empty-timeline assertion holds either way (no GPU → black; GPU → black floor).

    use palmier_engine::compositor::provider::FrameProvider;
    use palmier_media::decode::frame::{PixelLayout, Plane};
    use palmier_media::{DecodedFrame, SeekMode};
    use palmier_model::{Clip, ClipType, Track};
    use std::sync::Arc;

    /// A provider that returns a single solid-color RGBA frame for any request (so a
    /// one-clip timeline composites without a real media file).
    struct SolidProvider {
        w: u32,
        h: u32,
        rgba: [u8; 4],
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

    fn decode_b64(frame: &PreviewFrame) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD
            .decode(&frame.rgba_base64)
            .unwrap()
    }

    #[test]
    fn render_core_one_clip_non_empty_and_empty_black() {
        let sizes: HashMap<String, (f64, f64)> = HashMap::new();
        let mut cache: Option<CachedCompositor> = None;

        // EMPTY timeline → opaque black at the downscaled size (no GPU → black; GPU →
        // black floor). 1920x1080 capped at 160 → 160x90.
        let empty = Timeline::default();
        let dead = SolidProvider { w: 1, h: 1, rgba: [0, 0, 0, 0] };
        let empty_frame = render_core(&empty, &sizes, &dead, &mut cache, 0, 160);
        assert_eq!((empty_frame.width, empty_frame.height), (160, 90));
        let bytes = decode_b64(&empty_frame);
        assert_eq!(bytes.len(), 160 * 90 * 4);
        let center = ((empty_frame.height / 2 * empty_frame.width + empty_frame.width / 2) * 4)
            as usize;
        assert_eq!(
            &bytes[center..center + 4],
            &[0, 0, 0, 255],
            "empty timeline → opaque black"
        );

        // ONE-clip timeline with a full-canvas red solid source. Skip the color
        // assertion when there's no GPU (the path degrades to black there).
        let tl = one_clip_timeline();
        // Backing: 320x180 capped at 160 → 160x90. The solid provider returns the
        // clip's natural size; the geometry resolver falls back to the canvas size, so
        // a full-canvas layer covers the frame.
        let red = SolidProvider { w: 160, h: 90, rgba: [255, 0, 0, 255] };
        let frame = render_core(&tl, &sizes, &red, &mut cache, 0, 160);
        assert_eq!((frame.width, frame.height), (160, 90));
        let bytes = decode_b64(&frame);
        assert_eq!(bytes.len(), 160 * 90 * 4);
        let center =
            ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
        let px = &bytes[center..center + 4];
        let has_gpu = cache.is_some();
        if has_gpu {
            assert!(
                px[0] > 200 && px[1] < 60 && px[2] < 60,
                "one-clip red layer composites red; got {px:?}"
            );
        } else {
            eprintln!("[preview_render] no GPU adapter — skipped one-clip color check");
        }
    }
}
