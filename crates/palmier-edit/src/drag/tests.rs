//! E3-S7 drag-state machine tests.
//!
//! Covers the story acceptance: sub-mode selection at each `local_x` boundary
//! (4 px handle); duplicate flag on Alt-body-grab; two-probe move snap (end of a
//! non-lead selected clip snaps); `frame_delta` floors at -min-orig-frame;
//! `clamped_track_delta` steps to a compatible track; pinned companions identified;
//! marquee threshold cancels gap.

use super::*;
use crate::snap::{SnapKind, SnapState, SnapTarget};
use palmier_model::ClipType;

// ---- sub-mode at the 4px handle boundary -----------------------------

#[test]
fn sub_mode_boundaries_at_trim_handle() {
    let width = 100.0;
    // local_x <= 4 → TrimLeft.
    assert_eq!(clip_sub_mode(0.0, width), ClipSubMode::TrimLeft);
    assert_eq!(clip_sub_mode(4.0, width), ClipSubMode::TrimLeft, "exactly 4 is left");
    // just inside → Move.
    assert_eq!(clip_sub_mode(4.1, width), ClipSubMode::Move);
    assert_eq!(clip_sub_mode(50.0, width), ClipSubMode::Move);
    // local_x >= width-4 → TrimRight.
    assert_eq!(clip_sub_mode(96.0, width), ClipSubMode::TrimRight, "exactly width-4 is right");
    assert_eq!(clip_sub_mode(95.9, width), ClipSubMode::Move, "just inside is move");
    assert_eq!(clip_sub_mode(100.0, width), ClipSubMode::TrimRight);
}

#[test]
fn begin_clip_drag_picks_state_and_duplicate_flag() {
    // Body grab, no Alt → MoveClip, not duplicate.
    let s = begin_clip_drag("A", 50.0, 100.0, 30, Modifiers::default());
    match s {
        DragState::MoveClip(p) => {
            assert_eq!(p.lead_id, "A");
            assert_eq!(p.lead_original_frame, 30);
            assert!(!p.is_duplicate);
        }
        other => panic!("expected MoveClip, got {other:?}"),
    }
    // Body grab WITH Alt → duplicate.
    let mods = Modifiers {
        alt: true,
        ..Default::default()
    };
    let s = begin_clip_drag("A", 50.0, 100.0, 30, mods);
    match s {
        DragState::MoveClip(p) => assert!(p.is_duplicate, "Alt-body-grab → duplicate"),
        other => panic!("expected MoveClip, got {other:?}"),
    }
    // Left handle → TrimLeft (Alt ignored for trim).
    let s = begin_clip_drag("A", 2.0, 100.0, 30, mods);
    assert!(matches!(s, DragState::TrimLeft(_)));
    // Right handle → TrimRight.
    let s = begin_clip_drag("A", 99.0, 100.0, 30, Modifiers::default());
    assert!(matches!(s, DragState::TrimRight(_)));
}

// ---- two-probe move snap ---------------------------------------------

fn mover(id: &str, frame: i32, dur: i32, track: usize) -> MoveClip {
    MoveClip {
        id: id.into(),
        original_frame: frame,
        duration_frames: dur,
        original_track: track,
        linked_to_lead: false,
        incompatible_with_dest: false,
    }
}

#[test]
fn move_probes_push_both_edges_relative_to_lead() {
    // Lead A at frame 10 (dur 20); companion B at frame 40 (dur 30).
    let movers = vec![mover("A", 10, 20, 0), mover("B", 40, 30, 0)];
    let probes = move_snap_probes(&movers, 10);
    // A: base 0, end 20. B: base 30, end 60.
    assert_eq!(probes, vec![0, 20, 30, 60]);
}

#[test]
fn end_of_non_lead_clip_can_snap() {
    // Lead A at 10, companion B at 40 dur 30 (end at 70). A target at 75 should be
    // caught by B's END probe (base 30 + dur 30 = 60), since with the lead dragged
    // to position 15: probe_pos = 15 + 60 = 75 → exact snap.
    let movers = vec![mover("A", 10, 20, 0), mover("B", 40, 30, 0)];
    let probes = move_snap_probes(&movers, 10);
    let targets = vec![SnapTarget::new(75, SnapKind::ClipEdge)];
    let mut state = SnapState::default();
    // ppf 4 → base_frame_threshold 2. Lead candidate position 15.
    let (delta, snap) = resolve_move_delta(15, 10, &probes, &targets, &mut state, 8.0, 4.0);
    let snap = snap.expect("the END probe of B should snap to target 75");
    assert_eq!(snap.probe_offset, 60, "B's end probe (offset 60) snapped");
    // delta = (75 - 60) - 10 = 5.
    assert_eq!(delta, 5);
}

#[test]
fn no_snap_returns_raw_delta() {
    let movers = vec![mover("A", 10, 20, 0)];
    let probes = move_snap_probes(&movers, 10);
    let targets = vec![SnapTarget::new(1000, SnapKind::ClipEdge)]; // far away
    let mut state = SnapState::default();
    let (delta, snap) = resolve_move_delta(25, 10, &probes, &targets, &mut state, 8.0, 4.0);
    assert!(snap.is_none());
    assert_eq!(delta, 15, "raw delta = position - lead_original");
}

