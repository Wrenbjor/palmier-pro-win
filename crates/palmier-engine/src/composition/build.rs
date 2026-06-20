//! Composition graph build — E5-S3.
//!
//! Port of the macOS reference `CompositionBuilder.build` + `buildVisuals`
//! (`Sources/PalmierPro/Preview/CompositionBuilder.swift`), **minus** the
//! AVFoundation track muxing. The reference assembles `AVMutableComposition`
//! tracks and attaches an `AVVideoComposition` of layer instructions; we instead
//! resolve, **per output frame**, which clips are active and emit a
//! [`CompositionFrame`] of bottom→top [`LayerRender`]s (FOUNDATION §6.5
//! frame-composition loop).
//!
//! ## What this story owns (pure assembly)
//!
//! - **z-order**: tracks bottom→top = render order (reference iterates
//!   `timeline.tracks.enumerated()`; track 0 is the bottom layer).
//! - **clip → source-frame mapping** (reference `insertClip` retime):
//!   `sourceFrames = speed==1 ? durationFrames : max(1, round(durationFrames·speed))`;
//!   the source range is `[trimStart, trimStart + sourceFrames)` placed at
//!   `clipStart`; **images use `trimStart = max(0, trimStartFrame)`**. With speed
//!   retime the timeline span `[clipStart, clipStart+durationFrames)` maps linearly
//!   onto that source range. All source↔timeline rounding is **`f64::round`
//!   ties-away** (carry-forward), never `round_ties_even`.
//! - **overlap precedence** (risk #2): the reference serializes on-track clips and
//!   forbids on-track overlap (`startFrame >= previousEndFrame`); we replicate that
//!   per-track skip so the *same* clips are selected, then composite tracks
//!   directly by z-order (no separate-track serialization needed in the wgpu model).
//! - **black background**: NOT a layer — the compositor clears to black (§6.5). We
//!   emit no background `LayerRender`.
//! - text clips are **excluded** from video layering (reference drops `.text`); the
//!   text pass (E5-S9) handles them.
//!
//! This is presentation-agnostic: each visible clip becomes a [`FrameRef`]
//! `(media_ref, source_frame)`; E5-S8 fetches the pixels from `palmier-media`.
//! The per-layer transform/opacity/crop come from the E5-S4 [`sampler`].

use std::collections::HashMap;

use palmier_model::{Clip, ClipType, Timeline};

use super::sampler::{self, SourceInfo};
use super::types::{CompositionFrame, FrameRef, LayerRender, VisualLayer};

/// Resolves a `media_ref` to its decoded-source geometry ([`SourceInfo`]) — the
/// natural size + preferred transform the sampler needs. Supplied by the caller
/// (E5-S8 wires the decoder's metadata; tests supply a map). When a ref is
/// unknown (offline media), the clip is skipped (reference treats an unresolvable
/// source as `.offline`/`.unprocessable` and skips it).
pub trait SourceResolver {
    /// The source geometry for `media_ref`, or `None` if unresolvable.
    fn source_info(&self, media_ref: &str) -> Option<SourceInfo>;
}

/// Blanket impl so a plain closure `Fn(&str) -> Option<SourceInfo>` is a resolver.
impl<F> SourceResolver for F
where
    F: Fn(&str) -> Option<SourceInfo>,
{
    fn source_info(&self, media_ref: &str) -> Option<SourceInfo> {
        self(media_ref)
    }
}

/// A `HashMap` of `media_ref → SourceInfo` is a resolver (tests / static projects).
impl SourceResolver for HashMap<String, SourceInfo> {
    fn source_info(&self, media_ref: &str) -> Option<SourceInfo> {
        self.get(media_ref).copied()
    }
}

