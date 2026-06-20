//! E3-S6 — Edit orchestration: ripple / overwrite / split fan-out + atomic apply.
//!
//! The command layer over the pure engines ([`ripple`](crate::ripple),
//! [`overwrite`](crate::overwrite), [`split`](crate::split)) and the undo system
//! ([`palmier_history::History`]). It runs the engines against a **real**
//! `palmier_model::Timeline` via the [`adapter`](crate::adapter) seam, fans the
//! pure results across **sync-locked** and **linked** tracks, and applies them
//! **atomically** (validate-then-apply: refuse the whole edit on
//! negative-start/overlap **before mutating**).
//!
//! Ported from `EditorViewModel+Ripple.swift`, `EditorViewModel+ClipMutations.swift`
//! (`clearRegion`, `moveClips`, `splitClip`, `withTimelineSwap`), and
//! `EditorViewModel+Linking.swift` (link-group expansion). See
//! docs/reference/edit-engines.md §"Ripple orchestration" (lines 66-85) and
//! §"Mapping to FOUNDATION crates" (lines 216-217), and story E3-S6.
//!
//! ## Atomicity (refuse-the-whole-edit) — edit-engines.md lines 225-227
//!
//! Every multi-track ripple **validates all affected sync-locked tracks first**
//! (collecting shifts, dry-running [`validate_shifts`](crate::ripple::validate_shifts)).
//! Only if **every** track validates do we open the undo swap and mutate. A
//! refusal returns `Err(RefuseReason)` having touched nothing — the timeline is
//! byte-unchanged. There is no `NSSound.beep`; the caller surfaces a toast.
//!
//! ## Undo grouping — edit-engines.md lines 248-249
//!
//! Each public command runs its mutation inside **one**
//! [`History::with_user_swap`](palmier_history::History::with_user_swap). That
//! takes a whole-`Timeline` before/after snapshot and registers exactly **one**
//! named user-undo entry; nested swaps/pushes inside coalesce (the history crate
//! suppresses nested registration). A no-op edit (before == after) registers
//! nothing, matching the reference `guard before != after`.

use palmier_history::History;
use palmier_model::{Clip, FrameRange, Timeline};

use crate::adapter::{clip_to_split_clip, track_to_placements};
use crate::overwrite::{compute_overwrite_with, OverwriteAction};
use crate::ripple::{
    compute_ripple_push, compute_ripple_shifts, compute_ripple_shifts_for_ranges, merge_ranges,
    validate_shifts, RefuseReason,
};
use crate::rounding::round_ties_away;
use crate::split::split_clip;

/// New-UUID minter injected into the orchestration so split right-fragment / linked
/// regroup ids are deterministic in tests and real UUIDs in production.
pub type IdGen<'a> = &'a mut dyn FnMut() -> String;

/// A successful range-ripple report (reference `rippleDeleteRangesOnTrack` return,
/// drives MCP `ripple_delete_ranges`). edit-engines.md lines 76-77.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RippleRangeReport {
    /// Total frames removed (Σ merged range lengths).
    pub removed_frames: i32,
    /// Indices of the tracks whose region content was cleared.
    pub cleared_tracks: Vec<usize>,
    /// Number of clips that were shifted left to close the gaps.
    pub shifted_clips: usize,
    /// Ids of clips that were fully removed by the clear.
    pub removed_clip_ids: Vec<String>,
}

// =====================================================================
// internal helpers
// =====================================================================

/// Find the index of the clip with `id` on `track`, if present.
fn clip_pos(track: &palmier_model::Track, id: &str) -> Option<usize> {
    track.clips.iter().position(|c| c.id == id)
}

/// Apply a shift map to one track's clips in place (writes each clip's new
/// `start_frame`), then re-sort. The shifts must already be validated.
fn apply_shifts_to_track(track: &mut palmier_model::Track, shifts: &[palmier_model::ClipShift]) {
    for shift in shifts {
        if let Some(clip) = track.clips.iter_mut().find(|c| c.id == shift.clip_id) {
            clip.start_frame = shift.new_start_frame;
        }
    }
    track.sort_clips();
}

