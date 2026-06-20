//! E10-S8 — Transcript-driven cut path (FR-38), the UJ-1 climax e2e gate.
//!
//! This seals the §11.3 "agent-cut workflow" end-to-end over the **public tool
//! surface** (no internal calls): transcribe a clip (seeded cache) → `get_transcript`
//! returns words in **project frames** → the agent identifies a filler/dead-air range
//! → `ripple_delete_ranges` cuts it and **closes the gap** in one atomic, undoable op.
//!
//! The conversion + ripple + agent-undo glue this story is responsible for already
//! lives in the merged `ripple_delete_ranges` tool (the `clipId` path maps
//! source-seconds → project frames through trim/speed/position with `f64::round`
//! ties-away / speed-floor `0.0001` / half-open `[start, end)`, and wraps the edit as
//! ONE agent-undo step). These tests are the gate that proves the whole UJ-1 path
//! holds together and that the no-transcript edge (UJ-1: "transcribe first, don't
//! guess") is preserved.
//!
//! Cache seam (same as E10-S7's tests): `PALMIER_TRANSCRIPT_CACHE_DIR` points the
//! read-only `get_transcript` cache lookup at a per-test temp dir; the transcript is
//! seeded under `WHISPER_MODEL_ID` + `resolve_cache_language(None)` against a real file
//! on disk (the cache key hashes content). The env var is process-global, so the
//! cache-dir tests serialize on `ENV_LOCK`.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use palmier_model::{Clip, ClipType, MediaAsset, MediaLibrary, MediaSource, Track};
use palmier_transcribe::{
    TranscriptCache, TranscriptionResult, TranscriptionSegment, TranscriptionWord,
};
use palmier_tools::{
    resolve_cache_language, Block, EditorState, IdUniverse, ToolDispatch, ToolExecutor,
    CACHE_DIR_ENV, WHISPER_MODEL_ID,
};

/// Serializes every test that mutates the process-global cache-dir env var.
static ENV_LOCK: Mutex<()> = Mutex::new(());

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

fn agent_steps(exec: &ToolExecutor) -> usize {
    exec.with_state_ref(|s| s.history.agent_undo_len())
}

fn user_steps(exec: &ToolExecutor) -> usize {
    exec.with_state_ref(|s| s.history.user_undo_len())
}

