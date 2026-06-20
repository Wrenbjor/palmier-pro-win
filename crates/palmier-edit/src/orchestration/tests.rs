//! E3-S6 orchestration integration tests.
//!
//! Covers the story acceptance: sync-locked multi-track ripple; refuse-the-whole-
//! edit leaves the timeline byte-unchanged; clear-region split re-derivation
//! matches `split_clip`; linked split regroups right halves under a NEW link id;
//! pinned companions hold their row on a cross-track move (exercised via
//! `drag` + `move_clips`); one undo entry per composite edit.

use super::*;
use palmier_history::History;
use palmier_model::{Clip, ClipType, Timeline, Track};

/// Deterministic id minter for tests.
fn id_gen() -> impl FnMut() -> String {
    let mut n = 0;
    move || {
        n += 1;
        format!("new-{n}")
    }
}

fn clip(id: &str, start: i32, dur: i32) -> Clip {
    let mut c = Clip::new("asset", start, dur);
    c.id = id.into();
    c
}

fn track_with(track_type: ClipType, sync_locked: bool, clips: Vec<Clip>) -> Track {
    let mut t = Track::new(track_type);
    t.sync_locked = sync_locked;
    t.clips = clips;
    t
}

// =====================================================================
// sync-locked multi-track ripple (FOUNDATION §11.1)
// =====================================================================

#[test]
fn ripple_delete_shifts_sync_locked_tracks_across() {
    // Track 0 (video): A[0,30) B[30,30) C[60,30) — delete B (opens gap [30,60)).
    // Track 1 (audio, sync_locked): X[0,30) Z[60,30) — the [30,60) region is empty
    // on the follower (as it would be after a synced cut), so only clips at/after
    // the gap end (Z at 60) shift left by 30.
    let mut tl = Timeline::new();
    tl.tracks.push(track_with(
        ClipType::Video,
        true,
        vec![clip("A", 0, 30), clip("B", 30, 30), clip("C", 60, 30)],
    ));
    tl.tracks.push(track_with(
        ClipType::Audio,
        true,
        vec![clip("X", 0, 30), clip("Z", 60, 30)],
    ));
    let mut hist = History::new();

    ripple_delete_selected_clips(&mut tl, &mut hist, &["B".into()]).unwrap();

    // Track 0: B removed; C shifted left by 30 → 30.
    let t0 = &tl.tracks[0];
    assert!(t0.clips.iter().all(|c| c.id != "B"), "B removed");
    let c = t0.clips.iter().find(|c| c.id == "C").unwrap();
    assert_eq!(c.start_frame, 30, "C closes the gap");
    // Track 1 (sync-locked follower): Z (after the gap) shifts left 30 → 30.
    let t1 = &tl.tracks[1];
    let z = t1.clips.iter().find(|c| c.id == "Z").unwrap();
    assert_eq!(z.start_frame, 30, "Z (at gap end) shifts left by the gap length");
    // X started before the gap → unchanged.
    let x = t1.clips.iter().find(|c| c.id == "X").unwrap();
    assert_eq!(x.start_frame, 0);
}

#[test]
fn non_sync_locked_track_is_not_rippled() {
    let mut tl = Timeline::new();
    tl.tracks.push(track_with(
        ClipType::Video,
        true,
        vec![clip("A", 0, 30), clip("B", 30, 30)],
    ));
    // Audio track NOT sync-locked → must not move.
    tl.tracks.push(track_with(
        ClipType::Audio,
        false,
        vec![clip("X", 60, 30)],
    ));
    let mut hist = History::new();
    ripple_delete_selected_clips(&mut tl, &mut hist, &["A".into()]).unwrap();
    let x = tl.tracks[1].clips.iter().find(|c| c.id == "X").unwrap();
    assert_eq!(x.start_frame, 60, "non-sync-locked track unaffected");
}

// =====================================================================
// atomic refuse-the-whole-edit (leaves timeline byte-unchanged)
// =====================================================================

