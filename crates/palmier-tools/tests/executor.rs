//! Integration tests for the E7-S2/E7-S3 executor: the READ tool output shapes
//! (esp. `get_timeline` default-omission + the 200-row captionGroup cap),
//! arg-validation happy/error per tool, concurrent serialization through the
//! single owner, and ShortId in/out on real ids.

use std::sync::Arc;
use std::thread;

use serde_json::{json, Value};

use palmier_model::{
    Clip, ClipType, MediaAsset, MediaFolder, MediaLibrary, MediaSource, Track,
};
use palmier_tools::{
    EditorState, IdUniverse, ToolDispatch, ToolExecutor, CAPTION_ROW_LIMIT,
};

/// A `ToolContext` the executor ignores (it owns its own locked state) but the
/// trait still requires one from callers.
struct NullCtx;
impl palmier_tools::ToolContext for NullCtx {
    fn id_universe(&self) -> IdUniverse {
        IdUniverse::default()
    }
}

fn call(exec: &ToolExecutor, name: &str, args: Value) -> Value {
    let r = exec.execute(name, args, &NullCtx);
    // The single text block, parsed back to JSON for shape assertions.
    match &r.content[0] {
        palmier_tools::Block::Text(s) => {
            serde_json::from_str(s).unwrap_or_else(|_| json!({ "_raw": s }))
        }
        _ => panic!("expected a text block"),
    }
}

fn ok_text(exec: &ToolExecutor, name: &str, args: Value) -> (bool, String) {
    let r = exec.execute(name, args, &NullCtx);
    let text = match &r.content[0] {
        palmier_tools::Block::Text(s) => s.clone(),
        _ => panic!("expected text"),
    };
    (r.is_error, text)
}

// ── get_timeline: default-omission ───────────────────────────────────────────

#[test]
fn get_timeline_omits_default_clip_fields() {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    // A clip with ALL default props (speed 1, volume 1, opacity 1, no trims, …).
    let mut clip = Clip::new("asset-aaaaaaaa", 0, 30);
    clip.id = "11111111-1111-1111-1111-111111111111".into();
    track.clips.push(clip);
    lib.timeline.tracks.push(track);

    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let out = call(&exec, "get_timeline", json!({}));

    let clip_json = &out["tracks"][0]["clips"][0];
    // Always-present fields survive.
    assert!(clip_json.get("id").is_some());
    assert!(clip_json.get("mediaRef").is_some());
    assert_eq!(clip_json["startFrame"], json!(0));
    assert_eq!(clip_json["durationFrames"], json!(30));
    // Default-valued fields are OMITTED.
    assert!(clip_json.get("speed").is_none(), "default speed omitted");
    assert!(clip_json.get("volume").is_none(), "default volume omitted");
    assert!(clip_json.get("opacity").is_none(), "default opacity omitted");
    assert!(clip_json.get("trimStartFrame").is_none(), "default trim omitted");
    assert!(clip_json.get("transform").is_none(), "identity transform omitted");
    // sourceClipType == mediaType → dropped.
    assert!(clip_json.get("sourceClipType").is_none());

    // canGenerate + totalFrames injected.
    assert_eq!(out["canGenerate"], json!(false));
    assert_eq!(out["totalFrames"], json!(30));
    // fps/width/height come from the Timeline encoding.
    assert_eq!(out["fps"], json!(30));
    assert_eq!(out["width"], json!(1920));
}

#[test]
fn get_timeline_keeps_non_default_fields() {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    let mut clip = Clip::new("asset-1", 10, 60);
    clip.speed = 2.0;
    clip.volume = 0.5;
    track.clips.push(clip);
    lib.timeline.tracks.push(track);

    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let out = call(&exec, "get_timeline", json!({}));
    let clip_json = &out["tracks"][0]["clips"][0];
    assert_eq!(clip_json["speed"], json!(2));
    assert_eq!(clip_json["volume"], json!(0.5));
}

#[test]
fn get_timeline_strips_default_track_flags() {
    let mut lib = MediaLibrary::new();
    // A default track: muted=false, hidden=false, syncLocked=true.
    let track = Track::new(ClipType::Video);
    lib.timeline.tracks.push(track);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let out = call(&exec, "get_timeline", json!({}));
    let track_json = &out["tracks"][0];
    assert!(track_json.get("muted").is_none());
    assert!(track_json.get("hidden").is_none());
    assert!(track_json.get("syncLocked").is_none(), "default syncLocked=true omitted");
    // The display label is injected.
    assert_eq!(track_json["label"], json!("V1"));
}

