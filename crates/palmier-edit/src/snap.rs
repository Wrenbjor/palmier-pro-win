//! E3-S5 — SnapEngine (pure, side-effect-free).
//!
//! `collect_targets` + `find_snap` ported from
//! `Sources/PalmierPro/Timeline/SnapEngine.swift`. The reference's **one** side
//! effect — `NSHapticFeedbackManager.perform(.alignment)` in `findSnap` — is
//! **dropped** (edit-engines.md lines 32, 184-185): `find_snap` returns the snap
//! result as a pure value and the UI layer may fire feedback itself.
//!
//! See docs/reference/edit-engines.md §"SnapEngine" (lines 130-141) and story
//! E3-S5.
//!
//! ## Constants (ruling #10 / `Constants.swift`, edit-engines.md lines 47-50)
//! - base threshold = **8 px**
//! - playhead multiplier = **1.5×**
//! - **sticky multiplier = 1.5×** — the reference `Snap.stickyMultiplier` is
//!   **1.5**, NOT FOUNDATION §6.3's 2.5 (reconciliation ruling #10; the inline
//!   `SnapEngine.swift` comment "2.5x" is stale, the constant it reads is 1.5).
//! - `Defaults.pixels_per_frame = 4.0`

/// Base snap catch radius in pixels (reference `Snap.thresholdPixels`).
pub const BASE_THRESHOLD_PX: f64 = 8.0;
/// Playhead targets get a wider catch radius: `base × 1.5` (reference
/// `Snap.playheadMultiplier`).
pub const PLAYHEAD_MULTIPLIER: f64 = 1.5;
/// Sticky hold radius: a held snap survives until the probe moves more than
/// `base × 1.5` away. **1.5, not 2.5** (ruling #10).
pub const STICKY_MULTIPLIER: f64 = 1.5;
/// Trim-handle hit width in pixels (reference `Trim.handleWidth`; consumed by E3-S7).
pub const TRIM_HANDLE_PX: f64 = 4.0;
/// Default timeline zoom: pixels per frame (reference `Defaults.pixelsPerFrame`).
pub const DEFAULT_PIXELS_PER_FRAME: f64 = 4.0;

/// What kind of edge a snap target represents — drives the per-target threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    /// The playhead — gets the wider `1.5×` catch radius (priority).
    Playhead,
    /// A clip start or end edge — gets the base radius.
    ClipEdge,
}

/// A frame the moving thing can snap to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapTarget {
    /// Timeline frame of the target.
    pub frame: i32,
    /// Whether it's the playhead or a clip edge.
    pub kind: SnapKind,
}

impl SnapTarget {
    /// Construct a snap target at `frame` of `kind`.
    pub fn new(frame: i32, kind: SnapKind) -> Self {
        SnapTarget { frame, kind }
    }
}

/// The outcome of a successful snap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapResult {
    /// The target frame snapped to.
    pub frame: i32,
    /// Which probe offset snapped (`0` = the moving clip's start, `duration` = its end).
    pub probe_offset: i32,
    /// Pixel x of the snap indicator = `frame · pixels_per_frame`.
    pub x: f64,
}

/// Sticky state that persists across drag events so a snap "holds" until the
/// probe pulls far enough away.
///
/// Reference `SnapEngine.SnapState`. Default = not snapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SnapState {
    /// Frame currently held, if any.
    pub currently_snapped_to: Option<i32>,
    /// Which probe offset is sticky.
    pub current_probe_offset: i32,
}

/// A minimal clip view for snap-target collection: just the two edges.
///
/// (Snap only needs each clip's `start`/`end` frame and `id` for exclusion — see
/// the `placement.rs` decoupling note.)
#[derive(Debug, Clone, PartialEq)]
pub struct SnapClip {
    /// UUID-string clip id (for `exclude_clip_ids`).
    pub id: String,
    /// Clip start frame.
    pub start_frame: i32,
    /// Clip end frame.
    pub end_frame: i32,
}

