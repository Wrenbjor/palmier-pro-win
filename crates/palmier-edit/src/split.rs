//! E3-S4 — Split & Trim pure math (clip-relative, keyframe migration,
//! source↔timeline round-trip).
//!
//! Ported from `EditorViewModel+ClipMutations.swift` (`splitClip` /
//! `splitSingleClip`) and `EditorViewModel+Linking.swift` /
//! `EditorViewModel+Ripple.swift` (`trimValues`, `trimClipInternal`). See
//! docs/reference/edit-engines.md §"Split" (lines 99-110) and §"Trim"
//! (lines 112-128), and story E3-S4.
//!
//! ## Placement view (decoupling)
//!
//! Split/trim need more of a clip than ripple/overwrite do — fades, the source
//! `trim_end`, and the volume keyframe track — so this module operates over its
//! own [`SplitClip`] view (see `placement.rs` rationale). It deliberately does
//! **not** depend on the concurrently-built full `Clip` (E2-S5). The keyframe
//! sampling here re-implements the reference `KeyframeTrack.sample` semantics over
//! a local [`VolumeKeyframe`] list, using `palmier-model`'s `Interpolation`,
//! `lerp`, and `smoothstep` (which **are** on main).
//!
//! All `*speed` / `/speed` conversions use
//! [`round_ties_away`](crate::rounding::round_ties_away) (`f64::round`, ties
//! away).

use palmier_model::{lerp, smoothstep, Interpolation};

use crate::rounding::round_ties_away;

/// A single volume keyframe — `frame` is a **clip-relative offset** (FOUNDATION
/// §5.5), `value` is dB. Mirrors `palmier-model`'s `Keyframe<Double>` slice the
/// split needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeKeyframe {
    /// Clip-relative frame offset.
    pub frame: i32,
    /// Keyframe value (volume in dB).
    pub value: f64,
    /// How the segment from this keyframe to the next interpolates.
    pub interpolation_out: Interpolation,
}

impl VolumeKeyframe {
    /// Construct a keyframe at clip-relative `frame` with `value` and an explicit
    /// `interpolation_out`.
    pub fn new(frame: i32, value: f64, interpolation_out: Interpolation) -> Self {
        VolumeKeyframe {
            frame,
            value,
            interpolation_out,
        }
    }
}

/// Sample a sorted keyframe list at clip-relative `frame`, returning `fallback`
/// when empty.
///
/// Re-implements `KeyframeTrack.sample` (`Models/Keyframe.swift:232`): clamps to
/// the endpoints, then interpolates the bracketing segment by the *left*
/// keyframe's `interpolation_out` (`hold` holds, `linear` lerps, `smooth` eases
/// via `smoothstep`). Keyframes are assumed sorted by `frame` (the migration
/// preserves order).
pub fn sample_volume(keyframes: &[VolumeKeyframe], frame: i32, fallback: f64) -> f64 {
    if keyframes.is_empty() {
        return fallback;
    }
    if keyframes.len() == 1 {
        return keyframes[0].value;
    }
    if frame <= keyframes[0].frame {
        return keyframes[0].value;
    }
    let last = keyframes[keyframes.len() - 1];
    if frame >= last.frame {
        return last.value;
    }
    // First keyframe strictly after `frame` brackets the segment.
    let b_idx = keyframes
        .iter()
        .position(|k| k.frame > frame)
        .unwrap_or(keyframes.len() - 1);
    let a = keyframes[b_idx - 1];
    let b = keyframes[b_idx];
    let raw = (frame - a.frame) as f64 / (b.frame - a.frame) as f64;
    match a.interpolation_out {
        Interpolation::Hold => a.value,
        Interpolation::Linear => lerp(a.value, b.value, raw),
        Interpolation::Smooth => lerp(a.value, b.value, smoothstep(raw)),
    }
}

