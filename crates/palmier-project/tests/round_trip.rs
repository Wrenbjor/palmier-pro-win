//! E2-S10 — the §11.2 M1 import→edit→save→reopen round-trip gate (SM-7) + the
//! SM-1b open-speed gate, over the committed golden fixtures.
//!
//! Scenarios:
//! 1. **Round-trip fidelity (SM-7):** for each `tests/fixtures/golden_*.palmier`,
//!    load → mutate the in-memory model (move a clip, add a keyframe) → save to a
//!    scratch bundle → reopen → assert the reopened model **serializes
//!    byte-for-byte identically** to the saved model. Covers Timeline/Track/Clip,
//!    MediaManifest/MediaSource, GenerationLog.
//! 2. **Golden stability:** each committed fixture, loaded and re-saved unchanged,
//!    produces byte-identical `project.json` / `media.json` / `generation-log.json`
//!    — so a golden diff in CI blocks merge (R-5). Regenerate intentionally with
//!    `PALMIER_UPDATE_GOLDEN=1`.
//! 3. **SM-1b open speed:** a synthesized 30-clip 1080p project opens in < 1 s.
//!
//! The fixtures are built by `golden_fixtures.rs` (shared module) so the on-disk
//! bundles and the in-test expectations can't drift; `regenerate_goldens` writes
//! them when `PALMIER_UPDATE_GOLDEN=1`.

mod common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use palmier_model::{AnimPair, Clip, ClipType, Keyframe, KeyframeTrack, Timeline, Track};
use palmier_project::bundle::project;
use palmier_project::{read_bundle, write_bundle, BundleSnapshot, LoadedBundle};

use common::golden_fixtures::all_fixtures;

/// The committed fixtures live next to this test file.
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A unique scratch dir under the OS temp dir.
fn scratch() -> PathBuf {
    let p = std::env::temp_dir().join(format!("palmier-e2s10-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// The serialized JSON of a loaded bundle's three documents, concatenated — the
/// "model state" we assert byte-identity over (SM-7).
fn serialized_state(b: &LoadedBundle) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let timeline = serde_json::to_vec(&b.timeline).unwrap();
    let manifest = b
        .manifest
        .as_ref()
        .map(|m| serde_json::to_vec(m).unwrap())
        .unwrap_or_default();
    let log = b
        .generation_log
        .as_ref()
        .map(|l| serde_json::to_vec(l).unwrap())
        .unwrap_or_default();
    (timeline, manifest, log)
}

/// Regenerate the committed golden bundles from the builders, **exactly once per
/// test-binary run** (a `Once` guard — the tests run in parallel, and the atomic
/// directory swap would otherwise race on the shared fixture dir). Gated behind
/// `PALMIER_UPDATE_GOLDEN=1` so a normal `cargo test` never rewrites them (R-5
/// golden-update discipline).
fn maybe_regenerate_goldens() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if std::env::var("PALMIER_UPDATE_GOLDEN").as_deref() != Ok("1") {
            return;
        }
        let dir = fixtures_dir();
        std::fs::create_dir_all(&dir).unwrap();
        for (name, snap) in all_fixtures() {
            let bundle = dir.join(format!("{name}.palmier"));
            // Fresh: remove any prior fixture so stale files can't linger.
            let _ = std::fs::remove_dir_all(&bundle);
            write_bundle(&bundle, &snap).unwrap();
        }
        eprintln!("regenerated golden fixtures in {}", dir.display());
    });
}

#[test]
fn regenerate_goldens_when_requested() {
    // This test is the `--update-golden` entry point. With the env var unset it
    // is a no-op pass; with it set it (re)writes the committed fixtures once.
    maybe_regenerate_goldens();
}

#[test]
fn golden_fixtures_round_trip_with_edits() {
    maybe_regenerate_goldens();
    let dir = fixtures_dir();

    for (name, _) in all_fixtures() {
        let bundle = dir.join(format!("{name}.palmier"));
        assert!(
            bundle.join(project::TIMELINE_FILE).exists(),
            "missing golden fixture {name} — run with PALMIER_UPDATE_GOLDEN=1 to create"
        );

        // 1. Import (load the committed fixture).
        let loaded = read_bundle(&bundle)
            .unwrap_or_else(|e| panic!("failed to open golden fixture {name}: {e}"));

        // 2. Edit the in-memory model: move the first clip +15 frames and add a
        //    Linear opacity keyframe to it (a representative mutation touching the
        //    Clip + KeyframeTrack shapes).
        let mut timeline = loaded.timeline.clone();
        mutate_first_clip(&mut timeline);

        let mut snap = BundleSnapshot::new(timeline);
        snap.manifest = loaded.manifest.clone();
        snap.generation_log = loaded.generation_log.clone();

        // 3. Save to a scratch bundle.
        let out_root = scratch();
        let out = out_root.join(format!("{name}.palmier"));
        write_bundle(&out, &snap).unwrap();

        // The saved model (what we expect to read back).
        let saved = LoadedBundle {
            timeline: snap.timeline.clone(),
            manifest: snap.manifest.clone(),
            generation_log: snap.generation_log.clone(),
        };

        // 4. Reopen and assert byte-identical serialized state (SM-7).
        let reopened = read_bundle(&out).unwrap();
        assert_eq!(reopened, saved, "{name}: reopened model must equal saved model");
        assert_eq!(
            serialized_state(&reopened),
            serialized_state(&saved),
            "{name}: reopened model must serialize byte-for-byte identically (SM-7)"
        );

        std::fs::remove_dir_all(&out_root).unwrap();
    }
}

