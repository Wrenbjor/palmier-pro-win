//! E3-S2 — RippleEngine (pure).
//!
//! Gap-closing shifts for deletes and push shifts for inserts, plus
//! `validate_shifts` for the sync-locked refuse-the-whole-edit guard. Ported 1:1
//! from `Sources/PalmierPro/Editor/RippleEngine.swift` (`computeRippleShifts`,
//! `computeRippleShiftsForRanges`, `computeRipplePush`, `mergeRanges`) and the
//! `validateShifts` dry-run from `EditorViewModel+Ripple.swift`.
//!
//! See docs/reference/edit-engines.md §"RippleEngine (pure)" and the epic story
//! E3-S2. All ranges are **half-open `[start, end)`** and `merge_ranges` merges
//! **touching** ranges (`<=`) — see [`merge_ranges`].

use palmier_model::{ClipShift, FrameRange};

use crate::placement::ClipPlacement;

/// Why a ripple edit was refused (reference returns a human string; we keep a
/// typed reason so the orchestration layer can pick the toast copy and the tests
/// can assert without string-matching). `track_label` is filled by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefuseReason {
    /// A shifted clip would start before frame 0
    /// (reference: "would move past the timeline start").
    NegativeStart,
    /// Two clips would overlap after the shift
    /// (reference: "doesn't have room to ripple").
    Overlap,
}