/// Map an absolute timeline `frame` to the **source frame** of `clip` (the cache
/// key's frame component), verbatim with the reference `insertClip` retime.
///
/// Returns `None` when `frame` is outside `[start_frame, end_frame)`. Otherwise:
/// `rel = frame − start_frame` (∈ `[0, durationFrames)`); the timeline span
/// `durationFrames` maps onto `sourceFrames = speed==1 ? durationFrames :
/// max(1, round(durationFrames·speed))`, so the source offset is
/// `round(rel · sourceFrames / durationFrames)` (**ties-away** `f64::round`),
/// clamped to `[0, sourceFrames−1]`; the absolute source frame is
/// `trimStart + that offset`, where images clamp `trimStart = max(0, trimStartFrame)`.
pub fn source_frame_for(clip: &Clip, frame: i32) -> Option<u64> {
    if clip.duration_frames <= 0 || !clip.contains(frame) {
        return None;
    }
    let rel = frame - clip.start_frame;

    let source_frames = if clip.speed == 1.0 {
        clip.duration_frames
    } else {
        // ties-away f64::round, matching Swift `.rounded()` (carry-forward).
        (clip.duration_frames as f64 * clip.speed).round() as i32
    }
    .max(1);

    // Linear timeline→source offset across the (possibly retimed) span.
    let offset = if clip.duration_frames == source_frames {
        rel
    } else {
        let raw = rel as f64 * source_frames as f64 / clip.duration_frames as f64;
        raw.round() as i32 // ties-away
    }
    .clamp(0, source_frames - 1);

    let trim_start = if clip.media_type == ClipType::Image {
        clip.trim_start_frame.max(0)
    } else {
        clip.trim_start_frame
    };

    let abs_source = (trim_start + offset).max(0);
    Some(abs_source as u64)
}

/// Build the [`CompositionFrame`] for `timeline` at absolute `frame_index`.
///
/// Iterates tracks bottom→top; within each track, sorts clips by `start_frame`,
/// drops `.text`, and serializes on-track clips exactly as the reference
/// (`startFrame >= previousEndFrame`, `durationFrames > 0`). The clip active at
/// `frame_index` (if any) becomes one [`LayerRender`] with its sampled
/// transform/opacity/crop and a [`FrameRef`]. The resulting `layers` vec is
/// bottom→top (track order = z-order); the black background is the compositor's
/// clear, not a layer.
pub fn build_frame<R: SourceResolver>(
    timeline: &Timeline,
    frame_index: i32,
    resolver: &R,
) -> CompositionFrame {
    let render_size = (timeline.width as f64, timeline.height as f64);
    let mut layers: Vec<LayerRender> = Vec::new();

    for track in &timeline.tracks {
        // Audio tracks carry no visible layer (their volume mix is E5-S6).
        if track.track_type == ClipType::Audio {
            continue;
        }
        // A hidden track contributes nothing (reference `!track.hidden` gate in
        // `buildVisuals`; a hidden track's layers stay at opacity 0 / are omitted).
        if track.hidden {
            continue;
        }

        // Sort by start_frame and drop text (text never becomes a video layer).
        let mut sorted: Vec<&Clip> = track
            .clips
            .iter()
            .filter(|c| c.media_type != ClipType::Text)
            .collect();
        sorted.sort_by_key(|c| c.start_frame);

        // Serialize on-track clips: skip durationFrames<=0 and on-track overlaps,
        // matching the reference's `previousEndFrame` gate so the SAME clip is
        // selected as the AV path would have.
        let mut previous_end_frame = i32::MIN;
        for clip in sorted {
            if clip.duration_frames <= 0 || clip.start_frame < previous_end_frame {
                continue;
            }
            previous_end_frame = clip.end_frame();

            // Only the clip active at this frame produces a layer.
            if !clip.contains(frame_index) {
                continue;
            }
            let Some(source) = resolver.source_info(&clip.media_ref) else {
                continue; // offline / unresolvable → skip (reference behavior).
            };
            let Some(source_frame) = source_frame_for(clip, frame_index) else {
                continue;
            };

            let transform = sampler::layer_transform(clip, frame_index, &source, render_size);
            let opacity = sampler::layer_opacity(clip, frame_index);
            let crop = sampler::crop_rect(clip, frame_index, &source);

            let visual = VisualLayer {
                clip_id: clip.id.clone(),
                frame: FrameRef::new(clip.media_ref.clone(), source_frame),
                transform,
                opacity,
                crop,
                natural_size: source.natural_size,
                has_alpha: false, // set by E5-S8 from the decoded frame's pixfmt flag.
            };

            let layer = match clip.media_type {
                ClipType::Image => LayerRender::Image(visual),
                ClipType::Lottie => LayerRender::Lottie(visual),
                // Video (and any visual fallback) → Video.
                _ => LayerRender::Video(visual),
            };
            layers.push(layer);
        }
    }

    CompositionFrame {
        frame_index,
        layers,
    }
}

