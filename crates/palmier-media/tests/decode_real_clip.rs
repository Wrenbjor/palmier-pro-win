//! Integration test for the preview decode path against a REAL clip.
//!
//! This reproduces the production black-preview bug: the compositor calls
//! `FrameSource::request_frame(media_ref, 0, Exact, 1)` and it must return a
//! decoded frame, not `Err(Ffmpeg("Invalid data found when processing input"))`.
//!
//! The clip is generated on the fly with the FFmpeg CLI (matching how the app's
//! clips are produced: `testsrc2` + `libopenh264`, h264/yuv420p/1920x1080). If
//! FFmpeg isn't on PATH the test self-skips, but in the CI/dev MSVC+ffmpeg-env
//! wrapper it runs for real and is the GREEN gate for the decode fix.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use palmier_media::{DecodedFrame, FrameSource, SeekMode, UrlResolver};

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

fn resolver_to(media_ref: &'static str, path: PathBuf) -> UrlResolver {
    Arc::new(move |r: &str| if r == media_ref { Some(path.clone()) } else { None })
}

fn assert_real_frame(frame: &DecodedFrame, w: u32, h: u32) {
    assert_eq!(frame.width, w, "decoded width");
    assert_eq!(frame.height, h, "decoded height");
    assert_eq!(frame.source_frame, 0, "carries the requested source frame");
    assert!(!frame.planes.is_empty(), "frame has planes");
    let total: usize = frame.planes.iter().map(|p| p.bytes.len()).sum();
    assert!(total > 0, "frame planes carry pixel bytes");
    // testsrc2 is a vivid color-bars pattern — at least one plane must have
    // some non-zero pixels (a black/empty frame would fail the real bug check).
    let any_nonzero = frame
        .planes
        .iter()
        .any(|p| p.bytes.iter().any(|&b| b != 0));
    assert!(any_nonzero, "decoded frame has non-zero pixels (not black)");
}

/// The headline reproduction: full-HD h264 clip, request frame 0 Exact.
#[test]
fn decodes_first_frame_of_real_h264_clip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(clip) = make_clip(
        dir.path(),
        "clip.mp4",
        &[
            "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30:duration=4",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=4",
            "-c:v", "libopenh264", "-b:v", "3M", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-shortest",
        ],
    ) else {
        eprintln!("ffmpeg not available — skipping real-clip decode test");
        return;
    };

    let src = FrameSource::new(resolver_to("clip", clip));
    let res = src
        .request_frame("clip", 0, SeekMode::Exact, 1)
        .expect("decode frame 0 of a valid h264 clip must succeed");
    assert!(!res.pending, "Exact result is never pending");
    assert_real_frame(&res.frame, 1920, 1080);
}

/// A tiny testsrc clip — small dimensions, same decode path.
#[test]
fn decodes_first_frame_of_tiny_testsrc_clip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(clip) = make_clip(
        dir.path(),
        "tiny.mp4",
        &[
            "-f", "lavfi", "-i", "testsrc=size=320x240:rate=30:duration=2",
            "-c:v", "libopenh264", "-b:v", "1M", "-pix_fmt", "yuv420p",
        ],
    ) else {
        eprintln!("ffmpeg not available — skipping tiny-clip decode test");
        return;
    };

    let src = FrameSource::new(resolver_to("tiny", clip));
    let res = src
        .request_frame("tiny", 0, SeekMode::Exact, 1)
        .expect("decode frame 0 of a tiny clip must succeed");
    assert_real_frame(&res.frame, 320, 240);
}

/// A non-zero frame index exercises the seek-to-keyframe-then-decode-forward
/// path (the production scrub/playhead case).
#[test]
fn decodes_a_mid_frame_via_seek() {
    let dir = tempfile::tempdir().expect("tempdir");
    let Some(clip) = make_clip(
        dir.path(),
        "mid.mp4",
        &[
            "-f", "lavfi", "-i", "testsrc=size=320x240:rate=30:duration=4",
            "-c:v", "libopenh264", "-b:v", "1M", "-pix_fmt", "yuv420p",
        ],
    ) else {
        eprintln!("ffmpeg not available — skipping mid-frame decode test");
        return;
    };

    let src = FrameSource::new(resolver_to("mid", clip));
    let res = src
        .request_frame("mid", 45, SeekMode::Exact, 1)
        .expect("decode a mid frame via seek must succeed");
    assert_eq!(res.frame.source_frame, 45);
    assert!(res.frame.width == 320 && res.frame.height == 240);
    let total: usize = res.frame.planes.iter().map(|p| p.bytes.len()).sum();
    assert!(total > 0, "mid frame carries pixels");
}