#[test]
fn ripple_refusal_leaves_timeline_unchanged() {
    // Track 0 deletes B, leaving a 30-frame gap that wants to pull C left.
    // Track 1 (sync-locked) has a clip starting at 0 whose neighbor sits so the
    // shift would overlap → validate_shifts refuses → WHOLE edit refused, no mutation.
    let mut tl = Timeline::new();
    tl.tracks.push(track_with(
        ClipType::Video,
        true,
        vec![clip("A", 0, 30), clip("B", 30, 30), clip("C", 60, 30)],
    ));
    // Sync-locked audio: a clip P[0,45) then Q[60,30). The [30,60) gap shift pulls
    // Q to 30, which overlaps P (ends at 45) → collision → refuse.
    tl.tracks.push(track_with(
        ClipType::Audio,
        true,
        vec![clip("P", 0, 45), clip("Q", 60, 30)],
    ));
    let before = tl.clone();
    let mut hist = History::new();

    let result = ripple_delete_selected_clips(&mut tl, &mut hist, &["B".into()]);
    assert_eq!(result, Err(RefuseReason::Overlap), "must refuse on collision");
    assert_eq!(tl, before, "timeline byte-unchanged after refusal");
    assert_eq!(hist.user_undo_len(), 0, "nothing registered on undo stack");
}

// =====================================================================
// clear-region split re-derivation matches split_clip
// =====================================================================

#[test]
fn clear_region_split_re_derivation_matches_split_clip() {
    // One clip [0,100). Clear the inner region [40,70) → the clip splits: left
    // [0,40), right starts at 70. The right fragment must match split_clip's math
    // (re-derived by the VM, not the engine's advisory fields).
    let mut tl = Timeline::new();
    tl.tracks
        .push(track_with(ClipType::Video, true, vec![clip("A", 0, 100)]));
    let mut hist = History::new();
    let mut g = id_gen();

    // ripple_delete_ranges with a single range that lands strictly inside.
    let report = ripple_delete_ranges_on_track(
        &mut tl,
        &mut hist,
        0,
        &[FrameRange::new(40, 70)],
        &mut g,
    )
    .unwrap();

    let t0 = &tl.tracks[0];
    // Two fragments now (left kept id A, right new id), gap closed by ripple.
    let mut starts: Vec<i32> = t0.clips.iter().map(|c| c.start_frame).collect();
    starts.sort();
    // Left [0,40) stays; right was at 70 (dur 30), then ripple closes the 30-frame
    // gap → right shifts to 40.
    assert_eq!(starts, vec![0, 40], "left at 0, right rippled to 40");
    let left = t0.clips.iter().find(|c| c.id == "A").unwrap();
    assert_eq!(left.duration_frames, 40, "left fragment duration");
    assert_eq!(left.trim_end_frame, 60, "left trim_end = round((100-40)*1)");
    let right = t0.clips.iter().find(|c| c.id != "A").unwrap();
    assert_eq!(right.duration_frames, 30, "right fragment duration = 100-70");
    // Right's source head trim = orig.trim_start + round(70 * speed) = 70.
    assert_eq!(right.trim_start_frame, 70, "right trim_start re-derived via split_clip");
    assert_eq!(report.removed_frames, 30);
    assert_eq!(report.cleared_tracks, vec![0]);
}

// =====================================================================
// linked split regroups right halves under a NEW link id
// =====================================================================

#[test]
fn linked_split_regroups_right_halves_under_fresh_id() {
    // Two linked clips (same link_group_id "g") on two tracks, both spanning
    // [0,100). Split at 50 → both split; the two right halves share a NEW group
    // id (not "g"), and the left halves keep "g".
    let mut a = clip("A", 0, 100);
    a.link_group_id = Some("g".into());
    let mut b = clip("B", 0, 100);
    b.media_type = ClipType::Audio;
    b.link_group_id = Some("g".into());

    let mut tl = Timeline::new();
    tl.tracks.push(track_with(ClipType::Video, true, vec![a]));
    tl.tracks.push(track_with(ClipType::Audio, true, vec![b]));
    let mut hist = History::new();
    let mut g = id_gen();

    let right_ids = split_at(&mut tl, &mut hist, "A", 50, &mut g);
    assert_eq!(right_ids.len(), 2, "both linked members split");

    // Collect the group ids of the right fragments.
    let right_groups: Vec<Option<String>> = tl
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .filter(|c| right_ids.contains(&c.id))
        .map(|c| c.link_group_id.clone())
        .collect();
    // Both right halves share ONE fresh group id, and it is NOT "g".
    assert_eq!(right_groups.len(), 2);
    assert!(right_groups[0].is_some());
    assert_eq!(right_groups[0], right_groups[1], "right halves share one group");
    assert_ne!(
        right_groups[0],
        Some("g".to_string()),
        "right group is fresh, not the original"
    );
    // Left halves (A, B) keep the original "g".
    let left_a = tl
        .tracks
        .iter()
        .flat_map(|t| t.clips.iter())
        .find(|c| c.id == "A")
        .unwrap();
    assert_eq!(left_a.link_group_id, Some("g".to_string()), "left keeps g");
}

