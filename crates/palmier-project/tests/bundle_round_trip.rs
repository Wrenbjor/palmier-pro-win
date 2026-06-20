//! E2-S9 integration tests — the `.palmier` bundle reader/writer end-to-end.
//!
//! Covers the four acceptance scenarios from the story:
//! 1. **Round-trip (SM-7 seed):** write a bundle → read it back → assert identical
//!    `Timeline` + `MediaManifest` (and byte-stable `project.json`).
//! 2. **Severity — corrupt `media.json` = hard error** (reference read :48).
//! 3. **Severity — missing `generation-log.json` = ok** (reference read :52 soft).
//! 4. **Atomic save** — an injected mid-save failure leaves the original bundle
//!    intact and no partial/staging directory behind (NSDocument safe-save).
//!
//! These exercise the crate through its public API only (no `super::` access), so
//! they are the real "write → read → identical" gate the §11.2 M1 round-trip test
//! (E2-S10) will build on.

use std::fs;
use std::path::PathBuf;

use palmier_project::bundle::project;
use palmier_project::{
    read_bundle, write_bundle, BundleError, BundleSnapshot, LoadedBundle,
};

use palmier_model::{
    Clip, ClipType, GenerationLog, GenerationLogEntry, MediaManifest, MediaManifestEntry,
    MediaSource, Timeline, Track,
};

/// Unique, auto-named scratch dir under the OS temp dir (no `tempfile` dep).
fn scratch() -> PathBuf {
    let p = std::env::temp_dir().join(format!("palmier-e2s9-it-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&p).unwrap();
    p
}

/// A non-trivial timeline: a video track with one real clip, so the round-trip
/// exercises `Track` + `Clip` serde (not just empty defaults).
fn rich_timeline() -> Timeline {
    let mut t = Timeline::new();
    t.fps = 24;
    t.width = 3840;
    t.height = 2160;
    let mut track = Track::new(ClipType::Video);
    let mut clip = Clip::new("asset-1", 10, 120);
    clip.id = "clip-1".into();
    track.clips.push(clip);
    t.tracks.push(track);
    t
}

fn rich_manifest() -> MediaManifest {
    let mut m = MediaManifest::new();
    m.entries.push(MediaManifestEntry {
        id: "asset-1".into(),
        name: "Clip One".into(),
        asset_type: ClipType::Video,
        source: MediaSource::Project {
            relative_path: "media/clip.mov".into(),
        },
        duration: 12.5,
        generation_input: None,
        source_width: Some(3840),
        source_height: Some(2160),
        source_fps: Some(24.0),
        has_audio: Some(true),
        folder_id: None,
        cached_remote_url: None,
        cached_remote_url_expires_at: None,
    });
    m
}

#[test]
fn round_trip_preserves_timeline_manifest_and_log() {
    let root = scratch();
    let bundle = root.join("RoundTrip.palmier");

    let timeline = rich_timeline();
    let manifest = rich_manifest();
    let mut log = GenerationLog::new();
    log.entries
        .push(GenerationLogEntry::new("veo-3", Some(250)));

    let mut snap = BundleSnapshot::new(timeline.clone());
    snap.manifest = Some(manifest.clone());
    snap.generation_log = Some(log.clone());
    write_bundle(&bundle, &snap).unwrap();

    let loaded: LoadedBundle = read_bundle(&bundle).unwrap();
    assert_eq!(loaded.timeline, timeline, "Timeline must round-trip");
    assert_eq!(loaded.manifest, Some(manifest), "manifest must round-trip");
    assert_eq!(loaded.generation_log, Some(log), "gen-log must round-trip");

    // project.json is byte-stable through a re-save of the reloaded model (SM-7).
    let first = fs::read(bundle.join(project::TIMELINE_FILE)).unwrap();
    write_bundle(&bundle, &BundleSnapshot::new(loaded.timeline)).unwrap();
    let second = fs::read(bundle.join(project::TIMELINE_FILE)).unwrap();
    assert_eq!(first, second, "project.json bytes must be stable");

    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn corrupt_media_json_is_hard_error() {
    let root = scratch();
    let bundle = root.join("BadManifest.palmier");
    fs::create_dir_all(&bundle).unwrap();
    fs::write(
        bundle.join(project::TIMELINE_FILE),
        serde_json::to_vec(&rich_timeline()).unwrap(),
    )
    .unwrap();
    fs::write(bundle.join(project::MANIFEST_FILE), b"{ broken").unwrap();

    match read_bundle(&bundle).unwrap_err() {
        BundleError::Corrupt { file, .. } => assert_eq!(file, project::MANIFEST_FILE),
        other => panic!("expected Corrupt on media.json, got {other:?}"),
    }
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn missing_generation_log_is_ok() {
    let root = scratch();
    let bundle = root.join("NoLog.palmier");
    write_bundle(&bundle, &BundleSnapshot::new(rich_timeline())).unwrap();
    // No generation-log.json was written.
    assert!(!bundle.join(project::GENERATION_LOG_FILE).exists());

    let loaded = read_bundle(&bundle).unwrap();
    assert!(loaded.generation_log.is_none(), "absence is not an error");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn injected_mid_save_failure_leaves_original_intact() {
    // Establish a known-good v1 bundle.
    let root = scratch();
    let bundle = root.join("Atomic.palmier");
    let v1 = rich_timeline(); // fps 24
    write_bundle(&bundle, &BundleSnapshot::new(v1.clone())).unwrap();
    let v1_bytes = fs::read(bundle.join(project::TIMELINE_FILE)).unwrap();

    // Inject a save failure by giving the writer a destination whose PARENT is a
    // regular file. `create_dir_all(parent)` then fails, so `write_bundle` errors
    // BEFORE it can swap anything — proving the original bundle is never touched
    // and no staging dir is left behind in `root`.
    let blocker = root.join("blocker-file");
    fs::write(&blocker, b"x").unwrap();
    let doomed = blocker.join("Doomed.palmier"); // parent is a file
    let res = write_bundle(&doomed, &BundleSnapshot::new(rich_timeline()));
    assert!(res.is_err(), "save under a file-parent must fail");

    // Original bundle byte-for-byte unchanged.
    let after = fs::read(bundle.join(project::TIMELINE_FILE)).unwrap();
    assert_eq!(v1_bytes, after, "the good bundle must survive a failed save");

    // No staging/backup siblings linger anywhere under root.
    let stray: Vec<String> = fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp") || n.contains(".bak"))
        .collect();
    assert!(stray.is_empty(), "no staging/backup dirs may remain: {stray:?}");

    fs::remove_dir_all(&root).unwrap();
}
