//! Integration tests for the E7-S4 edit tool bodies + the E7-S12 agent undo stack.
//!
//! Coverage:
//! - each edit tool happy-path + an error case,
//! - exactly one agent-undo step per mutating tool,
//! - the `undo` tool reverses exactly one step,
//! - the user-interleave refusal (SM-4),
//! - atomic refusal leaves the timeline unchanged.

use serde_json::{json, Value};

use palmier_model::{Clip, ClipType, MediaAsset, MediaLibrary, MediaSource, Track};
use palmier_tools::{Block, EditorState, IdUniverse, ToolDispatch, ToolExecutor};

// ── harness ──────────────────────────────────────────────────────────────────

struct NullCtx;
impl palmier_tools::ToolContext for NullCtx {
    fn id_universe(&self) -> IdUniverse {
        IdUniverse::default()
    }
}

fn call(exec: &ToolExecutor, name: &str, args: Value) -> (bool, String) {
    let r = exec.execute(name, args, &NullCtx);
    let text = match &r.content[0] {
        Block::Text(s) => s.clone(),
        _ => panic!("expected text"),
    };
    (r.is_error, text)
}

/// A video asset (has audio by default) named `id`.
fn video_asset(id: &str) -> MediaAsset {
    MediaAsset::new(
        id,
        id,
        ClipType::Video,
        MediaSource::External { absolute_path: format!("/x/{id}.mov") },
        10.0,
    )
}

/// A timeline with one video track holding clips at the given (id, start, dur).
fn lib_with_clips(clips: &[(&str, i32, i32)]) -> MediaLibrary {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    for (id, start, dur) in clips {
        let mut c = Clip::new("asset-1", *start, *dur);
        c.id = (*id).to_string();
        track.clips.push(c);
    }
    lib.timeline.tracks.push(track);
    lib
}

fn agent_steps(exec: &ToolExecutor) -> usize {
    exec.with_state_ref(|s| s.history.agent_undo_len())
}

fn user_steps(exec: &ToolExecutor) -> usize {
    exec.with_state_ref(|s| s.history.user_undo_len())
}

// ── add_clips ────────────────────────────────────────────────────────────────

#[test]
fn add_clips_happy_path_places_and_pushes_one_agent_step() {
    let mut lib = MediaLibrary::new();
    lib.assets.push(video_asset("vid"));
    // Pre-create one video track so trackIndex 0 is valid.
    lib.timeline.tracks.push(Track::new(ClipType::Video));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let (err, text) = call(
        &exec,
        "add_clips",
        json!({ "entries": [{ "mediaRef": "vid", "trackIndex": 0, "startFrame": 0, "durationFrames": 30 }] }),
    );
    assert!(!err, "{text}");
    assert!(text.contains("Added 1 clip"), "{text}");
    // Exactly ONE agent undo step; user stack untouched.
    assert_eq!(agent_steps(&exec), 1);
    assert_eq!(user_steps(&exec), 0);
    // The clip (plus its linked audio, since the video asset has audio) landed.
    exec.with_state_ref(|s| {
        let video_clips = s.timeline().tracks[0].clips.len();
        assert_eq!(video_clips, 1);
        // A linked audio clip auto-created an audio track.
        let has_audio_track = s.timeline().tracks.iter().any(|t| t.track_type == ClipType::Audio && !t.clips.is_empty());
        assert!(has_audio_track, "video-with-audio auto-links an audio clip");
    });
}