impl SnapClip {
    /// Construct a snap clip from its id and edge frames.
    pub fn new(id: impl Into<String>, start_frame: i32, end_frame: i32) -> Self {
        SnapClip {
            id: id.into(),
            start_frame,
            end_frame,
        }
    }
}

/// Collect every clip edge (and optionally the playhead) as snap targets.
///
/// Reference `collectTargets` (edit-engines.md lines 131-132): each non-excluded
/// clip contributes its `start_frame` AND `end_frame` as `ClipEdge`; the playhead
/// is added as `Playhead` when `include_playhead`. `clips` is the flattened set
/// across all tracks (the caller flattens; snap doesn't care about tracks).
pub fn collect_targets(
    clips: &[SnapClip],
    playhead_frame: i32,
    exclude_clip_ids: &[String],
    include_playhead: bool,
) -> Vec<SnapTarget> {
    let mut targets = Vec::new();
    if include_playhead {
        targets.push(SnapTarget::new(playhead_frame, SnapKind::Playhead));
    }
    for clip in clips {
        if exclude_clip_ids.iter().any(|id| id == &clip.id) {
            continue;
        }
        targets.push(SnapTarget::new(clip.start_frame, SnapKind::ClipEdge));
        targets.push(SnapTarget::new(clip.end_frame, SnapKind::ClipEdge));
    }
    targets
}

