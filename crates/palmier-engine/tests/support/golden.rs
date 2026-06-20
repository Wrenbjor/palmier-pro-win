//! Shared E5-S11 golden / SM-2 support — fixture timelines, a deterministic frame
//! provider, and PNG golden I/O. **Not a test binary itself** (it declares no
//! `#[test]`s); it is `#[path]`-included by `golden_frame.rs`, `sm2_perf.rs`, and the
//! `render_frame` bench so the SM-C1 golden fixtures, the SM-2 representative
//! composition, and the PNG read/write live in one place and can never drift.
//!
//! ## Why mirror the palmier-project fixtures here
//! The canonical `golden_project_keyframes` / `golden_project_text` builders live in
//! `palmier-project`'s **test tree** (`tests/common/golden_fixtures.rs`) — private to
//! that crate's test binaries, not a public API. E5-S11 is scoped to `palmier-engine`
//! and may only *read* those fixtures, so we reconstruct the same timelines in memory
//! here (same clip ids, frames, transforms, keyframes, text styles). The on-disk
//! `golden_project_*.palmier` bundles are the cross-crate round-trip's concern (E2);
//! the SM-C1 gate's concern is the *rendered frame*, which it owns end-to-end.

#![allow(dead_code)] // each includer uses a subset.

use std::path::PathBuf;
use std::sync::Arc;

use palmier_engine::compositor::provider::FrameProvider;
use palmier_engine::{
    build_frame, build_text_layers, Canvas, CompositionFrame, FontRegistry, QualityTarget,
    RenderFrame, SourceInfo, SourceResolver, TextLayout,
};
use palmier_media::decode::frame::{PixelLayout, Plane};
use palmier_media::{DecodedFrame, SeekMode};
use palmier_model::{
    AnimPair, Clip, ClipType, Crop, Fill, Interpolation, Keyframe, KeyframeTrack, Rgba,
    TextAlignment, TextStyle, Timeline, Track, Transform,
};

// ---------------------------------------------------------------------------
// Deterministic frame provider
// ---------------------------------------------------------------------------

/// A provider that synthesizes a **deterministic** RGBA frame per `(media_ref,
/// source_frame)` so the golden render is byte-reproducible without any media file.
///
/// The frame is a smooth two-axis gradient tinted per `media_ref` (so distinct
/// sources read back as distinct colors — proves layer identity/ordering), with the
/// `source_frame` folded into the tint (so a wrong source-frame mapping shifts the
/// golden and fails SM-C1). No randomness, no timestamps → stable golden.
pub struct GradientProvider {
    pub w: u32,
    pub h: u32,
}

impl GradientProvider {
    pub fn new(w: u32, h: u32) -> Self {
        GradientProvider { w, h }
    }

    /// A stable per-source tint in `0..=255` from a cheap string hash.
    fn tint(media_ref: &str) -> (u8, u8, u8) {
        let mut hash: u32 = 2166136261;
        for b in media_ref.bytes() {
            hash = (hash ^ b as u32).wrapping_mul(16777619);
        }
        (
            (hash & 0xFF) as u8,
            ((hash >> 8) & 0xFF) as u8,
            ((hash >> 16) & 0xFF) as u8,
        )
    }
}

