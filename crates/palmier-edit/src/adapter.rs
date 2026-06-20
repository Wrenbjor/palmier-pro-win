//! The `Clip` ↔ placement-view adapter (the seam flagged in `lib.rs` / `placement.rs`).
//!
//! The pure engines ([`ripple`](crate::ripple), [`overwrite`](crate::overwrite),
//! [`snap`](crate::snap), [`split`](crate::split)) operate over the lightweight
//! placement views ([`ClipPlacement`], [`SplitClip`], [`SnapClip`]) so they can be
//! unit-tested with no dependency on the full model. This module bridges the real
//! `palmier_model::Clip` (now on main) onto those views, so the **orchestration**
//! layer (E3-S6) can run the engines against a live `Timeline`.
//!
//! ## Why a one-way adapter (Clip → view), not a `From<view> for Clip`
//!
//! The engines never construct a clip from a view — a view is a strict *subset* of
//! a clip (it drops keyframes, transform, text, media ref, …). The orchestration
//! layer mutates the *real* clip in place (e.g. applies a `ClipShift` by writing
//! `clip.start_frame`), using the engine's view-keyed output (`clip_id`,
//! `new_start_frame`, durations) as instructions. So the adapter is **read-only,
//! one-directional**: `Clip → ClipPlacement / SplitClip / SnapClip`.
//!
//! ## `has_no_source_media` (ruling carry-forward)
//!
//! `SplitClip::has_no_source_media` is `true` for **image** and **text** clips
//! (reference `hasNoSourceMedia`; edit-engines.md lines 115, 238) — they have no
//! bounded source, so trim fields may go negative / extend freely. We key this off
//! the clip's **`media_type`** (the current kind), matching the reference (which
//! reads the live clip type, not `source_clip_type`).
//!
//! ## Volume-keyframe mapping
//!
//! [`SplitClip::volume_track`] is a flat `Vec<VolumeKeyframe>` (clip-relative dB
//! offsets) that the split's keyframe migration reads. `Clip::volume_track` is a
//! `KeyframeTrack<f64>` whose `Keyframe::frame` is already clip-relative
//! (`palmier-model` stores keyframes clip-relative) and whose `value` is dB — so
//! the mapping is a direct per-keyframe copy, preserving `interpolation_out`. An
//! inactive (empty) track maps to `None`.

use palmier_model::{Clip, ClipType, Track};

use crate::placement::ClipPlacement;
use crate::snap::SnapClip;
use crate::split::{SplitClip, VolumeKeyframe};

/// A clip is "image/text" — has no bounded source media, so its trim fields are
/// uncapped (reference `hasNoSourceMedia`, ruling carry-forward).
pub fn has_no_source_media(media_type: ClipType) -> bool {
    matches!(media_type, ClipType::Image | ClipType::Text)
}

/// `Clip → ClipPlacement` for a clip on track `track_index`.
///
/// The placement carries the span/track slice plus `speed` + `trim_start_frame`
/// (the overwrite engine recomputes source-trim offsets from these). `track_index`
/// is supplied by the caller because a `Clip` does not know which track it lives on.
pub fn clip_to_placement(clip: &Clip, track_index: usize) -> ClipPlacement {
    ClipPlacement {
        id: clip.id.clone(),
        start_frame: clip.start_frame,
        duration_frames: clip.duration_frames,
        track_index,
        speed: clip.speed,
        trim_start_frame: clip.trim_start_frame,
    }
}

/// `Clip → SnapClip` — just the id and the two edge frames the snap engine reads.
pub fn clip_to_snap_clip(clip: &Clip) -> SnapClip {
    SnapClip {
        id: clip.id.clone(),
        start_frame: clip.start_frame,
        end_frame: clip.end_frame(),
    }
}

/// Map a `palmier_model::KeyframeTrack<f64>` (dB, clip-relative) onto the flat
/// `Vec<VolumeKeyframe>` the split engine migrates. `None` / empty → `None`.
fn volume_track_to_view(
    track: &Option<palmier_model::KeyframeTrack<f64>>,
) -> Option<Vec<VolumeKeyframe>> {
    let track = track.as_ref()?;
    if track.keyframes.is_empty() {
        return None;
    }
    Some(
        track
            .keyframes
            .iter()
            .map(|k| VolumeKeyframe::new(k.frame, k.value, k.interpolation_out))
            .collect(),
    )
}

/// `Clip → SplitClip` — the richer view the split/trim math reads and rewrites.
///
/// Carries both trims, both fades, the volume keyframes (clip-relative dB), and
/// the `has_no_source_media` flag derived from the clip's current `media_type`.
pub fn clip_to_split_clip(clip: &Clip) -> SplitClip {
    SplitClip {
        id: clip.id.clone(),
        start_frame: clip.start_frame,
        duration_frames: clip.duration_frames,
        trim_start_frame: clip.trim_start_frame,
        trim_end_frame: clip.trim_end_frame,
        speed: clip.speed,
        fade_in_frames: clip.fade_in_frames,
        fade_out_frames: clip.fade_out_frames,
        volume_track: volume_track_to_view(&clip.volume_track),
        has_no_source_media: has_no_source_media(clip.media_type),
    }
}

