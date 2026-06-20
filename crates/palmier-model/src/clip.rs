//! `Clip` — the core timeline entity + its computed/derived properties and the
//! render-critical value-sampling math (story E2-S5).
//!
//! Ported 1:1 from the macOS reference `Sources/PalmierPro/Models/Timeline.swift`
//! (`struct Clip`, all sampling methods, `FadeEdge`, the clamp/rescale wrappers).
//! See docs/reference/timeline-model.md "Data model" + "Clip value sampling" and
//! FOUNDATION §5.3.
//!
//! ## IDs are UUID **strings**, not typed `Uuid`
//!
//! `id`, `media_ref`, `link_group_id`, `caption_group_id` are plain `String`s
//! (reconciliation carry-forward; the reference stores `id: String =
//! UUID().uuidString`). Lenient decode regenerates a fresh UUID string when `id`
//! is absent (reference `init(from:)`: `(try? c.decode(...)) ?? UUID().uuidString`).
//!
//! ## Rounding: `f64::round`, ties-AWAY-from-zero
//!
//! `source_frames_consumed` and `timeline_frame` use Rust `f64::round`, which
//! rounds half-way cases **away from zero** — matching Swift's `Double.rounded()`
//! (`.toNearestOrAwayFromZero`). This is the carry-forward rule (PRD FR-7,
//! phase0-reconciliation): NEVER `round_ties_even` (banker's rounding). The
//! `source_frames_consumed_rounding_parity` test pins this on the x.5 cases.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::interpolation::{smoothstep, Interpolation};
use crate::keyframe::{
    clamp_keyframes_to_duration, rescale_keyframes, AnimPair, KeyframeTrack,
};
use crate::text_style::TextStyle;
use crate::transform::{Crop, Transform};
use crate::volume::VolumeScale;
use crate::ClipType;

/// Which edge of a clip a fade applies to (reference `enum FadeEdge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeEdge {
    Left,
    Right,
}

/// Fresh UUID string — used as the serde default for `Clip::id` so a project
/// missing the field decodes with a regenerated id (reference `init(from:)`).
fn new_uuid_string() -> String {
    Uuid::new_v4().to_string()
}

fn default_speed() -> f64 {
    1.0
}
fn default_volume() -> f64 {
    1.0
}
fn default_opacity() -> f64 {
    1.0
}
fn default_media_type() -> ClipType {
    ClipType::Video
}
fn default_fade_interp() -> Interpolation {
    // Reference: fadeIn/OutInterpolation default `.linear` (NOT the keyframe
    // Smooth default of ruling #8 — fades default Linear).
    Interpolation::Linear
}