/// A unique temp directory for one test (no `tempfile` dev-dep).
fn unique_temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("palmier-e10s8-{tag}-{pid}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Point `get_transcript`'s cache lookup at `cache_dir` for the duration of `f`.
fn with_cache_dir<R>(cache_dir: &std::path::Path, f: impl FnOnce() -> R) -> R {
    unsafe { std::env::set_var(CACHE_DIR_ENV, cache_dir) };
    let r = f();
    unsafe { std::env::remove_var(CACHE_DIR_ENV) };
    r
}

/// Write a real file with fixed bytes (the cache key hashes its content).
fn fixture_media_file(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(b"fake-audio-bytes-for-cache-key-stability").unwrap();
    f.flush().unwrap();
    path
}

/// A transcript with a clear "filler" word in the middle: "um" spans 1.0..2.0 s.
/// At 30 fps this is the 30..60 project-frame range on an untrimmed, unscaled clip.
fn transcript_with_filler() -> TranscriptionResult {
    TranscriptionResult {
        text: "hello um world".to_string(),
        language: Some("en-US".to_string()),
        words: vec![
            TranscriptionWord { text: "hello".into(), start: Some(0.0), end: Some(1.0) },
            TranscriptionWord { text: "um".into(), start: Some(1.0), end: Some(2.0) },
            TranscriptionWord { text: "world".into(), start: Some(2.0), end: Some(3.0) },
        ],
        segments: vec![TranscriptionSegment { text: "hello um world".into(), start: 0.0, end: 3.0 }],
    }
}

/// A library: one video asset (file = `file`) and one 90-frame clip referencing it on a
/// video track (untrimmed, speed 1) — 3 s @ 30 fps. Returns (lib, clip_id).
fn library_with_clip(media_id: &str, file: &PathBuf, dur: i32) -> (MediaLibrary, String) {
    let mut lib = MediaLibrary::new();
    let asset = MediaAsset::new(
        media_id,
        media_id,
        ClipType::Video,
        MediaSource::External { absolute_path: file.to_string_lossy().into_owned() },
        10.0,
    );
    lib.assets.push(asset);

    let mut track = Track::new(ClipType::Video);
    let mut clip = Clip::new(media_id, 0, dur);
    clip.media_type = ClipType::Video;
    let clip_id = clip.id.clone();
    track.clips.push(clip);
    lib.timeline.tracks.push(track);
    (lib, clip_id)
}

// ════════════════════════════════════════════════════════════════════════════
// UJ-1 climax: transcribe → get_transcript → ripple_delete_ranges → gap closes
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn transcript_driven_cut_collapses_filler_in_one_atomic_undoable_op() {
    // The full §11.3 agent-cut workflow over the public tool surface.
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = unique_temp_dir("cut-happy");
    let cache_dir = tmp.join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let file = fixture_media_file(&tmp, "speech.mov");

    // 1. "Transcribe": seed the on-device transcript cache the way add_captions/
    //    inspect_media would have, under the exact (model, language) get_transcript reads.
    let lang = resolve_cache_language(None);
    let cache = TranscriptCache::with_directory(&cache_dir);
    cache.store(&file, WHISPER_MODEL_ID, &lang, &transcript_with_filler()).unwrap();

    // Two abutting 90-frame copies of the clip so the cut on the FIRST leaves the SECOND
    // to ripple left — proving the gap actually closes on the timeline, not just inside
    // one clip. Both reference the same source, so both transcribe from the seeded cache.
    let (mut lib, clip_a) = library_with_clip("vid", &file, 90);
    let clip_b = {
        let mut c = Clip::new("vid", 90, 90); // abuts clip_a at frame 90
        c.media_type = ClipType::Video;
        let id = c.id.clone();
        lib.timeline.tracks[0].clips.push(c);
        id
    };
    let fps = lib.timeline.fps; // 30
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    // 2. get_transcript → words already in PROJECT FRAMES (UJ-1: agent reads, doesn't guess).
    let (err, text) = with_cache_dir(&cache_dir, || call(&exec, "get_transcript", json!({})));
    assert!(!err, "get_transcript failed: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["timing"], json!("projectFrames"));
    let clips = v["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 2, "two clips on the timeline: {clips:?}");

    // 3. The agent identifies the filler word "um" and reads its [startFrame, endFrame).
    //    On clip_a (0..90), "um" 1.0..2.0 s maps to frames 30..60.
    let words_a = clips[0]["words"].as_array().unwrap();
    let um = words_a
        .iter()
        .find(|w| w[0] == json!("um"))
        .expect("filler word present in transcript output");
    let um_start = um[1].as_i64().unwrap() as i32;
    let um_end = um[2].as_i64().unwrap() as i32;
    assert_eq!((um_start, um_end), (fps, fps * 2), "um maps to 30..60 project frames");

    // The emitted clipId is the ShortId-shortened prefix; pass it back AS RECEIVED.
    let emitted_clip_a = clips[0]["clipId"].as_str().unwrap().to_string();
    assert!(clip_a.starts_with(&emitted_clip_a));

    // 4. ripple_delete_ranges (clipId path, default units 'frames') — pass the word's
    //    frames straight back, as the get_transcript contract says.
    let total_before = exec.with_state_ref(|s| s.timeline().total_frames());
    let (err, text) = call(
        &exec,
        "ripple_delete_ranges",
        json!({ "clipId": emitted_clip_a, "ranges": [[um_start, um_end]] }),
    );
    assert!(!err, "ripple_delete_ranges failed: {text}");
    let report: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["removedFrames"], json!(um_end - um_start), "30 frames removed");

    // 5. The gap closed: total timeline length shrank by exactly the removed span, and
    //    clip_b rippled left by 30. (Half-open [30,60) → 30 frames.)
    let total_after = exec.with_state_ref(|s| s.timeline().total_frames());
    assert_eq!(total_after, total_before - 30, "timeline collapsed by the dead-air span");
    let b_start = exec.with_state_ref(|s| {
        s.timeline().tracks[0]
            .clips
            .iter()
            .find(|c| c.id == clip_b)
            .map(|c| c.start_frame)
            .unwrap()
    });
    assert_eq!(b_start, 60, "clip_b rippled left by the 30-frame gap (90 → 60)");

    // 6. ONE atomic agent-undo step; the user stack is untouched.
    assert_eq!(agent_steps(&exec), 1, "exactly one agent-undo step for the cut");
    assert_eq!(user_steps(&exec), 0, "user undo stack untouched");

    // 7. The cut is reversible via the agent `undo` tool (single atomic op).
    let (err, text) = call(&exec, "undo", json!({}));
    assert!(!err, "undo failed: {text}");
    let restored = exec.with_state_ref(|s| s.timeline().total_frames());
    assert_eq!(restored, total_before, "undo restored the dead-air span");
    let b_restored = exec.with_state_ref(|s| {
        s.timeline().tracks[0]
            .clips
            .iter()
            .find(|c| c.id == clip_b)
            .map(|c| c.start_frame)
            .unwrap()
    });
    assert_eq!(b_restored, 90, "clip_b back at its pre-cut position");
}

#[test]
fn transcript_driven_cut_source_seconds_path_maps_through_trim_and_speed() {
    // The story's named deliverable: a source-SECONDS range (e.g. from inspect_media)
    // converts to project frames through the clip's placement/trim/speed before the cut.
    // Here the clip is trimmed (trim_start 30) and placed at frame 100, so source 1.0 s
    // is NOT project frame 30 — the conversion must account for placement+trim.
    let tmp = unique_temp_dir("cut-seconds");
    let file = fixture_media_file(&tmp, "speech.mov");

    let mut lib = MediaLibrary::new();
    lib.assets.push(MediaAsset::new(
        "vid",
        "vid",
        ClipType::Video,
        MediaSource::External { absolute_path: file.to_string_lossy().into_owned() },
        10.0,
    ));
    let mut track = Track::new(ClipType::Video);
    let mut clip = Clip::new("vid", 100, 90); // placed at frame 100
    clip.media_type = ClipType::Video;
    clip.trim_start_frame = 30; // visible source window starts 1.0 s in (30 fps)
    let clip_id = clip.id.clone();
    track.clips.push(clip);
    lib.timeline.tracks.push(track);
    let fps = lib.timeline.fps; // 30
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    // Cut source seconds 1.5..2.0 s. Through the reference math:
    //   project = start_frame + (sec*fps - trim_start) / max(speed, 1e-4)
    //   1.5 s → 100 + (45 - 30)/1 = 115 ; 2.0 s → 100 + (60 - 30)/1 = 130
    // → removes the half-open [115, 130) = 15 frames.
    let total_before = exec.with_state_ref(|s| s.timeline().total_frames());
    let (err, text) = call(
        &exec,
        "ripple_delete_ranges",
        json!({ "clipId": clip_id, "units": "seconds", "ranges": [[1.5, 2.0]] }),
    );
    assert!(!err, "seconds-mode ripple failed: {text}");
    let report: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["removedFrames"], json!(15), "0.5 s @ 30fps = 15 frames removed");
    let total_after = exec.with_state_ref(|s| s.timeline().total_frames());
    assert_eq!(total_after, total_before - 15, "timeline collapsed by the 15-frame span");
    assert_eq!(agent_steps(&exec), 1, "one atomic agent-undo step");
    let _ = fps;
}

// ════════════════════════════════════════════════════════════════════════════
// UJ-1 edge: no transcript → agent is told to transcribe first, must not guess
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn no_transcript_yields_empty_words_so_agent_transcribes_first_not_guesses() {
    // With nothing cached, get_transcript returns the clip with empty words and NO
    // word frames to cut — the agent has nothing to guess from and (per the contract
    // description) must transcribe first. This is the UJ-1 edge the cut path depends on.
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = unique_temp_dir("cut-empty");
    let cache_dir = tmp.join("empty-cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let file = fixture_media_file(&tmp, "untranscribed.mov");

    let (lib, _clip_id) = library_with_clip("vid", &file, 90);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let (err, text) = with_cache_dir(&cache_dir, || call(&exec, "get_transcript", json!({})));
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    let clips = v["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0]["words"], json!([]), "no transcript → empty words, no cut points to guess");
    assert!(v.get("totalWords").is_none(), "no paging note when nothing transcribed");

    // No mutation has occurred; the agent has no basis to call ripple_delete_ranges.
    assert_eq!(agent_steps(&exec), 0, "no edit on the no-transcript path");
}