impl FrameProvider for GradientProvider {
    type Error = std::convert::Infallible;
    fn provide_frame(
        &self,
        media_ref: &str,
        source_frame: u64,
        _mode: SeekMode,
        _active_layers: u32,
    ) -> Result<DecodedFrame, Self::Error> {
        let (tr, tg, tb) = Self::tint(media_ref);
        let sf = (source_frame % 64) as u32;
        let mut bytes = Vec::with_capacity((self.w * self.h * 4) as usize);
        for y in 0..self.h {
            for x in 0..self.w {
                // Gradient base + per-source tint + source-frame offset, all clamped.
                let gx = ((x * 255) / self.w.max(1)) as u32;
                let gy = ((y * 255) / self.h.max(1)) as u32;
                let r = ((gx + tr as u32 + sf) % 256) as u8;
                let g = ((gy + tg as u32 + sf) % 256) as u8;
                let b = (((gx + gy) / 2 + tb as u32) % 256) as u8;
                bytes.extend_from_slice(&[r, g, b, 255]);
            }
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

/// Resolver mapping every media_ref to an upright source of the given natural size
/// (the gradient provider's dimensions).
pub fn solid_resolver(natural: (f64, f64)) -> impl SourceResolver {
    move |_r: &str| Some(SourceInfo::upright(natural))
}

// ---------------------------------------------------------------------------
// Fixture timelines (mirror of palmier-project's golden builders)
// ---------------------------------------------------------------------------

fn kf<V>(frame: i32, value: V, interp: Interpolation) -> Keyframe<V> {
    Keyframe::with_interpolation(frame, value, interp)
}

fn clip(id: &str, media_ref: &str, start: i32, dur: i32) -> Clip {
    let mut c = Clip::new(media_ref, start, dur);
    c.id = id.to_string();
    c
}

/// `golden_project_keyframes` — two video clips animated across opacity / position /
/// scale / rotation / crop / volume on Smooth / Linear / Hold. Verbatim mirror of
/// `palmier-project tests/common/golden_fixtures.rs::golden_project_keyframes`.
pub fn golden_keyframes_timeline() -> Timeline {
    let mut timeline = Timeline::new();
    timeline.fps = 30;
    timeline.width = 1920;
    timeline.height = 1080;
    timeline.settings_configured = true;

    let mut track = Track::new(ClipType::Video);
    track.id = "track-anim".into();

    let mut c1 = clip("clip-anim-1", "asset-v1", 0, 120);
    let mut opacity = KeyframeTrack::new();
    opacity.upsert(kf(0, 0.0_f64, Interpolation::Smooth));
    opacity.upsert(kf(60, 1.0, Interpolation::Smooth));
    opacity.upsert(kf(120, 0.0, Interpolation::Smooth));
    c1.opacity_track = Some(opacity);

    let mut position = KeyframeTrack::new();
    position.upsert(kf(0, AnimPair::new(0.0, 0.0), Interpolation::Linear));
    position.upsert(kf(120, AnimPair::new(0.5, 0.5), Interpolation::Linear));
    c1.position_track = Some(position);

    let mut scale = KeyframeTrack::new();
    scale.upsert(kf(0, AnimPair::new(0.5, 0.5), Interpolation::Hold));
    scale.upsert(kf(60, AnimPair::new(1.0, 1.0), Interpolation::Hold));
    c1.scale_track = Some(scale);
    track.clips.push(c1);

    let mut c2 = clip("clip-anim-2", "asset-v2", 120, 120);
    let mut rotation = KeyframeTrack::new();
    rotation.upsert(kf(0, 0.0_f64, Interpolation::Linear));
    rotation.upsert(kf(120, 90.0, Interpolation::Linear));
    c2.rotation_track = Some(rotation);

    let mut crop = KeyframeTrack::new();
    crop.upsert(kf(0, Crop::default(), Interpolation::Smooth));
    crop.upsert(kf(
        120,
        Crop { left: 0.1, top: 0.1, right: 0.1, bottom: 0.1 },
        Interpolation::Smooth,
    ));
    c2.crop_track = Some(crop);

    let mut volume = KeyframeTrack::new();
    volume.upsert(kf(0, -60.0_f64, Interpolation::Hold));
    volume.upsert(kf(60, 0.0, Interpolation::Hold));
    volume.upsert(kf(120, 6.0, Interpolation::Hold));
    c2.volume_track = Some(volume);
    track.clips.push(c2);

    timeline.tracks.push(track);
    timeline
}

/// `golden_project_text` — two text clips (title + lower third) with non-default
/// styles. Verbatim mirror of `palmier-project`'s `golden_project_text`.
pub fn golden_text_timeline() -> Timeline {
    let mut timeline = Timeline::new();
    timeline.fps = 30;
    timeline.width = 1920;
    timeline.height = 1080;
    timeline.settings_configured = true;

    let mut track = Track::new(ClipType::Text);
    track.id = "track-text".into();

    let mut title = clip("clip-title", "asset-title", 0, 90);
    title.media_type = ClipType::Text;
    title.source_clip_type = ClipType::Text;
    title.text_content = Some("Palmier Pro".into());
    title.text_style = Some(TextStyle {
        font_name: palmier_model::FontName::from_str("Helvetica-Bold"),
        font_size: 120.0,
        font_scale: 1.0,
        color: Rgba::new(1.0, 1.0, 1.0, 1.0),
        alignment: TextAlignment::Center,
        shadow: palmier_model::Shadow::default(),
        background: Fill { enabled: true, color: Rgba::new(0.0, 0.0, 0.0, 0.6) },
        border: Fill::default(),
    });
    title.transform = Transform::default();
    track.clips.push(title);

    let mut lower = clip("clip-lower", "asset-lower", 90, 90);
    lower.media_type = ClipType::Text;
    lower.source_clip_type = ClipType::Text;
    lower.text_content = Some("Lower third".into());
    lower.text_style = Some(TextStyle {
        font_size: 48.0,
        alignment: TextAlignment::Left,
        ..TextStyle::default()
    });
    lower.transform = Transform::default();
    track.clips.push(lower);

    timeline.tracks.push(track);
    timeline
}

/// A representative SM-2 composition: `clips` video clips across 2 tracks, all active
/// at frame 0, no keyframe motion (the SM-2 spec: "5 clips / 2 layers, no keyframe
/// motion"). Each clip is full-canvas so the compositor does real full-screen
/// overdraw — the worst case the FPS floor must hold under.
pub fn sm2_timeline(clips: usize, width: i32, height: i32) -> Timeline {
    let mut tl = Timeline::new();
    tl.fps = 30;
    tl.width = width;
    tl.height = height;
    let tracks = 2;
    let per_track = clips.div_ceil(tracks);
    for t in 0..tracks {
        let mut track = Track::new(ClipType::Video);
        for i in 0..per_track {
            let g = t * per_track + i;
            if g >= clips {
                break;
            }
            // All overlap at frame 0 across tracks but are laid out so the per-track
            // serialization keeps one active per track at the sampled frame.
            let mut c = clip(&format!("sm2-clip-{g}"), &format!("sm2-src-{t}"), 0, 300);
            c.transform = Transform::default(); // full canvas
            track.clips.push(c);
        }
        tl.tracks.push(track);
    }
    tl
}

/// Build a finalized [`RenderFrame`] for a video/keyframe timeline at `frame_index`.
pub fn render_frame_video(
    timeline: &Timeline,
    frame_index: i32,
    natural: (f64, f64),
) -> RenderFrame {
    let res = solid_resolver(natural);
    let cf = build_frame(timeline, frame_index, &res);
    let canvas = Canvas::new(timeline.width as u32, timeline.height as u32);
    RenderFrame::new(cf, canvas, QualityTarget::Full)
}

/// Build a finalized [`RenderFrame`] for a TEXT timeline at `frame_index`, laying the
/// glyph runs through `palmier-text` (the text layers the wgpu text pass renders).
pub fn render_frame_text(
    timeline: &Timeline,
    frame_index: i32,
    registry: &mut FontRegistry,
    layout: &mut TextLayout,
) -> RenderFrame {
    let cw = timeline.width as f64;
    let ch = timeline.height as f64;
    let mut cf = CompositionFrame::empty(frame_index);
    let text_layers = build_text_layers(timeline, frame_index, cw, ch, registry, layout);
    cf.layers.extend(text_layers);
    let canvas = Canvas::new(timeline.width as u32, timeline.height as u32);
    RenderFrame::new(cf, canvas, QualityTarget::Full)
}

// ---------------------------------------------------------------------------
// PNG golden I/O
// ---------------------------------------------------------------------------

/// Directory holding the committed golden PNGs.
pub fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Read a committed golden PNG as raw RGBA8 (width, height, bytes). `None` if absent.
pub fn read_golden_png(name: &str) -> Option<(u32, u32, Vec<u8>)> {
    let path = golden_dir().join(name);
    let file = std::fs::File::open(&path).ok()?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    // Golden is always written as RGBA8; assert the loaded layout matches.
    assert_eq!(info.color_type, png::ColorType::Rgba, "golden must be RGBA8");
    Some((info.width, info.height, buf))
}

/// Write `bytes` (RGBA8, row-major, unpadded) as a golden PNG. Used only under the
/// `UPDATE_GOLDEN` regeneration flag.
pub fn write_golden_png(name: &str, width: u32, height: u32, bytes: &[u8]) {
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).expect("create golden dir");
    let path = dir.join(name);
    let file = std::fs::File::create(&path).expect("create golden png");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(bytes).expect("png data");
}

/// Per-pixel-channel comparison of two RGBA8 buffers. Returns
/// `(max_abs_channel_diff, mean_abs_channel_diff, mismatch_fraction)` where a channel
/// counts as mismatched if it differs by more than `per_channel_tol`.
pub fn compare_rgba(
    a: &[u8],
    b: &[u8],
    per_channel_tol: u8,
) -> (u8, f64, f64) {
    assert_eq!(a.len(), b.len(), "golden size mismatch");
    let mut max_d = 0u8;
    let mut sum: u64 = 0;
    let mut mismatched: u64 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = x.abs_diff(*y);
        max_d = max_d.max(d);
        sum += d as u64;
        if d > per_channel_tol {
            mismatched += 1;
        }
    }
    let n = a.len().max(1) as f64;
    (max_d, sum as f64 / n, mismatched as f64 / n)
}

/// Whether golden regeneration was requested (`UPDATE_GOLDEN=1`).
pub fn update_golden() -> bool {
    std::env::var("UPDATE_GOLDEN")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}
