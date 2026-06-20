//! E3-S3 — OverwriteEngine (pure).
//!
//! `compute_overwrite(clips, region_start, region_end)` returns the exact
//! delete / trim / split actions that clear a region so a new clip can be placed
//! there. Ported 1:1 from `Sources/PalmierPro/Editor/OverwriteEngine.swift`
//! (`computeOverwrite`). See docs/reference/edit-engines.md §"OverwriteEngine
//! (pure)" (lines 85-97) and story E3-S3.
//!
//! All `*speed` conversions use [`round_ties_away`](crate::rounding::round_ties_away)
//! (`f64::round`, ties away — never `round_ties_even`).

use crate::placement::ClipPlacement;
use crate::rounding::round_ties_away;

/// One action the caller applies to clear an overwrite region.
///
/// Mirrors the reference `OverwriteEngine.Action` enum. The [`Split`] variant's
/// **right-fragment fields are advisory**: the orchestration layer (E3-S6)
/// ignores them and re-derives the right half via `split_clip(at = region_start)`
/// (edit-engines.md lines 96-97, 228-229). They are emitted here for completeness
/// and unit-test coverage.
///
/// [`Split`]: OverwriteAction::Split
#[derive(Debug, Clone, PartialEq)]
pub enum OverwriteAction {
    /// The clip is fully covered by the region → delete it.
    Remove {
        /// Id of the clip to remove.
        clip_id: String,
    },
    /// The region overlaps the clip's tail → trim its right edge.
    TrimEnd {
        /// Id of the clip to trim.
        clip_id: String,
        /// New (shorter) timeline duration = `region_start - clip.start`.
        new_duration: i32,
    },
    /// The region overlaps the clip's head → trim its left edge.
    TrimStart {
        /// Id of the clip to trim.
        clip_id: String,
        /// New timeline start = `region_end`.
        new_start_frame: i32,
        /// New source-domain head trim = `trim_start + round((region_end - cs)·speed)`.
        new_trim_start: i32,
        /// New (shorter) timeline duration = `clip.end - region_end`.
        new_duration: i32,
    },
    /// The region falls strictly inside the clip → split it in two, dropping the
    /// covered middle. **Right-fragment fields are advisory** (see type docs).
    Split {
        /// Id of the (kept) left fragment — same id as the original clip.
        clip_id: String,
        /// Left fragment's new timeline duration = `region_start - cs`.
        left_duration: i32,
        /// Freshly-minted id for the right fragment.
        right_id: String,
        /// Right fragment's timeline start = `region_end` (advisory).
        right_start_frame: i32,
        /// Right fragment's head trim = `trim_start + round((region_end - cs)·speed)` (advisory).
        right_trim_start: i32,
        /// Right fragment's timeline duration = `ce - region_end` (advisory).
        right_duration: i32,
    },
}

