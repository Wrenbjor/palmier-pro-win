//! Keyframe storage + the deterministic `sample(at:)` algorithm (story E2-S3).
//!
//! Ported 1:1 from the macOS reference `Sources/PalmierPro/Models/Keyframe.swift`
//! (`Keyframe`, `KeyframeTrack`, `AnimPair`, `sample`, `upsert`, `move`, and the
//! clamp/rescale helpers). See docs/reference/timeline-model.md "Keyframes &
//! sampling" and FOUNDATION §5.5.
//!
//! ## Frames are CLIP-RELATIVE in storage
//!
//! Every `Keyframe::frame` is stored **clip-relative** (offset from the owning
//! clip's `start_frame`). The reference's public API converts to/from absolute
//! timeline frames via `to_abs / to_offset = frame ± start_frame`
//! (`Models/Keyframe.swift` `toAbs`/`toOffset`). The [`to_abs`] / [`to_offset`]
//! free functions document that seam so callers never double-offset. The
//! `Clip` value-sampling methods (E2-S5) own the actual conversion when they
//! sample these tracks.
//!
//! ## Default interpolation = `Smooth` (ruling #8)
//!
//! `Keyframe::interpolation_out` defaults to [`Interpolation::Smooth`] — both the
//! reference (`interpolationOut: Interpolation = .smooth`) and reconciliation
//! ruling #8 override FOUNDATION §5.2/§5.5's "linear". An absent field decodes to
//! `Smooth` via `#[serde(default)]`.

use serde::{Deserialize, Serialize};

use crate::interpolation::{lerp, smoothstep, Interpolation, KeyframeInterpolatable};

/// Convert a clip-relative keyframe offset to an absolute timeline frame.
///
/// Mirrors the reference `Clip.toAbs(_:) = startFrame + offset`. Storage is
/// clip-relative; the public API is absolute. Exposed as a free function so the
/// clip-relative ↔ absolute seam is documented in one place (E2-S3 acceptance:
/// "document this seam to avoid double-offset").
#[inline]
pub fn to_abs(offset: i32, start_frame: i32) -> i32 {
    start_frame + offset
}

/// Convert an absolute timeline frame to a clip-relative keyframe offset.
///
/// Mirrors the reference `Clip.toOffset(_:) = timelineFrame - startFrame`.
#[inline]
pub fn to_offset(timeline_frame: i32, start_frame: i32) -> i32 {
    timeline_frame - start_frame
}

/// A single keyframe: a value at a clip-relative `frame`, plus how the segment
/// **leaving** this keyframe interpolates toward the next one.
///
/// Wire keys are the bare field names (`frame`, `value`, `interpolationOut`),
/// matching the Swift `Keyframe<Value>: Codable` derived encoding. `value` is
/// generic so the same shape stores `f64` (opacity/rotation/volume-dB),
/// [`AnimPair`] (position/scale), and [`Crop`](crate::Crop) (crop track).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe<V> {
    /// Clip-relative frame index (offset from the clip's `start_frame`).
    pub frame: i32,
    /// The keyframed value at this frame.
    pub value: V,
    /// Interpolation of the segment *leaving* this keyframe (ruling #8: default
    /// `Smooth`). The `sample` algorithm switches on the **left** keyframe's
    /// `interpolation_out`.
    #[serde(rename = "interpolationOut", default)]
    pub interpolation_out: Interpolation,
}

impl<V> Keyframe<V> {
    /// Construct a keyframe with the default (`Smooth`) interpolation, matching
    /// the reference's memberwise initializer with the defaulted argument.
    pub fn new(frame: i32, value: V) -> Self {
        Keyframe {
            frame,
            value,
            interpolation_out: Interpolation::Smooth,
        }
    }

    /// Construct a keyframe with an explicit interpolation.
    pub fn with_interpolation(frame: i32, value: V, interpolation_out: Interpolation) -> Self {
        Keyframe {
            frame,
            value,
            interpolation_out,
        }
    }
}

/// An ordered, frame-unique list of [`Keyframe`]s for one animatable property.
///
/// Wire key is `keyframes` (matching `KeyframeTrack<Value>: Codable`). Empty by
/// default; a track is "active" only when it holds at least one keyframe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyframeTrack<V> {
    #[serde(default = "Vec::new")]
    pub keyframes: Vec<Keyframe<V>>,
}

impl<V> Default for KeyframeTrack<V> {
    fn default() -> Self {
        KeyframeTrack {
            keyframes: Vec::new(),
        }
    }
}

