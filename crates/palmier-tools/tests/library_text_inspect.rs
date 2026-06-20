//! Integration tests for the E7-S8 (text/caption), E7-S10 (library), and E7-S5
//! (inspect) tool bodies.
//!
//! Coverage per the story BUILD GATE:
//! - each tool happy-path + an error case,
//! - library tools: ONE agent-undo step + dual-shape (direct vs entries[]),
//! - `add_texts` creates the clip(s) + one undo step,
//! - `delete_media`/`delete_folder` cascade-remove referencing clips in one step,
//! - `inspect_media` / `inspect_timeline` enforce the `max_frames ≤ 12` ceiling and
//!   the transcript caps; `add_captions` / `search_media` report not-available.

use std::path::PathBuf;

use serde_json::{json, Value};

use palmier_model::{
    Clip, ClipType, MediaAsset, MediaLibrary, MediaSource, Track,
};
use palmier_tools::{AgentStack, Block, EditorState, IdUniverse, ToolDispatch, ToolExecutor};

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
        _ => "<image>".to_string(),
    };
    (r.is_error, text)
}

fn call_blocks(exec: &ToolExecutor, name: &str, args: Value) -> palmier_tools::ToolResult {
    exec.execute(name, args, &NullCtx)
}

fn image_asset(id: &str) -> MediaAsset {
    let mut a = MediaAsset::new(
        id,
        id,
        ClipType::Image,
        MediaSource::External { absolute_path: format!("/x/{id}.png") },
        0.0,
    );
    a.source_width = Some(1920);
    a.source_height = Some(1080);
    a
}

fn video_asset(id: &str) -> MediaAsset {
    MediaAsset::new(
        id,
        id,
        ClipType::Video,
        MediaSource::External { absolute_path: format!("/x/{id}.mov") },
        10.0,
    )
}

fn agent_steps_timeline(exec: &ToolExecutor) -> usize {
    exec.with_state_ref(|s| s.history.agent_undo_len())
}
fn agent_steps_lib(exec: &ToolExecutor) -> usize {
    exec.with_state_ref(|s| s.lib_history.agent_undo_len())
}
fn last_agent(exec: &ToolExecutor) -> Option<AgentStack> {
    exec.with_state_ref(|s| s.last_agent_edit)
}

// ════════════════════════════════════════════════════════════════════════════
// E7-S8 — add_texts
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn add_texts_auto_creates_track_and_pushes_one_agent_step() {
    let exec = ToolExecutor::new();
    let (err, text) = call(
        &exec,
        "add_texts",
        json!({ "entries": [{ "startFrame": 0, "durationFrames": 60, "content": "Hello" }] }),
    );
    assert!(!err, "{text}");
    assert!(text.contains("Added 1 text clip"), "{text}");
    assert!(text.contains("Created track"), "auto-creates a new top track: {text}");
    assert_eq!(agent_steps_timeline(&exec), 1, "exactly one timeline agent step");
    assert_eq!(last_agent(&exec), Some(AgentStack::Timeline));
    // The text clip actually landed, typed text, with content + default style.
    exec.with_state_ref(|s| {
        let clip = s.timeline().tracks.iter().flat_map(|t| t.clips.iter()).next().unwrap();
        assert_eq!(clip.media_type, ClipType::Text);
        assert_eq!(clip.text_content.as_deref(), Some("Hello"));
        let style = clip.text_style.as_ref().unwrap();
        assert!((style.font_size - 96.0).abs() < 1e-9, "default fontSize 96");
    });
}

#[test]
fn add_texts_mixed_track_index_rejected() {
    let mut lib = MediaLibrary::new();
    lib.timeline.tracks.push(Track::new(ClipType::Video));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(
        &exec,
        "add_texts",
        json!({ "entries": [
            { "trackIndex": 0, "startFrame": 0, "durationFrames": 30, "content": "A" },
            { "startFrame": 60, "durationFrames": 30, "content": "B" }
        ] }),
    );
    assert!(err);
    assert!(text.contains("Mixed trackIndex"), "{text}");
    assert_eq!(agent_steps_timeline(&exec), 0);
}

