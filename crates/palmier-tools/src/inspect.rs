//! INSPECT tool bodies — `inspect_media`, `inspect_timeline` (E7-S5; reference
//! `ToolExecutor+InspectTimeline.swift`). `search_media` moved to [`crate::search`]
//! when it went functional (E11-S10).
//!
//! ## `inspect_media` (Read, async)
//! Looks at one source asset: an image's pixels + dimensions, video sample frames +
//! a transcript, audio's transcript, or a Lottie's frames. In M2:
//! - the **overview** (asset metadata: type, dimensions, duration, generation
//!   status) is real, and
//! - the **transcript** is empty — its on-device Whisper backing lands in **Epic 10
//!   (M3)** (the reference returns empty → the agent tells the user to transcribe).
//!
//! The two distinct caps are enforced on the *shape* now (FR-26, carry-forward):
//! - the **image-frame sampling ceiling** `max_frames ≤ 12` (default 6) — clamped;
//! - the **transcript pagination caps** 400 segments / 10000 words — these are
//!   page-size caps, NOT the frame ceiling (do not conflate `12` with a page size).
//!
//! ## `inspect_timeline` (Read, async)
//! Composites N timeline frames through the wgpu compositor (the `palmier-engine`
//! GPU path, reconciliation #22/#23) and returns each as a base64 PNG image block +
//! the sampled frame numbers. The GPU device is **feature-gated** behind
//! `gpu-inspect` (which turns on `palmier-engine/wgpu-compositor`), exactly like
//! E5-S8 — with the feature OFF the tool reports that timeline rendering is not
//! available in this build (the schema/dispatch/cap math still compile + test
//! GPU-free). With the feature ON, a box without a GPU returns a clean "no adapter"
//! message rather than failing the call.

use serde_json::{json, Value};

use palmier_model::{ClipType, GenerationStatus, MediaAsset};

use crate::editor::EditorState;
use crate::result::ToolResult;

/// Frame-sampling defaults/ceiling shared by `inspect_media` + `inspect_timeline`
/// (reference `inspectTimelineDefaultFrames`/`inspectTimelineMaxFrames`). This is
/// the **image-frame ceiling**, distinct from the transcript pagination caps.
const DEFAULT_FRAMES: i64 = 6;
const MAX_FRAMES: i64 = 12;

/// Transcript pagination caps (reference `inspectMaxSegments` / `inspectMaxWords`).
/// These are page sizes — NOT the frame ceiling above.
const MAX_SEGMENTS: usize = 400;
const MAX_WORDS: usize = 10000;

// ─────────────────────────────────────────────────────────────────────────────
// inspect_media
// ─────────────────────────────────────────────────────────────────────────────

