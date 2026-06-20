//! EDIT tool bodies — clip mutations (E7-S6; reference `ToolExecutor+Clips.swift`).
//!
//! `add_clips`, `remove_clips`, `remove_tracks`, `move_clips`, `split_clip`. Each
//! is **mutating**: the executor wraps the call in [`agent_edit`](crate::undo::agent_edit)
//! so the whole body is **one** named agent-undo step (E7-S12). The actual edit math
//! is **not** re-implemented here — it routes to palmier-edit's pure engines
//! (`compute_overwrite_with`, `split_at`, `move_clips`, `expand_to_link_group`) and
//! the model mutators, matching the reference's "the Tools layer is the parity
//! contract; the math lives in the engines".
//!
//! ## Action names (carry-forward — `mcp-tools.md` §"Agent undo stack")
//! The undo-group names must match the reference's `…(Agent)` strings exactly so
//! the agent-undo refusal check behaves identically:
//! `Add Clip(s) (Agent)`, `Move Clip(s) (Agent)`, `Split Clip (Agent)`,
//! plus `Remove Clip(s)` / `Remove Track(s)` for the non-`(Agent)` removals (the
//! reference routes those through the shared `removeClips`/`removeTracks` swaps).

use serde_json::Value;

use palmier_edit::adapter::track_to_placements;
use palmier_edit::{
    compute_overwrite_with, move_clips as edit_move_clips, round_ties_away, split_at, MoveSpec,
    OverwriteAction,
};
use palmier_model::{Clip, ClipType, MediaAsset, Timeline, Track};

use crate::editor::EditorState;
use crate::result::ToolResult;
use crate::undo::agent_edit;

// ─────────────────────────────────────────────────────────────────────────────
// shared model helpers (the palmier-edit orchestration covers ripple/overwrite/
// split/move; add/remove of clips + tracks live in the reference's
// EditorViewModel, ported minimally here over the real Timeline)
// ─────────────────────────────────────────────────────────────────────────────

/// A fresh production UUID string for new tracks/clips. palmier-model mints uuids
/// internally for `Clip::new` / `Track::new`; this borrows `Track::new`'s id
/// generator (a hyphenated UUID) for the few sites that need a standalone id (the
/// split right-fragment generator and link-group ids).
pub(crate) fn new_uuid() -> String {
    palmier_model::Track::new(ClipType::Video).id
}

/// Insert a new track of `track_type`, keeping the reference's partition invariant
/// (visual tracks sit at/above the first audio track; audio at/below it). Returns
/// the index of the inserted track. Reference `insertTrack(at:type:)` /
/// `partitionedInsertionIndex`.
pub(crate) fn insert_track(timeline: &mut Timeline, track_type: ClipType, requested: usize) -> usize {
    let first_audio = timeline
        .tracks
        .iter()
        .position(|t| t.track_type == ClipType::Audio)
        .unwrap_or(timeline.tracks.len());
    let bounded = requested.min(timeline.tracks.len());
    let at = match track_type {
        ClipType::Audio => bounded.max(first_audio),
        _ => bounded.min(first_audio),
    };
    timeline.tracks.insert(at, Track::new(track_type));
    at
}