/// A single clip on a track: a placed, trimmed, speed-scaled reference to one
/// media asset, plus its transform/crop, fades, static opacity/volume, and up to
/// six optional keyframe tracks.
///
/// Wire keys are the reference's `CodingKeys` (bare camelCase Swift property
/// names). Every optional field is `#[serde(default)]` so old/partial projects
/// decode (lenient decode is load-bearing). `media_ref`, `start_frame`, and
/// `duration_frames` are the only required fields (the reference decodes them
/// non-optionally).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// UUID **string** (regenerated if absent on decode).
    #[serde(default = "new_uuid_string")]
    pub id: String,
    /// Manifest id of the backing media (plain string).
    #[serde(rename = "mediaRef")]
    pub media_ref: String,
    /// Current media type (defaults to `video`).
    #[serde(rename = "mediaType", default = "default_media_type")]
    pub media_type: ClipType,
    /// Original media type for derived clips (color-coding).
    #[serde(rename = "sourceClipType", default = "default_media_type")]
    pub source_clip_type: ClipType,
    /// Start frame on the timeline.
    #[serde(rename = "startFrame")]
    pub start_frame: i32,
    /// Visible duration in timeline frames.
    #[serde(rename = "durationFrames")]
    pub duration_frames: i32,
    #[serde(rename = "trimStartFrame", default)]
    pub trim_start_frame: i32,
    #[serde(rename = "trimEndFrame", default)]
    pub trim_end_frame: i32,
    /// Playback speed multiplier (source frames per timeline frame).
    #[serde(default = "default_speed")]
    pub speed: f64,
    /// Static **linear** volume gain (keyframe volumes are dB — see `volume_at`).
    #[serde(default = "default_volume")]
    pub volume: f64,
    #[serde(rename = "fadeInFrames", default)]
    pub fade_in_frames: i32,
    #[serde(rename = "fadeOutFrames", default)]
    pub fade_out_frames: i32,
    #[serde(rename = "fadeInInterpolation", default = "default_fade_interp")]
    pub fade_in_interpolation: Interpolation,
    #[serde(rename = "fadeOutInterpolation", default = "default_fade_interp")]
    pub fade_out_interpolation: Interpolation,
    /// Static opacity (overridden by the opacity track when active).
    #[serde(default = "default_opacity")]
    pub opacity: f64,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub crop: Crop,
    #[serde(rename = "linkGroupId", default, skip_serializing_if = "Option::is_none")]
    pub link_group_id: Option<String>,
    #[serde(
        rename = "captionGroupId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub caption_group_id: Option<String>,
    #[serde(rename = "textContent", default, skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
    #[serde(rename = "textStyle", default, skip_serializing_if = "Option::is_none")]
    pub text_style: Option<TextStyle>,

    // Six optional keyframe tracks. `None` when no animation exists.
    #[serde(rename = "opacityTrack", default, skip_serializing_if = "Option::is_none")]
    pub opacity_track: Option<KeyframeTrack<f64>>,
    #[serde(
        rename = "positionTrack",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub position_track: Option<KeyframeTrack<AnimPair>>,
    #[serde(rename = "scaleTrack", default, skip_serializing_if = "Option::is_none")]
    pub scale_track: Option<KeyframeTrack<AnimPair>>,
    #[serde(
        rename = "rotationTrack",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub rotation_track: Option<KeyframeTrack<f64>>,
    #[serde(rename = "cropTrack", default, skip_serializing_if = "Option::is_none")]
    pub crop_track: Option<KeyframeTrack<Crop>>,
    #[serde(rename = "volumeTrack", default, skip_serializing_if = "Option::is_none")]
    pub volume_track: Option<KeyframeTrack<f64>>,
}

impl Clip {
    /// Minimal constructor for a video clip (the rest defaulted to reference
    /// values). Tests / edit code build clips from this and mutate.
    pub fn new(media_ref: impl Into<String>, start_frame: i32, duration_frames: i32) -> Self {
        Clip {
            id: new_uuid_string(),
            media_ref: media_ref.into(),
            media_type: ClipType::Video,
            source_clip_type: ClipType::Video,
            start_frame,
            duration_frames,
            trim_start_frame: 0,
            trim_end_frame: 0,
            speed: 1.0,
            volume: 1.0,
            fade_in_frames: 0,
            fade_out_frames: 0,
            fade_in_interpolation: Interpolation::Linear,
            fade_out_interpolation: Interpolation::Linear,
            opacity: 1.0,
            transform: Transform::default(),
            crop: Crop::default(),
            link_group_id: None,
            caption_group_id: None,
            text_content: None,
            text_style: None,
            opacity_track: None,
            position_track: None,
            scale_track: None,
            rotation_track: None,
            crop_track: None,
            volume_track: None,
        }
    }

    // ---- Derived properties ----

    /// Frame where this clip ends on the timeline (reference `endFrame`).
    pub fn end_frame(&self) -> i32 {
        self.start_frame + self.duration_frames
    }

