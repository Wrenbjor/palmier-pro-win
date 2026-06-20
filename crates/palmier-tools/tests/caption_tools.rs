//! E10-S7 — `add_captions` + `get_transcript` tool bodies (cache-driven).
//!
//! These exercise the parts that need the **transcript cache** seam (E10-S4): the
//! tools read/write `TranscriptCache` keyed by `sha256(content)+model+language`
//! (ruling #19). The `PALMIER_TRANSCRIPT_CACHE_DIR` env override points both tools at a
//! per-test temp cache so a seeded transcript is served WITHOUT invoking whisper/FFmpeg
//! (which aren't available in CI). Coverage per FOUNDATION §11.1 — happy path + 2 error
//! cases per tool:
//!
//! - `get_transcript`: cached words mapped to project frames (happy); no transcript →
//!   empty (UJ-1 edge); bad window (startFrame >= endFrame) rejected.
//! - `add_captions`: cache-hit plain path places a caption track as ONE undo step
//!   (happy); cache **bypass** when `censorProfanity`/`language` is set does NOT use the
//!   cache (so with no engine it produces no captions — proves bypass); `textCase:
//!   "title"` rejected (ruling #18).
//!
//! The env var is process-global, so the cache-dir tests serialize on `ENV_LOCK`.

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

/// A unique temp directory for one test (no `tempfile` dev-dep): a counter-suffixed
/// subdir under the OS temp root. Created fresh; the OS reaps temp on reboot.
fn unique_temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("palmier-e10s7-{tag}-{pid}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Point both caption tools at `cache_dir` via the env override for the duration of
/// `f`, then clear it. Rust 2024 makes `set_var`/`remove_var` unsafe (process-global,
/// non-thread-safe) — `ENV_LOCK` (held by every cache-dir test) provides the required
/// serialization, so these writes are sound here.
fn with_cache_dir<R>(cache_dir: &std::path::Path, f: impl FnOnce() -> R) -> R {
    unsafe { std::env::set_var(CACHE_DIR_ENV, cache_dir) };
    let r = f();
    unsafe { std::env::remove_var(CACHE_DIR_ENV) };
    r
}

/// Write a real file with fixed bytes (the cache key hashes its content) and return its
/// path. A real file must exist so `TranscriptCache::key` can hash it (store + read).
fn fixture_media_file(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(b"fake-audio-bytes-for-cache-key-stability").unwrap();
    f.flush().unwrap();
    path
}

/// A transcript: three timed words spanning 0..3 s, one untimed word (dropped by the
/// midpoint/window filters), plus segments so the filter `text` rebuilds cleanly.
fn sample_transcript() -> TranscriptionResult {
    TranscriptionResult {
        text: "hello world again".to_string(),
        language: Some("en-US".to_string()),
        words: vec![
            TranscriptionWord { text: "hello".into(), start: Some(0.0), end: Some(1.0) },
            TranscriptionWord { text: "world".into(), start: Some(1.0), end: Some(2.0) },
            TranscriptionWord { text: "again".into(), start: Some(2.0), end: Some(3.0) },
            TranscriptionWord { text: "untimed".into(), start: None, end: None },
        ],
        segments: vec![
            TranscriptionSegment { text: "hello world".into(), start: 0.0, end: 2.0 },
            TranscriptionSegment { text: "again".into(), start: 2.0, end: 3.0 },
        ],
    }
}

/// A library with one video asset (file = `file`) and a clip referencing it on a video
/// track (0..`dur` frames, untrimmed, speed 1). fps defaults to the Timeline default.
fn library_with_clip(media_id: &str, file: &PathBuf, dur: i32) -> (MediaLibrary, String) {
    let mut lib = MediaLibrary::new();
    let asset = MediaAsset::new(
        media_id,
        media_id,
        ClipType::Video,
        MediaSource::External { absolute_path: file.to_string_lossy().into_owned() },
        10.0,
    );
    // video asset → has_audio defaults true (MediaAsset::new).
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
// get_transcript
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn get_transcript_maps_cached_words_to_project_frames() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = unique_temp_dir("gt-happy");
    let cache_dir = tmp.join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let file = fixture_media_file(&tmp, "clip.mov");

    // Seed the cache under the exact (model, language) the tool reads with.
    let lang = resolve_cache_language(None);
    let cache = TranscriptCache::with_directory(&cache_dir);
    cache.store(&file, WHISPER_MODEL_ID, &lang, &sample_transcript()).unwrap();

    let (lib, clip_id) = library_with_clip("vid", &file, 90); // 90 frames @ 30fps = 3s.
    let fps = lib.timeline.fps;
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let (err, text) = with_cache_dir(&cache_dir, || call(&exec, "get_transcript", json!({})));

    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["timing"], json!("projectFrames"));
    let clips = v["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1, "one clip");
    let words = clips[0]["words"].as_array().unwrap();
    // The three timed words map to project frames; the untimed word is dropped.
    assert_eq!(words.len(), 3, "3 timed words, untimed dropped: {words:?}");
    // The executor shortens output ids to their >=8-char unique prefix, so the emitted
    // clipId is a prefix of the real id (not the full UUID).
    let emitted_id = clips[0]["clipId"].as_str().unwrap();
    assert!(clip_id.starts_with(emitted_id), "emitted clipId {emitted_id} prefixes {clip_id}");
    // First word "hello" 0..1s → frames 0..fps (untrimmed, speed 1).
    assert_eq!(words[0][0], json!("hello"));
    assert_eq!(words[0][1], json!(0));
    assert_eq!(words[0][2], json!(fps));
}

#[test]
fn get_transcript_empty_when_nothing_cached() {
    // UJ-1 edge: no cached transcript → clip emitted with empty words, no totalWords.
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = unique_temp_dir("gt-empty");
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
    assert_eq!(clips[0]["words"], json!([]), "no cached transcript → empty words");
    assert!(v.get("totalWords").is_none(), "no paging note when nothing transcribed");
}

#[test]
fn get_transcript_rejects_inverted_window() {
    // startFrame >= endFrame → error (reference window validation).
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "get_transcript", json!({ "startFrame": 100, "endFrame": 50 }));
    assert!(err, "{text}");
    assert!(text.contains("must be less than"), "{text}");
}