/// Clear `[start, end)` on a track via the overwrite engine (reference
/// `clearRegion`), returning the ids of clips fully removed. Re-derives split right
/// fragments through `split_clip` exactly as palmier-edit's orchestration does
/// (the engine's advisory Split fields are ignored).
pub(crate) fn clear_region(timeline: &mut Timeline, track_index: usize, start: i32, end: i32) -> Vec<String> {
    let Some(track) = timeline.tracks.get(track_index) else {
        return Vec::new();
    };
    let placements = track_to_placements(track, track_index);
    let mut mint = new_uuid;
    let actions = compute_overwrite_with(&placements, start, end, &mut mint);
    let mut removed = Vec::new();
    let track = &mut timeline.tracks[track_index];
    for action in actions {
        match action {
            OverwriteAction::Remove { clip_id } => {
                removed.push(clip_id.clone());
                track.clips.retain(|c| c.id != clip_id);
            }
            OverwriteAction::TrimEnd { clip_id, new_duration } => {
                if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                    let old_dur = clip.duration_frames;
                    let delta = round_ties_away((old_dur - new_duration) as f64 * clip.speed);
                    clip.trim_end_frame += delta;
                    clip.set_duration(new_duration);
                }
            }
            OverwriteAction::TrimStart { clip_id, new_start_frame, new_trim_start, new_duration } => {
                if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                    clip.start_frame = new_start_frame;
                    clip.trim_start_frame = new_trim_start;
                    clip.set_duration(new_duration);
                }
            }
            OverwriteAction::Split { clip_id, left_duration, right_duration, .. } => {
                // Keep the left fragment over the original; append a right fragment
                // resuming after the cleared region. Trim math uses ties-away
                // rounding to map the consumed source span (reference clearRegion).
                if let Some(pos) = track.clips.iter().position(|c| c.id == clip_id) {
                    let cs = track.clips[pos].start_frame;
                    let speed = track.clips[pos].speed;
                    let orig_trim_start = track.clips[pos].trim_start_frame;
                    let region_end = track.clips[pos].end_frame() - right_duration;
                    let consumed_to_region_end = round_ties_away((region_end - cs) as f64 * speed);
                    let mut right = track.clips[pos].clone();
                    right.id = new_uuid();
                    right.start_frame = region_end;
                    right.trim_start_frame = orig_trim_start + consumed_to_region_end;
                    right.fade_in_frames = 0;
                    right.set_duration(right_duration);
                    let orig = &mut track.clips[pos];
                    orig.fade_out_frames = 0;
                    orig.set_duration(left_duration);
                    track.clips.push(right);
                }
            }
        }
    }
    track.sort_clips();
    removed
}

/// Place one clip on a track, auto-creating a linked audio clip for a
/// video-with-audio asset on a video track (reference `placeClip`). Returns the new
/// clip ids (the video clip, then the linked audio clip if created).
fn place_clip(
    timeline: &mut Timeline,
    asset: &MediaAsset,
    track_index: usize,
    start_frame: i32,
    duration_frames: i32,
) -> Vec<String> {
    if timeline.tracks.get(track_index).is_none() {
        return Vec::new();
    }
    let target_is_video = timeline.tracks[track_index].track_type == ClipType::Video;
    let should_link =
        target_is_video && asset.asset_type == ClipType::Video && asset.has_audio;
    let link_group_id = if should_link { Some(new_uuid()) } else { None };

    let mut clip = Clip::new(asset.id.clone(), start_frame, duration_frames);
    clip.media_type = asset.asset_type;
    clip.source_clip_type = asset.asset_type;
    clip.link_group_id = link_group_id.clone();
    let video_id = clip.id.clone();
    timeline.tracks[track_index].clips.push(clip);
    timeline.tracks[track_index].sort_clips();
    let mut ids = vec![video_id];

    if let Some(gid) = link_group_id {
        // Resolve or create an audio track for the linked audio clip.
        let audio_index = timeline
            .tracks
            .iter()
            .position(|t| t.track_type == ClipType::Audio)
            .unwrap_or_else(|| insert_track(timeline, ClipType::Audio, timeline.tracks.len()));
        let mut audio = Clip::new(asset.id.clone(), start_frame, duration_frames);
        audio.media_type = ClipType::Audio;
        audio.source_clip_type = asset.asset_type;
        audio.link_group_id = Some(gid);
        let audio_id = audio.id.clone();
        timeline.tracks[audio_index].clips.push(audio);
        timeline.tracks[audio_index].sort_clips();
        ids.push(audio_id);
    }
    ids
}

/// Expand a clip-id set to whole link groups (reference `expandToLinkGroup`).
fn expand_to_link_group(timeline: &Timeline, ids: &[String]) -> Vec<String> {
    palmier_edit::expand_to_link_group(timeline, ids)
}

// ─────────────────────────────────────────────────────────────────────────────
// add_clips
// ─────────────────────────────────────────────────────────────────────────────

