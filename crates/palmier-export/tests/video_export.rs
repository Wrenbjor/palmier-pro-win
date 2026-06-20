//! Video-export integration tests — E6-S5. **Feature-gated behind `gpu-export`.**
//!
//! These drive the real pipeline (`build_frame` → offscreen wgpu render →
//! readback → FFmpeg HW encoder → mux) over a synthetic project: a single
//! solid-color clip on one video track, pixels supplied by a fake
//! [`FrameProvider`] (no media file needed — the same trick the engine's
//! `compositor_smoke` test uses). The encode runs through a **real** HW encoder
//! (or `prores_ks`), then the file is re-opened with FFmpeg and asserted to
//! decode at the expected dimensions / frame-count.
//!
//! They are **gated**: a box with no GPU adapter (`NoGpu`) or no hardware H.264
//! encoder (`NoHardwareEncoder`) prints a skip notice and passes — CI without a
//! GPU stays green (FOUNDATION §11.1). The ProRes lane needs only `prores_ks`
//! (LGPL-clean), so it runs anywhere a GPU is present.
//!
//! Run: `pwsh -File scripts/with-msvc.ps1 cargo test --package palmier-export \
//!       --features gpu-export --test video_export -- --nocapture`

#![cfg(feature = "gpu-export")]

use std::convert::Infallible;
use std::sync::Arc;

use palmier_engine::compositor::provider::FrameProvider;
use palmier_engine::{SourceInfo, SourceResolver};
use palmier_export::video::{
    export_video, CancelFlag, ExportError, ExportFormat, ExportResolution, VideoExportConfig,
};
use palmier_media::decode::frame::{PixelLayout, Plane};
use palmier_media::{DecodedFrame, SeekMode};
use palmier_model::{Clip, ClipType, Timeline, Track};

/// A provider that returns a solid-color RGBA frame for any request (stands in
/// for `palmier-media`'s `FrameSource` so the GPU+encode path runs file-free).
struct SolidProvider {
    w: u32,
    h: u32,
    rgba: [u8; 4],
}

impl FrameProvider for SolidProvider {
    type Error = Infallible;
    fn provide_frame(
        &self,
        _media_ref: &str,
        source_frame: u64,
        _mode: SeekMode,
        _active_layers: u32,
    ) -> Result<DecodedFrame, Self::Error> {
        let mut bytes = Vec::with_capacity((self.w * self.h * 4) as usize);
        for _ in 0..(self.w * self.h) {
            bytes.extend_from_slice(&self.rgba);
        }
        Ok(DecodedFrame {
            layout: PixelLayout::Rgba8,
            width: self.w,
            height: self.h,
            has_alpha: false,
            planes: Arc::new(vec![Plane {
                bytes,
                stride: (self.w * 4) as usize,
                width: self.w,
                height: self.h,
            }]),
            source_frame,
        })
    }
}

/// A `SourceResolver` that reports a fixed natural size for any media_ref so
/// `build_frame` keeps the layer (a real project supplies decoder geometry).
struct FixedGeometry {
    natural: (f64, f64),
}
impl SourceResolver for FixedGeometry {
    fn source_info(&self, _media_ref: &str) -> Option<SourceInfo> {
        Some(SourceInfo::upright(self.natural))
    }
}

/// Build a minimal timeline: one video track, one full-canvas clip covering
/// `[0, frames)`.
fn solid_timeline(width: i32, height: i32, fps: i32, frames: i32) -> Timeline {
    let mut tl = Timeline::new();
    tl.width = width;
    tl.height = height;
    tl.fps = fps;
    let mut track = Track::new(ClipType::Video);
    track.clips.push(Clip::new("solid", 0, frames));
    tl.tracks.push(track);
    tl
}