#[test]
fn add_texts_audio_track_rejected() {
    let mut lib = MediaLibrary::new();
    lib.timeline.tracks.push(Track::new(ClipType::Audio));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(
        &exec,
        "add_texts",
        json!({ "entries": [{ "trackIndex": 0, "startFrame": 0, "durationFrames": 30, "content": "A" }] }),
    );
    assert!(err);
    assert!(text.contains("audio track"), "{text}");
}

#[test]
fn add_texts_bad_color_rejected() {
    let exec = ToolExecutor::new();
    let (err, text) = call(
        &exec,
        "add_texts",
        json!({ "entries": [{ "startFrame": 0, "durationFrames": 30, "content": "A", "color": "#GGG" }] }),
    );
    assert!(err);
    assert!(text.contains("Invalid color"), "{text}");
}

// ════════════════════════════════════════════════════════════════════════════
// E7-S8 — add_captions (stub until Epic 10)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn add_captions_reports_not_available() {
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "add_captions", json!({}));
    assert!(err);
    assert!(text.contains("not yet available"), "{text}");
}

#[test]
fn add_captions_rejects_title_case() {
    // ruling #18: textCase ∈ {auto, upper, lower} — no title-case.
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "add_captions", json!({ "textCase": "title" }));
    assert!(err);
    assert!(text.contains("auto, upper, or lower"), "{text}");
}

// ════════════════════════════════════════════════════════════════════════════
// E7-S10 — create_folder (dual-shape) + one library undo step
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn create_folder_direct_returns_single_and_pushes_one_lib_step() {
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "create_folder", json!({ "name": "Footage" }));
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    // Direct form returns a single folder object (id + name), NOT { folders }.
    assert!(v.get("id").is_some(), "{text}");
    assert_eq!(v["name"], json!("Footage"));
    assert!(v.get("folders").is_none());
    assert_eq!(agent_steps_lib(&exec), 1);
    assert_eq!(last_agent(&exec), Some(AgentStack::Library));
    exec.with_state_ref(|s| assert_eq!(s.library.manifest.folders.len(), 1));
}

#[test]
fn create_folder_entries_returns_folders_array() {
    let exec = ToolExecutor::new();
    let (err, text) = call(
        &exec,
        "create_folder",
        json!({ "entries": [{ "name": "A" }, { "name": "B" }] }),
    );
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["folders"].as_array().unwrap().len(), 2);
    assert_eq!(agent_steps_lib(&exec), 1, "batch is one undo step");
}

#[test]
fn create_folder_both_shapes_rejected() {
    let exec = ToolExecutor::new();
    let (err, text) = call(
        &exec,
        "create_folder",
        json!({ "name": "A", "entries": [{ "name": "B" }] }),
    );
    assert!(err, "dual-shape XOR: not both");
    assert!(text.contains("not both"), "{text}");
    assert_eq!(agent_steps_lib(&exec), 0);
}

// ════════════════════════════════════════════════════════════════════════════
// E7-S10 — rename_media / rename_folder / move_to_folder
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn rename_media_direct_renames_and_one_step() {
    let mut lib = MediaLibrary::new();
    lib.assets.push(video_asset("vid"));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(&exec, "rename_media", json!({ "mediaRef": "vid", "name": "Intro" }));
    assert!(!err, "{text}");
    assert_eq!(agent_steps_lib(&exec), 1);
    exec.with_state_ref(|s| {
        assert_eq!(s.library.assets[0].name, "Intro");
    });
}

#[test]
fn rename_media_missing_asset_is_error() {
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "rename_media", json!({ "mediaRef": "nope", "name": "X" }));
    assert!(err);
    assert!(text.contains("not found"), "{text}");
}