/// Re-sample only the **visual** properties (transform / opacity / crop) of an
/// existing [`CompositionFrame`] at a (possibly new) `frame_index`, **without**
/// re-deciding which clips are active or re-resolving source frames — the
/// `refreshVisuals` fast path (risk #8 / reference `VideoEngine.refreshVisuals`).
///
/// This is the cheap edit path: editing a transform/opacity/volume re-samples
/// instructions on the existing layer skeleton and must NOT trigger a decode /
/// structural rebuild. Each layer is re-sampled against the clip it came from
/// (looked up by `clip_id`); layers whose clip vanished are dropped. The
/// `FrameRef` (which source frame to show) is **preserved** — a pure property
/// edit doesn't change which frame is decoded.
///
/// Use [`build_frame`] when the structure changes (clips added/removed/retimed).
pub fn refresh_visuals<R: SourceResolver>(
    frame: &mut CompositionFrame,
    timeline: &Timeline,
    resolver: &R,
) {
    let render_size = (timeline.width as f64, timeline.height as f64);

    // Index clips by id for O(1) lookup.
    let mut clips: HashMap<&str, &Clip> = HashMap::new();
    for track in &timeline.tracks {
        for clip in &track.clips {
            clips.insert(clip.id.as_str(), clip);
        }
    }

    let frame_index = frame.frame_index;
    frame.layers.retain_mut(|layer| {
        let clip_id = layer.clip_id().to_string();
        let Some(clip) = clips.get(clip_id.as_str()) else {
            return false; // clip gone → drop the stale layer.
        };
        match layer {
            LayerRender::Video(v) | LayerRender::Image(v) | LayerRender::Lottie(v) => {
                let Some(source) = resolver.source_info(&v.frame.media_ref) else {
                    return false;
                };
                v.transform = sampler::layer_transform(clip, frame_index, &source, render_size);
                v.opacity = sampler::layer_opacity(clip, frame_index);
                v.crop = sampler::crop_rect(clip, frame_index, &source);
                true
            }
            LayerRender::Text(t) => {
                t.opacity = sampler::layer_opacity(clip, frame_index);
                // Text transform is owned by E5-S9's geometry; leave it.
                true
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::Track;

    fn upright_resolver() -> impl SourceResolver {
        // Every media_ref resolves to a render-sized upright source.
        |_r: &str| Some(SourceInfo::upright((1920.0, 1080.0)))
    }

    fn timeline_with_tracks(tracks: Vec<Track>) -> Timeline {
        let mut tl = Timeline::new();
        tl.fps = 30;
        tl.width = 1920;
        tl.height = 1080;
        tl.tracks = tracks;
        tl
    }

    #[test]
    fn z_order_is_track_order_bottom_to_top() {
        // Two video tracks, each with a clip covering frame 10. Track 0 is the
        // bottom layer (layers[0]); track 1 is on top (layers[1]).
        let mut t0 = Track::new(ClipType::Video);
        let mut c0 = Clip::new("bottom", 0, 30);
        c0.id = "c0".into();
        t0.clips.push(c0);

        let mut t1 = Track::new(ClipType::Video);
        let mut c1 = Clip::new("top", 0, 30);
        c1.id = "c1".into();
        t1.clips.push(c1);

        let tl = timeline_with_tracks(vec![t0, t1]);
        let cf = build_frame(&tl, 10, &upright_resolver());
        assert_eq!(cf.layers.len(), 2);
        assert_eq!(cf.layers[0].clip_id(), "c0", "track 0 must be the bottom layer");
        assert_eq!(cf.layers[1].clip_id(), "c1", "track 1 must be on top");
    }

    #[test]
    fn only_active_clip_at_frame_is_layered() {
        // One track, two non-overlapping clips: [0,30) and [30,60). At frame 40
        // only the second is active.
        let mut t = Track::new(ClipType::Video);
        let mut a = Clip::new("a", 0, 30);
        a.id = "a".into();
        let mut b = Clip::new("b", 30, 30);
        b.id = "b".into();
        t.clips.push(a);
        t.clips.push(b);
        let tl = timeline_with_tracks(vec![t]);

        let cf = build_frame(&tl, 40, &upright_resolver());
        assert_eq!(cf.layers.len(), 1);
        assert_eq!(cf.layers[0].clip_id(), "b");

        // At frame 10 → only `a`.
        let cf2 = build_frame(&tl, 10, &upright_resolver());
        assert_eq!(cf2.layers[0].clip_id(), "a");
    }

    #[test]
    fn on_track_overlap_precedence_first_wins() {
        // Reference serializes on-track clips: a later clip whose start is before
        // the previous clip's end is SKIPPED. Clips [0,40) and [20,40) overlap at
        // frame 25; only the first survives the `previousEndFrame` gate.
        let mut t = Track::new(ClipType::Video);
        let mut a = Clip::new("a", 0, 40);
        a.id = "a".into();
        let mut b = Clip::new("b", 20, 20);
        b.id = "b".into();
        t.clips.push(a);
        t.clips.push(b);
        let tl = timeline_with_tracks(vec![t]);

        let cf = build_frame(&tl, 25, &upright_resolver());
        assert_eq!(cf.layers.len(), 1, "overlapping on-track clip must be skipped");
        assert_eq!(cf.layers[0].clip_id(), "a");
    }

    #[test]
    fn text_clips_excluded_from_video_layering() {
        let mut t = Track::new(ClipType::Video);
        let mut txt = Clip::new("caption", 0, 30);
        txt.id = "txt".into();
        txt.media_type = ClipType::Text;
        t.clips.push(txt);
        let tl = timeline_with_tracks(vec![t]);

        let cf = build_frame(&tl, 10, &upright_resolver());
        assert!(cf.layers.is_empty(), "text must not become a video layer");
    }

    #[test]
    fn zero_duration_and_audio_tracks_skipped() {
        let mut v = Track::new(ClipType::Video);
        let mut z = Clip::new("z", 0, 0); // durationFrames 0
        z.id = "z".into();
        v.clips.push(z);

        let mut a = Track::new(ClipType::Audio);
        let mut ac = Clip::new("ac", 0, 30);
        ac.id = "ac".into();
        ac.media_type = ClipType::Audio;
        a.clips.push(ac);

        let tl = timeline_with_tracks(vec![v, a]);
        let cf = build_frame(&tl, 5, &upright_resolver());
        assert!(cf.layers.is_empty());
    }

    #[test]
    fn hidden_track_contributes_no_layer() {
        let mut t = Track::new(ClipType::Video);
        t.hidden = true;
        let mut c = Clip::new("c", 0, 30);
        c.id = "c".into();
        t.clips.push(c);
        let tl = timeline_with_tracks(vec![t]);
        assert!(build_frame(&tl, 10, &upright_resolver()).layers.is_empty());
    }

    #[test]
    fn source_frame_mapping_speed_one_no_trim() {
        // speed 1, no trim: source frame == rel offset.
        let c = Clip::new("m", 100, 60);
        // frame 130 → rel 30 → source 30.
        assert_eq!(source_frame_for(&c, 130), Some(30));
        // frame 100 → rel 0 → source 0.
        assert_eq!(source_frame_for(&c, 100), Some(0));
        // out of range → None.
        assert_eq!(source_frame_for(&c, 160), None);
        assert_eq!(source_frame_for(&c, 99), None);
    }

    #[test]
    fn source_frame_mapping_with_trim_start() {
        let mut c = Clip::new("m", 0, 60);
        c.trim_start_frame = 15;
        // frame 0 → rel 0 → source = trimStart + 0 = 15.
        assert_eq!(source_frame_for(&c, 0), Some(15));
        // frame 10 → source 25.
        assert_eq!(source_frame_for(&c, 10), Some(25));
    }

    #[test]
    fn source_frame_mapping_speed_retime_ties_away() {
        // speed 2.0: duration 10 → sourceFrames = round(20) = 20. The 10 timeline
        // frames map onto 20 source frames: offset = round(rel*20/10) = rel*2.
        let mut c = Clip::new("m", 0, 10);
        c.speed = 2.0;
        assert_eq!(source_frame_for(&c, 0), Some(0));
        assert_eq!(source_frame_for(&c, 3), Some(6));
        assert_eq!(source_frame_for(&c, 9), Some(18));

        // Ties-away rounding: duration 4, speed 1.5 → sourceFrames round(6)=6.
        // rel 1 → raw 1*6/4 = 1.5 → round ties-away → 2.
        let mut c2 = Clip::new("m", 0, 4);
        c2.speed = 1.5;
        assert_eq!(source_frame_for(&c2, 1), Some(2));
        // rel 3 → raw 4.5 → 5, but clamp to sourceFrames-1 = 5 → 5.
        assert_eq!(source_frame_for(&c2, 3), Some(5));
    }

    #[test]
    fn image_trim_start_clamped_non_negative() {
        let mut c = Clip::new("m", 0, 30);
        c.media_type = ClipType::Image;
        c.trim_start_frame = -10; // images clamp to max(0, …).
        // frame 5 → rel 5 → trimStart max(0,-10)=0 → source 5.
        assert_eq!(source_frame_for(&c, 5), Some(5));
    }

    #[test]
    fn image_and_lottie_become_first_class_layers() {
        let mut t = Track::new(ClipType::Video);
        let mut img = Clip::new("img", 0, 30);
        img.id = "img".into();
        img.media_type = ClipType::Image;
        let mut lot = Clip::new("lot", 0, 30);
        lot.id = "lot".into();
        lot.media_type = ClipType::Lottie;
        // Put them on separate tracks (same track would serialize/overlap-skip).
        t.clips.push(img);
        let mut t2 = Track::new(ClipType::Video);
        t2.clips.push(lot);
        let tl = timeline_with_tracks(vec![t, t2]);

        let cf = build_frame(&tl, 10, &upright_resolver());
        assert_eq!(cf.layers.len(), 2);
        assert!(matches!(cf.layers[0], LayerRender::Image(_)));
        assert!(matches!(cf.layers[1], LayerRender::Lottie(_)));
    }

    #[test]
    fn offline_media_ref_is_skipped() {
        let mut t = Track::new(ClipType::Video);
        let mut c = Clip::new("missing", 0, 30);
        c.id = "c".into();
        t.clips.push(c);
        let tl = timeline_with_tracks(vec![t]);
        // Resolver returns None for everything → no layers.
        let cf = build_frame(&tl, 10, &|_r: &str| None);
        assert!(cf.layers.is_empty());
    }

    #[test]
    fn empty_timeline_is_black_frame() {
        let tl = timeline_with_tracks(vec![]);
        let cf = build_frame(&tl, 0, &upright_resolver());
        assert_eq!(cf.frame_index, 0);
        assert!(cf.layers.is_empty(), "no layers → compositor clears to black");
    }

    #[test]
    fn refresh_visuals_resamples_without_restructuring() {
        // Build at frame 0 with a fade-in; then refresh at frame 5 and confirm the
        // opacity updated but the FrameRef (source frame) is unchanged.
        let mut t = Track::new(ClipType::Video);
        let mut c = Clip::new("m", 0, 100);
        c.id = "c".into();
        c.fade_in_frames = 10;
        c.fade_in_interpolation = palmier_model::Interpolation::Linear;
        t.clips.push(c);
        let tl = timeline_with_tracks(vec![t]);

        let mut cf = build_frame(&tl, 0, &upright_resolver());
        assert_eq!(cf.layers.len(), 1);
        let orig_frame_ref = cf.layers[0].visual().unwrap().frame.clone();
        // At frame 0 fade opacity is 0.
        assert!(cf.layers[0].visual().unwrap().opacity.abs() < 1e-9);

        // Refresh to frame 5 → opacity 0.5, source frame preserved.
        cf.frame_index = 5;
        refresh_visuals(&mut cf, &tl, &upright_resolver());
        let v = cf.layers[0].visual().unwrap();
        assert!((v.opacity - 0.5).abs() < 1e-9, "opacity re-sampled: {}", v.opacity);
        assert_eq!(v.frame, orig_frame_ref, "refresh must NOT change the source frame");
    }

    #[test]
    fn refresh_visuals_drops_vanished_clip() {
        let mut t = Track::new(ClipType::Video);
        let mut c = Clip::new("m", 0, 30);
        c.id = "c".into();
        t.clips.push(c);
        let tl = timeline_with_tracks(vec![t]);
        let mut cf = build_frame(&tl, 10, &upright_resolver());
        assert_eq!(cf.layers.len(), 1);

        // Remove the clip's track → refresh drops the stale layer.
        let empty = timeline_with_tracks(vec![]);
        refresh_visuals(&mut cf, &empty, &upright_resolver());
        assert!(cf.layers.is_empty());
    }
}