/// Compute the actions that clear `[region_start, region_end)` across `clips`.
///
/// Reference `computeOverwrite` (edit-engines.md lines 85-94). Guards
/// `region_end > region_start` (returns empty otherwise). For each clip with
/// `cs = start_frame`, `ce = end_frame`, in input order:
///
/// 1. `ce <= region_start || cs >= region_end` → no overlap, skip.
/// 2. `cs >= region_start && ce <= region_end` → [`Remove`].
/// 3. `cs < region_start && ce > region_end` (region strictly inside) → [`Split`].
/// 4. `cs < region_start` (overlaps left, ce inside) → [`TrimEnd`].
/// 5. else (overlaps right, cs inside) → [`TrimStart`].
///
/// `new_id` mints the right-fragment id for the [`Split`] case — injected so
/// tests are deterministic; production passes a UUID generator.
///
/// [`Remove`]: OverwriteAction::Remove
/// [`Split`]: OverwriteAction::Split
/// [`TrimEnd`]: OverwriteAction::TrimEnd
/// [`TrimStart`]: OverwriteAction::TrimStart
pub fn compute_overwrite_with(
    clips: &[ClipPlacement],
    region_start: i32,
    region_end: i32,
    mut new_id: impl FnMut() -> String,
) -> Vec<OverwriteAction> {
    if region_end <= region_start {
        return Vec::new();
    }
    let mut actions = Vec::new();

    for clip in clips {
        let cs = clip.start_frame;
        let ce = clip.end_frame();

        // (1) no overlap
        if ce <= region_start || cs >= region_end {
            continue;
        }

        if cs >= region_start && ce <= region_end {
            // (2) fully covered
            actions.push(OverwriteAction::Remove {
                clip_id: clip.id.clone(),
            });
        } else if cs < region_start && ce > region_end {
            // (3) region strictly inside the clip → split, drop the middle.
            let left_duration = region_start - cs;
            let right_start_frame = region_end;
            let right_trim_start =
                clip.trim_start_frame + round_ties_away((region_end - cs) as f64 * clip.speed);
            let right_duration = ce - region_end;
            actions.push(OverwriteAction::Split {
                clip_id: clip.id.clone(),
                left_duration,
                right_id: new_id(),
                right_start_frame,
                right_trim_start,
                right_duration,
            });
        } else if cs < region_start {
            // (4) overlaps the clip's tail → trim right edge.
            actions.push(OverwriteAction::TrimEnd {
                clip_id: clip.id.clone(),
                new_duration: region_start - cs,
            });
        } else {
            // (5) overlaps the clip's head → trim left edge.
            let trim_amount = region_end - cs;
            let new_trim_start =
                clip.trim_start_frame + round_ties_away(trim_amount as f64 * clip.speed);
            actions.push(OverwriteAction::TrimStart {
                clip_id: clip.id.clone(),
                new_start_frame: region_end,
                new_trim_start,
                new_duration: ce - region_end,
            });
        }
    }

    actions
}