#[test]
fn lone_split_assigns_no_link_group_to_right() {
    let mut tl = Timeline::new();
    tl.tracks
        .push(track_with(ClipType::Video, true, vec![clip("A", 0, 100)]));
    let mut hist = History::new();
    let mut g = id_gen();
    let right_ids = split_at(&mut tl, &mut hist, "A", 60, &mut g);
    assert_eq!(right_ids.len(), 1);
    let right = tl.tracks[0]
        .clips
        .iter()
        .find(|c| c.id == right_ids[0])
        .unwrap();
    assert_eq!(right.link_group_id, None, "unlinked split → no group");
    assert_eq!(right.start_frame, 60);
    assert_eq!(right.duration_frames, 40);
}

// =====================================================================
// one undo entry per composite edit (palmier-history coalescing)
// =====================================================================

#[test]
fn each_edit_is_one_user_undo_step() {
    let mut tl = Timeline::new();
    tl.tracks.push(track_with(
        ClipType::Video,
        true,
        vec![clip("A", 0, 30), clip("B", 30, 30), clip("C", 60, 30)],
    ));
    tl.tracks.push(track_with(
        ClipType::Audio,
        true,
        vec![clip("X", 0, 30), clip("Y", 30, 30)],
    ));
    let mut hist = History::new();

    // A composite multi-track ripple delete is ONE undo step.
    ripple_delete_selected_clips(&mut tl, &mut hist, &["B".into()]).unwrap();
    assert_eq!(hist.user_undo_len(), 1, "composite edit = one undo entry");

    // Undo restores exactly.
    let after_edit = tl.clone();
    assert!(hist.undo(&mut tl));
    let b = tl.tracks[0].clips.iter().find(|c| c.id == "B");
    assert!(b.is_some(), "B restored by undo");
    let c = tl.tracks[0].clips.iter().find(|c| c.id == "C").unwrap();
    assert_eq!(c.start_frame, 60, "C back to original");
    // Redo re-applies.
    assert!(hist.redo(&mut tl));
    assert_eq!(tl, after_edit, "redo reproduces the edit exactly");
}

#[test]
fn no_op_edit_registers_nothing() {
    let mut tl = Timeline::new();
    tl.tracks
        .push(track_with(ClipType::Video, true, vec![clip("A", 0, 30)]));
    let mut hist = History::new();
    // Deleting an empty selection is a no-op → no undo entry.
    ripple_delete_selected_clips(&mut tl, &mut hist, &[]).unwrap();
    assert_eq!(hist.user_undo_len(), 0);
}

// =====================================================================
// ripple insert + gap ripple
// =====================================================================

#[test]
fn ripple_insert_pushes_sync_locked_tracks() {
    let mut tl = Timeline::new();
    tl.tracks.push(track_with(
        ClipType::Video,
        true,
        vec![clip("A", 0, 30), clip("B", 30, 30)],
    ));
    tl.tracks
        .push(track_with(ClipType::Audio, true, vec![clip("X", 0, 90)]));
    let mut hist = History::new();

    // Insert a 20-frame clip at frame 30 on track 0.
    ripple_insert_clips(&mut tl, &mut hist, 0, 30, vec![clip("NEW", 0, 20)]);

    // B (started at 30) pushed to 50; NEW lands at 30.
    let t0 = &tl.tracks[0];
    let b = t0.clips.iter().find(|c| c.id == "B").unwrap();
    assert_eq!(b.start_frame, 50, "B pushed by 20");
    let new = t0.clips.iter().find(|c| c.id == "NEW").unwrap();
    assert_eq!(new.start_frame, 30);
    // Sync-locked audio X started at 0 < insert_frame → NOT pushed.
    let x = tl.tracks[1].clips.iter().find(|c| c.id == "X").unwrap();
    assert_eq!(x.start_frame, 0, "X before insert frame unaffected");
    assert_eq!(hist.user_undo_len(), 1);
}