/// The slice of a clip the split / trim math reads and rewrites.
///
/// Timeline fields (`start_frame`, `duration_frames`) plus the source-domain trim
/// (`trim_start_frame` / `trim_end_frame`), `speed`, the fade lengths, the
/// `volume_track`, and the `has_no_source_media` flag (image/text — removes the
/// source trim cap so trim fields may go negative / extend freely; edit-engines.md
/// lines 115, 238).
#[derive(Debug, Clone, PartialEq)]
pub struct SplitClip {
    /// UUID-string clip id (the left/kept fragment keeps it; the right gets a new one).
    pub id: String,
    /// Timeline start frame.
    pub start_frame: i32,
    /// Timeline duration in frames.
    pub duration_frames: i32,
    /// Source frames trimmed off the head.
    pub trim_start_frame: i32,
    /// Source frames trimmed off the tail.
    pub trim_end_frame: i32,
    /// Playback speed (source frames per timeline frame).
    pub speed: f64,
    /// Fade-in length in timeline frames.
    pub fade_in_frames: i32,
    /// Fade-out length in timeline frames.
    pub fade_out_frames: i32,
    /// Volume keyframes (clip-relative offsets). `None` when the clip has no
    /// authored volume animation.
    pub volume_track: Option<Vec<VolumeKeyframe>>,
    /// `true` for image/text clips — they have no bounded source media, so trim
    /// fields are uncapped (reference `hasNoSourceMedia`).
    pub has_no_source_media: bool,
}

impl SplitClip {
    /// `end_frame = start_frame + duration_frames` (half-open span).
    pub fn end_frame(&self) -> i32 {
        self.start_frame + self.duration_frames
    }

    /// Clamp the fade lengths to the (current) duration — reference
    /// `clampFadesToDuration`: `fade_in ∈ [0, dur]`, `fade_out ∈ [0, dur-fade_in]`.
    fn clamp_fades_to_duration(&mut self) {
        self.fade_in_frames = self.fade_in_frames.clamp(0, self.duration_frames.max(0));
        let out_max = (self.duration_frames - self.fade_in_frames).max(0);
        self.fade_out_frames = self.fade_out_frames.clamp(0, out_max);
    }
}

/// Split `clip` at absolute timeline frame `at_frame` into `(left, right)`.
///
/// Returns `None` unless `start_frame < at_frame < end_frame` (reference guard).
/// `right` receives a **new id** from `new_id`. Math (edit-engines.md lines
/// 99-108):
/// - `split_offset = at_frame - start_frame`
/// - `left_source = round(split_offset · speed)`,
///   `right_source = round((duration - split_offset) · speed)`
/// - **left** = copy: `duration = split_offset`, `trim_end += right_source`,
///   `fade_out = 0`, clamp fades.
/// - **right** = copy + new id: `start = at_frame`,
///   `duration = orig.duration - split_offset`, `trim_start += left_source`,
///   `fade_in = 0`, clamp fades.
/// - **Volume keyframes** migrate around the cut (see [`migrate_volume_split`]).
pub fn split_clip(
    clip: &SplitClip,
    at_frame: i32,
    mut new_id: impl FnMut() -> String,
) -> Option<(SplitClip, SplitClip)> {
    if !(clip.start_frame < at_frame && at_frame < clip.end_frame()) {
        return None;
    }
    let split_offset = at_frame - clip.start_frame;
    let left_source = round_ties_away(split_offset as f64 * clip.speed);
    let right_source = round_ties_away((clip.duration_frames - split_offset) as f64 * clip.speed);

    let mut left = clip.clone();
    left.duration_frames = split_offset;
    left.trim_end_frame = clip.trim_end_frame + right_source;
    left.fade_out_frames = 0;
    left.clamp_fades_to_duration();

    let mut right = clip.clone();
    right.id = new_id();
    right.start_frame = at_frame;
    right.duration_frames = clip.duration_frames - split_offset;
    right.trim_start_frame = clip.trim_start_frame + left_source;
    right.fade_in_frames = 0;
    right.clamp_fades_to_duration();

    if let Some(track) = &clip.volume_track {
        let (left_kfs, right_kfs) = migrate_volume_split(track, split_offset);
        left.volume_track = if left_kfs.is_empty() { None } else { Some(left_kfs) };
        right.volume_track = if right_kfs.is_empty() {
            None
        } else {
            Some(right_kfs)
        };
    }

    Some((left, right))
}