#[test]
fn rename_folder_entries_batch_is_one_step() {
    let exec = ToolExecutor::new();
    // Create two folders first (one step), then rename both (one step).
    let (_e, text) = call(&exec, "create_folder", json!({ "entries": [{ "name": "A" }, { "name": "B" }] }));
    let v: Value = serde_json::from_str(&text).unwrap();
    let id_a = v["folders"][0]["id"].as_str().unwrap().to_string();
    let id_b = v["folders"][1]["id"].as_str().unwrap().to_string();

    let (err, _t) = call(
        &exec,
        "rename_folder",
        json!({ "entries": [{ "folderId": id_a, "name": "AA" }, { "folderId": id_b, "name": "BB" }] }),
    );
    assert!(!err);
    assert_eq!(agent_steps_lib(&exec), 2, "two lib steps: create + rename");
}

#[test]
fn move_to_folder_to_root_when_folder_omitted() {
    let mut lib = MediaLibrary::new();
    let mut a = image_asset("img");
    a.folder_id = Some("some-folder".into());
    lib.assets.push(a);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(&exec, "move_to_folder", json!({ "assetIds": ["img"] }));
    assert!(!err, "{text}");
    assert!(text.contains("to root"), "{text}");
    exec.with_state_ref(|s| assert_eq!(s.library.assets[0].folder_id, None));
}

// ════════════════════════════════════════════════════════════════════════════
// E7-S10 — delete_media / delete_folder cascade + one-step undo
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn delete_media_removes_referencing_clips_in_one_step_and_undoes() {
    let mut lib = MediaLibrary::new();
    lib.assets.push(video_asset("vid"));
    let mut track = Track::new(ClipType::Video);
    let mut clip = Clip::new("vid", 0, 30); // references the asset
    clip.id = "c1".into();
    track.clips.push(clip);
    lib.timeline.tracks.push(track);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let (err, text) = call(&exec, "delete_media", json!({ "assetIds": ["vid"] }));
    assert!(!err, "{text}");
    assert_eq!(agent_steps_lib(&exec), 1);
    exec.with_state_ref(|s| {
        assert!(s.library.assets.is_empty(), "asset removed");
        // Referencing clip removed, its now-empty track pruned.
        assert!(s.timeline().tracks.iter().all(|t| !t.clips.iter().any(|c| c.media_ref == "vid")));
    });

    // The single `undo` tool reverses the library edit (most-recent agent step).
    let (uerr, utext) = call(&exec, "undo", json!({}));
    assert!(!uerr, "{utext}");
    exec.with_state_ref(|s| {
        assert_eq!(s.library.assets.len(), 1, "asset restored by undo");
        let has_clip = s.timeline().tracks.iter().any(|t| t.clips.iter().any(|c| c.media_ref == "vid"));
        assert!(has_clip, "referencing clip restored by undo");
    });
}

#[test]
fn delete_folder_missing_is_error() {
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "delete_folder", json!({ "folderIds": ["nope"] }));
    assert!(err);
    assert!(text.contains("not found"), "{text}");
    assert_eq!(agent_steps_lib(&exec), 0);
}

#[test]
fn delete_folder_cascades_assets_and_clips() {
    let exec = ToolExecutor::new();
    call(&exec, "create_folder", json!({ "name": "F" }));
    // Read the FULL folder id from state (the tool output is ShortId-shortened, which
    // would not match the asset's stored folder_id below).
    let fid = exec.with_state_ref(|s| s.library.manifest.folders[0].id.clone());
    // Put an asset in F + a clip referencing it.
    exec.with_state_mut(|s| {
        let mut a = video_asset("inF");
        a.folder_id = Some(fid.clone());
        s.library.assets.push(a);
        let mut track = Track::new(ClipType::Video);
        track.clips.push(Clip::new("inF", 0, 30));
        s.library.timeline.tracks.push(track);
    });
    let (err, _t) = call(&exec, "delete_folder", json!({ "folderIds": [fid] }));
    assert!(!err);
    exec.with_state_ref(|s| {
        assert!(s.library.assets.iter().all(|a| a.id != "inF"), "asset in folder deleted");
        assert!(s.library.manifest.folders.is_empty(), "folder deleted");
    });
}