    /// Source frames consumed by the visible portion.
    ///
    /// **`f64::round` ties-AWAY-from-zero** (reference
    /// `Int((Double(durationFrames) * speed).rounded())`; Swift `.rounded()` is
    /// `.toNearestOrAwayFromZero`). NOT `round_ties_even`. This is the
    /// carry-forward rounding rule that the
    /// `source_frames_consumed_rounding_parity` test pins (satisfies E3-S1).
    pub fn source_frames_consumed(&self) -> i32 {
        (self.duration_frames as f64 * self.speed).round() as i32
    }

    /// Total source frames referenced, including both trims (reference
    /// `sourceDurationFrames`).
    pub fn source_duration_frames(&self) -> i32 {
        self.source_frames_consumed() + self.trim_start_frame + self.trim_end_frame
    }

    /// Absolute timeline frame `frame` is inside this clip's `[start, end)`
    /// (reference `contains(timelineFrame:)`).
    pub fn contains(&self, frame: i32) -> bool {
        frame >= self.start_frame && frame < self.end_frame()
    }

    /// Convert an absolute timeline frame to the clip-relative offset used by
    /// keyframe-track storage (reference `keyframeOffset(forFrame:)`).
    #[inline]
    fn keyframe_offset(&self, frame: i32) -> i32 {
        frame - self.start_frame
    }

    // ---- Value sampling (render-critical math) ----

    /// Effective opacity at `frame` = raw opacity × fade envelope.
    ///
    /// Fade is applied **only when `media_type != audio` AND a fade exists**
    /// (reference `opacityAt`).
    pub fn opacity_at(&self, frame: i32) -> f64 {
        let base = self.raw_opacity_at(frame);
        if self.media_type != ClipType::Audio && (self.fade_in_frames > 0 || self.fade_out_frames > 0)
        {
            base * self.fade_multiplier(frame)
        } else {
            base
        }
    }

    /// Authored opacity without the fade envelope (reference `rawOpacityAt`):
    /// the opacity track sample with the static `opacity` as fallback.
    pub fn raw_opacity_at(&self, frame: i32) -> f64 {
        match &self.opacity_track {
            Some(t) => t.sample(self.keyframe_offset(frame), self.opacity),
            None => self.opacity,
        }
    }

    /// Sampled rotation (degrees) at `frame` (reference `rotationAt`).
    pub fn rotation_at(&self, frame: i32) -> f64 {
        match &self.rotation_track {
            Some(t) => t.sample(self.keyframe_offset(frame), self.transform.rotation),
            None => self.transform.rotation,
        }
    }

    /// Sampled top-left (normalized canvas space) at `frame` (reference
    /// `topLeftAt`): the position track if active, else `transform.center`
    /// minus half the sampled size.
    pub fn top_left_at(&self, frame: i32) -> (f64, f64) {
        if let Some(track) = self.position_track.as_ref().filter(|t| t.is_active()) {
            let p = track.sample(self.keyframe_offset(frame), AnimPair::new(0.0, 0.0));
            return (p.a, p.b);
        }
        let (cx, cy) = self.transform.center();
        let (w, h) = self.size_at(frame);
        (cx - w / 2.0, cy - h / 2.0)
    }

    /// Sampled (width, height) at `frame` (reference `sizeAt`): the scale track
    /// (`AnimPair` a=w, b=h) if present, else `transform.width/height`.
    pub fn size_at(&self, frame: i32) -> (f64, f64) {
        let fallback = AnimPair::new(self.transform.width, self.transform.height);
        let s = match &self.scale_track {
            Some(t) => t.sample(self.keyframe_offset(frame), fallback),
            None => fallback,
        };
        (s.a, s.b)
    }

    /// Resolve the full `Transform` at `frame` (reference `transformAt`):
    /// top-left + size build the center, rotation comes from the rotation track.
    /// Flip flags carry from the static transform (reference rebuilds via
    /// `Transform(topLeft:width:height:)`, which leaves flips at their default
    /// `false` — but we preserve the static flips, which is the render intent).
    pub fn transform_at(&self, frame: i32) -> Transform {
        let (tlx, tly) = self.top_left_at(frame);
        let (w, h) = self.size_at(frame);
        let mut t = Transform::from_top_left((tlx, tly), w, h);
        t.rotation = self.rotation_at(frame);
        t.flip_horizontal = self.transform.flip_horizontal;
        t.flip_vertical = self.transform.flip_vertical;
        t
    }