impl<V> KeyframeTrack<V> {
    /// An empty track.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reference `isActive = !keyframes.isEmpty`.
    pub fn is_active(&self) -> bool {
        !self.keyframes.is_empty()
    }
}

impl<V: Clone> KeyframeTrack<V> {
    /// Insert or replace the keyframe at `kf.frame`, keeping the vec sorted by
    /// frame with unique frames (reference `upsert`). If a keyframe already
    /// exists at that frame it is overwritten; otherwise the new keyframe is
    /// inserted at the first index whose frame is greater (stable sort order).
    pub fn upsert(&mut self, kf: Keyframe<V>) {
        if let Some(i) = self.keyframes.iter().position(|k| k.frame == kf.frame) {
            self.keyframes[i] = kf;
        } else {
            let at = self
                .keyframes
                .iter()
                .position(|k| k.frame > kf.frame)
                .unwrap_or(self.keyframes.len());
            self.keyframes.insert(at, kf);
        }
    }

    /// Remove every keyframe at `frame` (reference `remove(at:)`).
    pub fn remove(&mut self, frame: i32) {
        self.keyframes.retain(|k| k.frame != frame);
    }

    /// Move the keyframe at `old_frame` to `new_frame`.
    ///
    /// **No-op if the target frame is already occupied** (reference
    /// `move(from:to:)`): `if newFrame != oldFrame && occupied(newFrame) { return }`.
    /// Also a no-op if no keyframe exists at `old_frame`.
    pub fn move_keyframe(&mut self, old_frame: i32, new_frame: i32) {
        let Some(i) = self.keyframes.iter().position(|k| k.frame == old_frame) else {
            return;
        };
        if new_frame != old_frame && self.keyframes.iter().any(|k| k.frame == new_frame) {
            return;
        }
        let mut kf = self.keyframes.remove(i);
        kf.frame = new_frame;
        self.upsert(kf);
    }
}

impl<V: KeyframeInterpolatable + Clone> KeyframeTrack<V> {
    /// Sample the track at a (clip-relative) `frame`, returning `fallback` when
    /// the track is empty.
    ///
    /// Verbatim port of the reference `KeyframeTrack.sample(at:fallback:)`
    /// (docs/reference/timeline-model.md "Keyframes & sampling"):
    /// - empty → `fallback`;
    /// - single keyframe → that value;
    /// - `frame ≤ first.frame` → first value;
    /// - `frame ≥ last.frame` → last value;
    /// - otherwise find the first keyframe `b` with `b.frame > frame`, take the
    ///   segment `[a, b]` (`a` is its predecessor), compute
    ///   `raw = (frame − a.frame) / (b.frame − a.frame)`, then switch on **`a`'s**
    ///   `interpolation_out`: `Hold → a`, `Linear → lerp(a, b, raw)`,
    ///   `Smooth → lerp(a, b, smoothstep(raw))`.
    pub fn sample(&self, frame: i32, fallback: V) -> V {
        if self.keyframes.is_empty() {
            return fallback;
        }
        if self.keyframes.len() == 1 {
            return self.keyframes[0].value.clone();
        }
        let first = &self.keyframes[0];
        let last = &self.keyframes[self.keyframes.len() - 1];
        if frame <= first.frame {
            return first.value.clone();
        }
        if frame >= last.frame {
            return last.value.clone();
        }

        let Some(b_idx) = self.keyframes.iter().position(|k| k.frame > frame) else {
            return last.value.clone();
        };
        let a = &self.keyframes[b_idx - 1];
        let b = &self.keyframes[b_idx];
        // Reference uses Double division; (b.frame - a.frame) is guaranteed > 0
        // here because frame is strictly between first and last and the vec is
        // sorted with unique frames.
        let raw = (frame - a.frame) as f64 / (b.frame - a.frame) as f64;
        match a.interpolation_out {
            Interpolation::Hold => a.value.clone(),
            Interpolation::Linear => V::keyframe_interpolate(a.value.clone(), b.value.clone(), raw),
            Interpolation::Smooth => {
                V::keyframe_interpolate(a.value.clone(), b.value.clone(), smoothstep(raw))
            }
        }
    }

    /// Drop keyframes outside `[0, duration]` (clip-relative), returning `None`
    /// when the result is empty — reference `clampedKeyframeTrack`. Used after a
    /// duration shrink (reference `clampKeyframesToDuration`).
    pub fn clamp_to_duration(self, duration: i32) -> Option<Self> {
        let mut normalized = KeyframeTrack::new();
        for kf in self.keyframes.into_iter() {
            if kf.frame >= 0 && kf.frame <= duration {
                normalized.upsert(kf);
            }
        }
        if normalized.keyframes.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }

    /// Multiply every keyframe frame by `scale` (with ties-away `f64::round`),
    /// used on a speed change — reference `rescaledKeyframeTrack`. A non-finite
    /// or non-positive `scale` returns the track unchanged.
    pub fn rescale(self, scale: f64) -> Option<Self> {
        if !scale.is_finite() || scale <= 0.0 {
            return if self.keyframes.is_empty() {
                None
            } else {
                Some(self)
            };
        }
        let mut normalized = KeyframeTrack::new();
        for mut kf in self.keyframes.into_iter() {
            // ties-away `f64::round` (carry-forward rule), matching Swift `.rounded()`.
            kf.frame = (kf.frame as f64 * scale).round() as i32;
            normalized.upsert(kf);
        }
        if normalized.keyframes.is_empty() {
            None
        } else {
            Some(normalized)
        }
    }
}

/// Free helper for the `Option<KeyframeTrack>` clamp pattern the clip uses on
/// every track. Mirrors the reference `clampedKeyframeTrack(_:)` signature.
pub fn clamp_keyframes_to_duration<V: KeyframeInterpolatable + Clone>(
    track: Option<KeyframeTrack<V>>,
    duration: i32,
) -> Option<KeyframeTrack<V>> {
    track.and_then(|t| t.clamp_to_duration(duration))
}

/// Free helper for the `Option<KeyframeTrack>` rescale pattern (speed change).
pub fn rescale_keyframes<V: KeyframeInterpolatable + Clone>(
    track: Option<KeyframeTrack<V>>,
    scale: f64,
) -> Option<KeyframeTrack<V>> {
    track.and_then(|t| t.rescale(scale))
}

/// A two-component keyframe value: **position `(x, y)`** AND **scale `(w, h)`**.
///
/// Reference `struct AnimPair { a, b }` (`Models/Keyframe.swift`). Component-wise
/// `KeyframeInterpolatable`. Wire keys are `a` / `b`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AnimPair {
    pub a: f64,
    pub b: f64,
}

impl AnimPair {
    pub fn new(a: f64, b: f64) -> Self {
        AnimPair { a, b }
    }
}