/// `inspect_media` (`media_ref`, opt `clip_id`, `max_frames ≤ 12 default 6`,
/// `start_seconds`, `end_seconds`, `word_timestamps`, `overview`): inspect one source
/// asset. Reference `inspectMedia`.
pub fn inspect_media(state: &EditorState, args: &Value) -> ToolResult {
    let media_ref = match args.get("mediaRef").and_then(Value::as_str) {
        Some(s) => s,
        None => return ToolResult::error("Missing required field 'mediaRef'"),
    };
    let asset: &MediaAsset = match state.library.assets.iter().find(|a| a.id == media_ref) {
        Some(a) => a,
        None => return ToolResult::error(format!("Media asset not found: {media_ref}")),
    };

    let overview = args.get("overview").and_then(Value::as_bool).unwrap_or(false);

    // Resolve the requested frame count: clamp to the 12-frame ceiling. When
    // overview=true the frame count is ignored entirely (reference: overview
    // replaces frame sampling with a storyboard grid).
    let requested = args.get("maxFrames").and_then(Value::as_i64).unwrap_or(DEFAULT_FRAMES);
    let frame_count = if overview { 0 } else { requested.clamp(1, MAX_FRAMES) };

    // Build the overview metadata (always real in M2).
    let mut body = json!({
        "mediaRef": asset.id,
        "name": asset.name,
        "type": clip_type_str(asset.asset_type),
        "duration": asset.duration_seconds,
        "generationStatus": generation_status_str(&asset.generation_status),
    });
    if let (Some(w), Some(h)) = (asset.source_width, asset.source_height) {
        body["width"] = json!(w);
        body["height"] = json!(h);
    }
    if let Some(fps) = asset.source_fps {
        body["fps"] = json!(fps);
    }
    body["hasAudio"] = json!(asset.has_audio);

    // Transcript: the on-device Whisper backing is Epic 10 (M3). Empty in M2, but the
    // shape (segments capped 400 / words capped 10000, paged) is fixed now. We surface
    // the caps so a client/test sees them honored.
    let want_words = args.get("wordTimestamps").and_then(Value::as_bool).unwrap_or(false);
    if matches!(asset.asset_type, ClipType::Video | ClipType::Audio) {
        body["transcript"] = json!({
            "segmentFormat": ["text", "start", "end"],
            "segments": [],
            "segmentCap": MAX_SEGMENTS,
            "wordCap": MAX_WORDS,
            "words": if want_words { json!([]) } else { Value::Null },
            "note": "On-device transcription is not yet available in this build; \
                     the transcript is empty (lands in a later milestone).",
        });
    }

    // Frame sampling (image bytes / video frames / Lottie frames) is backed by the
    // decode pipeline (Epic 4/5). In M2 we report the sample plan + ceiling rather
    // than decoding bytes here (the compositor/decoder GPU path is exercised by
    // inspect_timeline behind `gpu-inspect`). overview ignores maxFrames.
    if overview {
        body["overview"] = json!(true);
        body["frameCount"] = json!(0);
    } else {
        body["maxFrames"] = json!(frame_count);
        body["frameCeiling"] = json!(MAX_FRAMES);
    }

    match serde_json::to_string(&body) {
        Ok(s) => ToolResult::ok(s),
        Err(_) => ToolResult::error("Failed to serialize inspect_media result"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// inspect_timeline
// ─────────────────────────────────────────────────────────────────────────────

/// Sample `count` frame numbers evenly across `[start, end)` (reference
/// `inspectTimeline` `sampledFrames`, ties-down on the index math). `count` is the
/// already-clamped frame count.
fn sample_frames(start: i32, end: i32, count: i64) -> Vec<i32> {
    let span = (end - start).max(1) as f64;
    let count = count.max(1);
    (0..count)
        .map(|i| start + (span * (i as f64 + 0.5) / count as f64).floor() as i32)
        .collect()
}

/// Resolve the sampled frame numbers for `inspect_timeline` from the args + the
/// timeline length. Shared between the GPU and stub paths so the cap/sampling math
/// is identical and testable GPU-free. Returns `Err(msg)` on an invalid window.
fn resolve_inspect_timeline_frames(state: &EditorState, args: &Value) -> Result<Vec<i32>, String> {
    let total = state.timeline().total_frames();
    if total <= 0 {
        return Err("Timeline is empty — nothing to render.".to_string());
    }
    let start = args.get("startFrame").and_then(Value::as_i64).unwrap_or(0) as i32;
    if start < 0 || start >= total {
        return Err(format!("startFrame {start} out of range [0, {total})."));
    }
    match args.get("endFrame").and_then(Value::as_i64) {
        None => Ok(vec![start]),
        Some(raw_end) => {
            let end = (raw_end as i32).min(total);
            if end <= start {
                return Err(format!("endFrame must be greater than startFrame ({start})."));
            }
            let span = (end - start) as i64;
            // Clamp to the 12-frame ceiling AND the span (can't sample more frames
            // than exist in the window).
            let requested = args.get("maxFrames").and_then(Value::as_i64).unwrap_or(DEFAULT_FRAMES);
            let count = requested.clamp(1, MAX_FRAMES).min(span.max(1));
            Ok(sample_frames(start, end, count))
        }
    }
}

/// `inspect_timeline` (opt `start_frame` default 0, `end_frame`, `max_frames ≤ 12
/// default 6`): composite N timeline frames into base64 PNGs. Reference
/// `inspectTimeline`. GPU-gated behind `gpu-inspect`.
pub fn inspect_timeline(state: &EditorState, args: &Value) -> ToolResult {
    let frames = match resolve_inspect_timeline_frames(state, args) {
        Ok(f) => f,
        Err(msg) => return ToolResult::error(msg),
    };
    render_inspect_timeline(state, &frames)
}

#[cfg(not(feature = "gpu-inspect"))]
fn render_inspect_timeline(state: &EditorState, frames: &[i32]) -> ToolResult {
    // GPU-free build: the compositor device isn't compiled in. Report the sample
    // plan + the reason, so a client sees the tool dispatched and the frame math ran
    // (the cap/sampling is unit-tested regardless of the feature).
    let timeline = state.timeline();
    let body = json!({
        "fps": timeline.fps,
        "width": timeline.width,
        "height": timeline.height,
        "totalFrames": timeline.total_frames(),
        "frameNumbers": frames,
        "note": "Timeline rendering is not available in this build (compile with the \
                 'gpu-inspect' feature to composite frames). The frames that WOULD be \
                 sampled are listed in frameNumbers.",
    });
    ToolResult::ok(serde_json::to_string(&body).unwrap_or_default())
}

#[cfg(feature = "gpu-inspect")]
fn render_inspect_timeline(state: &EditorState, frames: &[i32]) -> ToolResult {
    gpu::render(state, frames)
}

/// The wgpu composite + PNG-encode path, compiled only with `gpu-inspect`.
#[cfg(feature = "gpu-inspect")]
mod gpu {
    use super::*;
    use std::collections::HashMap;

    use palmier_engine::composition::build_frame;
    use palmier_engine::composition::sampler::SourceInfo;
    use palmier_engine::{Canvas, Compositor, QualityTarget, RenderFrame};
    use palmier_media::{FrameSource, UrlResolver};
    use std::sync::Arc;

    /// Longest-edge cap on the rendered frame (reference
    /// `inspectTimelineMaxDimension` = 512).
    const MAX_DIMENSION: u32 = 512;

    /// A static `SourceInfo` resolver over the asset catalog (natural size from the
    /// asset's probed dimensions; falls back to the canvas size).
    fn source_info_map(state: &EditorState) -> HashMap<String, SourceInfo> {
        let mut map = HashMap::new();
        for a in &state.library.assets {
            if let (Some(w), Some(h)) = (a.source_width, a.source_height) {
                map.insert(a.id.clone(), SourceInfo::upright((w as f64, h as f64)));
            }
        }
        map
    }

    /// A `FrameSource` whose resolver maps a media_ref → its source path (External
    /// source). Project-internal sources are unresolved in M2 (no project bundle).
    fn frame_source(state: &EditorState) -> FrameSource {
        let mut paths: HashMap<String, String> = HashMap::new();
        for a in &state.library.assets {
            if let palmier_model::MediaSource::External { absolute_path } = &a.source {
                paths.insert(a.id.clone(), absolute_path.clone());
            }
        }
        let resolver: UrlResolver = Arc::new(move |media_ref: &str| {
            paths.get(media_ref).map(std::path::PathBuf::from)
        });
        FrameSource::new(resolver)
    }

    /// Aspect-preserving (w, h) whose longest edge is ≤ `MAX_DIMENSION`.
    fn fit(w: u32, h: u32) -> (u32, u32) {
        let longest = w.max(h);
        if longest <= MAX_DIMENSION || longest == 0 {
            return (w.max(1), h.max(1));
        }
        let scale = MAX_DIMENSION as f64 / longest as f64;
        (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
    }

    pub fn render(state: &EditorState, frames: &[i32]) -> ToolResult {
        let timeline = state.timeline();
        let (rw, rh) = fit(timeline.width as u32, timeline.height as u32);

        let mut compositor = match Compositor::new_headless(rw, rh) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(format!(
                    "inspect_timeline: GPU compositor unavailable ({e:?}). This box may have no \
                     usable GPU adapter."
                ))
            }
        };

        let info = source_info_map(state);
        let fs = frame_source(state);
        let canvas = Canvas::new(rw, rh);

        // `build_frame` takes a sized `R: SourceResolver`; the engine's blanket impl
        // makes a `Fn(&str) -> Option<SourceInfo>` closure one, so wrap the lookup.
        let resolver = |media_ref: &str| -> Option<SourceInfo> { info.get(media_ref).copied() };

        let mut blocks = Vec::new();
        let mut rendered: Vec<i32> = Vec::new();
        for &frame in frames {
            let composition = build_frame(timeline, frame, &resolver);
            let render_frame = RenderFrame::new(composition, canvas, QualityTarget::Full);
            if compositor.render(&render_frame, &fs).is_err() {
                continue;
            }
            let Some(img) = compositor.read_back() else { continue };
            let Some(png) = encode_png(img.width, img.height, &img.bytes) else { continue };
            blocks.push(crate::result::Block::Image {
                base64: base64_encode(&png),
                media_type: "image/png".to_string(),
            });
            rendered.push(frame);
        }

        if blocks.is_empty() {
            return ToolResult::error("Failed to render timeline frames.");
        }

        let meta = json!({
            "fps": timeline.fps,
            "width": rw,
            "height": rh,
            "totalFrames": timeline.total_frames(),
            "frameNumbers": rendered,
        });
        blocks.push(crate::result::Block::Text(
            serde_json::to_string(&meta).unwrap_or_default(),
        ));
        ToolResult { content: blocks, is_error: false }
    }

    /// Encode RGBA8 (row-major, unpadded) to PNG bytes via the `png` crate.
    fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().ok()?;
            writer.write_image_data(rgba).ok()?;
        }
        Some(out)
    }

    /// Standard base64 (RFC 4648) encode for the image block payload.
    fn base64_encode(data: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
        for chunk in data.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
            out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
            out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 { ALPHABET[((n >> 6) & 63) as usize] as char } else { '=' });
            out.push(if chunk.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
        }
        out
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// shared helpers
// ─────────────────────────────────────────────────────────────────────────────

fn clip_type_str(t: ClipType) -> &'static str {
    match t {
        ClipType::Video => "video",
        ClipType::Audio => "audio",
        ClipType::Image => "image",
        ClipType::Text => "text",
        ClipType::Lottie => "lottie",
    }
}

fn generation_status_str(status: &GenerationStatus) -> &'static str {
    match status {
        GenerationStatus::None => "none",
        GenerationStatus::Generating => "generating",
        GenerationStatus::Downloading => "downloading",
        GenerationStatus::Rendering => "generating",
        GenerationStatus::Failed(_) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_frames_respects_count_and_window() {
        let f = sample_frames(0, 100, 4);
        assert_eq!(f.len(), 4);
        assert!(f.iter().all(|&x| (0..100).contains(&x)));
        // strictly increasing
        assert!(f.windows(2).all(|w| w[0] < w[1]));
    }
}