/// One resolved add_clips placement: an owned snapshot of the asset to place +
/// where. Owned because the agent-swap closure only gets `&mut Timeline`, not the
/// asset library — the asset metadata must be captured before the swap.
struct AddSpec {
    asset: MediaAsset,
    track_index: Option<usize>,
    start_frame: i32,
    duration_frames: i32,
}

/// `add_clips` (`entries[{mediaRef, startFrame, durationFrames, trackIndex?}]`,
/// **all-or-none** `trackIndex`): place clips as ONE undo; video-with-audio
/// auto-creates a linked audio clip; same-track overlap → overwrite (via
/// `clear_region`). Reference `addClips`.
pub fn add_clips(state: &mut EditorState, args: &Value) -> ToolResult {
    // ---- parse + validate (read-only, before any mutation) ----
    let entries = match args.get("entries").and_then(Value::as_array) {
        Some(e) if !e.is_empty() => e,
        _ => return ToolResult::error("Missing or empty 'entries' array"),
    };

    let mut specs: Vec<AddSpec> = Vec::with_capacity(entries.len());
    for (idx, entry) in entries.iter().enumerate() {
        let media_ref = match entry.get("mediaRef").and_then(Value::as_str) {
            Some(s) => s,
            None => return ToolResult::error(format!("entries[{idx}]: missing 'mediaRef'")),
        };
        // Capture an owned copy of the asset (it must outlive the immutable borrow).
        let asset = match state.library.assets.iter().find(|a| a.id == media_ref) {
            Some(a) => a.clone(),
            None => return ToolResult::error(format!("Media asset not found: {media_ref}")),
        };
        let start_frame = match entry.get("startFrame").and_then(Value::as_i64) {
            Some(v) => v as i32,
            None => return ToolResult::error(format!("entries[{idx}]: missing 'startFrame'")),
        };
        let duration_frames = match entry.get("durationFrames").and_then(Value::as_i64) {
            Some(v) => v as i32,
            None => return ToolResult::error(format!("entries[{idx}]: missing 'durationFrames'")),
        };
        if duration_frames < 1 {
            return ToolResult::error(format!(
                "entries[{idx}]: durationFrames must be >= 1 (got {duration_frames})"
            ));
        }
        if start_frame < 0 {
            return ToolResult::error(format!(
                "entries[{idx}]: startFrame must be >= 0 (got {start_frame})"
            ));
        }
        let track_index = match entry.get("trackIndex").and_then(Value::as_i64) {
            Some(ti) => {
                let ti = ti as usize;
                let track = match state.library.timeline.tracks.get(ti) {
                    Some(t) => t,
                    None => {
                        return ToolResult::error(format!(
                            "entries[{idx}]: track index {ti} out of range (0..{})",
                            state.library.timeline.tracks.len().saturating_sub(1)
                        ))
                    }
                };
                if !asset.asset_type.is_compatible(track.track_type) {
                    return ToolResult::error(format!(
                        "entries[{idx}]: asset type {:?} is not compatible with the {:?} track at index {ti}",
                        asset.asset_type, track.track_type
                    ));
                }
                Some(ti)
            }
            None => None,
        };
        specs.push(AddSpec { asset, track_index, start_frame, duration_frames });
    }

    // All-or-none trackIndex (a new track at index 0 would shift explicit indices).
    let omitted = specs.iter().filter(|s| s.track_index.is_none()).count();
    if omitted != 0 && omitted != specs.len() {
        return ToolResult::error(format!(
            "Mixed trackIndex: {omitted} of {} entries omitted trackIndex. Either set it on \
             every entry or omit it on every entry (to auto-create shared tracks).",
            specs.len()
        ));
    }

    let action_name = if specs.len() == 1 { "Add Clip (Agent)" } else { "Add Clips (Agent)" };
    let count = specs.len();
    let all_omitted = omitted == count;
    agent_edit(state, action_name, move |timeline, _hist| {
        let mut created_tracks: Vec<String> = Vec::new();

        // Auto-create shared tracks when every entry omitted trackIndex: a video
        // track for any non-audio asset, an audio track for any audio asset.
        let mut shared_video: Option<usize> = None;
        let mut shared_audio: Option<usize> = None;
        if all_omitted {
            if specs.iter().any(|s| s.asset.asset_type != ClipType::Audio) {
                let i = insert_track(timeline, ClipType::Video, 0);
                created_tracks.push(format!("track {i} (video)"));
                shared_video = Some(i);
            }
            if specs.iter().any(|s| s.asset.asset_type == ClipType::Audio) {
                let i = insert_track(timeline, ClipType::Audio, 0);
                created_tracks.push(format!("track {i} (audio)"));
                shared_audio = Some(i);
            }
        }

        let mut summaries: Vec<String> = Vec::new();
        let mut total_ids = 0usize;
        for (i, spec) in specs.iter().enumerate() {
            let track_index = match spec.track_index {
                Some(ti) => ti,
                None => {
                    if spec.asset.asset_type == ClipType::Audio {
                        shared_audio.expect("audio track auto-created above")
                    } else {
                        shared_video.expect("video track auto-created above")
                    }
                }
            };
            // Overlap on the same track → clear the region first (overwrite).
            clear_region(
                timeline,
                track_index,
                spec.start_frame,
                spec.start_frame + spec.duration_frames,
            );
            let ids = place_clip(
                timeline,
                &spec.asset,
                track_index,
                spec.start_frame,
                spec.duration_frames,
            );
            let Some(primary) = ids.first() else {
                return Err(format!(
                    "entries[{i}]: failed to place clip on track {track_index} at frame {}",
                    spec.start_frame
                ));
            };
            total_ids += ids.len();
            let paired = if ids.len() > 1 { format!(" (+linked audio {})", ids[1]) } else { String::new() };
            summaries.push(format!(
                "{primary} on track {track_index} @ {} for {}{paired}",
                spec.start_frame, spec.duration_frames
            ));
        }
        let _ = total_ids;
        let prefix = if created_tracks.is_empty() {
            String::new()
        } else {
            format!("Created {}. ", created_tracks.join(", "))
        };
        Ok(ToolResult::ok(format!(
            "{prefix}Added {count} clip(s): {}",
            summaries.join("; ")
        )))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// remove_clips
// ─────────────────────────────────────────────────────────────────────────────

/// `remove_clips` (`clipIds[]`): removes the **whole link group** of any referenced
/// clip; prunes empty tracks. Reference `removeClips`.
pub fn remove_clips(state: &mut EditorState, args: &Value) -> ToolResult {
    let clip_ids: Vec<String> = match args.get("clipIds").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => {
            arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        }
        _ => return ToolResult::error("Missing or empty 'clipIds' array"),
    };
    // Validate existence first (no mutation on the error path).
    for id in &clip_ids {
        let found = state
            .library
            .timeline
            .tracks
            .iter()
            .any(|t| t.clips.iter().any(|c| &c.id == id));
        if !found {
            return ToolResult::error(format!("Clip not found: {id}"));
        }
    }

    let action_name = if clip_ids.len() == 1 { "Remove Clip" } else { "Remove Clips" };
    agent_edit(state, action_name, move |timeline, _hist| {
        let expanded = expand_to_link_group(timeline, &clip_ids);
        let tracks_before = timeline.tracks.len();
        for track in timeline.tracks.iter_mut() {
            track.clips.retain(|c| !expanded.iter().any(|id| id == &c.id));
        }
        // Prune tracks that became empty.
        timeline.tracks.retain(|t| !t.clips.is_empty());
        let pruned = tracks_before - timeline.tracks.len();

        let extras = expanded.len() - clip_ids.len();
        let linked_note = if extras > 0 { format!(" (+{extras} linked)") } else { String::new() };
        let prune_note = if pruned > 0 {
            format!(
                ". Pruned {pruned} empty track(s) — track indices have shifted; re-read with \
                 get_timeline before next index-based call"
            )
        } else {
            String::new()
        };
        Ok(ToolResult::ok(format!(
            "Removed {} clip(s){linked_note}{prune_note}: {}",
            expanded.len(),
            clip_ids.join(", ")
        )))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// remove_tracks
// ─────────────────────────────────────────────────────────────────────────────

/// `remove_tracks` (`trackIndexes[]`): removes whole tracks; remaining indices shift
/// down. Reference `removeTracks`.
pub fn remove_tracks(state: &mut EditorState, args: &Value) -> ToolResult {
    let raw = match args.get("trackIndexes").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => arr,
        _ => return ToolResult::error("remove_tracks: trackIndexes must be a non-empty array of integers"),
    };
    let mut indexes: Vec<usize> = Vec::new();
    for entry in raw {
        let i = match entry.as_i64() {
            Some(v) => v as usize,
            None => return ToolResult::error(format!("remove_tracks: trackIndexes must be integers (got {entry})")),
        };
        if indexes.contains(&i) {
            continue;
        }
        if state.library.timeline.tracks.get(i).is_none() {
            return ToolResult::error(format!(
                "remove_tracks: track index {i} out of range (timeline has {} tracks)",
                state.library.timeline.tracks.len()
            ));
        }
        indexes.push(i);
    }

    let action_name = if indexes.len() == 1 { "Remove Track" } else { "Remove Tracks" };
    agent_edit(state, action_name, move |timeline, _hist| {
        // Collect the ids to remove (indices change as we remove, so resolve ids first).
        let ids: Vec<String> = indexes
            .iter()
            .filter_map(|&i| timeline.tracks.get(i).map(|t| t.id.clone()))
            .collect();
        let removed = ids.len();
        timeline.tracks.retain(|t| !ids.contains(&t.id));
        Ok(ToolResult::ok(format!(
            "Removed {removed} track(s); remaining track indices have shifted — re-read with get_timeline."
        )))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// move_clips
// ─────────────────────────────────────────────────────────────────────────────

/// `move_clips` (`moves[{clipId, toTrack?, toFrame?}]`, ≥1): linked partners follow
/// the `startFrame` delta; track changes do not propagate to partners. Routes to
/// palmier-edit's atomic `move_clips`. Reference `moveClips`.
pub fn move_clips(state: &mut EditorState, args: &Value) -> ToolResult {
    let moves = match args.get("moves").and_then(Value::as_array) {
        Some(m) if !m.is_empty() => m,
        _ => return ToolResult::error("Missing or empty 'moves' array"),
    };

    struct ParsedMove {
        clip_id: String,
        to_track: Option<usize>,
        to_frame: Option<i32>,
    }
    let mut parsed: Vec<ParsedMove> = Vec::with_capacity(moves.len());
    for (idx, m) in moves.iter().enumerate() {
        let path = format!("moves[{idx}]");
        let clip_id = match m.get("clipId").and_then(Value::as_str) {
            Some(s) => s.to_string(),
            None => return ToolResult::error(format!("{path}: missing 'clipId'")),
        };
        let to_track = m.get("toTrack").and_then(Value::as_i64).map(|v| v as usize);
        let to_frame = m.get("toFrame").and_then(Value::as_i64).map(|v| v as i32);
        if to_track.is_none() && to_frame.is_none() {
            return ToolResult::error(format!("{path}: at least one of 'toTrack' or 'toFrame' is required"));
        }
        // Locate the clip + its current track/type for validation.
        let loc = state
            .library
            .timeline
            .tracks
            .iter()
            .enumerate()
            .find_map(|(ti, t)| t.clips.iter().find(|c| c.id == clip_id).map(|_| ti));
        let src_track = match loc {
            Some(ti) => ti,
            None => return ToolResult::error(format!("{path}: clip not found: {clip_id}")),
        };
        if let Some(ti) = to_track {
            let dest = match state.library.timeline.tracks.get(ti) {
                Some(t) => t,
                None => return ToolResult::error(format!(
                    "{path}: toTrack {ti} out of range (0..{})",
                    state.library.timeline.tracks.len().saturating_sub(1)
                )),
            };
            let src_type = state.library.timeline.tracks[src_track].track_type;
            if !dest.track_type.is_compatible(src_type) {
                return ToolResult::error(format!(
                    "{path}: toTrack {ti} ({:?}) is incompatible with the clip's {src_type:?} source track",
                    dest.track_type
                ));
            }
        }
        if let Some(f) = to_frame
            && f < 0
        {
            return ToolResult::error(format!("{path}: toFrame must be >= 0 (got {f})"));
        }
        parsed.push(ParsedMove { clip_id, to_track, to_frame });
    }

    let action_name = if parsed.len() == 1 { "Move Clip (Agent)" } else { "Move Clips (Agent)" };
    let primary_count = parsed.len();
    agent_edit(state, action_name, move |timeline, hist| {
        // Build resolved MoveSpecs, expanding linked partners by the startFrame delta.
        let mut specs: Vec<MoveSpec> = Vec::new();
        let mut seen: std::collections::HashSet<String> =
            parsed.iter().map(|p| p.clip_id.clone()).collect();
        for p in &parsed {
            let loc = timeline
                .tracks
                .iter()
                .enumerate()
                .find_map(|(ti, t)| t.clips.iter().find(|c| c.id == p.clip_id).map(|c| (ti, c.start_frame, c.link_group_id.clone())));
            let Some((cur_track, cur_frame, link_group)) = loc else { continue };
            let to_track = p.to_track.unwrap_or(cur_track);
            let to_frame = p.to_frame.unwrap_or(cur_frame);
            let delta = to_frame - cur_frame;
            specs.push(MoveSpec { clip_id: p.clip_id.clone(), to_track, to_frame });
            // Linked partners follow only the frame delta (not track changes).
            if let Some(gid) = link_group
                && delta != 0
            {
                let partner_ids: Vec<(String, usize, i32)> = timeline
                    .tracks
                    .iter()
                    .enumerate()
                    .flat_map(|(ti, t)| {
                        t.clips
                            .iter()
                            .filter(|c| c.link_group_id.as_deref() == Some(gid.as_str()))
                            .map(move |c| (c.id.clone(), ti, c.start_frame))
                    })
                    .collect();
                for (pid, pti, pframe) in partner_ids {
                    if seen.insert(pid.clone()) {
                        specs.push(MoveSpec { clip_id: pid, to_track: pti, to_frame: pframe + delta });
                    }
                }
            }
        }
        let linked = specs.len() - primary_count;
        let mut mint = new_uuid;
        edit_move_clips(timeline, hist, &specs, &mut mint);
        let linked_note = if linked > 0 { format!(" (+{linked} linked)") } else { String::new() };
        Ok(ToolResult::ok(format!("Moved {primary_count} clip(s){linked_note}")))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// split_clip
// ─────────────────────────────────────────────────────────────────────────────

/// `split_clip` (`clipId`, `atFrame` strictly between start/end): splits the clip
/// (and its whole link group) at `atFrame`, migrating keyframes into the new clip.
/// Routes to palmier-edit's `split_at`. Reference `splitClip`.
pub fn split_clip(state: &mut EditorState, args: &Value) -> ToolResult {
    let clip_id = match args.get("clipId").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return ToolResult::error("Missing required argument: clipId"),
    };
    let at_frame = match args.get("atFrame").and_then(Value::as_i64) {
        Some(v) => v as i32,
        None => return ToolResult::error("Missing required argument: atFrame"),
    };
    // Locate + range-check (no mutation on the error path).
    let clip = state
        .library
        .timeline
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find(|c| c.id == clip_id)
        .cloned();
    let Some(clip) = clip else {
        return ToolResult::error(format!("Clip not found: {clip_id}"));
    };
    if !(at_frame > clip.start_frame && at_frame < clip.end_frame()) {
        return ToolResult::error(format!(
            "Frame {at_frame} is outside clip range ({}..{})",
            clip.start_frame,
            clip.end_frame()
        ));
    }
    let start = clip.start_frame;
    let end = clip.end_frame();

    agent_edit(state, "Split Clip (Agent)", move |timeline, hist| {
        let mut mint = new_uuid;
        let right_ids = split_at(timeline, hist, &clip_id, at_frame, &mut mint);
        let right_note = if right_ids.is_empty() {
            String::new()
        } else {
            format!(" → new right clip(s): {}", right_ids.join(", "))
        };
        Ok(ToolResult::ok(format!(
            "Split clip {clip_id} at frame {at_frame}. Left: {clip_id} (frames {start}..{at_frame}){right_note} (right ends at {end})"
        )))
    })
}