    /// Sampled crop at `frame` (reference `cropAt`): the crop track if present,
    /// else the static `crop`.
    pub fn crop_at(&self, frame: i32) -> Crop {
        match &self.crop_track {
            Some(t) => t.sample(self.keyframe_offset(frame), self.crop),
            None => self.crop,
        }
    }

    /// Effective **linear** volume at `frame` = static linear volume × keyframe
    /// gain × fade (reference `volumeAt`).
    ///
    /// `kf_gain = linear_from_db(volume_track.sample(.., fallback = 0 dB))` when
    /// the volume track is active, else `1.0`. **Volume keyframe values are dB**;
    /// static `volume` is linear.
    pub fn volume_at(&self, frame: i32) -> f64 {
        self.volume * self.kf_gain(frame) * self.fade_multiplier(frame)
    }

    /// Linear volume without the fade envelope (reference `rawVolumeAt`).
    pub fn raw_volume_at(&self, frame: i32) -> f64 {
        self.volume * self.kf_gain(frame)
    }

    /// Keyframe gain (linear) from the dB volume track; `1.0` when inactive.
    fn kf_gain(&self, frame: i32) -> f64 {
        match &self.volume_track {
            Some(t) if t.is_active() => {
                let db = t.sample(self.keyframe_offset(frame), 0.0);
                VolumeScale::linear_from_db(db)
            }
            _ => 1.0,
        }
    }

    /// The 0…1 fade envelope at `frame` (reference `fadeMultiplier`).
    ///
    /// `rel = frame − start_frame`; returns `0` outside `[0, duration]`.
    /// `in_mul = fade_in>0 ? (Smooth ? smoothstep(t) : t) : 1` where
    /// `t = min(1, rel/fade_in)`. `out_mul` is symmetric on `duration − rel`.
    /// Returns `min(in_mul, out_mul)`. **Linear and Hold both ramp linearly for
    /// fades; only `Smooth` bends.**
    pub fn fade_multiplier(&self, frame: i32) -> f64 {
        let rel = frame - self.start_frame;
        if rel < 0 || rel > self.duration_frames {
            return 0.0;
        }
        let in_mul = if self.fade_in_frames > 0 {
            let t = (rel as f64 / self.fade_in_frames as f64).min(1.0);
            if self.fade_in_interpolation == Interpolation::Smooth {
                smoothstep(t)
            } else {
                t
            }
        } else {
            1.0
        };
        let out_rem = self.duration_frames - rel;
        let out_mul = if self.fade_out_frames > 0 {
            let t = (out_rem as f64 / self.fade_out_frames as f64).min(1.0);
            if self.fade_out_interpolation == Interpolation::Smooth {
                smoothstep(t)
            } else {
                t
            }
        } else {
            1.0
        };
        in_mul.min(out_mul)
    }

    /// Map source-seconds → project-timeline-frame through this clip's placement,
    /// trim, and speed (reference `timelineFrame(sourceSeconds:fps:)`).
    ///
    /// `source_frame = t·fps`; `offset = source_frame − trim_start` (`None` if
    /// `< 0`); **`frame = f64::round(start_frame + offset / max(speed, 1e-4))`**
    /// (ties-away); `None` unless the result is in `[start_frame, end_frame)`.
    /// Consumed by Epic 7/10 (transcript-seconds → timeline-frame).
    pub fn timeline_frame(&self, source_seconds: f64, fps: i32) -> Option<i32> {
        let source_frame = source_seconds * fps as f64;
        let offset_from_trim = source_frame - self.trim_start_frame as f64;
        if offset_from_trim < 0.0 {
            return None;
        }
        // ties-away `f64::round`, matching Swift `.rounded()`.
        let frame =
            (self.start_frame as f64 + offset_from_trim / self.speed.max(0.0001)).round() as i32;
        if frame >= self.start_frame && frame < self.end_frame() {
            Some(frame)
        } else {
            None
        }
    }