/// The set of clip ids (across all tracks) selected, expanded to whole link groups.
///
/// Reference `expandToLinkGroup`: any selected clip that carries a `link_group_id`
/// pulls in **every** clip sharing that group id (edit-engines.md lines 152-154,
/// 162-163). Returns the expanded id set.
pub fn expand_to_link_group(timeline: &Timeline, ids: &[String]) -> Vec<String> {
    // Collect the link groups touched by the selection.
    let mut groups: Vec<String> = Vec::new();
    for t in &timeline.tracks {
        for c in &t.clips {
            if ids.iter().any(|i| i == &c.id)
                && let Some(g) = &c.link_group_id
                    && !groups.contains(g) {
                        groups.push(g.clone());
                    }
        }
    }
    let mut out: Vec<String> = ids.to_vec();
    for t in &timeline.tracks {
        for c in &t.clips {
            if let Some(g) = &c.link_group_id
                && groups.contains(g) && !out.contains(&c.id) {
                    out.push(c.id.clone());
                }
        }
    }
    out
}

/// Ids of clips on **any** track that are the linked partners of a clip the ranges
/// overlap on the anchor track. Reference `rippleDeleteRangesOnTrack`'s
/// `clearTrackIds` fan-out (edit-engines.md lines 72-74): a range overlaps a clip
/// iff `r.start < c.end_frame && r.end > c.start_frame`.
fn linked_partner_track_indices(
    timeline: &Timeline,
    anchor_track: usize,
    merged: &[FrameRange],
) -> Vec<usize> {
    // Link groups of clips on the anchor track that the ranges overlap.
    let mut groups: Vec<String> = Vec::new();
    if let Some(track) = timeline.tracks.get(anchor_track) {
        for c in &track.clips {
            let overlaps = merged
                .iter()
                .any(|r| r.start < c.end_frame() && r.end > c.start_frame);
            if overlaps
                && let Some(g) = &c.link_group_id
                    && !groups.contains(g) {
                        groups.push(g.clone());
                    }
        }
    }
    let mut tracks: Vec<usize> = Vec::new();
    for (ti, t) in timeline.tracks.iter().enumerate() {
        if ti == anchor_track {
            continue;
        }
        let has_partner = t
            .clips
            .iter()
            .any(|c| c.link_group_id.as_ref().is_some_and(|g| groups.contains(g)));
        if has_partner {
            tracks.push(ti);
        }
    }
    tracks
}

/// Apply a single [`OverwriteAction`] to a track in place, re-deriving the `Split`
/// right fragment via [`split_clip`] (the engine's advisory fields are ignored —
/// edit-engines.md lines 95-97, 228-229). New ids come from `new_id`.
fn apply_overwrite_action(
    track: &mut palmier_model::Track,
    action: &OverwriteAction,
    new_id: &mut dyn FnMut() -> String,
) {
    match action {
        OverwriteAction::Remove { clip_id } => {
            track.clips.retain(|c| &c.id != clip_id);
        }
        OverwriteAction::TrimEnd {
            clip_id,
            new_duration,
        } => {
            if let Some(clip) = track.clips.iter_mut().find(|c| &c.id == clip_id) {
                // Recompute source tail trim: trim_end += round((old_dur - new_dur)·speed)
                // (edit-engines.md line 95).
                let old_dur = clip.duration_frames;
                clip.trim_end_frame +=
                    round_ties_away((old_dur - new_duration) as f64 * clip.speed);
                clip.set_duration(*new_duration);
            }
        }
        OverwriteAction::TrimStart {
            clip_id,
            new_start_frame,
            new_trim_start,
            new_duration,
        } => {
            if let Some(clip) = track.clips.iter_mut().find(|c| &c.id == clip_id) {
                clip.start_frame = *new_start_frame;
                clip.trim_start_frame = *new_trim_start;
                clip.set_duration(*new_duration);
            }
        }
        OverwriteAction::Split { clip_id, .. } => {
            // VM re-derives the right fragment via split_clip(at = region_start);
            // the engine's right-fragment fields are advisory. We split the real
            // clip at the region start, keep the left, then trim the right's head
            // to region_end by re-splitting it at region_end.
            // The engine's `Split` carries left_duration → region_start = cs + left_duration.
            if let OverwriteAction::Split {
                left_duration,
                right_duration,
                ..
            } = action
                && let Some(pos) = clip_pos(track, clip_id) {
                    let cs = track.clips[pos].start_frame;
                    let region_start = cs + left_duration;
                    let region_end = track.clips[pos].end_frame() - right_duration;
                    let view = clip_to_split_clip(&track.clips[pos]);
                    // First split at region_start → left (kept), mid+right.
                    if let Some((left, right_full)) = split_clip(&view, region_start, &mut *new_id) {
                        // Re-split the right_full at region_end to drop the covered middle.
                        let right_kept = split_clip(&right_full, region_end, &mut *new_id)
                            .map(|(_, r)| r)
                            .unwrap_or(right_full);
                        // Write the left fragment back over the original clip.
                        let orig = &mut track.clips[pos];
                        orig.start_frame = left.start_frame;
                        orig.trim_start_frame = left.trim_start_frame;
                        orig.trim_end_frame = left.trim_end_frame;
                        orig.fade_out_frames = left.fade_out_frames;
                        orig.set_duration(left.duration_frames);
                        // Build the right clip as a fresh copy of the original.
                        let mut right_clip = track.clips[pos].clone();
                        right_clip.id = right_kept.id;
                        right_clip.start_frame = right_kept.start_frame;
                        right_clip.trim_start_frame = right_kept.trim_start_frame;
                        right_clip.trim_end_frame = right_kept.trim_end_frame;
                        right_clip.fade_in_frames = right_kept.fade_in_frames;
                        right_clip.fade_out_frames = right_kept.fade_out_frames;
                        right_clip.set_duration(right_kept.duration_frames);
                        track.clips.push(right_clip);
                    }
                }
        }
    }
    track.sort_clips();
}