/// Flatten a whole `Track`'s clips to placements (carrying the track index).
pub fn track_to_placements(track: &Track, track_index: usize) -> Vec<ClipPlacement> {
    track
        .clips
        .iter()
        .map(|c| clip_to_placement(c, track_index))
        .collect()
}

/// Collect every clip across every track as a flat `SnapClip` list (the snap
/// engine flattens tracks — see [`crate::snap::collect_targets`]).
pub fn tracks_to_snap_clips(tracks: &[Track]) -> Vec<SnapClip> {
    tracks
        .iter()
        .flat_map(|t| t.clips.iter().map(clip_to_snap_clip))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::{Interpolation, Keyframe, KeyframeTrack};

    fn sample_clip() -> Clip {
        let mut c = Clip::new("asset-1", 30, 60);
        c.id = "c1".into();
        c.trim_start_frame = 5;
        c.trim_end_frame = 7;
        c.speed = 1.5;
        c.fade_in_frames = 4;
        c.fade_out_frames = 6;
        c
    }

    // ---- Clip → view round-trip (the adapter seam) -----------------------

    #[test]
    fn clip_to_placement_carries_span_track_speed_trim() {
        let c = sample_clip();
        let p = clip_to_placement(&c, 2);
        assert_eq!(p.id, "c1");
        assert_eq!(p.start_frame, 30);
        assert_eq!(p.duration_frames, 60);
        assert_eq!(p.track_index, 2);
        assert_eq!(p.speed, 1.5);
        assert_eq!(p.trim_start_frame, 5);
        // The placement's derived end matches the clip's.
        assert_eq!(p.end_frame(), c.end_frame());
    }

    #[test]
    fn clip_to_snap_clip_carries_both_edges() {
        let c = sample_clip();
        let s = clip_to_snap_clip(&c);
        assert_eq!(s.id, "c1");
        assert_eq!(s.start_frame, 30);
        assert_eq!(s.end_frame, 90);
    }

    #[test]
    fn clip_to_split_clip_carries_trims_fades_and_flag() {
        let c = sample_clip();
        let s = clip_to_split_clip(&c);
        assert_eq!(s.id, "c1");
        assert_eq!(s.start_frame, 30);
        assert_eq!(s.duration_frames, 60);
        assert_eq!(s.trim_start_frame, 5);
        assert_eq!(s.trim_end_frame, 7);
        assert_eq!(s.speed, 1.5);
        assert_eq!(s.fade_in_frames, 4);
        assert_eq!(s.fade_out_frames, 6);
        // Video → media-bounded.
        assert!(!s.has_no_source_media);
        assert!(s.volume_track.is_none());
    }

    #[test]
    fn has_no_source_media_only_image_and_text() {
        assert!(has_no_source_media(ClipType::Image));
        assert!(has_no_source_media(ClipType::Text));
        assert!(!has_no_source_media(ClipType::Video));
        assert!(!has_no_source_media(ClipType::Audio));
        assert!(!has_no_source_media(ClipType::Lottie));
    }

    #[test]
    fn split_clip_view_flags_image_text_as_no_source_media() {
        let mut img = Clip::new("img", 0, 30);
        img.media_type = ClipType::Image;
        assert!(clip_to_split_clip(&img).has_no_source_media);

        let mut txt = Clip::new("txt", 0, 30);
        txt.media_type = ClipType::Text;
        assert!(clip_to_split_clip(&txt).has_no_source_media);
    }

    #[test]
    fn volume_track_maps_clip_relative_db_keyframes() {
        let mut c = Clip::new("aud", 0, 100);
        c.media_type = ClipType::Audio;
        let mut track = KeyframeTrack::new();
        track.upsert(Keyframe::with_interpolation(0, -6.0, Interpolation::Linear));
        track.upsert(Keyframe::with_interpolation(50, 0.0, Interpolation::Hold));
        c.volume_track = Some(track);

        let kfs = clip_to_split_clip(&c).volume_track.unwrap();
        assert_eq!(kfs.len(), 2);
        // Frames are clip-relative (the model already stores them that way).
        assert_eq!(kfs[0].frame, 0);
        assert_eq!(kfs[0].value, -6.0);
        assert_eq!(kfs[0].interpolation_out, Interpolation::Linear);
        assert_eq!(kfs[1].frame, 50);
        assert_eq!(kfs[1].value, 0.0);
        assert_eq!(kfs[1].interpolation_out, Interpolation::Hold);
    }

    #[test]
    fn empty_volume_track_maps_to_none() {
        let mut c = Clip::new("aud", 0, 100);
        c.volume_track = Some(KeyframeTrack::new()); // empty → inactive
        assert!(clip_to_split_clip(&c).volume_track.is_none());
    }

    #[test]
    fn track_and_tracks_flatten() {
        let mut v = Track::new(ClipType::Video);
        v.clips.push(Clip::new("a", 0, 30));
        v.clips.push(Clip::new("b", 30, 30));
        let placements = track_to_placements(&v, 0);
        assert_eq!(placements.len(), 2);
        assert!(placements.iter().all(|p| p.track_index == 0));

        let mut a = Track::new(ClipType::Audio);
        a.clips.push(Clip::new("c", 0, 60));
        let snaps = tracks_to_snap_clips(&[v, a]);
        // 2 video + 1 audio clips → 3 snap clips.
        assert_eq!(snaps.len(), 3);
    }
}
