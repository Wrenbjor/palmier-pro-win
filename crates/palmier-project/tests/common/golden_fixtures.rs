//! Shared golden-fixture builders (story E2-S10).
//!
//! These functions construct the three canonical golden projects in memory. The
//! `round_trip.rs` integration test both (a) regenerates the committed
//! `tests/fixtures/golden_project_*.palmier/` bundles when
//! `PALMIER_UPDATE_GOLDEN=1` is set, and (b) loads them back and asserts
//! round-trip fidelity. Keeping the builders in one module means the on-disk
//! fixtures and the round-trip expectations can never drift.
//!
//! The three fixtures (FOUNDATION §11.5, consumed downstream by Epic 5 SM-C1 and
//! Epic 6 XMEML):
//! - `golden_project_minimal` — a representative bundle: two tracks
//!   (video + audio), a handful of plain clips, a small manifest + generation log.
//! - `golden_project_keyframes` — clips carrying Smooth / Linear / Hold keyframe
//!   tracks across multiple animatable properties (opacity, position, scale,
//!   rotation, crop, volume).
//! - `golden_project_text` — text clips with `text_content` + `text_style`.
//!
//! All builders are **deterministic**: fixed UUID strings, fixed frame numbers,
//! no timestamps, so the serialized JSON is byte-stable across runs (a golden
//! fixture must diff cleanly).

#![allow(dead_code)] // each test binary uses a subset.

use palmier_model::{
    AnimPair, Clip, ClipType, Crop, Fill, GenerationLog, GenerationLogEntry, Interpolation,
    Keyframe, KeyframeTrack, MediaManifest, MediaManifestEntry, MediaSource, Rgba, TextAlignment,
    TextStyle, Timeline, Track,
};
use palmier_project::BundleSnapshot;

/// A deterministic clip with a fixed id (so JSON is byte-stable).
fn clip(id: &str, media_ref: &str, start: i32, dur: i32) -> Clip {
    let mut c = Clip::new(media_ref, start, dur);
    c.id = id.to_string();
    c
}

/// A manifest entry for a project-internal media file.
fn project_entry(id: &str, name: &str, kind: ClipType, rel: &str, duration: f64) -> MediaManifestEntry {
    MediaManifestEntry {
        id: id.into(),
        name: name.into(),
        asset_type: kind,
        source: MediaSource::Project {
            relative_path: rel.into(),
        },
        duration,
        generation_input: None,
        source_width: Some(1920),
        source_height: Some(1080),
        source_fps: Some(30.0),
        has_audio: Some(kind == ClipType::Video || kind == ClipType::Audio),
        folder_id: None,
        cached_remote_url: None,
        cached_remote_url_expires_at: None,
    }
}

// ---- golden_project_minimal ----

/// A representative two-track project with plain clips, a small manifest, and a
/// generation-log entry. The base fixture Epic 5/6 build on.
pub fn golden_project_minimal() -> BundleSnapshot {
    let mut timeline = Timeline::new();
    timeline.fps = 30;
    timeline.width = 1920;
    timeline.height = 1080;
    timeline.settings_configured = true;

    let mut video = Track::new(ClipType::Video);
    video.id = "track-video".into();
    video.clips.push(clip("clip-v1", "asset-v1", 0, 90));
    video.clips.push(clip("clip-v2", "asset-v2", 90, 60));

    let mut audio = Track::new(ClipType::Audio);
    audio.id = "track-audio".into();
    let mut music = clip("clip-a1", "asset-a1", 0, 150);
    music.media_type = ClipType::Audio;
    music.source_clip_type = ClipType::Audio;
    music.volume = 0.8;
    audio.clips.push(music);

    timeline.tracks.push(video);
    timeline.tracks.push(audio);

    let mut manifest = MediaManifest::new();
    manifest.entries.push(project_entry(
        "asset-v1",
        "Intro",
        ClipType::Video,
        "media/intro.mov",
        3.0,
    ));
    manifest.entries.push(project_entry(
        "asset-v2",
        "Scene",
        ClipType::Video,
        "media/scene.mov",
        2.0,
    ));
    manifest.entries.push(project_entry(
        "asset-a1",
        "Track",
        ClipType::Audio,
        "media/track.m4a",
        5.0,
    ));

    let mut log = GenerationLog::new();
    log.entries.push(GenerationLogEntry {
        id: "log-1".into(),
        model: "veo-3".into(),
        cost_credits: Some(250),
        created_at: None, // deterministic: no timestamp
    });

    let mut snap = BundleSnapshot::new(timeline);
    snap.manifest = Some(manifest);
    snap.generation_log = Some(log);
    snap
}

// ---- golden_project_keyframes ----

fn kf<V>(frame: i32, value: V, interp: Interpolation) -> Keyframe<V> {
    Keyframe::with_interpolation(frame, value, interp)
}