    // ---- Mutation helpers (clamp / fades / duration) ----

    /// Whether a transform animation is present (reference `hasTransformAnimation`).
    pub fn has_transform_animation(&self) -> bool {
        self.position_track.as_ref().is_some_and(|t| t.is_active())
            || self.scale_track.as_ref().is_some_and(|t| t.is_active())
            || self.rotation_track.as_ref().is_some_and(|t| t.is_active())
    }

    /// Drop keyframes outside `[0, duration]` on every track (reference
    /// `clampKeyframesToDuration`).
    pub fn clamp_keyframes_to_duration(&mut self) {
        let d = self.duration_frames;
        self.opacity_track = clamp_keyframes_to_duration(self.opacity_track.take(), d);
        self.position_track = clamp_keyframes_to_duration(self.position_track.take(), d);
        self.scale_track = clamp_keyframes_to_duration(self.scale_track.take(), d);
        self.rotation_track = clamp_keyframes_to_duration(self.rotation_track.take(), d);
        self.crop_track = clamp_keyframes_to_duration(self.crop_track.take(), d);
        self.volume_track = clamp_keyframes_to_duration(self.volume_track.take(), d);
    }

    /// Multiply every keyframe frame by `scale` (reference `rescaleKeyframes`,
    /// used on a speed change).
    pub fn rescale_keyframes(&mut self, scale: f64) {
        self.opacity_track = rescale_keyframes(self.opacity_track.take(), scale);
        self.position_track = rescale_keyframes(self.position_track.take(), scale);
        self.scale_track = rescale_keyframes(self.scale_track.take(), scale);
        self.rotation_track = rescale_keyframes(self.rotation_track.take(), scale);
        self.crop_track = rescale_keyframes(self.crop_track.take(), scale);
        self.volume_track = rescale_keyframes(self.volume_track.take(), scale);
    }

    /// Clamp fade ramps so head + tail can't exceed the duration (reference
    /// `clampFadesToDuration`).
    pub fn clamp_fades_to_duration(&mut self) {
        self.fade_in_frames = self.fade_in_frames.clamp(0, self.duration_frames);
        self.fade_out_frames = self
            .fade_out_frames
            .clamp(0, self.duration_frames - self.fade_in_frames);
    }

    /// Set the fade length for one edge and clamp to fit (reference `setFade`).
    pub fn set_fade(&mut self, edge: FadeEdge, frames: i32) {
        let v = frames.max(0);
        match edge {
            FadeEdge::Left => self.fade_in_frames = v,
            FadeEdge::Right => self.fade_out_frames = v,
        }
        self.clamp_fades_to_duration();
    }