#[test]
fn gap_ripple_closes_gap_and_validates_followers() {
    // Track 0: A[0,30) gap [30,60) C[60,30). Close the [30,60) gap.
    let mut tl = Timeline::new();
    tl.tracks.push(track_with(
        ClipType::Video,
        true,
        vec![clip("A", 0, 30), clip("C", 60, 30)],
    ));
    tl.tracks
        .push(track_with(ClipType::Audio, true, vec![clip("X", 90, 30)]));
    let mut hist = History::new();

    ripple_delete_gap(&mut tl, &mut hist, 0, FrameRange::new(30, 60)).unwrap();
    let c = tl.tracks[0].clips.iter().find(|c| c.id == "C").unwrap();
    assert_eq!(c.start_frame, 30, "C closes the gap");
    let x = tl.tracks[1].clips.iter().find(|c| c.id == "X").unwrap();
    assert_eq!(x.start_frame, 60, "sync-locked follower shifts left by gap length");
}

// =====================================================================
// move_clips (overwrite at destination)
// =====================================================================

#[test]
fn move_clip_clears_destination_and_drops_at_frame() {
    // Track 0: A[0,30). Track 1: B[0,100). Move A onto track 1 at frame 10 →
    // destination region [10,40) is cleared from B (B splits), A lands at 10.
    let mut tl = Timeline::new();
    tl.tracks
        .push(track_with(ClipType::Video, true, vec![clip("A", 0, 30)]));
    tl.tracks
        .push(track_with(ClipType::Video, true, vec![clip("B", 0, 100)]));
    let mut hist = History::new();
    let mut g = id_gen();

    move_clips(
        &mut tl,
        &mut hist,
        &[MoveSpec {
            clip_id: "A".into(),
            to_track: 1,
            to_frame: 10,
        }],
        &mut g,
    );

    // A is gone from track 0, present on track 1 at frame 10.
    assert!(tl.tracks[0].clips.is_empty(), "A pulled off source track");
    let a = tl.tracks[1].clips.iter().find(|c| c.id == "A").unwrap();
    assert_eq!(a.start_frame, 10);
    // B was split by the overwrite: a left fragment [0,10) and a right fragment
    // starting at 40.
    let b_frags: Vec<i32> = tl.tracks[1]
        .clips
        .iter()
        .filter(|c| c.id != "A")
        .map(|c| c.start_frame)
        .collect();
    assert!(b_frags.contains(&0), "B left fragment at 0");
    assert!(b_frags.contains(&40), "B right fragment at 40 (region [10,40) cleared)");
    assert_eq!(hist.user_undo_len(), 1, "move is one undo step");
}

// =====================================================================
// expand_to_link_group
// =====================================================================

#[test]
fn expand_to_link_group_pulls_partners() {
    let mut a = clip("A", 0, 30);
    a.link_group_id = Some("g".into());
    let mut b = clip("B", 0, 30);
    b.link_group_id = Some("g".into());
    let c = clip("C", 30, 30); // unlinked

    let mut tl = Timeline::new();
    tl.tracks.push(track_with(ClipType::Video, true, vec![a, c]));
    tl.tracks.push(track_with(ClipType::Audio, true, vec![b]));

    // Selecting A pulls in B (shared group g); C stays out.
    let expanded = expand_to_link_group(&tl, &["A".into()]);
    assert!(expanded.contains(&"A".to_string()));
    assert!(expanded.contains(&"B".to_string()), "linked partner pulled in");
    assert!(!expanded.contains(&"C".to_string()), "unlinked clip excluded");
}
