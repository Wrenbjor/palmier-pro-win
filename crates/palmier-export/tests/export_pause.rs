//! E11-S6 follow-up — export-pause coupling.
//!
//! Verifies that palmier-export bumps the search subsystem's process-global
//! `ExportPauseCounter` for the duration of an export run, so visual-search
//! indexing pauses while an export is in flight (FOUNDATION §6.10, search.md
//! "Export-pause coupling"). Mirrors the reference `ExportService.isExporting`
//! → `SearchIndexCoordinator.exportDidBegin()/exportDidEnd()` coupling.
//!
//! These tests exercise the **default (non-`gpu-export`)** build via the bundle
//! export entrypoint [`export_palmier_project`], which is GPU/FFmpeg-free — so the
//! `palmier-search` dependency they prove is wired stays light (plain atomics, no
//! `ort`/ONNX). The instrumentation on the video run (`export_video`, behind
//! `gpu-export`) uses the same RAII guard at the same boundary.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use palmier_export::export_palmier_project;
use palmier_model::{
    ClipType, GenerationLog, MediaManifest, MediaManifestEntry, MediaSource, Timeline,
};
use palmier_search::{export_active, ExportPauseGuard};

/// The export-pause counter is a process-global static; serialize these tests so a
/// concurrent run can't observe another test's in-flight pause.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|p| p.into_inner())
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("palmier-export-pause-{tag}-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&d).unwrap();
    d
}

fn external_entry(id: &str, name: &str, abs: &str) -> MediaManifestEntry {
    MediaManifestEntry {
        id: id.into(),
        name: name.into(),
        asset_type: ClipType::Video,
        source: MediaSource::External {
            absolute_path: abs.into(),
        },
        duration: 4.0,
        generation_input: None,
        source_width: None,
        source_height: None,
        source_fps: None,
        has_audio: None,
        folder_id: None,
        cached_remote_url: None,
        cached_remote_url_expires_at: None,
    }
}

/// Sanity: with no export running and the wiring present, indexing is NOT paused.
#[test]
fn baseline_is_not_active() {
    let _g = lock();
    assert!(
        !export_active(),
        "no export in flight ⇒ indexing must not be paused"
    );
}

/// A real bundle export run holds the export pause for exactly its duration: the
/// counter is active *during* the run and returns to inactive once it completes.
#[test]
fn bundle_export_pauses_indexing_for_its_duration() {
    let _g = lock();
    assert!(!export_active(), "precondition: not active before the run");

    let work = temp_dir("during");
    // A reasonably large media file so the copy gives the observer thread a real
    // window to see `export_active() == true` mid-run.
    let media = work.join("clip.mov");
    fs::write(&media, vec![0u8; 16 * 1024 * 1024]).unwrap();

    let mut manifest = MediaManifest::new();
    manifest.entries.push(external_entry(
        "asset-1",
        "clip.mov",
        &media.to_string_lossy(),
    ));

    let timeline = Timeline::new();
    let log = GenerationLog::new();
    let dest = work.join("out.palmier");
    let temp_root = temp_dir("during-stage");

    // Observe the counter from another thread while the export runs. The flag is
    // set true the instant we see the pause take effect.
    let saw_active = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let observer = {
        let saw_active = Arc::clone(&saw_active);
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                if export_active() {
                    saw_active.store(true, Ordering::Release);
                    break;
                }
                std::hint::spin_loop();
            }
        })
    };

    let report = export_palmier_project(
        &timeline,
        &manifest,
        &log,
        None,
        &dest,
        &temp_root,
    )
    .expect("bundle export should succeed");

    done.store(true, Ordering::Release);
    observer.join().unwrap();

    assert!(
        saw_active.load(Ordering::Acquire),
        "indexing must be paused (export_active == true) while the export runs"
    );
    assert!(
        !export_active(),
        "the pause must be released once the export run completes"
    );
    assert_eq!(report.collected, vec!["asset-1".to_string()]);

    let _ = fs::remove_dir_all(&work);
    let _ = fs::remove_dir_all(&temp_root);
}

/// The pause is released even when the export run fails partway (parity with the
/// reference `defer { isExporting = false }`): the RAII guard ends on the early
/// error return, so a failed export can't wedge indexing.
#[test]
fn failed_export_still_releases_the_pause() {
    let _g = lock();
    assert!(!export_active(), "precondition: not active");

    let work = temp_dir("fail");
    let mut manifest = MediaManifest::new();
    // Point a Project source at a dir we never provide (None below) — and force a
    // write failure by making `dest`'s parent un-creatable: use a dest whose
    // parent is an existing *file*, so `create_dir_all(parent)` fails → the
    // export returns Err while the guard is held.
    let blocker = work.join("not-a-dir");
    fs::write(&blocker, b"x").unwrap();
    let dest = blocker.join("nested").join("out.palmier");

    manifest.entries.push(external_entry(
        "asset-1",
        "clip.mov",
        &work.join("missing.mov").to_string_lossy(),
    ));

    let result = export_palmier_project(
        &Timeline::new(),
        &manifest,
        &GenerationLog::new(),
        None,
        &dest,
        &temp_dir("fail-stage"),
    );
    assert!(result.is_err(), "export should fail (parent is a file)");
    assert!(
        !export_active(),
        "the pause must be released even when the export run errors out"
    );

    let _ = fs::remove_dir_all(&work);
}

/// The public guard the export path consumes round-trips the global counter.
#[test]
fn guard_round_trips_the_counter() {
    let _g = lock();
    assert!(!export_active());
    {
        let _p = ExportPauseGuard::begin();
        assert!(export_active(), "guard begins the pause");
    }
    assert!(!export_active(), "drop ends the pause");
}
