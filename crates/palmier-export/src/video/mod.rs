//! Video export pipeline — E6-S5 (FR-21, FOUNDATION §6.12).
//!
//! Renders a project's composition to a real media file via FFmpeg: per output
//! frame, build the composition (the **same** `palmier-engine` frame builder the
//! preview uses) → render to an offscreen wgpu texture → read back RGBA → feed an
//! FFmpeg encoder; mix audio (the `palmier-engine` audio mixer) → AAC; mux +
//! finalize. Reproduces `ExportService.makeExportSession` semantics on the
//! Windows/Linux stack.
//!
//! ## Module split (pure vs. GPU/FFmpeg)
//!
//! The load-bearing **decisions** are pure and always compiled so they unit-test
//! without a GPU or FFmpeg:
//! - [`spec`] — [`ExportFormat`] / [`ExportResolution`], the even-snapped
//!   [`render_size`](spec::render_size), the frame-count math, the BT.709 color
//!   tags, and the container/extension per format.
//! - [`encoder`] — the **HW-encoder selection + fallback chain** (NVENC → QSV →
//!   AMF → MediaFoundation → `prores_ks`) and the codec/muxer config. The
//!   *selection logic* is pure (a probe list → an ordered plan); the actual
//!   FFmpeg `find_encoder` probe is behind the `gpu-export` feature.
//!
//! The actual encode lives behind the **`gpu-export`** feature (it pulls
//! `ffmpeg-next` + the engine's `wgpu-compositor`):
//! - [`render`] (feature `gpu-export`) — the per-frame
//!   build→render→readback→encode loop, the audio→AAC mux, progress, and
//!   cancellation.
//!
//! ## LGPL encoder constraint (docs/windows-harness-notes.md)
//!
//! The provisioned FFmpeg is the **LGPL** build, which **excludes libx264/libx265**
//! (GPL). Software H.264/H.265 encode is therefore **unavailable**; H.264/H.265
//! must go through a **hardware** encoder (NVENC / QSV / AMF / MediaFoundation). If
//! none is present, [`select_encoder`](encoder::select_encoder) yields
//! [`ExportError::NoHardwareEncoder`] with the chain it tried — never a silent
//! fall-through to a missing software encoder. **ProRes 422** (`prores_ks`) and all
//! decode are LGPL-clean, so the ProRes lane always works.

pub mod encoder;
pub mod spec;

#[cfg(feature = "gpu-export")]
pub mod render;

pub use encoder::{select_encoder, EncoderPlan, HwVendor, ENCODER_FALLBACK_H264, ENCODER_FALLBACK_H265};
pub use spec::{frame_count, render_size, ColorTags, ExportFormat, ExportResolution, BT709};

#[cfg(feature = "gpu-export")]
pub use render::{export_video, CancelFlag, VideoExportConfig, VideoExportOutcome};

/// Errors the video export pipeline can surface.
///
/// `Cancelled` is **not** a failure — it mirrors the reference's
/// `NSUserCancelledError`-as-cancel: a clean early return when the caller's
/// cancellation flag is set at a frame boundary.
#[derive(Debug)]
pub enum ExportError {
    /// H.264/H.265 was requested but **no** hardware encoder is available in this
    /// FFmpeg build (the LGPL build has no libx264/libx265 software fallback).
    /// Carries the ordered chain that was probed for diagnostics.
    NoHardwareEncoder {
        /// The codec family requested (`"H.264"` / `"H.265"`).
        codec: &'static str,
        /// The encoder names probed, in order.
        tried: Vec<&'static str>,
    },
    /// FFmpeg open / configure / encode / mux failure.
    Ffmpeg(String),
    /// The engine compositor could not be created (no GPU adapter on a headless
    /// box) — the GPU export lane is unavailable; the caller may fall back.
    NoGpu(String),
    /// I/O error (deleting the existing output, writing the file).
    Io(String),
    /// The export was cancelled at a frame boundary (clean cancel, not an error
    /// condition — the partial file is removed).
    Cancelled,
    /// The timeline produced zero output frames (nothing to encode).
    Empty,
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::NoHardwareEncoder { codec, tried } => write!(
                f,
                "{codec} export needs a hardware encoder, but none is available in this \
                 FFmpeg LGPL build (libx264/libx265 are GPL and excluded). Probed: {}. \
                 Install an NVIDIA/Intel/AMD GPU driver with the matching encoder, or \
                 export ProRes 422 (.mov) instead.",
                tried.join(", ")
            ),
            ExportError::Ffmpeg(m) => write!(f, "ffmpeg: {m}"),
            ExportError::NoGpu(m) => write!(f, "no GPU for export render: {m}"),
            ExportError::Io(m) => write!(f, "io: {m}"),
            ExportError::Cancelled => write!(f, "export cancelled"),
            ExportError::Empty => write!(f, "timeline produced no output frames"),
        }
    }
}

impl std::error::Error for ExportError {}

/// Alias re-exported at the crate root as `VideoExportError` to disambiguate
/// from the bundle export's `ExportError` (E6-S7), which the crate root already
/// re-exports under that name.
pub type VideoExportError = ExportError;