// ════════════════════════════════════════════════════════════════════════════
// E7-S10 — import_media (path + bytes synchronous; url stub)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn import_media_from_path_imports_and_one_step() {
    // Write a real (extension-classified) PNG-named file to a temp path.
    let dir = std::env::temp_dir().join(format!("palmier-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("clip.png");
    std::fs::write(&file, b"not-a-real-png-but-classified-by-extension").unwrap();

    let exec = ToolExecutor::new();
    let (err, text) = call(
        &exec,
        "import_media",
        json!({ "source": { "path": file.to_string_lossy() } }),
    );
    assert!(!err, "{text}");
    assert!(text.contains("Imported"), "{text}");
    assert_eq!(agent_steps_lib(&exec), 1);
    exec.with_state_ref(|s| assert_eq!(s.library.assets.len(), 1));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_media_path_not_found_is_error() {
    let exec = ToolExecutor::new();
    let missing = PathBuf::from("/definitely/not/here/x.png");
    let (err, text) = call(
        &exec,
        "import_media",
        json!({ "source": { "path": missing.to_string_lossy() } }),
    );
    assert!(err);
    assert!(text.contains("File not found"), "{text}");
}

#[test]
fn import_media_bytes_imports() {
    // base64("hello") = "aGVsbG8=".
    let exec = ToolExecutor::new();
    let (err, text) = call(
        &exec,
        "import_media",
        json!({ "source": { "bytes": "aGVsbG8=", "mimeType": "image/png" } }),
    );
    assert!(!err, "{text}");
    assert!(text.contains("Imported"), "{text}");
    assert_eq!(agent_steps_lib(&exec), 1);
}

#[test]
fn import_media_bytes_requires_mime() {
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "import_media", json!({ "source": { "bytes": "aGVsbG8=" } }));
    assert!(err);
    assert!(text.contains("mimeType is required"), "{text}");
}

#[test]
fn import_media_url_reports_not_available() {
    let exec = ToolExecutor::new();
    let (err, text) = call(
        &exec,
        "import_media",
        json!({ "source": { "url": "https://example.com/clip.mp4" } }),
    );
    assert!(err);
    assert!(text.contains("URL import is not yet available"), "{text}");
}

#[test]
fn import_media_two_sources_rejected() {
    let exec = ToolExecutor::new();
    let (err, text) = call(
        &exec,
        "import_media",
        json!({ "source": { "path": "/a.png", "bytes": "aGVsbG8=" } }),
    );
    assert!(err);
    assert!(text.contains("exactly one"), "{text}");
}

// ════════════════════════════════════════════════════════════════════════════
// E7-S5 — inspect_media caps
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn inspect_media_clamps_max_frames_to_twelve() {
    let mut lib = MediaLibrary::new();
    lib.assets.push(video_asset("vid"));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(&exec, "inspect_media", json!({ "mediaRef": "vid", "maxFrames": 99 }));
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["maxFrames"], json!(12), "max_frames clamped to the 12-frame ceiling");
    assert_eq!(v["frameCeiling"], json!(12));
    // The transcript pagination caps (400 / 10000) are distinct from the frame ceiling.
    assert_eq!(v["transcript"]["segmentCap"], json!(400));
    assert_eq!(v["transcript"]["wordCap"], json!(10000));
    assert_eq!(v["transcript"]["segments"], json!([]), "empty transcript in M2");
}

#[test]
fn inspect_media_overview_ignores_max_frames() {
    let mut lib = MediaLibrary::new();
    lib.assets.push(video_asset("vid"));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(
        &exec,
        "inspect_media",
        json!({ "mediaRef": "vid", "maxFrames": 8, "overview": true }),
    );
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["overview"], json!(true));
    assert_eq!(v["frameCount"], json!(0), "overview ignores maxFrames");
}