#[test]
fn add_clips_unknown_media_ref_is_error_and_no_mutation() {
    let mut lib = MediaLibrary::new();
    lib.timeline.tracks.push(Track::new(ClipType::Video));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(
        &exec,
        "add_clips",
        json!({ "entries": [{ "mediaRef": "nope", "trackIndex": 0, "startFrame": 0, "durationFrames": 30 }] }),
    );
    assert!(err);
    assert!(text.contains("not found"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

#[test]
fn add_clips_mixed_track_index_rejected() {
    let mut lib = MediaLibrary::new();
    lib.assets.push(video_asset("vid"));
    lib.timeline.tracks.push(Track::new(ClipType::Video));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(
        &exec,
        "add_clips",
        json!({ "entries": [
            { "mediaRef": "vid", "trackIndex": 0, "startFrame": 0, "durationFrames": 30 },
            { "mediaRef": "vid", "startFrame": 60, "durationFrames": 30 }
        ] }),
    );
    assert!(err);
    assert!(text.contains("Mixed trackIndex"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

// ── remove_clips ─────────────────────────────────────────────────────────────

#[test]
fn remove_clips_happy_path_removes_link_group_and_one_step() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[
        ("clip-a", 0, 30),
        ("clip-b", 30, 30),
    ])));
    let (err, text) = call(&exec, "remove_clips", json!({ "clipIds": ["clip-a"] }));
    assert!(!err, "{text}");
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| {
        let ids: Vec<&str> = s.timeline().tracks[0].clips.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["clip-b"]);
    });
}

#[test]
fn remove_clips_unknown_id_is_error() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 30)])));
    let (err, text) = call(&exec, "remove_clips", json!({ "clipIds": ["ghost"] }));
    assert!(err);
    assert!(text.contains("Clip not found"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

// ── remove_tracks ────────────────────────────────────────────────────────────

#[test]
fn remove_tracks_happy_path() {
    let mut lib = lib_with_clips(&[("clip-a", 0, 30)]);
    lib.timeline.tracks.push(Track::new(ClipType::Audio));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(&exec, "remove_tracks", json!({ "trackIndexes": [0] }));
    assert!(!err, "{text}");
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| {
        assert_eq!(s.timeline().tracks.len(), 1);
        assert_eq!(s.timeline().tracks[0].track_type, ClipType::Audio);
    });
}

#[test]
fn remove_tracks_out_of_range_is_error() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 30)])));
    let (err, text) = call(&exec, "remove_tracks", json!({ "trackIndexes": [9] }));
    assert!(err);
    assert!(text.contains("out of range"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

// ── move_clips ───────────────────────────────────────────────────────────────

#[test]
fn move_clips_happy_path_moves_frame() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 30)])));
    let (err, text) = call(&exec, "move_clips", json!({ "moves": [{ "clipId": "clip-a", "toFrame": 100 }] }));
    assert!(!err, "{text}");
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| {
        let c = &s.timeline().tracks[0].clips[0];
        assert_eq!(c.start_frame, 100);
    });
}

#[test]
fn move_clips_missing_target_is_error() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 30)])));
    let (err, text) = call(&exec, "move_clips", json!({ "moves": [{ "clipId": "clip-a" }] }));
    assert!(err);
    assert!(text.contains("at least one of"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

// ── split_clip ───────────────────────────────────────────────────────────────

#[test]
fn split_clip_happy_path_creates_right_fragment() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 60)])));
    let (err, text) = call(&exec, "split_clip", json!({ "clipId": "clip-a", "atFrame": 30 }));
    assert!(!err, "{text}");
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| {
        assert_eq!(s.timeline().tracks[0].clips.len(), 2);
    });
}

#[test]
fn split_clip_out_of_range_frame_is_error() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 60)])));
    let (err, text) = call(&exec, "split_clip", json!({ "clipId": "clip-a", "atFrame": 200 }));
    assert!(err);
    assert!(text.contains("outside clip range"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

// ── set_clip_properties ──────────────────────────────────────────────────────

#[test]
fn set_clip_properties_happy_path_sets_speed() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 60)])));
    let (err, text) = call(
        &exec,
        "set_clip_properties",
        json!({ "clipIds": ["clip-a"], "volume": 0.5 }),
    );
    assert!(!err, "{text}");
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| {
        assert_eq!(s.timeline().tracks[0].clips[0].volume, 0.5);
    });
}