// ── get_timeline: caption groups + 200-row cap ───────────────────────────────

#[test]
fn get_timeline_collapses_captions_into_groups_with_cap() {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Text);
    // 250 caption clips sharing one captionGroupId, identical style → all modal.
    let gid = "cccccccc-cccc-cccc-cccc-cccccccccccc";
    for i in 0..250 {
        let mut clip = Clip::new("asset-x", i * 10, 10);
        clip.id = format!("{:08}-0000-0000-0000-000000000000", i);
        clip.media_type = ClipType::Text;
        clip.source_clip_type = ClipType::Text;
        clip.caption_group_id = Some(gid.to_string());
        clip.text_content = Some(format!("word{i}"));
        track.clips.push(clip);
    }
    lib.timeline.tracks.push(track);

    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let out = call(&exec, "get_timeline", json!({}));

    let track_json = &out["tracks"][0];
    // No loose clips — all collapsed into a captionGroup.
    assert_eq!(track_json["clips"].as_array().unwrap().len(), 0);
    let groups = track_json["captionGroups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    // The captionGroupId is in the id universe, so it's shortened to its 8-char
    // unique prefix on output (ShortId).
    assert_eq!(group["captionGroupId"], json!(&gid[..8]));
    // clipCount = the full 250 (pre-cap).
    assert_eq!(group["clipCount"], json!(250));
    // Rows capped at 200.
    let rows = group["clips"].as_array().unwrap();
    assert_eq!(rows.len(), CAPTION_ROW_LIMIT);
    assert_eq!(rows.len(), 200);
    // Each row is [clipId, startFrame, durationFrames, text].
    assert_eq!(rows[0].as_array().unwrap().len(), 4);
    assert_eq!(rows[0][3], json!("word0"));
    // clipFormat advertised.
    assert_eq!(
        group["clipFormat"],
        json!(["clipId", "startFrame", "durationFrames", "text"])
    );
    // Over-cap paging note present.
    assert!(group["clipsNote"].as_str().unwrap().contains("Showing 200 of 250"));
}

#[test]
fn get_timeline_window_pages_caption_rows() {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Text);
    let gid = "dddddddd-dddd-dddd-dddd-dddddddddddd";
    for i in 0..50 {
        let mut clip = Clip::new("asset-x", i * 10, 10);
        clip.id = format!("{:08}-0000-0000-0000-000000000000", i);
        clip.media_type = ClipType::Text;
        clip.source_clip_type = ClipType::Text;
        clip.caption_group_id = Some(gid.to_string());
        clip.text_content = Some(format!("w{i}"));
        track.clips.push(clip);
    }
    lib.timeline.tracks.push(track);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    // Window [0, 100) → only clips starting in [0,100) intersect (clips 0..10ish).
    let out = call(&exec, "get_timeline", json!({ "startFrame": 0, "endFrame": 100 }));
    let group = &out["tracks"][0]["captionGroups"][0];
    let rows = group["clips"].as_array().unwrap();
    // clips at start 0,10,...,90 intersect [0,100) → 10 rows.
    assert_eq!(rows.len(), 10);
    // clipCount still reports the full group total (50).
    assert_eq!(group["clipCount"], json!(50));
}

#[test]
fn get_timeline_window_rejects_inverted_range() {
    let exec = ToolExecutor::with_state(EditorState::new());
    let (is_err, text) = ok_text(&exec, "get_timeline", json!({ "startFrame": 50, "endFrame": 10 }));
    assert!(is_err);
    assert!(text.contains("startFrame must be less than endFrame"));
}

// ── get_media ────────────────────────────────────────────────────────────────