#[test]
fn inspect_media_missing_asset_is_error() {
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "inspect_media", json!({ "mediaRef": "nope" }));
    assert!(err);
    assert!(text.contains("not found"), "{text}");
}

// ════════════════════════════════════════════════════════════════════════════
// E7-S5 — inspect_timeline frame sampling + ceiling (GPU-free path)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn inspect_timeline_empty_is_error() {
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "inspect_timeline", json!({}));
    assert!(err);
    assert!(text.contains("empty"), "{text}");
}

// These two assert the GPU-FREE shape (the body returns the sampled-frame PLAN as
// JSON when `gpu-inspect` is off). With the feature on, inspect_timeline returns
// image blocks (or a clean no-adapter error) — exercised by the gpu smoke test below.
#[cfg(not(feature = "gpu-inspect"))]
#[test]
fn inspect_timeline_clamps_sampled_frames_to_twelve() {
    // A timeline 1000 frames long; request maxFrames 50 over the whole span.
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    track.clips.push(Clip::new("asset", 0, 1000));
    lib.timeline.tracks.push(track);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let r = call_blocks(
        &exec,
        "inspect_timeline",
        json!({ "startFrame": 0, "endFrame": 1000, "maxFrames": 50 }),
    );
    assert!(!r.is_error);
    let text = match &r.content[0] {
        Block::Text(s) => s.clone(),
        _ => panic!("expected text in the GPU-free build"),
    };
    let v: Value = serde_json::from_str(&text).unwrap();
    let frames = v["frameNumbers"].as_array().unwrap();
    assert_eq!(frames.len(), 12, "sampled frame count clamped to the 12 ceiling");
}

#[cfg(not(feature = "gpu-inspect"))]
#[test]
fn inspect_timeline_single_frame_default() {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    track.clips.push(Clip::new("asset", 0, 100));
    lib.timeline.tracks.push(track);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let r = call_blocks(&exec, "inspect_timeline", json!({ "startFrame": 10 }));
    assert!(!r.is_error);
    let text = match &r.content[0] { Block::Text(s) => s.clone(), _ => panic!() };
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["frameNumbers"], json!([10]), "no endFrame → single frame");
}

/// With `gpu-inspect` on: a single-frame composite either renders image blocks or
/// returns a clean "no GPU adapter" error (headless CI box) — never panics. Proves
/// the compositor path is wired (FOUNDATION §11.1: GPU paths run headless or skip).
#[cfg(feature = "gpu-inspect")]
#[test]
fn inspect_timeline_gpu_renders_or_skips_cleanly() {
    let mut lib = MediaLibrary::new();
    let mut track = Track::new(ClipType::Video);
    track.clips.push(Clip::new("asset", 0, 100));
    lib.timeline.tracks.push(track);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let r = call_blocks(&exec, "inspect_timeline", json!({ "startFrame": 10 }));
    if r.is_error {
        // No usable GPU adapter on this box — acceptable, must be the adapter message.
        let msg = match &r.content[0] { Block::Text(s) => s.clone(), _ => String::new() };
        assert!(
            msg.contains("GPU compositor unavailable") || msg.contains("Failed to render"),
            "{msg}"
        );
    } else {
        // Rendered: the first block is a PNG image, trailing block is the JSON meta.
        assert!(matches!(r.content[0], Block::Image { .. }), "first block is an image");
        let meta = r.content.last().unwrap();
        assert!(matches!(meta, Block::Text(_)), "trailing meta block");
    }
}

// ════════════════════════════════════════════════════════════════════════════
// E7-S9 — search_media stub
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn search_media_reports_not_indexed() {
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "search_media", json!({ "query": "harbor at sunset" }));
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["visual"]["status"], json!("not_indexed"));
    assert_eq!(v["visual"]["moments"], json!([]));
}