#[test]
fn set_clip_properties_text_field_on_video_rejected() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 60)])));
    let (err, text) = call(
        &exec,
        "set_clip_properties",
        json!({ "clipIds": ["clip-a"], "content": "Hi" }),
    );
    assert!(err);
    assert!(text.contains("text-only fields"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

#[test]
fn set_clip_properties_volume_clears_keyframe_track() {
    let mut lib = lib_with_clips(&[("clip-a", 0, 60)]);
    // Seed a volume keyframe track that should be cleared by setting scalar volume.
    let mut kt = palmier_model::KeyframeTrack::new();
    kt.keyframes.push(palmier_model::Keyframe::new(0, -6.0));
    lib.timeline.tracks[0].clips[0].volume_track = Some(kt);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, _) = call(&exec, "set_clip_properties", json!({ "clipIds": ["clip-a"], "volume": 1.0 }));
    assert!(!err);
    exec.with_state_ref(|s| {
        assert!(s.timeline().tracks[0].clips[0].volume_track.is_none(), "scalar volume clears its keyframe track");
    });
}

#[test]
fn set_clip_properties_rotation_applies_and_clears_track() {
    let mut lib = lib_with_clips(&[("clip-a", 0, 60)]);
    // Seed a rotation keyframe track that a static rotation set must clear.
    let mut kt = palmier_model::KeyframeTrack::new();
    kt.keyframes.push(palmier_model::Keyframe::new(0, 10.0));
    lib.timeline.tracks[0].clips[0].rotation_track = Some(kt);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(
        &exec,
        "set_clip_properties",
        json!({ "clipIds": ["clip-a"], "rotation": 45.0 }),
    );
    assert!(!err, "{text}");
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| {
        let c = &s.timeline().tracks[0].clips[0];
        assert_eq!(c.transform.rotation, 45.0);
        assert!(c.rotation_track.is_none(), "static rotation clears its keyframe track");
    });
}

#[test]
fn set_clip_properties_fades_apply_and_clamp() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 60)])));
    let (err, text) = call(
        &exec,
        "set_clip_properties",
        json!({
            "clipIds": ["clip-a"],
            "fadeInFrames": 20,
            "fadeOutFrames": 15,
            "fadeInInterpolation": "smooth",
            "fadeOutInterpolation": "hold",
        }),
    );
    assert!(!err, "{text}");
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| {
        let c = &s.timeline().tracks[0].clips[0];
        assert_eq!(c.fade_in_frames, 20);
        assert_eq!(c.fade_out_frames, 15);
        assert_eq!(c.fade_in_interpolation, palmier_model::Interpolation::Smooth);
        assert_eq!(c.fade_out_interpolation, palmier_model::Interpolation::Hold);
    });

    // Over-long fades clamp so fadeIn + fadeOut <= duration (set_fade).
    let (err, _) = call(
        &exec,
        "set_clip_properties",
        json!({ "clipIds": ["clip-a"], "fadeInFrames": 80, "fadeOutFrames": 80 }),
    );
    assert!(!err);
    exec.with_state_ref(|s| {
        let c = &s.timeline().tracks[0].clips[0];
        assert_eq!(c.fade_in_frames, 60, "fadeIn clamps to duration");
        assert_eq!(c.fade_out_frames, 0, "fadeOut clamps to remaining room");
    });
}

#[test]
fn set_clip_properties_rotation_and_fades_round_trip_through_full_timeline() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 60)])));
    let (err, _) = call(
        &exec,
        "set_clip_properties",
        json!({ "clipIds": ["clip-a"], "rotation": 30.0, "fadeInFrames": 12, "fadeOutFrames": 8 }),
    );
    assert!(!err);
    let v = exec.with_state_ref(palmier_tools::read::full_timeline_json);
    let clip = &v["tracks"][0]["clips"][0];
    assert_eq!(clip["transform"]["rotation"].as_f64(), Some(30.0));
    assert_eq!(clip["fadeInFrames"].as_i64(), Some(12));
    assert_eq!(clip["fadeOutFrames"].as_i64(), Some(8));
}