    /// Set the new duration and run clamp + fade-clamp (reference `setDuration`).
    pub fn set_duration(&mut self, new_duration: i32) {
        self.duration_frames = new_duration;
        self.clamp_keyframes_to_duration();
        self.clamp_fades_to_duration();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keyframe::Keyframe;

    #[test]
    fn derived_end_and_source_frames() {
        let mut c = Clip::new("m", 30, 60);
        assert_eq!(c.end_frame(), 90);
        // speed 1.0 → consumed == duration.
        assert_eq!(c.source_frames_consumed(), 60);
        // source_duration adds both trims.
        c.trim_start_frame = 5;
        c.trim_end_frame = 3;
        assert_eq!(c.source_duration_frames(), 60 + 5 + 3);
    }

    #[test]
    fn source_frames_consumed_non_integer_rounds() {
        // duration 10, speed 1.7 → 17.0 exactly.
        let mut c = Clip::new("m", 0, 10);
        c.speed = 1.7;
        assert_eq!(c.source_frames_consumed(), 17);
        // duration 3, speed 1.5 → 4.5 → round ties-away → 5.
        let mut c2 = Clip::new("m", 0, 3);
        c2.speed = 1.5;
        assert_eq!(c2.source_frames_consumed(), 5);
    }

    /// The carry-forward rounding-parity test (E2-S5 acceptance / E3-S1
    /// carry-forward): `f64::round` ties-AWAY-from-zero, NOT `round_ties_even`.
    /// Speed ∈ {0.25, 0.5, 1.0, 1.7, 4.0} on durations chosen to hit x.5 cases.
    #[test]
    fn source_frames_consumed_rounding_parity() {
        // (duration, speed, expected ties-away result, what round_ties_even would give)
        // Pick durations so duration*speed lands on an exact .5 to prove the tie
        // direction (away-from-zero, matching Swift `.rounded()`).
        let cases = [
            // speed 0.5: duration 3 → 1.5 → away→2 ; banker's→2 (even) — not a divergence,
            //            duration 5 → 2.5 → away→3 ; banker's→2 (even) — DIVERGENCE.
            (5, 0.5_f64, 3),
            // duration 7 → 3.5 → away→4 ; banker's→4 (even) — not divergence.
            //            duration 9 → 4.5 → away→5 ; banker's→4 — DIVERGENCE.
            (9, 0.5, 5),
            // speed 0.25: duration 2 → 0.5 → away→1 ; banker's→0 — DIVERGENCE.
            (2, 0.25, 1),
            //            duration 6 → 1.5 → away→2 ; banker's→2 — not divergence.
            //            duration 10 → 2.5 → away→3 ; banker's→2 — DIVERGENCE.
            (10, 0.25, 3),
            // speed 1.0: integer, no rounding.
            (60, 1.0, 60),
            // speed 1.7: duration 5 → 8.5 → away→9 ; banker's→8 — DIVERGENCE.
            (5, 1.7, 9),
            // speed 4.0: integer.
            (7, 4.0, 28),
        ];
        for (dur, speed, expected) in cases {
            let mut c = Clip::new("m", 0, dur);
            c.speed = speed;
            assert_eq!(
                c.source_frames_consumed(),
                expected,
                "ties-away parity failed for duration={dur} speed={speed}"
            );
            // Explicitly confirm f64::round (away) is what we use, and that the
            // banker's-rounding result would differ on the divergence cases.
            let product = dur as f64 * speed;
            assert_eq!(product.round() as i32, expected);
        }

        // Direct divergence proof: 2.5_f64.round() == 3 (away), not 2 (banker's).
        assert_eq!(2.5_f64.round() as i32, 3);
        assert_eq!((-2.5_f64).round() as i32, -3);
    }

    #[test]
    fn fade_multiplier_edges_linear_vs_smooth() {
        // Linear fade-in over 10 frames on a 100-frame clip starting at 0.
        let mut c = Clip::new("m", 0, 100);
        c.fade_in_frames = 10;
        c.fade_in_interpolation = Interpolation::Linear;
        // rel 0 → t=0 → 0.
        assert_eq!(c.fade_multiplier(0), 0.0);
        // rel 5 → t=0.5 → linear 0.5.
        assert!((c.fade_multiplier(5) - 0.5).abs() < 1e-12);
        // rel 10 (== fade len) → t=1 → 1.0 (no fade-out so out_mul=1).
        assert!((c.fade_multiplier(10) - 1.0).abs() < 1e-12);
        // Outside [0, duration] → 0.
        assert_eq!(c.fade_multiplier(-1), 0.0);
        assert_eq!(c.fade_multiplier(101), 0.0);

        // Smooth fade-in: rel 5 → t=0.5 → smoothstep(0.5)=0.5 (coincides at mid),
        // rel 2 → t=0.2 → smoothstep=0.104.
        c.fade_in_interpolation = Interpolation::Smooth;
        assert!((c.fade_multiplier(2) - 0.104).abs() < 1e-12);

        // Hold behaves as linear ramp for fades (only Smooth bends).
        c.fade_in_interpolation = Interpolation::Hold;
        assert!((c.fade_multiplier(2) - 0.2).abs() < 1e-12);
    }

    #[test]
    fn fade_multiplier_fade_out_symmetric() {
        let mut c = Clip::new("m", 0, 100);
        c.fade_out_frames = 10;
        c.fade_out_interpolation = Interpolation::Linear;
        // rel 100 → out_rem 0 → t=0 → 0.
        assert_eq!(c.fade_multiplier(100), 0.0);
        // rel 95 → out_rem 5 → t=0.5 → 0.5.
        assert!((c.fade_multiplier(95) - 0.5).abs() < 1e-12);
        // rel 90 → out_rem 10 → t=1 → 1.0.
        assert!((c.fade_multiplier(90) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn volume_at_db_keyframe_vs_static_linear() {
        // Static linear volume only (no track): volume_at == volume (no fades).
        let mut c = Clip::new("m", 0, 100);
        c.media_type = ClipType::Audio;
        c.volume = 0.5;
        assert!((c.volume_at(50) - 0.5).abs() < 1e-12);

        // Add a dB volume track at 0 dB (unity) → kf_gain 1.0 → still 0.5.
        let mut track = KeyframeTrack::new();
        track.upsert(Keyframe::new(0, 0.0)); // 0 dB
        track.upsert(Keyframe::new(100, 0.0));
        c.volume_track = Some(track);
        assert!((c.volume_at(50) - 0.5).abs() < 1e-12);

        // A +6 dB keyframe ≈ ×1.995 linear gain → 0.5 * ~1.995 ≈ ~0.997.
        let mut track2 = KeyframeTrack::new();
        track2.upsert(Keyframe::new(0, 6.0));
        track2.upsert(Keyframe::new(100, 6.0));
        c.volume_track = Some(track2);
        let expected = 0.5 * VolumeScale::linear_from_db(6.0);
        assert!((c.volume_at(50) - expected).abs() < 1e-12);

        // raw_volume_at omits the fade (here equal since no fade).
        assert!((c.raw_volume_at(50) - expected).abs() < 1e-12);
    }

    #[test]
    fn opacity_at_audio_vs_visual_fade_gating() {
        // Visual clip with a fade: fade is applied.
        let mut v = Clip::new("m", 0, 100);
        v.media_type = ClipType::Video;
        v.fade_in_frames = 10;
        v.fade_in_interpolation = Interpolation::Linear;
        // rel 5 → fade 0.5 → opacity 1.0 * 0.5 = 0.5.
        assert!((v.opacity_at(5) - 0.5).abs() < 1e-12);

        // Audio clip with the same fade: fade NOT applied to opacity → stays raw.
        let mut a = Clip::new("m", 0, 100);
        a.media_type = ClipType::Audio;
        a.fade_in_frames = 10;
        assert!((a.opacity_at(5) - 1.0).abs() < 1e-12);

        // Visual clip with NO fade: raw opacity (track or static).
        let mut s = Clip::new("m", 0, 100);
        s.opacity = 0.3;
        assert!((s.opacity_at(5) - 0.3).abs() < 1e-12);
    }

    #[test]
    fn timeline_frame_in_and_out_of_range() {
        // Clip at start 30, duration 60 (frames 30..90), speed 1, no trim, fps 30.
        let c = Clip::new("m", 30, 60);
        // source 1.0s → 30 source frames → offset 30 → frame round(30 + 30/1) = 60.
        assert_eq!(c.timeline_frame(1.0, 30), Some(60));
        // source 0.0s → frame 30 (start, in range).
        assert_eq!(c.timeline_frame(0.0, 30), Some(30));
        // source 2.0s → frame 90 == end_frame → OUT (half-open [start, end)).
        assert_eq!(c.timeline_frame(2.0, 30), None);

        // With trim_start: a source time before the trim window → None.
        let mut t = Clip::new("m", 0, 60);
        t.trim_start_frame = 30;
        // source 0.0s → offset = 0 - 30 = -30 < 0 → None.
        assert_eq!(t.timeline_frame(0.0, 30), None);
        // source 1.0s → 30 source frames → offset 0 → frame 0 (start, in range).
        assert_eq!(t.timeline_frame(1.0, 30), Some(0));
    }

    #[test]
    fn transform_at_uses_tracks_when_active() {
        let mut c = Clip::new("m", 0, 100);
        c.transform = Transform {
            center_x: 0.5,
            center_y: 0.5,
            width: 0.4,
            height: 0.4,
            rotation: 10.0,
            ..Transform::default()
        };
        // No tracks → transform_at rebuilds from static center/size/rotation.
        let t0 = c.transform_at(50);
        assert!((t0.center_x - 0.5).abs() < 1e-12);
        assert!((t0.width - 0.4).abs() < 1e-12);
        assert!((t0.rotation - 10.0).abs() < 1e-12);

        // Active position track overrides top-left.
        let mut pos = KeyframeTrack::new();
        pos.upsert(Keyframe::new(0, AnimPair::new(0.1, 0.2)));
        pos.upsert(Keyframe::new(100, AnimPair::new(0.1, 0.2)));
        c.position_track = Some(pos);
        let t1 = c.transform_at(50);
        let (tlx, tly) = t1.top_left();
        assert!((tlx - 0.1).abs() < 1e-12);
        assert!((tly - 0.2).abs() < 1e-12);
    }

    #[test]
    fn missing_id_decodes_with_regenerated_uuid() {
        // A clip JSON without `id` decodes with a fresh, well-formed UUID string.
        let json = r#"{"mediaRef":"asset-1","startFrame":0,"durationFrames":30}"#;
        let c: Clip = serde_json::from_str(json).unwrap();
        assert!(!c.id.is_empty());
        // Parses as a UUID.
        assert!(Uuid::parse_str(&c.id).is_ok(), "id `{}` is not a UUID", c.id);
        // Defaults applied.
        assert_eq!(c.media_type, ClipType::Video);
        assert_eq!(c.speed, 1.0);
        assert_eq!(c.volume, 1.0);
        assert_eq!(c.opacity, 1.0);
        assert_eq!(c.fade_in_interpolation, Interpolation::Linear);
    }

    #[test]
    fn clip_round_trips_with_tracks() {
        let mut c = Clip::new("asset-1", 10, 50);
        c.id = "fixed-id".to_string();
        c.link_group_id = Some("grp".to_string());
        c.text_content = Some("hi".to_string());
        let mut op = KeyframeTrack::new();
        op.upsert(Keyframe::with_interpolation(0, 0.0, Interpolation::Linear));
        op.upsert(Keyframe::with_interpolation(50, 1.0, Interpolation::Linear));
        c.opacity_track = Some(op);

        let json = serde_json::to_string(&c).unwrap();
        let back: Clip = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn set_duration_clamps_keyframes_and_fades() {
        let mut c = Clip::new("m", 0, 100);
        c.fade_in_frames = 60;
        c.fade_out_frames = 60;
        let mut op = KeyframeTrack::new();
        op.upsert(Keyframe::new(0, 0.0_f64));
        op.upsert(Keyframe::new(80, 1.0));
        c.opacity_track = Some(op);

        // Shrink to 50: kf at 80 is dropped; fades clamp so in+out ≤ 50.
        c.set_duration(50);
        let kf_frames: Vec<i32> = c
            .opacity_track
            .as_ref()
            .unwrap()
            .keyframes
            .iter()
            .map(|k| k.frame)
            .collect();
        assert_eq!(kf_frames, vec![0]);
        assert!(c.fade_in_frames + c.fade_out_frames <= 50);
        assert_eq!(c.fade_in_frames, 50);
        assert_eq!(c.fade_out_frames, 0);
    }
}