/// Migrate a volume keyframe track across a split at clip-relative `split_offset`.
///
/// Reference `splitSingleClip` keyframe block (edit-engines.md lines 105-108):
/// the boundary value is `sample(at: split_offset)`.
/// - **left** keeps keyframes with `frame <= split_offset`; appends a boundary kf
///   at `split_offset` if the last kept frame isn't already there.
/// - **right** keeps keyframes with `frame >= split_offset`, **re-bases each by
///   `frame -= split_offset`**, and inserts a boundary kf at frame 0 if the first
///   isn't already there.
///
/// Keeps the curve continuous across the cut. Returns `(left_kfs, right_kfs)`.
pub fn migrate_volume_split(
    keyframes: &[VolumeKeyframe],
    split_offset: i32,
) -> (Vec<VolumeKeyframe>, Vec<VolumeKeyframe>) {
    let boundary_value = sample_volume(keyframes, split_offset, 0.0);

    // Left: keep frame <= split_offset; ensure a boundary kf at split_offset.
    let mut left: Vec<VolumeKeyframe> = keyframes
        .iter()
        .copied()
        .filter(|k| k.frame <= split_offset)
        .collect();
    if left.last().map(|k| k.frame) != Some(split_offset) {
        // Reference appends with default interpolation (Keyframe(frame:value:)
        // → interpolationOut defaults to Smooth, ruling #8).
        left.push(VolumeKeyframe::new(
            split_offset,
            boundary_value,
            Interpolation::default(),
        ));
    }

    // Right: keep frame >= split_offset, re-base by -split_offset, ensure frame 0.
    let mut right: Vec<VolumeKeyframe> = keyframes
        .iter()
        .copied()
        .filter(|k| k.frame >= split_offset)
        .map(|k| VolumeKeyframe::new(k.frame - split_offset, k.value, k.interpolation_out))
        .collect();
    if right.first().map(|k| k.frame) != Some(0) {
        right.insert(
            0,
            VolumeKeyframe::new(0, boundary_value, Interpolation::default()),
        );
    }

    (left, right)
}

/// Which edge of a clip a trim drag is grabbing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrimEdge {
    /// Left (head) edge — `trim_start_frame` moves.
    Left,
    /// Right (tail) edge — `trim_end_frame` moves.
    Right,
}

/// Compute the new `(trim_start_frame, trim_end_frame)` for a trim-drag commit.
///
/// Reference `trimValues` (edit-engines.md lines 119-122): converts the
/// timeline-frame `delta_frames` to a SOURCE-frame delta
/// (`source_delta = round(delta_frames · speed)`) and applies it to the grabbed
/// edge. For media-bounded clips the result is floored at 0 (can't trim before
/// the source's first frame); for `has_no_source_media` (image/text) it is
/// **unbounded** (may go negative → free extension).
///
/// Does **not** move neighbors — trim is overwrite-style, in place
/// (edit-engines.md line 125).
pub fn trim_values(clip: &SplitClip, edge: TrimEdge, delta_frames: i32) -> (i32, i32) {
    let source_delta = round_ties_away(delta_frames as f64 * clip.speed);
    match edge {
        TrimEdge::Left => {
            let new_start = clip.trim_start_frame + source_delta;
            let v = if clip.has_no_source_media {
                new_start
            } else {
                new_start.max(0)
            };
            (v, clip.trim_end_frame)
        }
        TrimEdge::Right => {
            let new_end = clip.trim_end_frame - source_delta;
            let v = if clip.has_no_source_media {
                new_end
            } else {
                new_end.max(0)
            };
            (clip.trim_start_frame, v)
        }
    }
}

/// The timeline result of applying a source-frame trim to a clip.
///
/// Produced by [`apply_trim_internal`] — the back-conversion that turns the new
/// source trim values into the clip's new timeline `start_frame` /
/// `duration_frames`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrimResult {
    /// New timeline start frame.
    pub new_start_frame: i32,
    /// New timeline duration in frames.
    pub new_duration: i32,
    /// New source head trim (echoes the input — for the caller's convenience).
    pub new_trim_start_frame: i32,
    /// New source tail trim (echoes the input).
    pub new_trim_end_frame: i32,
}