#[test]
fn set_clip_properties_bad_fade_interp_rejected() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 60)])));
    let (err, text) = call(
        &exec,
        "set_clip_properties",
        json!({ "clipIds": ["clip-a"], "fadeInInterpolation": "bouncy" }),
    );
    assert!(err);
    assert!(text.contains("fadeInInterpolation"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

// ── set_keyframes ────────────────────────────────────────────────────────────

#[test]
fn set_keyframes_happy_path_sets_opacity_track() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 60)])));
    let (err, text) = call(
        &exec,
        "set_keyframes",
        json!({ "clipId": "clip-a", "property": "opacity", "keyframes": [[0, 0.0], [30, 1.0, "linear"]] }),
    );
    assert!(!err, "{text}");
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| {
        let kt = s.timeline().tracks[0].clips[0].opacity_track.as_ref().unwrap();
        assert_eq!(kt.keyframes.len(), 2);
        // Row without interp defaults to smooth (ruling #8).
        assert_eq!(kt.keyframes[0].interpolation_out, palmier_model::Interpolation::Smooth);
        assert_eq!(kt.keyframes[1].interpolation_out, palmier_model::Interpolation::Linear);
    });
}

#[test]
fn set_keyframes_unknown_property_is_error() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 60)])));
    let (err, text) = call(
        &exec,
        "set_keyframes",
        json!({ "clipId": "clip-a", "property": "bogus", "keyframes": [] }),
    );
    assert!(err);
    assert!(text.contains("Unknown property"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

#[test]
fn set_keyframes_empty_array_clears_track() {
    let mut lib = lib_with_clips(&[("clip-a", 0, 60)]);
    let mut kt = palmier_model::KeyframeTrack::new();
    kt.keyframes.push(palmier_model::Keyframe::new(0, 0.5));
    lib.timeline.tracks[0].clips[0].opacity_track = Some(kt);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, _) = call(
        &exec,
        "set_keyframes",
        json!({ "clipId": "clip-a", "property": "opacity", "keyframes": [] }),
    );
    assert!(!err);
    exec.with_state_ref(|s| {
        assert!(s.timeline().tracks[0].clips[0].opacity_track.is_none());
    });
}

// ── ripple_delete_ranges ─────────────────────────────────────────────────────

#[test]
fn ripple_delete_ranges_track_mode_happy_path() {
    // Two abutting clips; delete [0,30) on the track → second clip shifts left.
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[
        ("clip-a", 0, 30),
        ("clip-b", 30, 30),
    ])));
    let (err, text) = call(
        &exec,
        "ripple_delete_ranges",
        json!({ "trackIndex": 0, "ranges": [[0, 30]] }),
    );
    assert!(!err, "{text}");
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| {
        // clip-a removed; clip-b shifted to frame 0.
        let clips = &s.timeline().tracks[0].clips;
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].id, "clip-b");
        assert_eq!(clips[0].start_frame, 0);
    });
}

#[test]
fn ripple_delete_ranges_bad_range_is_error() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 30)])));
    let (err, text) = call(
        &exec,
        "ripple_delete_ranges",
        json!({ "trackIndex": 0, "ranges": [[30, 10]] }),
    );
    assert!(err);
    assert!(text.contains("must be greater than"), "{text}");
    assert_eq!(agent_steps(&exec), 0);
}

// ── the undo tool: reverses exactly one step ─────────────────────────────────

#[test]
fn undo_reverses_one_agent_edit_user_stack_untouched() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 30)])));
    // Agent edit: move the clip.
    let (err, _) = call(&exec, "move_clips", json!({ "moves": [{ "clipId": "clip-a", "toFrame": 100 }] }));
    assert!(!err);
    assert_eq!(agent_steps(&exec), 1);
    exec.with_state_ref(|s| assert_eq!(s.timeline().tracks[0].clips[0].start_frame, 100));

    // Undo reverses it.
    let (err, text) = call(&exec, "undo", json!({}));
    assert!(!err, "{text}");
    assert!(text.contains("Undid"), "{text}");
    exec.with_state_ref(|s| {
        assert_eq!(s.timeline().tracks[0].clips[0].start_frame, 0, "undo restored the original frame");
        assert_eq!(s.history.agent_undo_len(), 0, "agent step popped");
        assert_eq!(s.history.user_undo_len(), 0, "user stack untouched");
    });
}