/// Merge a set of (possibly overlapping / touching / unsorted) half-open ranges
/// into a minimal sorted set.
///
/// Reference `RippleEngine.mergeRanges`: sort by `start`; fold — if
/// `range.start <= last.end` extend `last.end = max(last.end, range.end)`, else
/// push. The `<=` (not `<`) means **touching ranges merge**: `[0,10)` and
/// `[10,20)` collapse to `[0,20)` (edit-engines.md lines 55-56).
pub fn merge_ranges(ranges: &[FrameRange]) -> Vec<FrameRange> {
    let mut sorted: Vec<FrameRange> = ranges.to_vec();
    sorted.sort_by_key(|r| r.start);
    let mut merged: Vec<FrameRange> = Vec::with_capacity(sorted.len());
    for range in sorted {
        if let Some(last) = merged.last_mut() {
            // Touching ranges merge because the test is `<=`, not `<`.
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}

/// Shift clips leftward to close the gaps defined by `removed_ranges`.
///
/// Reference `computeRippleShiftsForRanges` (edit-engines.md lines 57-60): merge
/// the ranges, then for each clip (iterated sorted by `start_frame`) the shift is
/// the **sum of the lengths of every merged gap lying entirely before the clip**
/// (`r.end <= clip.start_frame`). A `ClipShift` is emitted **only when
/// `shift > 0`**: a clip that overlaps a gap is assumed already removed/cleared,
/// so it never shifts here.
///
/// Note the boundary is `<=`: a clip starting **exactly** at a gap's end (`r.end
/// == clip.start_frame`) **does** shift — that gap lies fully before it.
pub fn compute_ripple_shifts_for_ranges(
    clips: &[ClipPlacement],
    removed_ranges: &[FrameRange],
) -> Vec<ClipShift> {
    let merged = merge_ranges(removed_ranges);
    if merged.is_empty() {
        return Vec::new();
    }

    let mut sorted: Vec<&ClipPlacement> = clips.iter().collect();
    sorted.sort_by_key(|c| c.start_frame);

    let mut shifts = Vec::new();
    for clip in sorted {
        let shift: i32 = merged
            .iter()
            .filter(|r| r.end <= clip.start_frame)
            .map(|r| r.length())
            .sum();
        if shift > 0 {
            shifts.push(ClipShift::new(clip.id.clone(), clip.start_frame - shift));
        }
    }
    shifts
}

/// After removing the clips named in `removed_ids` from a track, compute the
/// shifts for the **remaining** clips that close the resulting gaps.
///
/// Reference `computeRippleShifts` (edit-engines.md lines 61-62): derive the
/// removed ranges from the removed clips' `[start_frame, end_frame)`, then call
/// [`compute_ripple_shifts_for_ranges`] on the clips **not** in `removed_ids`.
pub fn compute_ripple_shifts(clips: &[ClipPlacement], removed_ids: &[String]) -> Vec<ClipShift> {
    let removed: Vec<FrameRange> = clips
        .iter()
        .filter(|c| removed_ids.iter().any(|id| id == &c.id))
        .map(|c| FrameRange::new(c.start_frame, c.end_frame()))
        .collect();
    let remaining: Vec<ClipPlacement> = clips
        .iter()
        .filter(|c| !removed_ids.iter().any(|id| id == &c.id))
        .cloned()
        .collect();
    compute_ripple_shifts_for_ranges(&remaining, &removed)
}

/// Push every clip at or after `insert_frame` forward by `push_amount`, excluding
/// `exclude_ids`.
///
/// Reference `computeRipplePush` (edit-engines.md lines 63-64): every clip with
/// `start_frame >= insert_frame` and `id ∉ exclude_ids` →
/// `ClipShift(id, start_frame + push_amount)`.
pub fn compute_ripple_push(
    clips: &[ClipPlacement],
    insert_frame: i32,
    push_amount: i32,
    exclude_ids: &[String],
) -> Vec<ClipShift> {
    clips
        .iter()
        .filter(|c| c.start_frame >= insert_frame && !exclude_ids.iter().any(|id| id == &c.id))
        .map(|c| ClipShift::new(c.id.clone(), c.start_frame + push_amount))
        .collect()
}

/// Validate a shift map against one track's clips **before** applying it
/// (sync-locked ripple refuse-the-whole-edit guard).
///
/// Reference `validateShifts` (`EditorViewModel+Ripple.swift`, edit-engines.md
/// lines 80-82): apply the shift map to the track's clips; refuse if any clip
/// would start `< 0` ([`RefuseReason::NegativeStart`]) or any two intervals
/// overlap after sorting by start ([`RefuseReason::Overlap`]). Clips absent from
/// the shift map keep their current `start_frame`.
///
/// `track_clips` must already be the placements **for that one track** (the
/// orchestration layer slices per track before calling). Returns `Ok(())` when
/// the shift is safe to apply.
pub fn validate_shifts(
    track_clips: &[ClipPlacement],
    shifts: &[ClipShift],
) -> Result<(), RefuseReason> {
    if shifts.is_empty() {
        return Ok(());
    }
    // Build the post-shift intervals: shifted clips take their new start, others
    // keep theirs. Reference uses a dict keyed by clip id.
    let mut intervals: Vec<FrameRange> = Vec::with_capacity(track_clips.len());
    for clip in track_clips {
        let start = shifts
            .iter()
            .find(|s| s.clip_id == clip.id)
            .map(|s| s.new_start_frame)
            .unwrap_or(clip.start_frame);
        if start < 0 {
            return Err(RefuseReason::NegativeStart);
        }
        intervals.push(FrameRange::new(start, start + clip.duration_frames));
    }
    intervals.sort_by_key(|r| r.start);
    // Overlap iff a later clip starts before the previous one ends (half-open:
    // touching `prev.end == next.start` is fine).
    for i in 1..intervals.len() {
        if intervals[i].start < intervals[i - 1].end {
            return Err(RefuseReason::Overlap);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(id: &str, start: i32, dur: i32) -> ClipPlacement {
        ClipPlacement::new(id, start, dur, 0)
    }

    // ---- merge_ranges -----------------------------------------------------

    #[test]
    fn merge_ranges_sorts_and_merges_overlapping() {
        let merged = merge_ranges(&[
            FrameRange::new(30, 40),
            FrameRange::new(0, 10),
            FrameRange::new(5, 20), // overlaps [0,10)
        ]);
        assert_eq!(merged, vec![FrameRange::new(0, 20), FrameRange::new(30, 40)]);
    }

    #[test]
    fn merge_ranges_merges_touching() {
        // Ruling/edit-engines.md lines 55-56: touching ranges merge because `<=`.
        let merged = merge_ranges(&[FrameRange::new(0, 10), FrameRange::new(10, 20)]);
        assert_eq!(
            merged,
            vec![FrameRange::new(0, 20)],
            "[0,10) and [10,20) must collapse to [0,20)"
        );
    }

    #[test]
    fn merge_ranges_keeps_disjoint_separate() {
        let merged = merge_ranges(&[FrameRange::new(0, 10), FrameRange::new(11, 20)]);
        assert_eq!(merged.len(), 2, "a 1-frame gap keeps them separate");
    }

    // ---- compute_ripple_shifts (single track, own removed clips) ----------

    #[test]
    fn ripple_shifts_single_removed_range() {
        // Three abutting 10-frame clips; remove the middle → the third shifts left
        // by 10 to close the gap.
        let clips = vec![clip("a", 0, 10), clip("b", 10, 10), clip("c", 20, 10)];
        let shifts = compute_ripple_shifts(&clips, &["b".to_string()]);
        assert_eq!(shifts, vec![ClipShift::new("c", 10)]);
    }

    #[test]
    fn ripple_shifts_multi_range_merges_and_sums() {
        // Remove a (0..10) and c (20..30) — two separate gaps. b (10..20) only has
        // the first gap before it → shift 10. d (30..40) has both → shift 20.
        let clips = vec![
            clip("a", 0, 10),
            clip("b", 10, 10),
            clip("c", 20, 10),
            clip("d", 30, 10),
        ];
        let shifts = compute_ripple_shifts(&clips, &["a".to_string(), "c".to_string()]);
        assert_eq!(
            shifts,
            vec![ClipShift::new("b", 0), ClipShift::new("d", 10)]
        );
    }

    #[test]
    fn ripple_shifts_for_ranges_touching_merge_affects_sum() {
        // Two touching removed ranges [0,10)+[10,20) merge to one length-20 gap.
        // A clip at 20 starts exactly at the merged gap end → shifts by 20.
        let clips = vec![clip("x", 20, 10)];
        let shifts = compute_ripple_shifts_for_ranges(
            &clips,
            &[FrameRange::new(0, 10), FrameRange::new(10, 20)],
        );
        assert_eq!(shifts, vec![ClipShift::new("x", 0)]);
    }

    #[test]
    fn ripple_clip_starting_exactly_at_gap_end_shifts() {
        // Boundary `r.end == clip.start_frame` (`<=`) → the gap lies fully before
        // the clip, so it shifts (edit-engines.md line 58).
        let clips = vec![clip("x", 10, 5)];
        let shifts = compute_ripple_shifts_for_ranges(&clips, &[FrameRange::new(0, 10)]);
        assert_eq!(shifts, vec![ClipShift::new("x", 0)]);
    }

    #[test]
    fn ripple_clip_overlapping_gap_emits_no_shift() {
        // A clip overlapping the gap (start 5 < gap end 10) is "assumed already
        // removed/cleared" — no shift emitted (edit-engines.md lines 59-60).
        let clips = vec![clip("x", 5, 20)];
        let shifts = compute_ripple_shifts_for_ranges(&clips, &[FrameRange::new(0, 10)]);
        assert!(shifts.is_empty(), "overlapping clip must not shift");
    }

    // ---- compute_ripple_push ---------------------------------------------

    #[test]
    fn ripple_push_shifts_at_or_after_insert() {
        let clips = vec![clip("a", 0, 10), clip("b", 10, 10), clip("c", 20, 10)];
        let shifts = compute_ripple_push(&clips, 10, 100, &[]);
        // a is before the insert frame → untouched; b (==) and c (>) push.
        assert_eq!(
            shifts,
            vec![ClipShift::new("b", 110), ClipShift::new("c", 120)]
        );
    }

    #[test]
    fn ripple_push_respects_excluded_ids() {
        let clips = vec![clip("a", 10, 10), clip("b", 20, 10)];
        let shifts = compute_ripple_push(&clips, 0, 50, &["a".to_string()]);
        assert_eq!(shifts, vec![ClipShift::new("b", 70)], "a excluded");
    }

    // ---- validate_shifts (sync-locked refuse guard) ----------------------

    #[test]
    fn validate_shifts_ok_when_no_collision() {
        let clips = vec![clip("a", 0, 10), clip("b", 20, 10)];
        // Shift b left to abut a exactly — touching is allowed (half-open).
        let shifts = vec![ClipShift::new("b", 10)];
        assert_eq!(validate_shifts(&clips, &shifts), Ok(()));
    }

    #[test]
    fn validate_shifts_refuses_negative_start() {
        let clips = vec![clip("a", 5, 10)];
        let shifts = vec![ClipShift::new("a", -1)];
        assert_eq!(
            validate_shifts(&clips, &shifts),
            Err(RefuseReason::NegativeStart)
        );
    }

    #[test]
    fn validate_shifts_refuses_overlap() {
        // Shift b to start at 5 — collides with a's [0,10).
        let clips = vec![clip("a", 0, 10), clip("b", 20, 10)];
        let shifts = vec![ClipShift::new("b", 5)];
        assert_eq!(validate_shifts(&clips, &shifts), Err(RefuseReason::Overlap));
    }

    #[test]
    fn validate_shifts_empty_is_ok() {
        let clips = vec![clip("a", 0, 10)];
        assert_eq!(validate_shifts(&clips, &[]), Ok(()));
    }
}
