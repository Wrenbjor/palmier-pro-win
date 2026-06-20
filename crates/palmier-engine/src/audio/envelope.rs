//! Per-clip volume envelope — the offset-set / piecewise-linear ramp algorithm.
//!
//! Verbatim port of the macOS reference `CompositionBuilder.buildVisuals` audio-mix
//! branch (`emitVolumeEnvelope` + the shared `emitEnvelopeRamps`) plus the
//! `Clip.volumeAt(frame:)` / `fadeMultiplier(at:)` sampling
//! (`Sources/PalmierPro/Preview/CompositionBuilder.swift`,
//! `Sources/PalmierPro/Models/Timeline.swift`). See docs/reference/preview-engine.md
//! "Audio mix (`buildVisuals`)".
//!
//! The reference emits AVFoundation `setVolumeRamp` instructions; AVFoundation then
//! interpolates **linearly** between instruction times, so the reference *pre-bakes*
//! smooth curves by subdividing each segment into `SMOOTH_SEGMENTS = 8` linear ramps.
//! Our mixer applies gain per audio frame, so we keep the **same** subdivision +
//! `smoothstep` so a rendered envelope is byte-for-byte the reference's pre-baked
//! ramp set (preview-engine.md risk #4 / carry-forward "smoothSegments=8").
//!
//! ## Input model (E2 not yet landed)
//!
//! E5-S6's stated dep — Epic 2 `palmier-model` clip volume/fade/speed — is not on
//! main yet (only the E2-S1 leaf enums are). The fields the mixer reads are defined
//! here as [`AudioClip`] / [`VolumeKeyframe`], mirroring the reference `Clip` audio
//! fields exactly (`volume`, `volumeTrack`, `fadeIn/OutFrames`, `fadeIn/Out
//! Interpolation`, `speed`, `startFrame`, `durationFrames`). When E2-S2..S4 land the
//! real `Clip`, this becomes a thin `From<&Clip>` adapter — the algorithm is
//! unchanged. Reuses `palmier_model::{Interpolation, smoothstep}` (already on main).

use palmier_model::{smoothstep, Interpolation};

use super::volume_scale::linear_from_db;

/// Smooth-curve subdivision count for non-linear keyframe segments.
///
/// Verbatim from the reference `CompositionBuilder.smoothSegments` (carry-forward
/// port-critical constant). Changing this drifts every smooth fade/volume curve.
pub const SMOOTH_SEGMENTS: i32 = 8;

/// A single volume keyframe, stored in **clip-relative** frame offsets and **dB**
/// (matching `clip.volumeTrack` in the reference — `track.sample(...)` returns dB
/// that `volumeAt` runs through `VolumeScale.linearFromDb`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeKeyframe {
    /// Clip-relative frame offset (`0..=duration_frames`).
    pub frame: i32,
    /// Authored gain in dB.
    pub db: f64,
    /// How this keyframe interpolates to the next.
    pub interpolation_out: Interpolation,
}

/// The audio-relevant projection of a timeline clip the mixer consumes.
///
/// Mirrors the reference `Clip` audio fields (see module docs). Frames are integer
/// timeline frames; keyframe offsets are clip-relative.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioClip {
    /// Absolute timeline start frame.
    pub start_frame: i32,
    /// Visible duration in timeline frames (`endFrame = start_frame + duration_frames`).
    pub duration_frames: i32,
    /// Static (outer) linear gain — `clip.volume`.
    pub volume: f64,
    /// Playback speed multiplier; `1.0` == natural speed.
    pub speed: f64,
    /// Optional keyframe track (clip-relative offsets, dB values). Empty == inactive.
    pub volume_keyframes: Vec<VolumeKeyframe>,
    /// Fade-in length in frames (`0` == none).
    pub fade_in_frames: i32,
    /// Fade-out length in frames (`0` == none).
    pub fade_out_frames: i32,
    /// Fade-in easing (only `.linear` / `.smooth` are meaningful for fades).
    pub fade_in_interpolation: Interpolation,
    /// Fade-out easing.
    pub fade_out_interpolation: Interpolation,
}