/// Clear `[region_start, region_end)` on one track (reference `clearRegion`):
/// compute overwrite actions, apply them (re-deriving splits). Returns the ids of
/// clips fully removed. Does **not** ripple — overwrite-style, in place.
fn clear_region(
    track: &mut palmier_model::Track,
    region_start: i32,
    region_end: i32,
    new_id: &mut dyn FnMut() -> String,
    track_index: usize,
) -> Vec<String> {
    let placements = track_to_placements(track, track_index);
    let actions = compute_overwrite_with(&placements, region_start, region_end, &mut *new_id);
    let mut removed = Vec::new();
    for action in &actions {
        if let OverwriteAction::Remove { clip_id } = action {
            removed.push(clip_id.clone());
        }
        apply_overwrite_action(track, action, &mut *new_id);
    }
    removed
}

// =====================================================================
// public commands
// =====================================================================

/// **Ripple delete selected clips** (reference `rippleDeleteSelectedClips`,
/// edit-engines.md lines 66-70).
///
/// Builds `global_removed_ranges` from the selected clips' spans (expanded to link
/// groups). For each track: if it has its **own** removed clips →
/// [`compute_ripple_shifts`]; else if `sync_locked` →
/// [`compute_ripple_shifts_for_ranges`] over the global ranges, then
/// [`validate_shifts`]. **Validation runs across all tracks first; on any failure
/// the whole edit is refused** (`Err`) with the timeline untouched. On success the
/// edit is applied inside **one** undo swap.
pub fn ripple_delete_selected_clips(
    timeline: &mut Timeline,
    history: &mut History<Timeline>,
    selected_ids: &[String],
) -> Result<(), RefuseReason> {
    let expanded = expand_to_link_group(timeline, selected_ids);
    if expanded.is_empty() {
        return Ok(());
    }

    // Global removed ranges = the spans of every selected (expanded) clip.
    let global_removed: Vec<FrameRange> = timeline
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| expanded.iter().any(|id| id == &c.id))
        .map(|c| FrameRange::new(c.start_frame, c.end_frame()))
        .collect();

    // --- VALIDATE FIRST (atomic refuse-the-whole-edit) ---
    // Per track, compute the shift map it will receive, then dry-run validate the
    // post-removal track. We compute on the *remaining* clips for own-removal
    // tracks (compute_ripple_shifts already excludes removed), and on all clips
    // for sync-locked followers.
    struct Plan {
        track_index: usize,
        shifts: Vec<palmier_model::ClipShift>,
    }
    let mut plans: Vec<Plan> = Vec::new();
    for (ti, track) in timeline.tracks.iter().enumerate() {
        let own_removed: Vec<String> = track
            .clips
            .iter()
            .filter(|c| expanded.iter().any(|id| id == &c.id))
            .map(|c| c.id.clone())
            .collect();
        let placements = track_to_placements(track, ti);
        let shifts = if !own_removed.is_empty() {
            compute_ripple_shifts(&placements, &own_removed)
        } else if track.sync_locked {
            compute_ripple_shifts_for_ranges(&placements, &global_removed)
        } else {
            continue;
        };
        // Dry-run validation against the remaining clips (after removal) on that track.
        let remaining: Vec<_> = placements
            .iter()
            .filter(|p| !own_removed.iter().any(|id| id == &p.id))
            .cloned()
            .collect();
        validate_shifts(&remaining, &shifts)?;
        plans.push(Plan {
            track_index: ti,
            shifts,
        });
    }

    // --- APPLY (one undo swap) ---
    history.with_user_swap("Ripple Delete", timeline, |tl| {
        // Remove the selected clips everywhere.
        for track in tl.tracks.iter_mut() {
            track.clips.retain(|c| !expanded.iter().any(|id| id == &c.id));
        }
        // Apply the validated shifts.
        for plan in &plans {
            if let Some(track) = tl.tracks.get_mut(plan.track_index) {
                apply_shifts_to_track(track, &plan.shifts);
            }
        }
    });
    Ok(())
}