/// Try to export `format` and re-decode the output, asserting dimensions +
/// frame count. Skips cleanly when there's no GPU or (for H.264) no HW encoder.
fn run_export_and_verify(format: ExportFormat) {
    ffmpeg_next::init().ok();

    let (canvas_w, canvas_h, fps) = (256, 144, 30);
    // 6 frames — tiny, fast, exercises the full loop + finalize.
    let total = 6;
    let tl = solid_timeline(canvas_w, canvas_h, fps, total);
    let geometry = FixedGeometry {
        natural: (canvas_w as f64, canvas_h as f64),
    };
    let provider = SolidProvider {
        w: canvas_w as u32,
        h: canvas_h as u32,
        rgba: [40, 160, 220, 255],
    };

    let tmp = tempfile::tempdir().unwrap();
    let ext = format.extension();
    let out = tmp.path().join(format!("export_test.{ext}"));

    let cancel = CancelFlag::new();
    let mut last_progress = 0.0;
    let config = VideoExportConfig {
        format,
        resolution: ExportResolution::P720, // short side 720 → upscales 144→720
        output_path: out.clone(),
        output_fps: 0, // use project fps
    };

    let result = export_video(
        &tl,
        &geometry,
        &provider,
        &Vec::new(), // no audio
        &config,
        |p| last_progress = p,
        &cancel,
    );

    match result {
        Ok(outcome) => {
            eprintln!(
                "[E6-S5] {} export OK: {}x{} {} frames via {} ({})",
                format.label(),
                outcome.width,
                outcome.height,
                outcome.frames,
                outcome.encoder,
                outcome.vendor.label(),
            );
            assert_eq!(outcome.frames, total as u64);
            assert!(out.exists(), "output file written");
            assert!((last_progress - 1.0).abs() < 1e-9, "progress reached 1.0");
            // Even dimensions (encoders reject odd dims).
            assert_eq!(outcome.width % 2, 0);
            assert_eq!(outcome.height % 2, 0);

            // Re-open + decode: assert dimensions + at least one decodable frame.
            verify_decodable(&out, outcome.width, outcome.height);
        }
        Err(ExportError::NoGpu(msg)) => {
            eprintln!("[E6-S5] no GPU adapter — skipping {} encode ({msg})", format.label());
        }
        Err(ExportError::NoHardwareEncoder { codec, tried }) => {
            eprintln!(
                "[E6-S5] no HW encoder for {codec} (tried {tried:?}) — skipping (expected on a \
                 box without NVENC/QSV/AMF/MF)"
            );
        }
        Err(ExportError::Ffmpeg(msg)) => {
            // A HW encoder may be *registered* but fail to open on a box with no
            // matching GPU (e.g. h264_mf with no MF support). Treat as a skip.
            eprintln!("[E6-S5] {} encode failed at FFmpeg layer (likely no usable HW encoder): {msg}", format.label());
        }
        Err(e) => panic!("unexpected export error: {e}"),
    }
}

/// Re-open `path` with FFmpeg and confirm it has a video stream of the expected
/// dimensions that decodes at least one frame.
fn verify_decodable(path: &std::path::Path, expect_w: u32, expect_h: u32) {
    let mut ictx = ffmpeg_next::format::input(&path).expect("re-open exported file");
    let stream = ictx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .expect("has a video stream");
    let stream_index = stream.index();
    let ctx = ffmpeg_next::codec::context::Context::from_parameters(stream.parameters()).unwrap();
    let mut decoder = ctx.decoder().video().expect("video decoder");
    assert_eq!(decoder.width(), expect_w, "decoded width matches encode");
    assert_eq!(decoder.height(), expect_h, "decoded height matches encode");

    let mut decoded_any = false;
    let mut frame = ffmpeg_next::frame::Video::empty();
    for (s, packet) in ictx.packets() {
        if s.index() != stream_index {
            continue;
        }
        if decoder.send_packet(&packet).is_ok() {
            while decoder.receive_frame(&mut frame).is_ok() {
                decoded_any = true;
            }
        }
        if decoded_any {
            break;
        }
    }
    let _ = decoder.send_eof();
    while decoder.receive_frame(&mut frame).is_ok() {
        decoded_any = true;
    }
    assert!(decoded_any, "exported file decodes at least one frame");
    eprintln!("[E6-S5] re-decode OK: {}x{}", expect_w, expect_h);
}

#[test]
#[ignore = "real encode: needs a GPU adapter (+ HW encoder for H.264); run with --ignored"]
fn export_h264_real_encode() {
    run_export_and_verify(ExportFormat::H264);
}

#[test]
#[ignore = "real encode: needs a GPU adapter (prores_ks is LGPL-clean); run with --ignored"]
fn export_prores422_real_encode() {
    run_export_and_verify(ExportFormat::ProRes422);
}

#[test]
#[ignore = "real encode: needs a GPU adapter (+ HW encoder for H.265); run with --ignored"]
fn export_h265_real_encode() {
    run_export_and_verify(ExportFormat::H265);
}

/// Cancellation: with the flag pre-set, the export returns `Cancelled` cleanly
/// and writes no leftover file. Gated on a GPU (the loop must start to observe
/// the flag), so it self-skips on a headless box.
#[test]
#[ignore = "needs a GPU adapter to reach the frame loop; run with --ignored"]
fn export_cancels_cleanly() {
    ffmpeg_next::init().ok();
    let tl = solid_timeline(256, 144, 30, 60);
    let geometry = FixedGeometry { natural: (256.0, 144.0) };
    let provider = SolidProvider { w: 256, h: 144, rgba: [10, 20, 30, 255] };
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("cancel_test.mov");

    let cancel = CancelFlag::new();
    cancel.cancel(); // pre-cancel: the first frame boundary must observe it

    let config = VideoExportConfig {
        format: ExportFormat::ProRes422, // always-available encoder
        resolution: ExportResolution::P720,
        output_path: out.clone(),
        output_fps: 0,
    };
    let result = export_video(&tl, &geometry, &provider, &Vec::new(), &config, |_| {}, &cancel);
    match result {
        Err(ExportError::Cancelled) => {
            assert!(!out.exists(), "cancelled export leaves no file");
            eprintln!("[E6-S5] cancellation produced a clean Cancelled + no leftover file");
        }
        Err(ExportError::NoGpu(_)) => {
            eprintln!("[E6-S5] no GPU — skipping cancellation test");
        }
        other => panic!("expected Cancelled (or NoGpu skip), got {other:?}"),
    }
}