/// Back-convert new source-frame trim values into the clip's new timeline span.
///
/// Reference `trimClipInternal` (edit-engines.md lines 122-125): the incoming
/// `trim_start_frame` / `trim_end_frame` are **source** frames; their deltas vs
/// the clip's current trims are divided by `speed` (ties away) to get timeline
/// deltas:
/// - `delta_start_timeline = round((new_trim_start - old_trim_start) / speed)`
/// - `delta_end_timeline   = round((new_trim_end   - old_trim_end)   / speed)`
/// - `new_duration = old_duration - delta_start_timeline - delta_end_timeline`
/// - `new_start    = old_start + delta_start_timeline`
///
/// Pairing this with [`trim_values`] gives the frame-stable source↔timeline
/// round-trip the golden test asserts.
pub fn apply_trim_internal(
    clip: &SplitClip,
    new_trim_start_frame: i32,
    new_trim_end_frame: i32,
) -> TrimResult {
    let delta_start_source = new_trim_start_frame - clip.trim_start_frame;
    let delta_end_source = new_trim_end_frame - clip.trim_end_frame;
    let delta_start_timeline = round_ties_away(delta_start_source as f64 / clip.speed);
    let delta_end_timeline = round_ties_away(delta_end_source as f64 / clip.speed);
    TrimResult {
        new_start_frame: clip.start_frame + delta_start_timeline,
        new_duration: clip.duration_frames - delta_start_timeline - delta_end_timeline,
        new_trim_start_frame,
        new_trim_end_frame,
    }
}

/// Trim-drag clamp bounds for one edge (consumed by E3-S5/E3-S7 drag clamping).
///
/// Reference (edit-engines.md lines 113-118, 236-238). `min_delta` / `max_delta`
/// are **timeline-frame** drag deltas:
/// - **Left:** `max = original_duration - 1`;
///   `min = has_no_source_media ? -original_start_frame : -original_trim_start`.
/// - **Right:** `min = -(original_duration - 1)`;
///   `max = has_no_source_media ? i32::MAX (unbounded) : original_trim_end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrimClamp {
    /// Minimum allowed timeline-frame drag delta.
    pub min_delta: i32,
    /// Maximum allowed timeline-frame drag delta (`i32::MAX` = unbounded).
    pub max_delta: i32,
}