impl KeyframeInterpolatable for AnimPair {
    /// Component-wise lerp (reference `AnimPair.keyframeInterpolate`).
    fn keyframe_interpolate(from: AnimPair, to: AnimPair, t: f64) -> AnimPair {
        AnimPair {
            a: lerp(from.a, to.a, t),
            b: lerp(from.b, to.b, t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Crop;

    #[test]
    fn keyframe_default_interpolation_is_smooth() {
        // Ruling #8: absent `interpolationOut` decodes to Smooth.
        let kf: Keyframe<f64> = serde_json::from_str(r#"{"frame":0,"value":1.0}"#).unwrap();
        assert_eq!(kf.interpolation_out, Interpolation::Smooth);
        // `new` also defaults to Smooth.
        assert_eq!(Keyframe::new(5, 2.0).interpolation_out, Interpolation::Smooth);
    }

    #[test]
    fn keyframe_wire_keys_match_reference() {
        let kf = Keyframe::with_interpolation(3, 0.5_f64, Interpolation::Linear);
        let json = serde_json::to_string(&kf).unwrap();
        assert_eq!(json, r#"{"frame":3,"value":0.5,"interpolationOut":"linear"}"#);
        let back: Keyframe<f64> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kf);
    }

    #[test]
    fn track_is_active() {
        let mut t: KeyframeTrack<f64> = KeyframeTrack::new();
        assert!(!t.is_active());
        t.upsert(Keyframe::new(0, 1.0));
        assert!(t.is_active());
    }

    #[test]
    fn upsert_keeps_sorted_and_unique() {
        let mut t: KeyframeTrack<f64> = KeyframeTrack::new();
        t.upsert(Keyframe::new(10, 1.0));
        t.upsert(Keyframe::new(0, 2.0));
        t.upsert(Keyframe::new(5, 3.0));
        let frames: Vec<i32> = t.keyframes.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![0, 5, 10]);
        // upsert at an existing frame overwrites.
        t.upsert(Keyframe::new(5, 99.0));
        assert_eq!(t.keyframes.len(), 3);
        assert_eq!(t.keyframes[1].value, 99.0);
    }

    #[test]
    fn move_is_noop_when_target_occupied() {
        let mut t: KeyframeTrack<f64> = KeyframeTrack::new();
        t.upsert(Keyframe::new(0, 1.0));
        t.upsert(Keyframe::new(10, 2.0));
        // Target frame 10 is occupied → no-op.
        t.move_keyframe(0, 10);
        let frames: Vec<i32> = t.keyframes.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![0, 10]);
        assert_eq!(t.keyframes[0].value, 1.0);
        // Moving to a free frame works.
        t.move_keyframe(0, 5);
        let frames: Vec<i32> = t.keyframes.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![5, 10]);
    }

    // ---- Golden boundary sampling (FOUNDATION §11.1, PRD §10 gate (a)) ----
    //
    // For each interpolation (Smooth / Linear / Hold) we assert `sample` at
    // `t=0`, `t=end`, exact-on-key, and a between-keys frame against
    // hand-computed reference values. Both a 2-keyframe and a 3-keyframe track.
    mod keyframe_boundary_sampling {
        use super::*;

        /// Build a 2-keyframe f64 track [0→0.0, 10→1.0] with the given out-interp
        /// on the FIRST keyframe (the segment-leaving interp drives sampling).
        fn two_kf(interp: Interpolation) -> KeyframeTrack<f64> {
            let mut t = KeyframeTrack::new();
            t.upsert(Keyframe::with_interpolation(0, 0.0, interp));
            t.upsert(Keyframe::with_interpolation(10, 1.0, interp));
            t
        }

        #[test]
        fn empty_track_returns_fallback() {
            let t: KeyframeTrack<f64> = KeyframeTrack::new();
            assert_eq!(t.sample(0, 0.42), 0.42);
            assert_eq!(t.sample(100, -1.0), -1.0);
        }

        #[test]
        fn single_keyframe_returns_its_value_everywhere() {
            let mut t = KeyframeTrack::new();
            t.upsert(Keyframe::new(5, 7.0_f64));
            // Even far before/after, a 1-kf track returns the kf value, NOT fallback.
            assert_eq!(t.sample(-100, 0.0), 7.0);
            assert_eq!(t.sample(5, 0.0), 7.0);
            assert_eq!(t.sample(1000, 0.0), 7.0);
        }

        #[test]
        fn linear_boundaries_and_midpoint() {
            let t = two_kf(Interpolation::Linear);
            // t=0 (≤ first) → first value.
            assert_eq!(t.sample(0, -9.0), 0.0);
            // before first → clamps to first.
            assert_eq!(t.sample(-5, -9.0), 0.0);
            // exact-on-key (last) → last value.
            assert_eq!(t.sample(10, -9.0), 1.0);
            // after last → clamps to last.
            assert_eq!(t.sample(50, -9.0), 1.0);
            // between: frame 5 → raw 0.5 → lerp(0,1,0.5)=0.5.
            assert!((t.sample(5, -9.0) - 0.5).abs() < 1e-12);
            // frame 3 → raw 0.3 → 0.3.
            assert!((t.sample(3, -9.0) - 0.3).abs() < 1e-12);
        }

        #[test]
        fn smooth_boundaries_and_midpoint() {
            let t = two_kf(Interpolation::Smooth);
            assert_eq!(t.sample(0, -9.0), 0.0);
            assert_eq!(t.sample(10, -9.0), 1.0);
            // frame 5 → raw 0.5 → smoothstep(0.5)=0.5 → lerp(0,1,0.5)=0.5.
            assert!((t.sample(5, -9.0) - 0.5).abs() < 1e-12);
            // frame 2 → raw 0.2 → smoothstep(0.2)=0.04*(3-0.4)=0.04*2.6=0.104.
            assert!((t.sample(2, -9.0) - 0.104).abs() < 1e-12);
            // frame 8 → raw 0.8 → smoothstep(0.8)=0.64*(3-1.6)=0.64*1.4=0.896.
            assert!((t.sample(8, -9.0) - 0.896).abs() < 1e-12);
        }

        #[test]
        fn hold_returns_left_keyframe_until_next_key() {
            let t = two_kf(Interpolation::Hold);
            assert_eq!(t.sample(0, -9.0), 0.0);
            // Hold: any frame strictly before the last key holds the left value.
            assert_eq!(t.sample(3, -9.0), 0.0);
            assert_eq!(t.sample(9, -9.0), 0.0);
            // Exact-on-last-key → last value (clamp branch).
            assert_eq!(t.sample(10, -9.0), 1.0);
        }

        #[test]
        fn three_keyframe_mixed_interpolation() {
            // [0→0.0 Linear, 10→1.0 Hold, 20→3.0 Smooth].
            let mut t = KeyframeTrack::new();
            t.upsert(Keyframe::with_interpolation(0, 0.0, Interpolation::Linear));
            t.upsert(Keyframe::with_interpolation(10, 1.0, Interpolation::Hold));
            t.upsert(Keyframe::with_interpolation(20, 3.0, Interpolation::Smooth));

            // t=0 → first.
            assert_eq!(t.sample(0, -9.0), 0.0);
            // First segment Linear: frame 5 → 0.5.
            assert!((t.sample(5, -9.0) - 0.5).abs() < 1e-12);
            // Exact-on-middle-key → that value.
            assert_eq!(t.sample(10, -9.0), 1.0);
            // Second segment Hold: frame 15 holds the middle value (1.0).
            assert_eq!(t.sample(15, -9.0), 1.0);
            // t=end → last value.
            assert_eq!(t.sample(20, -9.0), 3.0);
        }
    }

    #[test]
    fn anim_pair_interpolates_componentwise() {
        let a = AnimPair::new(0.0, 10.0);
        let b = AnimPair::new(2.0, 20.0);
        let mid = AnimPair::keyframe_interpolate(a, b, 0.5);
        assert!((mid.a - 1.0).abs() < 1e-12);
        assert!((mid.b - 15.0).abs() < 1e-12);
    }

    #[test]
    fn anim_pair_track_samples() {
        let mut t = KeyframeTrack::new();
        t.upsert(Keyframe::with_interpolation(
            0,
            AnimPair::new(0.0, 0.0),
            Interpolation::Linear,
        ));
        t.upsert(Keyframe::with_interpolation(
            10,
            AnimPair::new(1.0, 2.0),
            Interpolation::Linear,
        ));
        let s = t.sample(5, AnimPair::new(0.0, 0.0));
        assert!((s.a - 0.5).abs() < 1e-12);
        assert!((s.b - 1.0).abs() < 1e-12);
    }

    #[test]
    fn crop_track_samples() {
        // Crop (from E2-S2) is KeyframeInterpolatable — confirm a crop track samples.
        let mut t = KeyframeTrack::new();
        t.upsert(Keyframe::with_interpolation(
            0,
            Crop::default(),
            Interpolation::Linear,
        ));
        t.upsert(Keyframe::with_interpolation(
            10,
            Crop {
                left: 0.2,
                top: 0.4,
                right: 0.0,
                bottom: 0.0,
            },
            Interpolation::Linear,
        ));
        let s = t.sample(5, Crop::default());
        assert!((s.left - 0.1).abs() < 1e-12);
        assert!((s.top - 0.2).abs() < 1e-12);
    }

    #[test]
    fn clamp_to_duration_drops_out_of_range() {
        let mut t = KeyframeTrack::new();
        t.upsert(Keyframe::new(-5, 1.0_f64));
        t.upsert(Keyframe::new(0, 2.0));
        t.upsert(Keyframe::new(10, 3.0));
        t.upsert(Keyframe::new(20, 4.0));
        // duration 10 keeps frames in [0, 10].
        let clamped = t.clamp_to_duration(10).unwrap();
        let frames: Vec<i32> = clamped.keyframes.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![0, 10]);

        // All-out-of-range collapses to None.
        let mut t2 = KeyframeTrack::new();
        t2.upsert(Keyframe::new(50, 1.0_f64));
        assert!(t2.clamp_to_duration(10).is_none());
    }

    #[test]
    fn rescale_multiplies_frames_ties_away() {
        let mut t = KeyframeTrack::new();
        t.upsert(Keyframe::new(2, 1.0_f64));
        t.upsert(Keyframe::new(5, 2.0));
        // scale 0.5: frame 2 → 1, frame 5 → round(2.5) = 3 (ties away from zero).
        let r = t.rescale(0.5).unwrap();
        let frames: Vec<i32> = r.keyframes.iter().map(|k| k.frame).collect();
        assert_eq!(frames, vec![1, 3]);

        // Non-positive scale returns unchanged.
        let mut t2 = KeyframeTrack::new();
        t2.upsert(Keyframe::new(2, 1.0_f64));
        let r2 = t2.rescale(0.0).unwrap();
        assert_eq!(r2.keyframes[0].frame, 2);
    }

    #[test]
    fn clip_relative_abs_offset_seam() {
        // Document the seam: to_abs / to_offset are inverses around start_frame.
        let start = 30;
        let abs = to_abs(7, start);
        assert_eq!(abs, 37);
        assert_eq!(to_offset(abs, start), 7);
    }
}
