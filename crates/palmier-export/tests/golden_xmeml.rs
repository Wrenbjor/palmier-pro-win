//! Golden XMEML byte-fidelity gate (E6-S1..S4, SM-7 / FOUNDATION §11.3 / §905).
//!
//! Each test builds a fixture [`Timeline`] + manifest, emits XMEML via the
//! production [`palmier_export::export_xmeml`], and asserts the output is
//! **byte-exact** against a committed golden under `tests/fixtures/`.
//!
//! Regenerate the goldens with `UPDATE_GOLDEN=1` set (review-gated — any diff
//! blocks merge, R-5):
//!   `UPDATE_GOLDEN=1 cargo test -p palmier-export --test golden_xmeml`
//! The goldens committed here were generated from this emitter and reviewed for
//! structural correctness against docs/reference/export.md §B.

use std::path::PathBuf;

use palmier_export::{export_xmeml, ManifestResolver};
use palmier_model::{
    AnimPair, Clip, ClipType, Crop, Interpolation, Keyframe, KeyframeTrack, MediaManifest,
    MediaManifestEntry, MediaSource, Timeline, Track, Transform,
};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Assert `actual` equals the committed golden `name`, or write it when
/// `UPDATE_GOLDEN` is set. Compares bytes exactly (LF-normalized authoring).
fn assert_golden(name: &str, actual: &str) {
    let path = fixtures_dir().join(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, actual.as_bytes()).unwrap();
        eprintln!("updated golden: {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("missing golden {} ({e}); run with UPDATE_GOLDEN=1", path.display()));
    // Normalize any CRLF the VCS/editor may have introduced on Windows; the
    // emitter only ever produces LF.
    let expected = expected.replace("\r\n", "\n");
    assert_eq!(
        actual, expected,
        "golden mismatch for {name} — emitter output diverged from the committed XML"
    );
}

// ---- Fixture builders ----

fn video_entry(id: &str, name: &str, path: &str, fps: f64, w: i32, h: i32, dur: f64) -> MediaManifestEntry {
    MediaManifestEntry {
        id: id.into(),
        name: name.into(),
        asset_type: ClipType::Video,
        source: MediaSource::External { absolute_path: path.into() },
        duration: dur,
        generation_input: None,
        source_width: Some(w),
        source_height: Some(h),
        source_fps: Some(fps),
        has_audio: Some(true),
        folder_id: None,
        cached_remote_url: None,
        cached_remote_url_expires_at: None,
    }
}

fn audio_entry(id: &str, name: &str, path: &str, dur: f64) -> MediaManifestEntry {
    MediaManifestEntry {
        id: id.into(),
        name: name.into(),
        asset_type: ClipType::Audio,
        source: MediaSource::External { absolute_path: path.into() },
        duration: dur,
        generation_input: None,
        source_width: None,
        source_height: None,
        source_fps: None,
        has_audio: Some(true),
        folder_id: None,
        cached_remote_url: None,
        cached_remote_url_expires_at: None,
    }
}

/// A timeline with one video track holding a single 4-second clip with a fixed id
/// (so the golden is stable).
#[test]
fn golden_minimal() {
    let mut manifest = MediaManifest::new();
    manifest.entries.push(video_entry(
        "media-a",
        "ClipA.mov",
        "/Users/test/Footage/ClipA.mov",
        30.0,
        1920,
        1080,
        4.0,
    ));
    let resolver = ManifestResolver::new(manifest);

    let mut tl = Timeline::new(); // 30 fps, 1920x1080
    let mut v = Track::new(ClipType::Video);
    let mut clip = Clip::new("media-a", 0, 120);
    clip.id = "clip-a".into();
    v.clips.push(clip);
    tl.tracks.push(v);

    assert_golden("golden_xmeml_minimal.xml", &export_xmeml(&tl, &resolver));
}

/// A clip exercising every static filter: speed (time remap), scaled+rotated+
/// off-center transform (basic motion), crop, opacity, plus keyframed transform/
/// opacity/crop on a SECOND clip.
#[test]
fn golden_keyframes() {
    let mut manifest = MediaManifest::new();
    manifest.entries.push(video_entry(
        "media-a",
        "ClipA.mov",
        "/Users/test/Footage/ClipA.mov",
        29.97, // NTSC source → ntsc TRUE, DF timecode
        1280,
        720,
        4.0,
    ));
    let resolver = ManifestResolver::new(manifest);

    let mut tl = Timeline::new();
    let mut v = Track::new(ClipType::Video);

    // Static-filter clip: speed 2x, scaled/rotated/off-center, cropped, 50% opacity.
    let mut a = Clip::new("media-a", 0, 60);
    a.id = "clip-static".into();
    a.speed = 2.0;
    a.opacity = 0.5;
    a.transform = Transform {
        center_x: 0.6,
        center_y: 0.4,
        width: 0.8,
        height: 0.8,
        rotation: 15.0,
        flip_horizontal: false,
        flip_vertical: false,
    };
    a.crop = Crop { left: 0.1, top: 0.05, right: 0.1, bottom: 0.0 };
    v.clips.push(a);

    // Keyframed clip: position + scale + rotation + opacity + crop tracks.
    let mut b = Clip::new("media-a", 60, 60);
    b.id = "clip-kf".into();

    let mut pos = KeyframeTrack::new();
    pos.upsert(Keyframe::with_interpolation(0, AnimPair::new(0.0, 0.0), Interpolation::Linear));
    pos.upsert(Keyframe::with_interpolation(30, AnimPair::new(0.2, 0.1), Interpolation::Linear));
    b.position_track = Some(pos);

    let mut scale = KeyframeTrack::new();
    scale.upsert(Keyframe::with_interpolation(0, AnimPair::new(1.0, 1.0), Interpolation::Linear));
    scale.upsert(Keyframe::with_interpolation(30, AnimPair::new(0.5, 0.5), Interpolation::Linear));
    b.scale_track = Some(scale);

    let mut rot = KeyframeTrack::new();
    rot.upsert(Keyframe::with_interpolation(0, 0.0, Interpolation::Linear));
    rot.upsert(Keyframe::with_interpolation(30, 90.0, Interpolation::Linear));
    b.rotation_track = Some(rot);

    let mut op = KeyframeTrack::new();
    op.upsert(Keyframe::with_interpolation(0, 0.0, Interpolation::Linear));
    op.upsert(Keyframe::with_interpolation(30, 1.0, Interpolation::Linear));
    b.opacity_track = Some(op);

    let mut crop = KeyframeTrack::new();
    crop.upsert(Keyframe::with_interpolation(0, Crop::default(), Interpolation::Linear));
    crop.upsert(Keyframe::with_interpolation(
        30,
        Crop { left: 0.25, top: 0.0, right: 0.0, bottom: 0.0 },
        Interpolation::Linear,
    ));
    b.crop_track = Some(crop);

    v.clips.push(b);
    tl.tracks.push(v);

    assert_golden("golden_xmeml_keyframes.xml", &export_xmeml(&tl, &resolver));
}

/// Two video tracks (reversal check) + one audio track (natural order), fades on
/// the video clip, a linked A/V pair, and audio levels — the full feature golden.
#[test]
fn golden_text_and_links() {
    let mut manifest = MediaManifest::new();
    manifest.entries.push(video_entry(
        "media-a",
        "ClipA.mov",
        "/Users/test/Footage/ClipA.mov",
        30.0,
        1920,
        1080,
        4.0,
    ));
    manifest.entries.push(video_entry(
        "media-b",
        "Overlay.mov",
        "/Users/test/Footage/Overlay.mov",
        30.0,
        1920,
        1080,
        4.0,
    ));
    manifest
        .entries
        .push(audio_entry("media-c", "Music.wav", "/Users/test/Audio/Music.wav", 4.0));
    let resolver = ManifestResolver::new(manifest);

    let mut tl = Timeline::new();

    // Top video track (model order 0) → emitted SECOND (reversed).
    let mut v_top = Track::new(ClipType::Video);
    let mut top_clip = Clip::new("media-b", 0, 60);
    top_clip.id = "clip-top".into();
    v_top.clips.push(top_clip);
    tl.tracks.push(v_top);

    // Bottom video track (model order 1) → emitted FIRST. Has fades + an A/V link
    // partner on the audio track below.
    let mut v_bot = Track::new(ClipType::Video);
    let mut bot_clip = Clip::new("media-a", 0, 120);
    bot_clip.id = "clip-bot".into();
    bot_clip.fade_in_frames = 10;
    bot_clip.fade_out_frames = 15;
    bot_clip.link_group_id = Some("link-1".into());
    v_bot.clips.push(bot_clip);
    tl.tracks.push(v_bot);

    // Audio track (natural order). Linked to the bottom video clip; has volume.
    let mut a_track = Track::new(ClipType::Audio);
    let mut a_clip = Clip::new("media-c", 0, 120);
    a_clip.id = "clip-audio".into();
    a_clip.volume = 0.5;
    a_clip.link_group_id = Some("link-1".into());
    a_track.clips.push(a_clip);
    tl.tracks.push(a_track);

    assert_golden("golden_xmeml_text.xml", &export_xmeml(&tl, &resolver));
}