/// Convenience wrapper over [`compute_overwrite_with`] using a fixed placeholder
/// id (`"split-right"`) for any split's right fragment.
///
/// Suitable for callers that re-derive the right fragment anyway (E3-S6 ignores
/// the advisory fields) and for non-split call sites.
pub fn compute_overwrite(
    clips: &[ClipPlacement],
    region_start: i32,
    region_end: i32,
) -> Vec<OverwriteAction> {
    compute_overwrite_with(clips, region_start, region_end, || "split-right".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: &str, start: i32, dur: i32) -> ClipPlacement {
        ClipPlacement::new(id, start, dur, 0)
    }

    /// Deterministic id factory for split tests.
    fn fixed_id() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("right-{n}")
        }
    }

    #[test]
    fn guard_empty_region_returns_nothing() {
        let clips = vec![clip("a", 0, 100)];
        assert!(compute_overwrite(&clips, 50, 50).is_empty(), "equal bounds");
        assert!(compute_overwrite(&clips, 60, 50).is_empty(), "reversed bounds");
    }

    #[test]
    fn no_overlap_skips_clip() {
        // Region [50,60) misses a clip that ends at 50 (half-open) and one starting at 60.
        let clips = vec![clip("before", 0, 50), clip("after", 60, 50)];
        assert!(compute_overwrite(&clips, 50, 60).is_empty());
    }

    // ---- the four named cases (edit-engines.md / story E3-S3) -------------

    #[test]
    fn case_inside_region_strictly_inside_clip_splits() {
        // clip [0,100); region [40,60) strictly inside → split.
        let clips = vec![clip("c", 0, 100)];
        let actions = compute_overwrite_with(&clips, 40, 60, fixed_id());
        assert_eq!(
            actions,
            vec![OverwriteAction::Split {
                clip_id: "c".into(),
                left_duration: 40,        // region_start - cs
                right_id: "right-1".into(),
                right_start_frame: 60,    // region_end
                right_trim_start: 60,     // trim 0 + round(60*1.0)
                right_duration: 40,       // ce - region_end
            }]
        );
    }

    #[test]
    fn case_overlap_start_trims_left() {
        // Region overlaps the clip's HEAD (cs inside region): clip [40,140),
        // region [0,60) → TrimStart, new start 60.
        let clips = vec![clip("c", 40, 100)];
        let actions = compute_overwrite_with(&clips, 0, 60, fixed_id());
        assert_eq!(
            actions,
            vec![OverwriteAction::TrimStart {
                clip_id: "c".into(),
                new_start_frame: 60,                 // region_end
                new_trim_start: 20,                  // 0 + round((60-40)*1.0)
                new_duration: 80,                    // ce(140) - region_end(60)
            }]
        );
    }

    #[test]
    fn case_overlap_end_trims_right() {
        // Region overlaps the clip's TAIL (ce inside region): clip [0,100),
        // region [60,200) → TrimEnd, new duration 60.
        let clips = vec![clip("c", 0, 100)];
        let actions = compute_overwrite_with(&clips, 60, 200, fixed_id());
        assert_eq!(
            actions,
            vec![OverwriteAction::TrimEnd {
                clip_id: "c".into(),
                new_duration: 60, // region_start - cs
            }]
        );
    }

    #[test]
    fn case_cover_multi_mixes_remove_and_bookend_trims() {
        // Region [40,160) over three abutting clips:
        //   left  [0,50)  → partial tail in region → TrimEnd(new_dur 40)
        //   mid   [50,100)→ fully covered          → Remove
        //   right [100,200)→ partial head in region → TrimStart(start 160)
        let clips = vec![
            clip("left", 0, 50),
            clip("mid", 50, 50),
            clip("right", 100, 100),
        ];
        let actions = compute_overwrite_with(&clips, 40, 160, fixed_id());
        assert_eq!(
            actions,
            vec![
                OverwriteAction::TrimEnd {
                    clip_id: "left".into(),
                    new_duration: 40,
                },
                OverwriteAction::Remove {
                    clip_id: "mid".into(),
                },
                OverwriteAction::TrimStart {
                    clip_id: "right".into(),
                    new_start_frame: 160,
                    new_trim_start: 60, // 0 + round((160-100)*1.0)
                    new_duration: 40,   // 200 - 160
                },
            ]
        );
    }

    // ---- rounding parity on the source-offset (speed ∈ {0.5, 1.7, 4.0}) ----

    #[test]
    fn trim_start_source_offset_rounds_ties_away() {
        // TrimStart head-trim recompute uses round((region_end - cs)·speed).
        // clip [0,100) speed 1.7, region [0,5): round(5*1.7)=round(8.5)=9 (away).
        let clips = vec![clip("c", 0, 100).with_speed(1.7)];
        let actions = compute_overwrite(&clips, 0, 5);
        match &actions[0] {
            OverwriteAction::TrimStart { new_trim_start, .. } => {
                assert_eq!(*new_trim_start, 9, "round(8.5) ties away → 9");
            }
            other => panic!("expected TrimStart, got {other:?}"),
        }
    }

    #[test]
    fn split_right_trim_rounds_for_half_and_quad_speed() {
        // speed 0.5: clip [0,100), region [40,60) → round((60-0)*0.5)=30.
        let half = compute_overwrite(&[clip("c", 0, 100).with_speed(0.5)], 40, 60);
        match &half[0] {
            OverwriteAction::Split { right_trim_start, .. } => assert_eq!(*right_trim_start, 30),
            other => panic!("expected Split, got {other:?}"),
        }
        // speed 4.0: same region → round(60*4.0)=240.
        let quad = compute_overwrite(&[clip("c", 0, 100).with_speed(4.0)], 40, 60);
        match &quad[0] {
            OverwriteAction::Split { right_trim_start, .. } => assert_eq!(*right_trim_start, 240),
            other => panic!("expected Split, got {other:?}"),
        }
    }

    #[test]
    fn split_carries_clip_existing_trim_start() {
        // The recompute is additive on the clip's existing trim_start.
        let clips = vec![clip("c", 0, 100).with_speed(1.0).with_trim_start(10)];
        let actions = compute_overwrite(&clips, 40, 60);
        match &actions[0] {
            OverwriteAction::Split { right_trim_start, .. } => {
                assert_eq!(*right_trim_start, 70, "10 + round(60*1.0)");
            }
            other => panic!("expected Split, got {other:?}"),
        }
    }
}
