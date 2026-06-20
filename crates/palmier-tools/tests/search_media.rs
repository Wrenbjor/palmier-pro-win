//! Integration tests for the E11-S10 `search_media` tool body.
//!
//! Coverage (FOUNDATION §11.1: happy path + 2 error cases) + the SM-12 spoken/visual
//! integration gates:
//! - **SM-12 spoken** (`scope=spoken`): seeds the on-device transcript cache with a
//!   planted keyword (the E10-S7 `PALMIER_TRANSCRIPT_CACHE_DIR` pattern) and asserts
//!   `search_media` returns the matching segment with its `start`/`end` range. This
//!   works with **no model download** (keyword recall, FR-40).
//! - **SM-12 visual** (`scope=visual`): needs the real SigLIP2 embedder + downloaded
//!   weights + a planted `golden_search` fixture, so it is `#[ignore]`d and gated on
//!   `--features ort` (matching how E11-S1/S4 gate their live encode tests). The exact
//!   run command is in the test's comment.
//! - Error cases: empty query; an unknown `mediaRef`.
//!
//! The default build keeps the **visual scope reporting `disabled`/empty** (no ort, no
//! gateway wired) — asserted in `library_text_inspect.rs`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use palmier_model::{ClipType, MediaAsset, MediaLibrary, MediaSource};
// The spoken path reads the cache under palmier-search's transcript defaults
// (`ggml-small.en` / `en`) — seed the SM-12 fixture under the SAME pair so the lookup
// keys identically (the E10-S4 key folds in model_id + language, ruling #19).
use palmier_search::{DEFAULT_LANGUAGE, DEFAULT_MODEL_ID};
use palmier_transcribe::{TranscriptCache, TranscriptionResult, TranscriptionSegment};
use palmier_tools::{Block, EditorState, IdUniverse, ToolDispatch, ToolExecutor};

// ── harness ──────────────────────────────────────────────────────────────────

const CACHE_DIR_ENV: &str = "PALMIER_TRANSCRIPT_CACHE_DIR";

/// Serializes the process-global `PALMIER_TRANSCRIPT_CACHE_DIR` env writes (Rust 2024
/// `set_var`/`remove_var` are unsafe; the lock makes them sound across these tests).
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

fn unique_temp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("palmier-e11s10-{tag}-{pid}-{n}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Point the spoken path at `cache_dir` via the env override for the duration of `f`.
fn with_cache_dir<R>(cache_dir: &std::path::Path, f: impl FnOnce() -> R) -> R {
    unsafe { std::env::set_var(CACHE_DIR_ENV, cache_dir) };
    let r = f();
    unsafe { std::env::remove_var(CACHE_DIR_ENV) };
    r
}

/// A real media file with fixed bytes (the cache key hashes file content), returning its path.
fn fixture_media_file(dir: &std::path::Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, b"fake-media-bytes-for-cache-key-stability").unwrap();
    path
}

fn video_asset(id: &str, file: &PathBuf) -> MediaAsset {
    MediaAsset::new(
        id,
        id,
        ClipType::Video,
        MediaSource::External { absolute_path: file.to_string_lossy().into_owned() },
        10.0,
    )
}

// ════════════════════════════════════════════════════════════════════════════
// SM-12 spoken — planted transcript keyword (no model download)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn sm12_spoken_hits_planted_transcript_keyword() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = unique_temp_dir("spoken");
    let cache_dir = tmp.join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let file = fixture_media_file(&tmp, "interview.mov");

    // Plant a transcript with a known keyword segment under the bundled-English pair
    // the spoken path reads with.
    let transcript = TranscriptionResult {
        text: "Welcome to the budget meeting today".into(),
        language: Some("en".into()),
        words: vec![],
        segments: vec![
            TranscriptionSegment { text: "Welcome to the budget meeting".into(), start: 0.0, end: 2.5 },
            TranscriptionSegment { text: "unrelated chatter here".into(), start: 2.5, end: 4.0 },
        ],
    };
    let cache = TranscriptCache::with_directory(&cache_dir);
    cache.store(&file, DEFAULT_MODEL_ID, DEFAULT_LANGUAGE, &transcript).unwrap();

    let mut lib = MediaLibrary::new();
    lib.assets.push(video_asset("vid", &file));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    let (err, text) = with_cache_dir(&cache_dir, || {
        call(&exec, "search_media", json!({ "query": "budget meeting", "scope": "spoken" }))
    });

    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    let spoken = v["spoken"].as_array().expect("spoken group is an array");
    assert_eq!(spoken.len(), 1, "exactly the matching segment: {text}");
    assert_eq!(spoken[0]["mediaRef"], json!("vid"));
    assert_eq!(spoken[0]["text"], json!("Welcome to the budget meeting"));
    // start/end → range mapping.
    assert_eq!(spoken[0]["startSeconds"], json!(0.0));
    assert_eq!(spoken[0]["endSeconds"], json!(2.5));
    // scope=spoken ⇒ no visual group.
    assert!(v.get("visual").is_none());
}