impl AudioClip {
    /// Absolute timeline end frame (exclusive of the last sample's tail, matching the
    /// reference `endFrame = startFrame + durationFrames`).
    #[inline]
    pub fn end_frame(&self) -> i32 {
        self.start_frame + self.duration_frames
    }

    #[inline]
    fn has_active_keyframes(&self) -> bool {
        !self.volume_keyframes.is_empty()
    }

    #[inline]
    fn has_fade(&self) -> bool {
        self.fade_in_frames > 0 || self.fade_out_frames > 0
    }

    /// Sample the **dB** keyframe track at a clip-relative offset (linear/hold/smooth),
    /// returning `fallback` when no track is active. Port of
    /// `KeyframeTrack.sample(at:fallback:)` specialized for `f64`/dB.
    fn sample_keyframe_db(&self, offset: i32, fallback: f64) -> f64 {
        let kfs = &self.volume_keyframes;
        if kfs.is_empty() {
            return fallback;
        }
        if kfs.len() == 1 {
            return kfs[0].db;
        }
        if offset <= kfs[0].frame {
            return kfs[0].db;
        }
        let last = kfs[kfs.len() - 1];
        if offset >= last.frame {
            return last.db;
        }
        // First kf strictly past `offset`.
        let b_idx = kfs.iter().position(|k| k.frame > offset).unwrap_or(kfs.len() - 1);
        let a = kfs[b_idx - 1];
        let b = kfs[b_idx];
        let raw = (offset - a.frame) as f64 / (b.frame - a.frame) as f64;
        match a.interpolation_out {
            Interpolation::Hold => a.db,
            Interpolation::Linear => palmier_model::lerp(a.db, b.db, raw),
            Interpolation::Smooth => palmier_model::lerp(a.db, b.db, smoothstep(raw)),
        }
    }

    /// `0..=1` envelope from the fade head/tail ramps at an absolute timeline `frame`.
    ///
    /// Verbatim port of `Clip.fadeMultiplier(at:)`.
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

    /// Effective **linear** volume at an absolute timeline `frame`.
    ///
    /// Verbatim port of `Clip.volumeAt(frame:)`: `volume * kfGain * fadeMultiplier`,
    /// where `kfGain = VolumeScale.linearFromDb(keyframe dB)` (or `1.0` when no track).
    pub fn volume_at(&self, frame: i32) -> f64 {
        let kf_gain = if self.has_active_keyframes() {
            let db = self.sample_keyframe_db(frame - self.start_frame, 0.0);
            linear_from_db(db)
        } else {
            1.0
        };
        self.volume * kf_gain * self.fade_multiplier(frame)
    }
}

/// A single linear gain ramp over a clip-relative `[start_offset, end_offset)` frame
/// span, with start/end **linear** gains. Equivalent to one reference
/// `setVolumeRamp(from:to:timeRange:)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeRamp {
    /// Clip-relative start offset (frames).
    pub start_offset: i32,
    /// Clip-relative end offset (frames, exclusive).
    pub end_offset: i32,
    /// Linear gain at `start_offset`.
    pub start_gain: f32,
    /// Linear gain at `end_offset`.
    pub end_gain: f32,
}

/// Interior subdivision offsets for a smooth ramp between two clip-relative offsets
/// (excluding endpoints). Verbatim port of `CompositionBuilder.smoothSubdivisions`.
///
/// Dedups + sorts after rounding (`Set` in the reference), so adjacent integer frames
/// never produce a zero-length ramp.
fn smooth_subdivisions(a: i32, b: i32) -> Vec<i32> {
    if b <= a {
        return Vec::new();
    }
    let span = (b - a) as f64;
    let mut raw: Vec<i32> = (1..SMOOTH_SEGMENTS)
        .map(|s| a + (span * s as f64 / SMOOTH_SEGMENTS as f64).round() as i32)
        .collect();
    raw.sort_unstable();
    raw.dedup();
    raw
}