/// Compute the [`TrimClamp`] for a trim drag on `edge`.
pub fn trim_clamp(clip: &SplitClip, edge: TrimEdge) -> TrimClamp {
    match edge {
        TrimEdge::Left => TrimClamp {
            max_delta: clip.duration_frames - 1,
            min_delta: if clip.has_no_source_media {
                -clip.start_frame
            } else {
                -clip.trim_start_frame
            },
        },
        TrimEdge::Right => TrimClamp {
            min_delta: -(clip.duration_frames - 1),
            max_delta: if clip.has_no_source_media {
                i32::MAX
            } else {
                clip.trim_end_frame
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_clip() -> SplitClip {
        SplitClip {
            id: "c".into(),
            start_frame: 0,
            duration_frames: 100,
            trim_start_frame: 0,
            trim_end_frame: 0,
            speed: 1.0,
            fade_in_frames: 0,
            fade_out_frames: 0,
            volume_track: None,
            has_no_source_media: false,
        }
    }

    fn fixed_id() -> impl FnMut() -> String {
        move || "right".to_string()
    }

    // ---- split guards + basic math ---------------------------------------

    #[test]
    fn split_rejects_out_of_range_at_frame() {
        let c = base_clip();
        assert!(split_clip(&c, 0, fixed_id()).is_none(), "at start");
        assert!(split_clip(&c, 100, fixed_id()).is_none(), "at end");
        assert!(split_clip(&c, -5, fixed_id()).is_none(), "before start");
    }

    #[test]
    fn split_durations_and_trims_at_realtime() {
        let c = base_clip();
        let (left, right) = split_clip(&c, 40, fixed_id()).unwrap();
        assert_eq!(left.duration_frames, 40);
        assert_eq!(left.trim_end_frame, 60, "trim_end += round((100-40)*1.0)");
        assert_eq!(right.start_frame, 40);
        assert_eq!(right.duration_frames, 60);
        assert_eq!(right.trim_start_frame, 40, "trim_start += round(40*1.0)");
        assert_eq!(right.id, "right", "right gets a new id");
    }

    #[test]
    fn split_clears_inner_fades() {
        let mut c = base_clip();
        c.fade_in_frames = 10;
        c.fade_out_frames = 10;
        let (left, right) = split_clip(&c, 50, fixed_id()).unwrap();
        assert_eq!(left.fade_out_frames, 0, "left loses its fade-out");
        assert_eq!(left.fade_in_frames, 10, "left keeps its fade-in");
        assert_eq!(right.fade_in_frames, 0, "right loses its fade-in");
        assert_eq!(right.fade_out_frames, 10, "right keeps its fade-out");
    }

    // ---- keyframe migration ----------------------------------------------

    #[test]
    fn split_keyframes_both_sides_present() {
        // kfs at 0,30,70,100 around a split at 50 (no kf on the boundary).
        let kfs = vec![
            VolumeKeyframe::new(0, -6.0, Interpolation::Linear),
            VolumeKeyframe::new(30, -3.0, Interpolation::Linear),
            VolumeKeyframe::new(70, 0.0, Interpolation::Linear),
            VolumeKeyframe::new(100, -12.0, Interpolation::Linear),
        ];
        let boundary = sample_volume(&kfs, 50, 0.0);
        let (left, right) = migrate_volume_split(&kfs, 50);

        // Left keeps 0,30 and appends a boundary kf at 50.
        assert_eq!(left.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0, 30, 50]);
        assert_eq!(left.last().unwrap().value, boundary, "left ends at boundary value");

        // Right keeps 70,100 (rebased to 20,50) and inserts a boundary at 0.
        assert_eq!(right.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0, 20, 50]);
        assert_eq!(right[0].value, boundary, "right starts at boundary value");

        // Continuity: left's last value == right's first value.
        assert_eq!(left.last().unwrap().value, right[0].value);
    }

    #[test]
    fn split_keyframe_exactly_on_boundary_not_duplicated() {
        // A kf sits exactly at the split offset → no extra boundary kf inserted.
        let kfs = vec![
            VolumeKeyframe::new(0, -6.0, Interpolation::Linear),
            VolumeKeyframe::new(50, -3.0, Interpolation::Linear),
            VolumeKeyframe::new(100, 0.0, Interpolation::Linear),
        ];
        let (left, right) = migrate_volume_split(&kfs, 50);
        assert_eq!(left.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0, 50]);
        // Right keeps 50,100 rebased → 0,50; the kf at offset 0 already exists.
        assert_eq!(right.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0, 50]);
        assert_eq!(right[0].value, -3.0, "no synthetic boundary; real kf kept");
    }

    #[test]
    fn split_keyframe_no_kf_at_boundary_inserts_continuous_value() {
        // Single segment 0..100, split at 25: boundary = linear sample at 25.
        let kfs = vec![
            VolumeKeyframe::new(0, 0.0, Interpolation::Linear),
            VolumeKeyframe::new(100, 100.0, Interpolation::Linear),
        ];
        let boundary = sample_volume(&kfs, 25, 0.0);
        assert_eq!(boundary, 25.0, "linear quarter point");
        let (left, right) = migrate_volume_split(&kfs, 25);
        assert_eq!(left.last().unwrap().frame, 25);
        assert_eq!(left.last().unwrap().value, 25.0);
        assert_eq!(right[0].frame, 0);
        assert_eq!(right[0].value, 25.0, "right re-based, continuous");
    }

    // ---- split round-trip (left.dur + right.dur == orig.dur) --------------

    #[test]
    fn split_round_trip_durations_conserved_over_speeds() {
        for speed in [0.25, 0.5, 1.0, 1.7, 4.0] {
            let mut c = base_clip();
            c.speed = speed;
            c.duration_frames = 97; // odd to provoke ties
            for at in [1, 13, 48, 49, 96] {
                let (left, right) = split_clip(&c, at, fixed_id()).unwrap();
                assert_eq!(
                    left.duration_frames + right.duration_frames,
                    c.duration_frames,
                    "speed {speed} at {at}: durations must sum to original"
                );
                assert_eq!(right.start_frame, c.start_frame + left.duration_frames);
            }
        }
    }

    // ---- trim_values + apply_trim_internal round-trip --------------------

    #[test]
    fn trim_does_not_drift_round_trip_over_speeds() {
        // Golden: trim a media-bounded clip on each edge by a range of deltas at
        // each speed; the source↔timeline back-conversion must be frame-stable
        // (no ±1 drift) — edit-engines.md lines 221-222.
        for speed in [0.25, 0.5, 1.0, 1.7, 4.0] {
            let mut c = base_clip();
            c.speed = speed;
            c.duration_frames = 80;
            c.trim_start_frame = 200;
            c.trim_end_frame = 200;
            for delta in [-20, -7, -1, 1, 5, 13, 20] {
                // Left edge.
                let (ts, te) = trim_values(&c, TrimEdge::Left, delta);
                let r = apply_trim_internal(&c, ts, te);
                // The applied timeline start delta, converted back to a drag delta,
                // must reproduce the source delta we asked for (frame-stable).
                let applied_start_delta = r.new_start_frame - c.start_frame;
                let source_delta = round_ties_away(delta as f64 * speed);
                let expected_timeline = round_ties_away(source_delta as f64 / speed);
                assert_eq!(
                    applied_start_delta, expected_timeline,
                    "left speed {speed} delta {delta}: start drift"
                );
                // Duration stays consistent: new_dur = old_dur - timeline_start_delta.
                assert_eq!(r.new_duration, c.duration_frames - expected_timeline);

                // Right edge.
                let (ts2, te2) = trim_values(&c, TrimEdge::Right, delta);
                let r2 = apply_trim_internal(&c, ts2, te2);
                let source_delta_r = -round_ties_away(delta as f64 * speed);
                let expected_end_timeline = round_ties_away(source_delta_r as f64 / speed);
                assert_eq!(
                    r2.new_duration,
                    c.duration_frames - expected_end_timeline,
                    "right speed {speed} delta {delta}: duration drift"
                );
                assert_eq!(r2.new_start_frame, c.start_frame, "right edge keeps start");
            }
        }
    }

    #[test]
    fn trim_values_floors_at_zero_for_media_bounded() {
        let mut c = base_clip();
        c.trim_start_frame = 5;
        // Drag left edge left by 20 (delta -20) at speed 1 → source -20; floored.
        let (ts, _te) = trim_values(&c, TrimEdge::Left, -20);
        assert_eq!(ts, 0, "media-bounded trim_start floored at 0");
    }

    #[test]
    fn trim_values_unbounded_for_no_source_media() {
        let mut c = base_clip();
        c.has_no_source_media = true; // image/text
        c.trim_start_frame = 5;
        let (ts, _te) = trim_values(&c, TrimEdge::Left, -20);
        assert_eq!(ts, -15, "image/text trim_start may go negative (free extension)");
    }

    // ---- trim clamps (has_no_source_media path distinct) ------------------

    #[test]
    fn trim_clamp_media_bounded_caps_on_source() {
        let mut c = base_clip();
        c.trim_start_frame = 30;
        c.trim_end_frame = 40;
        let left = trim_clamp(&c, TrimEdge::Left);
        assert_eq!(left.max_delta, 99, "duration - 1");
        assert_eq!(left.min_delta, -30, "-original_trim_start");
        let right = trim_clamp(&c, TrimEdge::Right);
        assert_eq!(right.min_delta, -99);
        assert_eq!(right.max_delta, 40, "capped at original_trim_end");
    }

    #[test]
    fn trim_clamp_no_source_media_uncaps_tail_and_uses_start_for_head() {
        let mut c = base_clip();
        c.start_frame = 25;
        c.has_no_source_media = true;
        let left = trim_clamp(&c, TrimEdge::Left);
        assert_eq!(left.min_delta, -25, "-original_start_frame for image/text");
        let right = trim_clamp(&c, TrimEdge::Right);
        assert_eq!(right.max_delta, i32::MAX, "tail trim unbounded for image/text");
    }
}