/// **Ripple delete ranges on a track** (reference `rippleDeleteRangesOnTrack`,
/// drives MCP `ripple_delete_ranges`; edit-engines.md lines 71-77).
///
/// `merged = merge_ranges(ranges with length > 0)`. `clear_track_ids` = the anchor
/// track + every track holding a **linked partner** of any clip a range overlaps.
/// Every **non-cleared** sync-locked track is pre-validated; a collision refuses
/// the whole edit. On success, inside one undo swap: each cleared track has its
/// region cleared per merged range; cleared OR sync-locked tracks then get the
/// gap-closing shifts.
pub fn ripple_delete_ranges_on_track(
    timeline: &mut Timeline,
    history: &mut History<Timeline>,
    anchor_track: usize,
    ranges: &[FrameRange],
    new_id: IdGen<'_>,
) -> Result<RippleRangeReport, RefuseReason> {
    let merged = merge_ranges(
        &ranges
            .iter()
            .copied()
            .filter(|r| r.length() > 0)
            .collect::<Vec<_>>(),
    );
    if merged.is_empty() {
        return Ok(RippleRangeReport::default());
    }
    let removed_frames: i32 = merged.iter().map(|r| r.length()).sum();

    // Tracks to clear: anchor + tracks with linked partners of overlapped clips.
    let mut cleared_tracks = vec![anchor_track];
    for ti in linked_partner_track_indices(timeline, anchor_track, &merged) {
        if !cleared_tracks.contains(&ti) {
            cleared_tracks.push(ti);
        }
    }

    // --- VALIDATE FIRST: every non-cleared sync-locked track must have room. ---
    for (ti, track) in timeline.tracks.iter().enumerate() {
        if cleared_tracks.contains(&ti) || !track.sync_locked {
            continue;
        }
        let placements = track_to_placements(track, ti);
        let shifts = compute_ripple_shifts_for_ranges(&placements, &merged);
        validate_shifts(&placements, &shifts)?;
    }

    // --- APPLY (one undo swap) ---
    let mut report = RippleRangeReport {
        removed_frames,
        ..Default::default()
    };
    let cleared = cleared_tracks.clone();
    let merged_for_apply = merged.clone();
    history.with_user_swap("Ripple Delete Range", timeline, |tl| {
        // 1. Clear the region on each cleared track, per merged range.
        for &ti in &cleared {
            if let Some(track) = tl.tracks.get_mut(ti) {
                for r in &merged_for_apply {
                    let removed = clear_region(track, r.start, r.end, &mut *new_id, ti);
                    report.removed_clip_ids.extend(removed);
                }
            }
        }
        report.cleared_tracks = cleared.clone();
        // 2. Apply gap-closing shifts to cleared OR sync-locked tracks.
        for ti in 0..tl.tracks.len() {
            let is_cleared = cleared.contains(&ti);
            let is_sync = tl.tracks[ti].sync_locked;
            if !is_cleared && !is_sync {
                continue;
            }
            let placements = track_to_placements(&tl.tracks[ti], ti);
            let shifts = compute_ripple_shifts_for_ranges(&placements, &merged_for_apply);
            report.shifted_clips += shifts.len();
            apply_shifts_to_track(&mut tl.tracks[ti], &shifts);
        }
    });
    Ok(report)
}