#[test]
fn sm12_spoken_scopes_to_one_asset_via_media_ref() {
    let _guard = ENV_LOCK.lock().unwrap();
    let tmp = unique_temp_dir("spoken-ref");
    let cache_dir = tmp.join("cache");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let file_a = fixture_media_file(&tmp, "a.mov");
    let file_b = fixture_media_file(&tmp, "b.mov");

    let mk = |kw: &str| TranscriptionResult {
        text: kw.into(),
        language: Some("en".into()),
        words: vec![],
        segments: vec![TranscriptionSegment { text: kw.into(), start: 0.0, end: 1.0 }],
    };
    let cache = TranscriptCache::with_directory(&cache_dir);
    cache.store(&file_a, DEFAULT_MODEL_ID, DEFAULT_LANGUAGE, &mk("budget here")).unwrap();
    cache.store(&file_b, DEFAULT_MODEL_ID, DEFAULT_LANGUAGE, &mk("budget there")).unwrap();

    let mut lib = MediaLibrary::new();
    lib.assets.push(video_asset("a", &file_a));
    lib.assets.push(video_asset("b", &file_b));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));

    // Restrict to "a" ⇒ only a's segment, even though both match.
    let (err, text) = with_cache_dir(&cache_dir, || {
        call(
            &exec,
            "search_media",
            json!({ "query": "budget", "scope": "spoken", "mediaRef": "a" }),
        )
    });
    assert!(!err, "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    let spoken = v["spoken"].as_array().unwrap();
    assert_eq!(spoken.len(), 1);
    assert_eq!(spoken[0]["mediaRef"], json!("a"));
}

// ════════════════════════════════════════════════════════════════════════════
// Error cases (FOUNDATION §11.1 — 2 error cases)
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn empty_query_errors() {
    let exec = ToolExecutor::new();
    let (err, text) = call(&exec, "search_media", json!({ "query": "   " }));
    assert!(err, "blank query must error: {text}");
    assert!(text.contains("query is empty"), "{text}");
}

#[test]
fn unknown_media_ref_errors() {
    let mut lib = MediaLibrary::new();
    lib.assets.push(MediaAsset::new(
        "real",
        "real",
        ClipType::Video,
        MediaSource::External { absolute_path: "/x/real.mov".into() },
        10.0,
    ));
    let exec = ToolExecutor::with_state(EditorState::with_library(lib));
    let (err, text) = call(
        &exec,
        "search_media",
        json!({ "query": "x", "mediaRef": "ghost-asset" }),
    );
    assert!(err, "an unknown mediaRef must error: {text}");
    assert!(text.contains("no media asset"), "{text}");
}

// ════════════════════════════════════════════════════════════════════════════
// SM-12 visual — LIVE: needs the SigLIP2 embedder + weights + golden_search fixture
// ════════════════════════════════════════════════════════════════════════════
//
// This exercises the REAL visual path: it requires the `ort` feature (the ONNX encoder)
// AND the ~750 MB SigLIP2 weights downloaded on disk AND a planted `golden_search`
// B-roll frame to rank. It is therefore `#[ignore]`d (like E11-S1/S4's live encode
// tests) and only meaningful when a host wires a `VisualSearchGateway` over a coordinator
// with the live encoder. To run it once the weights are installed and a gateway harness
// exists:
//
//   pwsh -File scripts/with-msvc.ps1 cargo test --package palmier-tools --features ort -- --ignored sm12_visual
//
// Without the gateway wired the default build reports visual_status=disabled/empty — the
// contract asserted by `search_media_visual_disabled_on_default_build`.
#[test]
#[ignore = "needs --features ort + downloaded SigLIP2 weights + a wired VisualSearchGateway + golden_search fixture"]
fn sm12_visual_returns_golden_search_frame_in_top_k() {
    // Intentionally left as a documented placeholder: the live visual assertion needs
    // the host's gateway harness (coordinator + real encoder) which is wired in
    // palmier-tauri, not the tool crate. When that harness lands, build a coordinator
    // over an EmbeddingStore holding the indexed `golden_search` fixture, wire it behind
    // a VisualSearchGateway, and assert the planted frame ranks in the top-K with
    // visual_status=ready. See the run command above.
    panic!("placeholder — run only with the ort gateway harness wired (see comment)");
}
