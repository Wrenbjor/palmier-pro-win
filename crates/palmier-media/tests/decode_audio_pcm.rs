//! Integration test for the AUDIO PCM decode path against a REAL generated clip.
//!
//! Proves the audio-playback pipeline's decode stage: a clip with an audio stream
//! decodes to non-empty interleaved stereo f32 PCM at the project rate (48 kHz). The
//! clip is generated on the fly with the FFmpeg CLI (a `sine` tone muxed into an mp4,
//! matching how `decode_real_clip.rs` produces fixtures). If FFmpeg isn't on PATH the
//! test self-skips, but under the CI/dev MSVC+ffmpeg-env wrapper it runs for real and is
//! the GREEN gate for the audio decode.

use std::path::{Path, PathBuf};
use std::process::Command;

use palmier_media::{decode_audio_pcm, AudioPcmCache, TARGET_CHANNELS, TARGET_SAMPLE_RATE_HZ};

/// Build a clip with the FFmpeg CLI; returns its path, or `None` if ffmpeg is
/// unavailable (test self-skips).
fn make_clip(dir: &Path, name: &str, args: &[&str]) -> Option<PathBuf> {
    let out = dir.join(name);
    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y");
    cmd.args(args);
    cmd.arg(&out);
    let status = cmd.status().ok()?;
    if status.success() && out.exists() {
        Some(out)
    } else {
        None
    }
}

/// A 2-second 440 Hz stereo sine in an mp4 (aac) → must decode to non-empty 48 kHz
/// stereo PCM with audible (non-silent) content.
#[test]
fn decodes_audio_of_real_clip_to_stereo_48k() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(clip) = make_clip(
        dir.path(),
        "tone.mp4",
        &[
            "-f", "lavfi", "-i", "sine=frequency=440:duration=2:sample_rate=44100",
            "-ac", "2", "-c:a", "aac",
        ],
    ) else {
        eprintln!("ffmpeg not available — skipping real-clip audio decode test");
        return;
    };

    let decoded = decode_audio_pcm(&clip).expect("audio decodes");

    // Stereo at the project rate.
    assert_eq!(decoded.channels, TARGET_CHANNELS, "decoded to stereo");
    assert_eq!(decoded.sample_rate, TARGET_SAMPLE_RATE_HZ, "resampled to 48 kHz");

    // Non-empty: ~2 s at 48 kHz ≈ 96000 sample-frames (allow generous slack for codec
    // priming/trailing). The headline assertion: the buffer is NOT empty.
    assert!(!decoded.interleaved.is_empty(), "decoded PCM is non-empty");
    let frames = decoded.frame_count();
    assert!(
        frames > 48_000,
        "≈2 s of audio yields > 1 s of samples; got {frames} frames"
    );

    // A 440 Hz tone is clearly NOT silence — at least one sample is well above zero.
    let any_loud = decoded.interleaved.iter().any(|&s| s.abs() > 0.05);
    assert!(any_loud, "decoded a real (non-silent) tone");
}

/// The cache decodes once and serves the same `Arc` on the second request.
#[test]
fn cache_serves_decoded_audio_and_reuses() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(clip) = make_clip(
        dir.path(),
        "tone.wav",
        &[
            "-f", "lavfi", "-i", "sine=frequency=220:duration=1:sample_rate=48000",
            "-ac", "1", "-c:a", "pcm_s16le",
        ],
    ) else {
        eprintln!("ffmpeg not available — skipping audio cache test");
        return;
    };

    let cache = AudioPcmCache::new();
    let first = cache.get(&clip).expect("decodes via cache");
    let second = cache.get(&clip).expect("cache hit");
    // Same allocation (Arc reused, not re-decoded).
    assert!(std::sync::Arc::ptr_eq(&first, &second), "cache reused the decode");
    // Mono source duplicated to stereo by the decode adapter.
    assert_eq!(first.channels, TARGET_CHANNELS);
    assert!(!first.interleaved.is_empty());
}