/// **Ripple insert** (reference `rippleInsertClips`, edit-engines.md lines 78-79).
///
/// `total_push = Σ new_clips' duration`. For the target track + every sync-locked
/// track, push every clip at/after `at_frame` forward by `total_push`; then place
/// the new clips at `at_frame` on the target track. One undo swap.
pub fn ripple_insert_clips(
    timeline: &mut Timeline,
    history: &mut History<Timeline>,
    target_track: usize,
    at_frame: i32,
    new_clips: Vec<Clip>,
) {
    let total_push: i32 = new_clips.iter().map(|c| c.duration_frames).sum();
    if total_push == 0 {
        return;
    }
    history.with_user_swap("Insert", timeline, |tl| {
        for ti in 0..tl.tracks.len() {
            if ti != target_track && !tl.tracks[ti].sync_locked {
                continue;
            }
            let placements = track_to_placements(&tl.tracks[ti], ti);
            let shifts = compute_ripple_push(&placements, at_frame, total_push, &[]);
            apply_shifts_to_track(&mut tl.tracks[ti], &shifts);
        }
        if let Some(track) = tl.tracks.get_mut(target_track) {
            for mut clip in new_clips.clone() {
                clip.start_frame = at_frame;
                track.clips.push(clip);
            }
            track.sort_clips();
        }
    });
}

/// **Gap ripple** (reference `rippleDeleteSelectedGap`, edit-engines.md lines
/// 84-85).
///
/// Closes the gap `[gap_start, gap_end)` on `gap_track`: shifts the gap track and
/// its sync-locked followers left by the gap length. **Followers are validated**;
/// the gap track is not (the reference trusts the gap is genuinely empty). Refuses
/// the whole edit if any follower collides. One undo swap.
pub fn ripple_delete_gap(
    timeline: &mut Timeline,
    history: &mut History<Timeline>,
    gap_track: usize,
    gap: FrameRange,
) -> Result<(), RefuseReason> {
    if gap.length() <= 0 {
        return Ok(());
    }
    let ranges = [gap];

    // Validate sync-locked followers first.
    for (ti, track) in timeline.tracks.iter().enumerate() {
        if ti == gap_track || !track.sync_locked {
            continue;
        }
        let placements = track_to_placements(track, ti);
        let shifts = compute_ripple_shifts_for_ranges(&placements, &ranges);
        validate_shifts(&placements, &shifts)?;
    }

    history.with_user_swap("Close Gap", timeline, |tl| {
        for ti in 0..tl.tracks.len() {
            if ti != gap_track && !tl.tracks[ti].sync_locked {
                continue;
            }
            let placements = track_to_placements(&tl.tracks[ti], ti);
            let shifts = compute_ripple_shifts_for_ranges(&placements, &ranges);
            apply_shifts_to_track(&mut tl.tracks[ti], &shifts);
        }
    });
    Ok(())
}

/// **Linked split** (reference `splitClip` + link regroup, edit-engines.md lines
/// 109-110, 237).
///
/// Splits the clip with `clip_id` at absolute `at_frame`; if it belongs to a link
/// group, splits **every** member at the same `at_frame`. The right halves are
/// **regrouped under a fresh `link_group_id`** so the two halves no longer move
/// together (a stale cross-link is the classic bug). Razor-tool and Ctrl+K /
/// playhead split both route here. One undo swap. Returns the ids of the new right
/// fragments. A split that hits no clip's interior is a no-op (`Ok(vec![])`).
pub fn split_at(
    timeline: &mut Timeline,
    history: &mut History<Timeline>,
    clip_id: &str,
    at_frame: i32,
    new_id: IdGen<'_>,
) -> Vec<String> {
    // Determine the set of clips to split: the target, expanded to its link group.
    let group_ids = expand_to_link_group(timeline, &[clip_id.to_string()]);
    // Only the members that actually straddle `at_frame` can split.
    let splittable: Vec<String> = timeline
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| {
            group_ids.iter().any(|id| id == &c.id)
                && c.start_frame < at_frame
                && at_frame < c.end_frame()
        })
        .map(|c| c.id.clone())
        .collect();
    if splittable.is_empty() {
        return Vec::new();
    }

    // A fresh link group for the right halves — only assigned when >1 clip splits
    // (a lone split needs no new group). Computed before the swap so the id source
    // is deterministic.
    let regroup = splittable.len() > 1;
    let fresh_group = if regroup { Some(new_id()) } else { None };

    let mut right_ids = Vec::new();
    history.with_user_swap("Split", timeline, |tl| {
        for track in tl.tracks.iter_mut() {
            // Collect positions to split (can't mutate while iterating positions).
            let positions: Vec<usize> = track
                .clips
                .iter()
                .enumerate()
                .filter(|(_, c)| splittable.iter().any(|id| id == &c.id))
                .map(|(i, _)| i)
                .collect();
            for pos in positions {
                let view = clip_to_split_clip(&track.clips[pos]);
                if let Some((left, right)) = split_clip(&view, at_frame, &mut *new_id) {
                    // Write left back over the original clip.
                    let orig = &mut track.clips[pos];
                    orig.trim_end_frame = left.trim_end_frame;
                    orig.fade_out_frames = left.fade_out_frames;
                    orig.set_duration(left.duration_frames);
                    // Migrate the left volume keyframes if the view carried any.
                    if let Some(kfs) = left.volume_track {
                        write_volume_track(orig, &kfs);
                    }
                    // Build the right clip from a clone of the (now-left) original.
                    let mut right_clip = track.clips[pos].clone();
                    right_clip.id = right.id.clone();
                    right_clip.start_frame = right.start_frame;
                    right_clip.trim_start_frame = right.trim_start_frame;
                    right_clip.trim_end_frame = right.trim_end_frame;
                    right_clip.fade_in_frames = right.fade_in_frames;
                    right_clip.fade_out_frames = right.fade_out_frames;
                    right_clip.set_duration(right.duration_frames);
                    if let Some(kfs) = right.volume_track {
                        write_volume_track(&mut right_clip, &kfs);
                    } else {
                        right_clip.volume_track = None;
                    }
                    // Regroup the right half under the fresh link group id.
                    right_clip.link_group_id = fresh_group.clone();
                    right_ids.push(right_clip.id.clone());
                    track.clips.push(right_clip);
                }
            }
            track.sort_clips();
        }
    });
    right_ids
}