/// Drop keyframes whose offset falls outside `0..=duration`, then sort by frame.
/// Last-write-wins on duplicate frames (reference uses a `[Int: Keyframe]` map).
/// Port of `CompositionBuilder.normalizedKeyframes`.
fn normalized_keyframes(kfs: &[VolumeKeyframe], duration: i32) -> Vec<VolumeKeyframe> {
    use std::collections::BTreeMap;
    let mut keyed: BTreeMap<i32, VolumeKeyframe> = BTreeMap::new();
    for &kf in kfs {
        if kf.frame >= 0 && kf.frame <= duration {
            keyed.insert(kf.frame, kf);
        }
    }
    keyed.into_values().collect()
}

/// Build the piecewise-linear volume envelope for a clip as a list of [`VolumeRamp`]s
/// in clip-relative offsets.
///
/// Verbatim port of `emitVolumeEnvelope` + `emitEnvelopeRamps`:
/// - **No keyframes and no fade** → a single flat ramp `[0, dur)` at `volumeAt(start)`
///   (skipped if the gain is non-finite or `dur <= 0`).
/// - **Else** → build the offset set `{0, dur} ∪ keyframes ∪ smooth/hold subdivisions
///   ∪ fadeIn/Out edges (+ their smooth subdivisions)`, sort, and emit one ramp per
///   adjacent pair sampling `volumeAt(start + offset)` at each boundary.
///
/// The same offset-set algorithm drives opacity in E5-S5; this is its audio twin
/// (preview-engine.md "the same offset-set / piecewise-linear ramp algorithm").
pub fn build_volume_envelope(clip: &AudioClip) -> Vec<VolumeRamp> {
    let dur = clip.duration_frames;
    if dur <= 0 {
        return Vec::new();
    }

    // Flat fast-path: no keyframes, no fade → one ramp at the static volume.
    if !clip.has_active_keyframes() && !clip.has_fade() {
        let v = clip.volume_at(clip.start_frame) as f32;
        if !v.is_finite() {
            return Vec::new();
        }
        return vec![VolumeRamp {
            start_offset: 0,
            end_offset: dur,
            start_gain: v,
            end_gain: v,
        }];
    }

    let kfs = normalized_keyframes(&clip.volume_keyframes, dur);

    // Offset set: endpoints + keyframes + per-segment subdivisions.
    use std::collections::BTreeSet;
    let mut offsets: BTreeSet<i32> = BTreeSet::new();
    offsets.insert(0);
    offsets.insert(dur);
    for kf in &kfs {
        offsets.insert(kf.frame);
    }
    // Per keyframe-segment subdivision (smooth → 8 interior points; hold → b-1).
    for i in 0..kfs.len().saturating_sub(1) {
        let a = kfs[i];
        let b = kfs[i + 1];
        match a.interpolation_out {
            Interpolation::Smooth => {
                for o in smooth_subdivisions(a.frame, b.frame) {
                    offsets.insert(o);
                }
            }
            Interpolation::Hold => {
                if b.frame - a.frame > 1 {
                    offsets.insert(b.frame - 1);
                }
            }
            Interpolation::Linear => {}
        }
    }
    // Fade-in edge + (smooth) subdivisions.
    if clip.fade_in_frames > 0 {
        let end_offset = dur.min(clip.fade_in_frames);
        offsets.insert(end_offset);
        if clip.fade_in_interpolation == Interpolation::Smooth {
            for o in smooth_subdivisions(0, end_offset) {
                offsets.insert(o);
            }
        }
    }
    // Fade-out edge + (smooth) subdivisions.
    if clip.fade_out_frames > 0 {
        let start_offset = (dur - clip.fade_out_frames).max(0);
        offsets.insert(start_offset);
        if clip.fade_out_interpolation == Interpolation::Smooth {
            for o in smooth_subdivisions(start_offset, dur) {
                offsets.insert(o);
            }
        }
    }

    let sorted: Vec<i32> = offsets.into_iter().collect();
    let mut ramps = Vec::with_capacity(sorted.len().saturating_sub(1));
    for w in sorted.windows(2) {
        let (a_off, b_off) = (w[0], w[1]);
        if b_off <= a_off {
            continue;
        }
        let start_gain = clip.volume_at(clip.start_frame + a_off) as f32;
        let end_gain = clip.volume_at(clip.start_frame + b_off) as f32;
        ramps.push(VolumeRamp {
            start_offset: a_off,
            end_offset: b_off,
            start_gain,
            end_gain,
        });
    }
    ramps
}