/// Find the nearest snap target for a moving thing, with sticky hold + playhead
/// priority. **Side-effect-free** (the reference's haptic call is removed).
///
/// Reference `findSnap` (edit-engines.md lines 133-141):
/// - `base_frame_threshold = base_threshold_px / pixels_per_frame`.
/// - **Sticky:** if `state.currently_snapped_to` is set and the sticky probe
///   (`position + state.current_probe_offset`) is within
///   `base_frame_threshold × STICKY_MULTIPLIER` of it **and** that target still
///   exists → return the held snap unchanged. Otherwise clear the state and
///   fall through.
/// - **Find best:** for each `probe_offset`, `probe_pos = position + offset`; for
///   each target, the threshold is `base_frame_threshold × PLAYHEAD_MULTIPLIER`
///   for a `Playhead`, else `base_frame_threshold`. Record the `(offset, target)`
///   with the **smallest** distance within its threshold. On a tie the first
///   target wins (strict `<`), matching the reference's `< best.distance`.
/// - On success, set `state` sticky and return the [`SnapResult`].
///
/// `position` is the moving thing's lead frame; `probe_offsets` are added to it
/// (e.g. `[0, duration]` so either edge of the moving clip can snap).
pub fn find_snap(
    position: i32,
    probe_offsets: &[i32],
    targets: &[SnapTarget],
    state: &mut SnapState,
    base_threshold_px: f64,
    pixels_per_frame: f64,
) -> Option<SnapResult> {
    let base_frame_threshold = base_threshold_px / pixels_per_frame;

    // --- Sticky: hold the current snap until the probe pulls away. ---
    if let Some(snapped) = state.currently_snapped_to {
        let hold_threshold = base_frame_threshold * STICKY_MULTIPLIER;
        let probe_pos = position + state.current_probe_offset;
        if (probe_pos - snapped).abs() as f64 <= hold_threshold
            && targets.iter().any(|t| t.frame == snapped)
        {
            return Some(SnapResult {
                frame: snapped,
                probe_offset: state.current_probe_offset,
                x: snapped as f64 * pixels_per_frame,
            });
        }
        state.currently_snapped_to = None;
        state.current_probe_offset = 0;
    }

    // --- Find the closest (probe, target) within threshold. ---
    let mut best: Option<(i32, SnapTarget, f64)> = None;
    for &probe_offset in probe_offsets {
        let probe_pos = position + probe_offset;
        for target in targets {
            let threshold = match target.kind {
                SnapKind::Playhead => base_frame_threshold * PLAYHEAD_MULTIPLIER,
                SnapKind::ClipEdge => base_frame_threshold,
            };
            let dist = (probe_pos - target.frame).abs() as f64;
            if dist <= threshold && dist < best.as_ref().map(|b| b.2).unwrap_or(f64::INFINITY) {
                best = Some((probe_offset, *target, dist));
            }
        }
    }

    let (probe_offset, target, _) = best?;
    // (Reference fires NSHapticFeedbackManager here — dropped; pure value return.)
    state.currently_snapped_to = Some(target.frame);
    state.current_probe_offset = probe_offset;
    Some(SnapResult {
        frame: target.frame,
        probe_offset,
        x: target.frame as f64 * pixels_per_frame,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PPF: f64 = DEFAULT_PIXELS_PER_FRAME; // 4.0 → base_frame_threshold = 2.0 frames

    fn edge(frame: i32) -> SnapTarget {
        SnapTarget::new(frame, SnapKind::ClipEdge)
    }

    // ---- collect_targets --------------------------------------------------

    #[test]
    fn collect_targets_emits_both_edges_and_skips_excluded() {
        let clips = vec![SnapClip::new("a", 0, 10), SnapClip::new("b", 20, 30)];
        let targets = collect_targets(&clips, 5, &["b".to_string()], true);
        // playhead + a.start + a.end (b excluded).
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0], SnapTarget::new(5, SnapKind::Playhead));
        assert_eq!(targets[1], edge(0));
        assert_eq!(targets[2], edge(10));
    }

    #[test]
    fn collect_targets_omits_playhead_when_not_requested() {
        let clips = vec![SnapClip::new("a", 0, 10)];
        let targets = collect_targets(&clips, 5, &[], false);
        assert_eq!(targets, vec![edge(0), edge(10)]);
    }

    // ---- find_snap: within / just outside --------------------------------

    #[test]
    fn snaps_within_threshold() {
        // base_frame_threshold = 8/4 = 2 frames. Target at 100, probe at 101 → dist 1 ≤ 2.
        let targets = vec![edge(100)];
        let mut state = SnapState::default();
        let r = find_snap(101, &[0], &targets, &mut state, BASE_THRESHOLD_PX, PPF);
        assert_eq!(r.unwrap().frame, 100);
        assert_eq!(state.currently_snapped_to, Some(100));
    }

    #[test]
    fn does_not_snap_just_outside_threshold() {
        // Target at 100, probe at 103 → dist 3 > 2 (clip-edge threshold).
        let targets = vec![edge(100)];
        let mut state = SnapState::default();
        let r = find_snap(103, &[0], &targets, &mut state, BASE_THRESHOLD_PX, PPF);
        assert!(r.is_none(), "3 frames away exceeds the 2-frame clip-edge radius");
        assert_eq!(state.currently_snapped_to, None);
    }

    // ---- sticky stays until probe moves > 1.5× threshold (ruling #10) -----

    #[test]
    fn sticky_holds_until_one_point_five_times_threshold() {
        // base_frame_threshold = 2; sticky hold = 2 * 1.5 = 3 frames.
        let targets = vec![edge(100)];
        let mut state = SnapState::default();

        // Initial snap at 100.
        assert!(find_snap(100, &[0], &targets, &mut state, BASE_THRESHOLD_PX, PPF).is_some());
        assert_eq!(state.currently_snapped_to, Some(100));

        // Move to 103: |103-100| = 3 ≤ 3 → still held (sticky).
        let held = find_snap(103, &[0], &targets, &mut state, BASE_THRESHOLD_PX, PPF);
        assert_eq!(held.unwrap().frame, 100, "held at 1.5× threshold boundary");
        assert_eq!(state.currently_snapped_to, Some(100));

        // Move to 104: |104-100| = 4 > 3 → releases. 4 is also > 2 (catch radius)
        // so no fresh snap → None and state cleared.
        let released = find_snap(104, &[0], &targets, &mut state, BASE_THRESHOLD_PX, PPF);
        assert!(released.is_none(), "released past 1.5× threshold (NOT 2.5×)");
        assert_eq!(state.currently_snapped_to, None);
    }

    #[test]
    fn sticky_would_release_earlier_than_two_point_five_multiplier() {
        // Guard against a 2.5× regression: at 2.5× the hold radius would be 5
        // frames, so probe at 104 (dist 4) would STILL be held. With the correct
        // 1.5× it releases. This asserts the multiplier is 1.5, not 2.5.
        let targets = vec![edge(100)];
        let mut state = SnapState {
            currently_snapped_to: Some(100),
            current_probe_offset: 0,
        };
        let r = find_snap(104, &[0], &targets, &mut state, BASE_THRESHOLD_PX, PPF);
        assert!(r.is_none(), "must release at dist 4 (1.5× ⇒ hold 3); 2.5× would hold");
    }

    // ---- playhead priority via wider radius ------------------------------

    #[test]
    fn playhead_wider_radius_wins_over_clip_edge_at_equal_distance() {
        // Playhead at 98, clip edge at 102, probe at 100 → both dist 2. Clip-edge
        // threshold = 2 (just catches); playhead threshold = 3. Iteration order
        // puts the playhead first, and with equal distance `< best` keeps the
        // first → playhead wins.
        let targets = vec![
            SnapTarget::new(98, SnapKind::Playhead),
            edge(102),
        ];
        let mut state = SnapState::default();
        let r = find_snap(100, &[0], &targets, &mut state, BASE_THRESHOLD_PX, PPF).unwrap();
        assert_eq!(r.frame, 98, "playhead caught at equal distance");
    }

    #[test]
    fn playhead_catches_outside_clip_edge_radius() {
        // Probe 3 frames from playhead: clip-edge radius 2 misses, playhead 3 catches.
        let targets = vec![SnapTarget::new(100, SnapKind::Playhead)];
        let mut state = SnapState::default();
        let r = find_snap(103, &[0], &targets, &mut state, BASE_THRESHOLD_PX, PPF);
        assert_eq!(r.unwrap().frame, 100, "playhead 1.5× radius catches at dist 3");
    }

    // ---- two probe offsets (move-drag, both edges) -----------------------

    #[test]
    fn two_probe_offsets_let_either_edge_snap() {
        // Moving clip lead at 0, duration 50 → probes [0, 50]. Target at 51 is
        // caught by the END probe (50+1), not the start probe.
        let targets = vec![edge(51)];
        let mut state = SnapState::default();
        let r = find_snap(0, &[0, 50], &targets, &mut state, BASE_THRESHOLD_PX, PPF).unwrap();
        assert_eq!(r.frame, 51);
        assert_eq!(r.probe_offset, 50, "the end probe snapped");
    }

    #[test]
    fn closest_probe_target_pair_wins() {
        // Two targets; the nearer one (to either probe) is chosen.
        let targets = vec![edge(2), edge(49)];
        let mut state = SnapState::default();
        // lead 0, probes [0,50]: start probe→2 (dist 2), end probe→49 (dist 1). 49 wins.
        let r = find_snap(0, &[0, 50], &targets, &mut state, BASE_THRESHOLD_PX, PPF).unwrap();
        assert_eq!(r.frame, 49);
    }

    #[test]
    fn sticky_clears_when_target_disappears() {
        // Held at 100 but that target is gone from the set → state clears, no snap.
        let mut state = SnapState {
            currently_snapped_to: Some(100),
            current_probe_offset: 0,
        };
        let targets = vec![edge(500)]; // 100 no longer present
        let r = find_snap(100, &[0], &targets, &mut state, BASE_THRESHOLD_PX, PPF);
        assert!(r.is_none());
        assert_eq!(state.currently_snapped_to, None);
    }
}
