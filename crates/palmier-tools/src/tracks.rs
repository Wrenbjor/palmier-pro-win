//! Track-level state toggles — mute / hide / sync-lock (UI-only, not an MCP tool).
//!
//! The reference exposes these as **view-model actions** (`EditorViewModel+Tracks`'s
//! `toggleTrackMute` / `toggleTrackHidden` / `toggleTrackSyncLock`), each a single
//! undoable `Bool` flip on a track — they are **not** among the 30 MCP tools
//! (`ToolDefinitions.swift` has no `set_track_properties`). So, exactly like
//! [`crate::library::relink_media`] and [`crate::library::move_folders`], this lives
//! behind a dedicated `editor_set_track_properties` command rather than the 30-tool
//! dispatch — keeping the parity-locked tool count at 30 (SM-C2) while still giving
//! the UI an undoable, shared-executor toggle.
//!
//! Only the fields the caller provides change (a `None` leaves the flag untouched),
//! and the whole change is **one** agent-undo step over the timeline snapshot — the
//! same [`crate::undo::agent_edit`] seam the clip tools use, so an `undo` reverses it.

use palmier_model::Track;

use crate::editor::EditorState;
use crate::result::ToolResult;
use crate::undo::agent_edit;

/// Apply mute / hide / sync-lock state to one track as a single undoable edit.
///
/// `track_id` is the track's UUID (from `get_timeline`). Each of `muted` / `hidden` /
/// `locked` is optional: only the provided flags change; omitted ones keep their
/// current value. Returns an error (and registers no undo step) when the track id is
/// unknown or no flag was supplied. A toggle that results in no actual change is a
/// no-op (the `agent_edit` change-guard registers nothing — reference
/// `editor.timeline != before`).
pub fn set_track_properties(
    state: &mut EditorState,
    track_id: &str,
    muted: Option<bool>,
    hidden: Option<bool>,
    locked: Option<bool>,
) -> ToolResult {
    if muted.is_none() && hidden.is_none() && locked.is_none() {
        return ToolResult::error(
            "set_track_properties needs at least one of muted, hidden, locked",
        );
    }
    if !state.library.timeline.tracks.iter().any(|t| t.id == track_id) {
        return ToolResult::error(format!("Track not found: {track_id}"));
    }

    let track_id = track_id.to_string();
    // Action name mirrors the reference toggle labels; the dominant single-flag case
    // names that flip, else a generic "Set Track Properties".
    let action = match (muted, hidden, locked) {
        (Some(m), None, None) => {
            if m { "Mute Track" } else { "Unmute Track" }
        }
        (None, Some(h), None) => {
            if h { "Hide Track" } else { "Show Track" }
        }
        (None, None, Some(l)) => {
            if l { "Lock Track" } else { "Unlock Track" }
        }
        _ => "Set Track Properties",
    };

    agent_edit(state, action, move |timeline, _hist| {
        let Some(track): Option<&mut Track> =
            timeline.tracks.iter_mut().find(|t| t.id == track_id)
        else {
            // Re-checked above; defensive only.
            return Err(format!("Track not found: {track_id}"));
        };
        let mut changed: Vec<String> = Vec::new();
        if let Some(m) = muted {
            if track.muted != m {
                track.muted = m;
                changed.push(format!("muted={m}"));
            }
        }
        if let Some(h) = hidden {
            if track.hidden != h {
                track.hidden = h;
                changed.push(format!("hidden={h}"));
            }
        }
        if let Some(l) = locked {
            if track.sync_locked != l {
                track.sync_locked = l;
                changed.push(format!("locked={l}"));
            }
        }
        let summary = if changed.is_empty() {
            format!("Track {track_id} (no-op)")
        } else {
            format!("Track {track_id}: {}", changed.join(", "))
        };
        Ok(ToolResult::ok(summary))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::{ClipType, MediaLibrary, Track};

    fn state_with_track(id: &str, ty: ClipType) -> EditorState {
        let mut lib = MediaLibrary::new();
        let mut track = Track::new(ty);
        track.id = id.to_string();
        lib.timeline.tracks.push(track);
        EditorState::with_library(lib)
    }

    fn text_of(r: &ToolResult) -> String {
        match &r.content[0] {
            crate::result::Block::Text(s) => s.clone(),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn set_track_properties_sets_muted_and_records_undo() {
        let mut state = state_with_track("t1", ClipType::Audio);
        let r = set_track_properties(&mut state, "t1", Some(true), None, None);
        assert!(!r.is_error, "{}", text_of(&r));
        assert!(state.library.timeline.tracks[0].muted);
        // exactly one agent-undo step recorded.
        assert_eq!(state.history.agent_undo_len(), 1);
        assert!(text_of(&r).contains("muted=true"));
    }

    #[test]
    fn set_track_properties_sets_hidden() {
        let mut state = state_with_track("t1", ClipType::Video);
        let r = set_track_properties(&mut state, "t1", None, Some(true), None);
        assert!(!r.is_error);
        assert!(state.library.timeline.tracks[0].hidden);
        assert_eq!(state.history.agent_undo_len(), 1);
    }

    #[test]
    fn set_track_properties_sets_locked() {
        let mut state = state_with_track("t1", ClipType::Video);
        // tracks default sync_locked = true; turn it OFF so a change is registered.
        let r = set_track_properties(&mut state, "t1", None, None, Some(false));
        assert!(!r.is_error);
        assert!(!state.library.timeline.tracks[0].sync_locked);
        assert_eq!(state.history.agent_undo_len(), 1);
    }

    #[test]
    fn set_track_properties_only_provided_fields_change() {
        let mut state = state_with_track("t1", ClipType::Audio);
        // start: muted=false, hidden=false, sync_locked=true
        let r = set_track_properties(&mut state, "t1", Some(true), None, None);
        assert!(!r.is_error);
        let t = &state.library.timeline.tracks[0];
        assert!(t.muted);
        assert!(!t.hidden, "hidden untouched");
        assert!(t.sync_locked, "lock untouched");
    }

    #[test]
    fn set_track_properties_no_op_change_records_no_undo() {
        let mut state = state_with_track("t1", ClipType::Audio);
        // muted already false → setting muted=false is a no-op.
        let r = set_track_properties(&mut state, "t1", Some(false), None, None);
        assert!(!r.is_error, "{}", text_of(&r));
        assert_eq!(state.history.agent_undo_len(), 0, "no change → no undo step");
        assert!(text_of(&r).contains("no-op"));
    }

    #[test]
    fn set_track_properties_unknown_track_errors() {
        let mut state = state_with_track("t1", ClipType::Video);
        let r = set_track_properties(&mut state, "nope", Some(true), None, None);
        assert!(r.is_error);
        assert!(text_of(&r).contains("Track not found"));
        assert_eq!(state.history.agent_undo_len(), 0);
    }

    #[test]
    fn set_track_properties_no_fields_errors() {
        let mut state = state_with_track("t1", ClipType::Video);
        let r = set_track_properties(&mut state, "t1", None, None, None);
        assert!(r.is_error);
        assert!(text_of(&r).contains("at least one"));
    }

    #[test]
    fn set_track_properties_undo_reverses_the_flip() {
        let mut state = state_with_track("t1", ClipType::Audio);
        set_track_properties(&mut state, "t1", Some(true), Some(true), None);
        assert!(state.library.timeline.tracks[0].muted);
        assert!(state.library.timeline.tracks[0].hidden);
        // undo reverses the single combined step.
        let u = crate::undo::undo(&mut state);
        assert!(!u.is_error, "{}", text_of(&u));
        assert!(!state.library.timeline.tracks[0].muted, "muted restored");
        assert!(!state.library.timeline.tracks[0].hidden, "hidden restored");
        assert_eq!(state.history.agent_undo_len(), 0);
    }
}