/// Sample the rendered envelope at a clip-relative offset (linear gain), interpolating
/// inside whichever [`VolumeRamp`] covers it. This is what the per-sample mixer calls.
///
/// Equivalent to evaluating the reference's emitted `setVolumeRamp` instruction list:
/// before the first ramp → first ramp's start gain; after the last → last end gain.
pub fn sample_envelope(ramps: &[VolumeRamp], offset: i32) -> f32 {
    if ramps.is_empty() {
        return 0.0;
    }
    if offset <= ramps[0].start_offset {
        return ramps[0].start_gain;
    }
    let last = &ramps[ramps.len() - 1];
    if offset >= last.end_offset {
        return last.end_gain;
    }
    for r in ramps {
        if offset >= r.start_offset && offset < r.end_offset {
            let span = (r.end_offset - r.start_offset) as f32;
            if span <= 0.0 {
                return r.start_gain;
            }
            let t = (offset - r.start_offset) as f32 / span;
            return r.start_gain + (r.end_gain - r.start_gain) * t;
        }
    }
    last.end_gain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_clip(volume: f64) -> AudioClip {
        AudioClip {
            start_frame: 0,
            duration_frames: 30,
            volume,
            speed: 1.0,
            volume_keyframes: Vec::new(),
            fade_in_frames: 0,
            fade_out_frames: 0,
            fade_in_interpolation: Interpolation::Smooth,
            fade_out_interpolation: Interpolation::Smooth,
        }
    }

    #[test]
    fn flat_no_kf_no_fade_emits_single_ramp() {
        // preview-engine.md: "no keyframes and no fade → one flat setVolumeRamp".
        let clip = flat_clip(0.8);
        let env = build_volume_envelope(&clip);
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].start_offset, 0);
        assert_eq!(env[0].end_offset, 30);
        assert!((env[0].start_gain - 0.8).abs() < 1e-6);
        assert!((env[0].end_gain - 0.8).abs() < 1e-6);
    }

    #[test]
    fn zero_duration_emits_nothing() {
        let mut clip = flat_clip(1.0);
        clip.duration_frames = 0;
        assert!(build_volume_envelope(&clip).is_empty());
    }

    #[test]
    fn fade_in_ramps_from_zero_to_full() {
        let mut clip = flat_clip(1.0);
        clip.fade_in_frames = 10;
        clip.fade_in_interpolation = Interpolation::Linear;
        let env = build_volume_envelope(&clip);
        // Start silent, full by the fade-in edge.
        assert!((sample_envelope(&env, 0) - 0.0).abs() < 1e-6);
        assert!((sample_envelope(&env, 10) - 1.0).abs() < 1e-6);
        // Linear midpoint.
        assert!((sample_envelope(&env, 5) - 0.5).abs() < 1e-6);
        // Holds full after the fade.
        assert!((sample_envelope(&env, 20) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fade_out_ramps_to_zero_at_end() {
        let mut clip = flat_clip(1.0);
        clip.fade_out_frames = 10;
        clip.fade_out_interpolation = Interpolation::Linear;
        let env = build_volume_envelope(&clip);
        // Full until the fade-out start (frame 20), zero at end.
        assert!((sample_envelope(&env, 20) - 1.0).abs() < 1e-6);
        assert!((sample_envelope(&env, 25) - 0.5).abs() < 1e-6);
        assert!((sample_envelope(&env, 30) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn smooth_fade_in_uses_smoothstep_subdivisions() {
        // Smooth fade must follow smoothstep, NOT linear — proves 8-segment parity.
        let mut clip = flat_clip(1.0);
        clip.fade_in_frames = 8;
        clip.fade_in_interpolation = Interpolation::Smooth;
        let env = build_volume_envelope(&clip);
        // At the midpoint smoothstep(0.5)=0.5 — same as linear there.
        assert!((sample_envelope(&env, 4) - 0.5).abs() < 1e-6);
        // At 1/4, smoothstep(0.25)=0.15625; linear would be 0.25 — must NOT be 0.25.
        let v = sample_envelope(&env, 2);
        assert!((v - 0.156_25).abs() < 1e-3, "smoothstep value, got {v}");
        assert!((v - 0.25).abs() > 0.05, "must not be the linear value");
    }

    #[test]
    fn keyframe_db_envelope_folds_static_gain() {
        // volume(static) × keyframe(dB→linear). Two kfs: 0 dB → -6.0206 dB (≈0.5x).
        let mut clip = flat_clip(0.5);
        clip.volume_keyframes = vec![
            VolumeKeyframe { frame: 0, db: 0.0, interpolation_out: Interpolation::Linear },
            VolumeKeyframe { frame: 30, db: -6.020_599_91, interpolation_out: Interpolation::Linear },
        ];
        let env = build_volume_envelope(&clip);
        // At start: 0.5 * linear(0 dB)=1.0 = 0.5
        assert!((sample_envelope(&env, 0) - 0.5).abs() < 1e-4);
        // At end: 0.5 * linear(-6.02 dB)≈0.5 = 0.25
        assert!((sample_envelope(&env, 30) - 0.25).abs() < 1e-3);
    }

    #[test]
    fn hold_keyframe_steps() {
        let mut clip = flat_clip(1.0);
        clip.volume_keyframes = vec![
            VolumeKeyframe { frame: 0, db: 0.0, interpolation_out: Interpolation::Hold },
            VolumeKeyframe { frame: 20, db: -60.0, interpolation_out: Interpolation::Hold },
        ];
        let env = build_volume_envelope(&clip);
        // Holds unity right up to the last frame before the step.
        assert!((sample_envelope(&env, 19) - 1.0).abs() < 1e-4);
        // After the second kf, hard mute (linear_from_db(-60) == 0).
        assert!((sample_envelope(&env, 25) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_keyframes_dropped_from_offset_set() {
        // Parity with the reference: `volumeTrack.isActive` is true for ANY non-empty
        // raw track, so the keyframe *path* is taken and `volumeAt` samples the raw
        // track (matching `Clip.volumeAt` / `KeyframeTrack.sample`). Out-of-range kfs
        // are dropped only from the *offset set* (`normalizedKeyframes` in
        // `emitEnvelopeRamps`), not from sampling. Here both kfs sit at -60 dB, so the
        // sampled gain is hard-mute (0), exactly as the reference would emit.
        let mut clip = flat_clip(1.0);
        clip.volume_keyframes = vec![
            VolumeKeyframe { frame: -5, db: -60.0, interpolation_out: Interpolation::Linear },
            VolumeKeyframe { frame: 999, db: -60.0, interpolation_out: Interpolation::Linear },
        ];
        let env = build_volume_envelope(&clip);
        // Offset set has no interior kf → just the {0, dur} endpoints.
        assert_eq!(env.len(), 1);
        // Both raw kfs are -60 dB → linear 0 everywhere between them.
        assert!((sample_envelope(&env, 15) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn smooth_subdivision_count_and_dedup() {
        // 8 segments over [0,8] → interior points 1..=7 (7 points), endpoints excluded.
        assert_eq!(smooth_subdivisions(0, 8), vec![1, 2, 3, 4, 5, 6, 7]);
        // Short span: rounding collapses to the endpoints (deduped) — never a
        // zero-length ramp, since the offset set already carries {0, dur}.
        assert_eq!(smooth_subdivisions(0, 1), vec![0, 1]);
        assert!(smooth_subdivisions(5, 5).is_empty());
        assert!(smooth_subdivisions(8, 0).is_empty());
    }
}