// ════════════════════════════════════════════════════════════════════════════
// add_captions
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn add_captions_cache_hit_places_one_undoable_caption_track() {
    // Plain path (no censor / no locale) → cache HIT serves the transcript, captions
    // are generated and placed as ONE agent-undo step named "Generate Captions".
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = unique_temp_dir("ac-happy");
    let cache_dir = tmp.join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let file = fixture_media_file(&tmp, "speech.mov");

    let lang = resolve_cache_language(None);
    let cache = TranscriptCache::with_directory(&cache_dir);
    cache.store(&file, WHISPER_MODEL_ID, &lang, &sample_transcript()).unwrap();

    let (lib, clip_id) = library_with_clip("vid", &file, 90);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    // Caption that one clip explicitly (no auto-detect ambiguity).
    let (err, text) =
        with_cache_dir(&cache_dir, || call(&exec, "add_captions", json!({ "clipIds": [clip_id] })));

    assert!(!err, "expected captions placed, got: {text}");
    assert!(text.contains("Added") && text.contains("caption"), "{text}");
    assert_eq!(agent_steps(&exec), 1, "exactly one agent-undo step");

    // A new video track was inserted at index 0 carrying text/caption clips, and the
    // undo step is named exactly "Generate Captions".
    exec.with_state_ref(|s| {
        let top = &s.timeline().tracks[0];
        assert_eq!(top.track_type, ClipType::Video);
        let n_text = top.clips.iter().filter(|c| c.media_type == ClipType::Text).count();
        assert!(n_text >= 1, "caption track has text clips");
        assert!(
            top.clips.iter().all(|c| c.caption_group_id.is_some()),
            "caption clips carry the shared group id"
        );
    });
    let name =
        exec.with_state_ref(|s| s.history.current_undo_action_name().map(str::to_string));
    assert_eq!(name.as_deref(), Some("Generate Captions"), "verbatim undo-group name");
}

#[test]
fn add_captions_bypass_does_not_use_cache() {
    // censorProfanity set → BYPASS the cache. The seeded transcript must NOT be used;
    // with no engine available the transcription fails, so no captions are produced
    // (no source transcribed → empty result → "No speech detected to caption.").
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = unique_temp_dir("ac-bypass");
    let cache_dir = tmp.join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let file = fixture_media_file(&tmp, "speech.mov");

    let lang = resolve_cache_language(None);
    let cache = TranscriptCache::with_directory(&cache_dir);
    cache.store(&file, WHISPER_MODEL_ID, &lang, &sample_transcript()).unwrap();

    let (lib, clip_id) = library_with_clip("vid", &file, 90);
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let (err, text) = with_cache_dir(&cache_dir, || {
        call(&exec, "add_captions", json!({ "clipIds": [clip_id], "censorProfanity": true }))
    });

    // Bypass means the cached transcript is ignored → engine call (unavailable) yields
    // nothing → no captions placed, no undo step. (If the cache were wrongly consulted
    // we'd get a caption track + success instead.)
    assert!(err, "bypass with no engine should not succeed: {text}");
    assert!(text.contains("No speech detected"), "{text}");
    assert_eq!(agent_steps(&exec), 0, "no undo step when bypass transcribes nothing");
}

#[test]
fn add_captions_rejects_unsupported_language() {
    // A non-English BCP-47 language is rejected up front (the bundled model is .en).
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "add_captions", json!({ "language": "zz" }));
    assert!(err, "{text}");
    assert!(text.contains("does not support language"), "{text}");
}