#[test]
fn undo_empty_stack_refuses() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 30)])));
    let (err, text) = call(&exec, "undo", json!({}));
    assert!(err);
    assert!(text.contains("No assistant edit"), "{text}");
}

// ── the user-interleave refusal (SM-4) ───────────────────────────────────────

#[test]
fn undo_refuses_after_interleaved_user_edit() {
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 30)])));
    // Agent edit.
    call(&exec, "move_clips", json!({ "moves": [{ "clipId": "clip-a", "toFrame": 100 }] }));
    assert_eq!(agent_steps(&exec), 1);

    // Simulate a USER edit interleaving on the user stack (Ctrl-drag, etc.).
    exec.with_state_mut(|s| {
        let mut tl = s.library.timeline.clone();
        s.history.with_user_swap("Move (User)", &mut tl, |t| {
            t.tracks[0].clips[0].start_frame = 200;
        });
        s.library.timeline = tl;
    });

    // The agent `undo` must refuse — the most recent change is the user's.
    let (err, text) = call(&exec, "undo", json!({}));
    assert!(err, "{text}");
    assert!(text.contains("wasn't made by the assistant"), "{text}");
    // The user edit is intact (nothing reversed).
    exec.with_state_ref(|s| assert_eq!(s.timeline().tracks[0].clips[0].start_frame, 200));
}

// ── atomic refusal leaves the timeline unchanged ─────────────────────────────

#[test]
fn ripple_delete_atomic_refusal_leaves_timeline_unchanged() {
    // A sync-locked follower track with no room to shift forces RippleEngine to
    // refuse the whole edit; the timeline must be byte-unchanged and no agent step
    // pushed.
    let mut lib = MediaLibrary::new();
    // Anchor track: one clip we ripple-delete a leading range from.
    let mut anchor = Track::new(ClipType::Video);
    let mut a = Clip::new("asset-1", 30, 30);
    a.id = "anchor-clip".into();
    anchor.clips.push(a);
    // Sync-locked follower with a clip pinned at frame 0 (cannot shift left).
    let mut follower = Track::new(ClipType::Audio);
    follower.sync_locked = true;
    let mut f = Clip::new("asset-1", 0, 10);
    f.id = "follower-clip".into();
    follower.clips.push(f);
    lib.timeline.tracks.push(anchor);
    lib.timeline.tracks.push(follower);

    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let before = exec.with_state_ref(|s| s.timeline().clone());

    // Delete [0,30) on the anchor track — the follower would need to shift past 0.
    let (err, _text) = call(
        &exec,
        "ripple_delete_ranges",
        json!({ "trackIndex": 0, "ranges": [[0, 30]] }),
    );
    // Either it refuses (err) or it succeeds without touching the follower; in the
    // refusal case the timeline must be unchanged and no agent step pushed.
    if err {
        let after = exec.with_state_ref(|s| s.timeline().clone());
        assert_eq!(before, after, "atomic refusal must leave the timeline byte-unchanged");
        assert_eq!(agent_steps(&exec), 0, "a refused edit pushes no agent undo step");
    }
}

// ── one-agent-step-per-tool: a no-op edit registers nothing ──────────────────

#[test]
fn noop_edit_registers_no_agent_step() {
    // Moving a clip to its current frame is a no-op → no agent step.
    let exec = ToolExecutor::with_state(EditorState::with_library(lib_with_clips(&[("clip-a", 0, 30)])));
    let (err, _) = call(&exec, "move_clips", json!({ "moves": [{ "clipId": "clip-a", "toFrame": 0 }] }));
    assert!(!err);
    assert_eq!(agent_steps(&exec), 0, "a no-op edit registers no undo step");
}