#[test]
fn get_media_projects_asset_fields() {
    let mut lib = MediaLibrary::new();
    let mut asset = MediaAsset::new(
        "asset-1",
        "Clip One",
        ClipType::Video,
        MediaSource::External { absolute_path: "/x/clip.mov".into() },
        12.5,
    );
    asset.folder_id = Some("folder-1".into());
    lib.assets.push(asset);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let out = call(&exec, "get_media", json!({}));
    let a = &out["assets"][0];
    assert_eq!(a["id"], json!("asset-1"));
    assert_eq!(a["name"], json!("Clip One"));
    assert_eq!(a["type"], json!("video"));
    assert_eq!(a["duration"], json!(12.5));
    assert_eq!(a["generationStatus"], json!("none"));
    assert_eq!(a["folderId"], json!("folder-1"));
}

// ── list_folders ─────────────────────────────────────────────────────────────

#[test]
fn list_folders_omits_parent_at_root() {
    let mut lib = MediaLibrary::new();
    lib.manifest.folders.push(MediaFolder { id: "f1".into(), name: "Top".into(), parent_id: None });
    lib.manifest.folders.push(MediaFolder {
        id: "f2".into(),
        name: "Child".into(),
        parent_id: Some("f1".into()),
    });
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let out = call(&exec, "list_folders", json!({}));
    let folders = out["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 2);
    assert!(folders[0].get("parentFolderId").is_none(), "root folder omits parent");
    assert_eq!(folders[1]["parentFolderId"], json!("f1"));
}

// ── list_models ──────────────────────────────────────────────────────────────

#[test]
fn list_models_returns_unloaded_empty_catalog() {
    let exec = ToolExecutor::with_state(EditorState::new());
    let out = call(&exec, "list_models", json!({}));
    assert_eq!(out["loaded"], json!(false));
    assert_eq!(out["models"], json!([]));
}

// ── get_transcript (empty in M2, but ordered/scoped) ─────────────────────────

#[test]
fn get_transcript_walks_av_clips_empty_words() {
    let mut lib = MediaLibrary::new();
    let mut v = Track::new(ClipType::Video);
    let mut c1 = Clip::new("a1", 30, 30);
    c1.id = "clip-late".into();
    let mut c2 = Clip::new("a2", 0, 30);
    c2.id = "clip-early".into();
    v.clips.push(c1);
    v.clips.push(c2);
    lib.timeline.tracks.push(v);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let out = call(&exec, "get_transcript", json!({}));
    assert_eq!(out["timing"], json!("projectFrames"));
    assert_eq!(out["wordFormat"], json!(["text", "start", "end"]));
    let clips = out["clips"].as_array().unwrap();
    // Two A/V clips, ordered by startFrame (early first).
    assert_eq!(clips.len(), 2);
    assert_eq!(clips[0]["clipId"], json!("clip-early"));
    assert_eq!(clips[1]["clipId"], json!("clip-late"));
    // No words in M2 (no transcription store).
    assert_eq!(clips[0]["words"], json!([]));
}

#[test]
fn get_transcript_clip_filter_not_found_errors() {
    let exec = ToolExecutor::with_state(EditorState::new());
    let (is_err, text) = ok_text(&exec, "get_transcript", json!({ "clipId": "nope" }));
    assert!(is_err);
    assert!(text.contains("not found"));
}

// ── arg validation: happy + error per representative tool ────────────────────

#[test]
fn validation_unknown_key_is_error_shape() {
    let exec = ToolExecutor::with_state(EditorState::new());
    let r = exec.execute("get_timeline", json!({ "bogus": 1 }), &NullCtx);
    assert!(r.is_error);
    let wire = r.to_mcp_json();
    assert_eq!(wire["isError"], json!(true));
    assert!(wire["content"][0]["text"].as_str().unwrap().contains("unknown field(s) 'bogus'"));
}

#[test]
fn validation_missing_required_is_error() {
    let exec = ToolExecutor::with_state(EditorState::new());
    let r = exec.execute("split_clip", json!({ "clipId": "x" }), &NullCtx);
    assert!(r.is_error);
}

#[test]
fn validation_generate_audio_no_required_passes_dispatch() {
    let exec = ToolExecutor::with_state(EditorState::new());
    // generate_audio has no required field; it dispatches (to the stub body).
    let r = exec.execute("generate_audio", json!({}), &NullCtx);
    assert!(!r.is_error, "generate_audio with empty args should pass validation");
}

#[test]
fn validation_dual_shape_both_rejected() {
    let exec = ToolExecutor::with_state(EditorState::new());
    let r = exec.execute(
        "create_folder",
        json!({ "name": "A", "entries": [{ "name": "B" }] }),
        &NullCtx,
    );
    assert!(r.is_error);
}

#[test]
fn validation_ripple_requires_one_anchor() {
    let exec = ToolExecutor::with_state(EditorState::new());
    let r = exec.execute("ripple_delete_ranges", json!({ "ranges": [[0, 5]] }), &NullCtx);
    assert!(r.is_error);
}

#[test]
fn unknown_tool_name_is_error() {
    let exec = ToolExecutor::with_state(EditorState::new());
    let r = exec.execute("not_a_tool", json!({}), &NullCtx);
    assert!(r.is_error);
    assert!(r.to_mcp_json()["content"][0]["text"].as_str().unwrap().contains("Unknown tool"));
}

// ── ShortId in/out on real ids ───────────────────────────────────────────────

#[test]
fn shortid_shortens_real_clip_ids_in_get_timeline() {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    let mut clip = Clip::new("asset-1", 0, 30);
    // A full UUID id that is unique at the 8-char floor.
    clip.id = "abcdef01-2345-6789-abcd-ef0123456789".into();
    track.clips.push(clip);
    lib.timeline.tracks.push(track);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let (_is_err, text) = ok_text(&exec, "get_timeline", json!({}));
    // The full UUID is shortened to its 8-char unique prefix in the output text.
    assert!(text.contains("abcdef01"), "short prefix present");
    assert!(!text.contains("abcdef01-2345-6789-abcd-ef0123456789"), "full id shortened away");
}

#[test]
fn shortid_expands_input_prefix_for_get_transcript_clip_filter() {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    let mut clip = Clip::new("a1", 0, 30);
    clip.id = "feedface-0000-1111-2222-333344445555".into();
    track.clips.push(clip);
    lib.timeline.tracks.push(track);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    // Pass an 8-char unique prefix for clipId; expansion resolves it to the full
    // id, so the clip-filter matches and the call succeeds (not "not found").
    let (is_err, _text) = ok_text(&exec, "get_transcript", json!({ "clipId": "feedface" }));
    assert!(!is_err, "prefix should expand and match the clip");
}

#[test]
fn shortid_ambiguous_input_prefix_is_error() {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    let mut c1 = Clip::new("a1", 0, 30);
    c1.id = "aaaaaaaa-1111-1111-1111-111111111111".into();
    let mut c2 = Clip::new("a2", 30, 30);
    c2.id = "aaaaaaaa-2222-2222-2222-222222222222".into();
    track.clips.push(c1);
    track.clips.push(c2);
    lib.timeline.tracks.push(track);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    // "aaaaaaaa" matches both clip ids → ambiguous → error shape.
    let r = exec.execute("get_transcript", json!({ "clipId": "aaaaaaaa" }), &NullCtx);
    assert!(r.is_error);
    assert!(r.to_mcp_json()["content"][0]["text"].as_str().unwrap().contains("Ambiguous id"));
}

// ── concurrent serialization through the single owner ────────────────────────

#[test]
fn concurrent_execute_calls_serialize_through_single_owner() {
    // Build a non-trivial library so each get_timeline does real shaping work.
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    for i in 0..200 {
        let mut clip = Clip::new("asset-1", i * 30, 30);
        clip.id = format!("{:08}-0000-0000-0000-000000000000", i);
        track.clips.push(clip);
    }
    lib.timeline.tracks.push(track);

    let exec = Arc::new(ToolExecutor::with_state(EditorState::with_library(lib)));

    // 16 threads each hammering get_timeline + get_media concurrently. The Mutex
    // serializes them; the test passes if there is no data race / panic and every
    // call returns a well-formed, non-error result.
    let mut handles = Vec::new();
    for _ in 0..16 {
        let exec = Arc::clone(&exec);
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                let r = exec.execute("get_timeline", json!({}), &NullCtx);
                assert!(!r.is_error);
                let r2 = exec.execute("get_media", json!({}), &NullCtx);
                assert!(!r2.is_error);
            }
        }));
    }
    for h in handles {
        h.join().expect("worker thread panicked — serialization broke");
    }

    // The state is intact and readable after the storm.
    exec.with_state_ref(|s| {
        assert_eq!(s.timeline().tracks[0].clips.len(), 200);
    });
}