/// Move the first clip of the first non-empty track by +15 frames and give it a
/// fresh Linear opacity keyframe pair — the canonical edit the round-trip applies.
fn mutate_first_clip(timeline: &mut Timeline) {
    for track in &mut timeline.tracks {
        if let Some(clip) = track.clips.first_mut() {
            clip.start_frame += 15;
            let mut op = clip.opacity_track.take().unwrap_or_else(KeyframeTrack::new);
            op.upsert(Keyframe::with_interpolation(
                0,
                1.0_f64,
                palmier_model::Interpolation::Linear,
            ));
            op.upsert(Keyframe::with_interpolation(
                clip.duration_frames,
                0.0_f64,
                palmier_model::Interpolation::Linear,
            ));
            clip.opacity_track = Some(op);
            return;
        }
    }
}

#[test]
fn golden_fixtures_are_byte_stable_on_resave() {
    maybe_regenerate_goldens();
    let dir = fixtures_dir();

    for (name, _) in all_fixtures() {
        let bundle = dir.join(format!("{name}.palmier"));
        assert!(
            bundle.join(project::TIMELINE_FILE).exists(),
            "missing golden fixture {name} — run with PALMIER_UPDATE_GOLDEN=1"
        );

        // The committed bytes.
        let committed_project = std::fs::read(bundle.join(project::TIMELINE_FILE)).unwrap();
        let committed_manifest = std::fs::read(bundle.join(project::MANIFEST_FILE)).ok();
        let committed_log = std::fs::read(bundle.join(project::GENERATION_LOG_FILE)).ok();

        // Load and re-save unchanged into a scratch bundle.
        let loaded = read_bundle(&bundle).unwrap();
        let mut snap = BundleSnapshot::new(loaded.timeline);
        snap.manifest = loaded.manifest;
        snap.generation_log = loaded.generation_log;
        let out_root = scratch();
        let out = out_root.join(format!("{name}.palmier"));
        write_bundle(&out, &snap).unwrap();

        // Re-saved bytes must match the committed bytes exactly (golden gate).
        assert_eq!(
            std::fs::read(out.join(project::TIMELINE_FILE)).unwrap(),
            committed_project,
            "{name}: project.json drifted from the golden — re-run PALMIER_UPDATE_GOLDEN=1 if intentional"
        );
        assert_eq!(
            std::fs::read(out.join(project::MANIFEST_FILE)).ok(),
            committed_manifest,
            "{name}: media.json drifted from the golden"
        );
        assert_eq!(
            std::fs::read(out.join(project::GENERATION_LOG_FILE)).ok(),
            committed_log,
            "{name}: generation-log.json drifted from the golden"
        );

        std::fs::remove_dir_all(&out_root).unwrap();
    }
}

/// SM-1b: opening an existing 30-clip 1080p project completes well under 1 s on
/// the §10 reference HW. We synthesize the 30-clip bundle (no golden needed — it's
/// a performance seed, not a fidelity fixture), save it, then time the open.
///
/// The 1 s budget is the §10 acceptance; on dev HW the open is sub-millisecond, so
/// the gate is "the timing test exists and passes". The HW assumption is asserted
/// in the message, not pinned to a machine.
#[test]
fn open_30_clip_1080p_project_is_fast() {
    let mut timeline = Timeline::new();
    timeline.fps = 30;
    timeline.width = 1920;
    timeline.height = 1080;
    timeline.settings_configured = true;

    let mut track = Track::new(ClipType::Video);
    track.id = "track-30".into();
    for i in 0..30 {
        let mut c = Clip::new(format!("asset-{i}"), i * 90, 90);
        c.id = format!("clip-{i}");
        // Give every clip a small animation so the open does real decode work.
        let mut pos = KeyframeTrack::new();
        pos.upsert(Keyframe::new(0, AnimPair::new(0.0, 0.0)));
        pos.upsert(Keyframe::new(90, AnimPair::new(1.0, 1.0)));
        c.position_track = Some(pos);
        track.clips.push(c);
    }
    timeline.tracks.push(track);

    let root = scratch();
    let bundle = root.join("Thirty.palmier");
    write_bundle(&bundle, &BundleSnapshot::new(timeline)).unwrap();

    let start = Instant::now();
    let loaded = read_bundle(&bundle).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(loaded.timeline.tracks[0].clips.len(), 30);
    assert!(
        elapsed.as_secs_f64() < 1.0,
        "SM-1b: 30-clip 1080p open took {elapsed:?}, budget is < 1 s (§10 HW)"
    );

    std::fs::remove_dir_all(&root).unwrap();
}