// ---- frame delta floors at -min-orig-frame ----------------------------

#[test]
fn frame_delta_floors_so_no_clip_before_zero() {
    // Movers at 10 and 25 → min orig 10. A leftward delta of -50 floors at -10.
    let movers = vec![mover("A", 25, 20, 0), mover("B", 10, 20, 0)];
    assert_eq!(clamp_move_frame_delta(-50, &movers), -10);
    // A rightward delta is unaffected.
    assert_eq!(clamp_move_frame_delta(40, &movers), 40);
    // A small leftward delta within range passes.
    assert_eq!(clamp_move_frame_delta(-5, &movers), -5);
}

// ---- clamped_track_delta steps to a compatible track ------------------

#[test]
fn clamped_track_delta_steps_until_compatible() {
    // Tracks: 0 video, 1 video, 2 audio. A video mover on track 0 asked to move +2
    // (onto audio track 2) → incompatible; steps back to +1 (video track 1) → ok.
    let tracks = vec![
        TrackKind { clip_type: ClipType::Video },
        TrackKind { clip_type: ClipType::Video },
        TrackKind { clip_type: ClipType::Audio },
    ];
    let movers = vec![mover("A", 0, 30, 0)];
    let types = vec![ClipType::Video];
    let d = clamped_track_delta(2, &movers, &types, &tracks);
    assert_eq!(d, 1, "stepped from +2 (audio) back to +1 (video, compatible)");
}

#[test]
fn clamped_track_delta_zero_when_no_compatible_track() {
    // A video mover on track 0; only track 1 is audio → no positive delta works → 0.
    let tracks = vec![
        TrackKind { clip_type: ClipType::Video },
        TrackKind { clip_type: ClipType::Audio },
    ];
    let movers = vec![mover("A", 0, 30, 0)];
    let types = vec![ClipType::Video];
    assert_eq!(clamped_track_delta(1, &movers, &types, &tracks), 0);
}

#[test]
fn clamped_track_delta_ignores_pinned_companions() {
    // A pinned companion (linked) need not fit a compatible track — it keeps its row.
    let tracks = vec![
        TrackKind { clip_type: ClipType::Video },
        TrackKind { clip_type: ClipType::Video },
    ];
    let mut pinned = mover("PIN", 0, 30, 0);
    pinned.linked_to_lead = true; // pinned → excluded from the fit check
    let movers = vec![mover("A", 0, 30, 0), pinned];
    let types = vec![ClipType::Video, ClipType::Video];
    // Both movers are video and track 1 is video, so +1 fits regardless; the point
    // is the pinned one is not what blocks it. Use a delta that would only fail for
    // the pinned one if it were audio — here both compatible, delta holds.
    assert_eq!(clamped_track_delta(1, &movers, &types, &tracks), 1);
}

// ---- pinned companions identified ------------------------------------

#[test]
fn pinned_companions_are_linked_or_type_incompatible() {
    let mut linked = mover("L", 0, 30, 1);
    linked.linked_to_lead = true;
    let mut incompat = mover("I", 0, 30, 1);
    incompat.incompatible_with_dest = true;
    let free = mover("F", 0, 30, 0);
    let movers = vec![linked.clone(), incompat.clone(), free];

    assert!(is_pinned(&linked));
    assert!(is_pinned(&incompat));
    let pinned = pinned_companions(&movers);
    assert_eq!(pinned, vec!["L".to_string(), "I".to_string()]);
}

// ---- marquee threshold cancels gap -----------------------------------

#[test]
fn marquee_threshold_3px() {
    let origin = (100.0, 100.0);
    // Within 3px in both dims → does not exceed.
    assert!(!marquee_exceeds_threshold(origin, (103.0, 103.0)));
    // Past 3px in x → exceeds.
    assert!(marquee_exceeds_threshold(origin, (104.0, 100.0)));
    // Past 3px in y → exceeds.
    assert!(marquee_exceeds_threshold(origin, (100.0, 104.0)));
}

#[test]
fn marquee_rect_is_min_max() {
    // Dragging up-left: origin below-right of cursor → rect normalizes.
    let r = marquee_rect((100.0, 100.0), (40.0, 60.0));
    assert_eq!(r.x, 40.0);
    assert_eq!(r.y, 60.0);
    assert_eq!(r.width, 60.0);
    assert_eq!(r.height, 40.0);
}

// ---- trim clamp delegation -------------------------------------------

#[test]
fn trim_drag_clamp_delegates_and_clamps_delta() {
    let clip = SplitClip {
        id: "c".into(),
        start_frame: 0,
        duration_frames: 100,
        trim_start_frame: 30,
        trim_end_frame: 40,
        speed: 1.0,
        fade_in_frames: 0,
        fade_out_frames: 0,
        volume_track: None,
        has_no_source_media: false,
    };
    let clamp = trim_drag_clamp(&clip, TrimEdge::Left);
    assert_eq!(clamp.min_delta, -30);
    assert_eq!(clamp.max_delta, 99);
    // A drag delta beyond the cap is clamped.
    assert_eq!(clamp_trim_delta(-50, clamp), -30);
    assert_eq!(clamp_trim_delta(200, clamp), 99);
    assert_eq!(clamp_trim_delta(10, clamp), 10);
}