/// Overwrite the clip's volume keyframe track from a view list (clip-relative dB).
fn write_volume_track(clip: &mut Clip, kfs: &[crate::split::VolumeKeyframe]) {
    let mut track = palmier_model::KeyframeTrack::new();
    for kf in kfs {
        track.keyframes.push(palmier_model::Keyframe::with_interpolation(
            kf.frame,
            kf.value,
            kf.interpolation_out,
        ));
    }
    clip.volume_track = if track.keyframes.is_empty() {
        None
    } else {
        Some(track)
    };
}

/// A single clip move instruction for [`move_clips`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveSpec {
    /// Id of the clip to move.
    pub clip_id: String,
    /// Destination track index.
    pub to_track: usize,
    /// Destination start frame.
    pub to_frame: i32,
}

/// **Move clips** (reference `moveClips`, edit-engines.md lines 164-170).
///
/// Pulls the movers off their source tracks, clears each destination region
/// (overwrite), and drops each mover at its exact `to_frame` on `to_track`.
/// Destination clearing uses [`clear_region`] so an existing clip under the drop is
/// overwritten in place. One undo swap. (The drag-state machine in
/// [`crate::drag`] computes *which* clips move and to where — pinned companions,
/// clamps — and hands the resolved [`MoveSpec`]s here.)
pub fn move_clips(
    timeline: &mut Timeline,
    history: &mut History<Timeline>,
    moves: &[MoveSpec],
    new_id: IdGen<'_>,
) {
    if moves.is_empty() {
        return;
    }
    let move_ids: Vec<String> = moves.iter().map(|m| m.clip_id.clone()).collect();
    history.with_user_swap("Move", timeline, |tl| {
        // Pull the movers off their source tracks, keeping their clip data.
        let mut pulled: Vec<Clip> = Vec::new();
        for track in tl.tracks.iter_mut() {
            let mut i = 0;
            while i < track.clips.len() {
                if move_ids.iter().any(|id| id == &track.clips[i].id) {
                    pulled.push(track.clips.remove(i));
                } else {
                    i += 1;
                }
            }
        }
        // Drop each mover at its destination, clearing the region first.
        for spec in moves {
            let Some(mut clip) = pulled.iter().find(|c| c.id == spec.clip_id).cloned() else {
                continue;
            };
            clip.start_frame = spec.to_frame;
            let region_start = spec.to_frame;
            let region_end = spec.to_frame + clip.duration_frames;
            if let Some(track) = tl.tracks.get_mut(spec.to_track) {
                clear_region(track, region_start, region_end, &mut *new_id, spec.to_track);
                track.clips.push(clip);
                track.sort_clips();
            }
        }
    });
}

#[cfg(test)]
mod tests;