/// Clips animated with Smooth / Linear / Hold keyframes across every animatable
/// property — the fidelity fixture for the sampling math (PRD §10 gate (a)).
pub fn golden_project_keyframes() -> BundleSnapshot {
    let mut timeline = Timeline::new();
    timeline.fps = 30;
    timeline.settings_configured = true;

    let mut track = Track::new(ClipType::Video);
    track.id = "track-anim".into();

    // Clip 1: opacity (Smooth) + position (Linear) + scale (Hold).
    let mut c1 = clip("clip-anim-1", "asset-v1", 0, 120);
    let mut opacity = KeyframeTrack::new();
    opacity.upsert(kf(0, 0.0_f64, Interpolation::Smooth));
    opacity.upsert(kf(60, 1.0, Interpolation::Smooth));
    opacity.upsert(kf(120, 0.0, Interpolation::Smooth));
    c1.opacity_track = Some(opacity);

    let mut position = KeyframeTrack::new();
    position.upsert(kf(0, AnimPair::new(0.0, 0.0), Interpolation::Linear));
    position.upsert(kf(120, AnimPair::new(0.5, 0.5), Interpolation::Linear));
    c1.position_track = Some(position);

    let mut scale = KeyframeTrack::new();
    scale.upsert(kf(0, AnimPair::new(0.5, 0.5), Interpolation::Hold));
    scale.upsert(kf(60, AnimPair::new(1.0, 1.0), Interpolation::Hold));
    c1.scale_track = Some(scale);
    track.clips.push(c1);

    // Clip 2: rotation (Linear) + crop (Smooth) + volume (Hold, dB values).
    let mut c2 = clip("clip-anim-2", "asset-v2", 120, 120);
    let mut rotation = KeyframeTrack::new();
    rotation.upsert(kf(0, 0.0_f64, Interpolation::Linear));
    rotation.upsert(kf(120, 90.0, Interpolation::Linear));
    c2.rotation_track = Some(rotation);

    let mut crop = KeyframeTrack::new();
    crop.upsert(kf(0, Crop::default(), Interpolation::Smooth));
    crop.upsert(kf(
        120,
        Crop {
            left: 0.1,
            top: 0.1,
            right: 0.1,
            bottom: 0.1,
        },
        Interpolation::Smooth,
    ));
    c2.crop_track = Some(crop);

    let mut volume = KeyframeTrack::new();
    volume.upsert(kf(0, -60.0_f64, Interpolation::Hold)); // dB
    volume.upsert(kf(60, 0.0, Interpolation::Hold));
    volume.upsert(kf(120, 6.0, Interpolation::Hold));
    c2.volume_track = Some(volume);
    track.clips.push(c2);

    timeline.tracks.push(track);

    let mut manifest = MediaManifest::new();
    manifest.entries.push(project_entry(
        "asset-v1",
        "Anim A",
        ClipType::Video,
        "media/a.mov",
        4.0,
    ));
    manifest.entries.push(project_entry(
        "asset-v2",
        "Anim B",
        ClipType::Video,
        "media/b.mov",
        4.0,
    ));

    let mut snap = BundleSnapshot::new(timeline);
    snap.manifest = Some(manifest);
    snap
}

// ---- golden_project_text ----

/// Text clips carrying `text_content` + a non-default `text_style` — the fixture
/// exercising the text shapes (consumed by Epic 5 text rendering).
pub fn golden_project_text() -> BundleSnapshot {
    let mut timeline = Timeline::new();
    timeline.fps = 30;
    timeline.settings_configured = true;

    let mut track = Track::new(ClipType::Text);
    track.id = "track-text".into();

    let mut title = clip("clip-title", "asset-title", 0, 90);
    title.media_type = ClipType::Text;
    title.source_clip_type = ClipType::Text;
    title.text_content = Some("Palmier Pro".into());
    title.text_style = Some(TextStyle {
        font_name: palmier_model::FontName::from_str("Helvetica-Bold"),
        font_size: 120.0,
        font_scale: 1.0,
        color: Rgba::new(1.0, 1.0, 1.0, 1.0),
        alignment: TextAlignment::Center,
        shadow: palmier_model::Shadow::default(),
        background: Fill {
            enabled: true,
            color: Rgba::new(0.0, 0.0, 0.0, 0.6),
        },
        border: Fill::default(),
    });
    track.clips.push(title);

    let mut lower = clip("clip-lower", "asset-lower", 90, 90);
    lower.media_type = ClipType::Text;
    lower.source_clip_type = ClipType::Text;
    lower.text_content = Some("Lower third".into());
    lower.text_style = Some(TextStyle {
        font_size: 48.0,
        alignment: TextAlignment::Left,
        ..TextStyle::default()
    });
    track.clips.push(lower);

    timeline.tracks.push(track);

    let mut manifest = MediaManifest::new();
    // Text clips are synthetic; entries record them as project assets for the
    // manifest round-trip (the reference stores text assets too).
    manifest.entries.push(project_entry(
        "asset-title",
        "Title",
        ClipType::Text,
        "media/title.txt",
        3.0,
    ));
    manifest.entries.push(project_entry(
        "asset-lower",
        "Lower",
        ClipType::Text,
        "media/lower.txt",
        3.0,
    ));

    let mut snap = BundleSnapshot::new(timeline);
    snap.manifest = Some(manifest);
    snap
}

/// The three fixtures by `(name, builder)` — the round-trip test iterates these.
pub fn all_fixtures() -> Vec<(&'static str, BundleSnapshot)> {
    vec![
        ("golden_project_minimal", golden_project_minimal()),
        ("golden_project_keyframes", golden_project_keyframes()),
        ("golden_project_text", golden_project_text()),
    ]
}
