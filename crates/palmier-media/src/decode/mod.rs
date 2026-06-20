//! Preview **decode pipeline** — the root of the preview/playback frame source
//! (story E5-S2, FOUNDATION §6.5, `docs/reference/preview-engine.md`).
//!
//! This is the `palmier-media` half of the one-decode-owner contract (Glossary,
//! FOUNDATION §4): `palmier-media` opens FFmpeg, decodes, and caches frames;
//! `palmier-engine` consumes decoded frames through a handle and **never opens
//! an `AVFormatContext`**. The GPU-texture upload is the engine's job (E5-S8) —
//! here we produce **CPU-side decoded frames** (YUV/RGBA planes + metadata).
//!
//! ## Pieces
//! * [`frame`] — the [`DecodedFrame`] value type (planes + metadata) handed to
//!   the engine. Cheap to clone (`Arc`-backed planes).
//! * [`decoder`] — the FFmpeg [`Decoder`]: one `AVFormatContext`+`AVCodecContext`
//!   per source URL, **HW decode when available** (d3d11va/dxva2 on Windows,
//!   vaapi on Linux) with a transparent **CPU fallback** ([`HwDecodeStatus`]).
//! * [`cache`] — the [`FrameCache`]: an in-RAM LRU keyed by `(media_ref,
//!   source_frame)`, **evicting by distance from the playhead** under the
//!   **512 MB system-RAM ceiling** (FOUNDATION §6.5; texture/VRAM accounting is
//!   the engine's).
//! * [`seek`] — [`SeekMode`] (`Exact`/`InteractiveScrub`) plus the reference's
//!   tolerance/throttle math (`min(0.75, 0.15*activeLayerCount)` s, 1/30 s
//!   throttle), ported from `VideoEngine.swift`.
//! * [`source`] — [`FrameSource`], the engine-facing handle wiring pool + cache:
//!   `request_frame(media_ref, source_frame, mode)` → frame or nearest+pending,
//!   plus `prefetch` and `cache_stats`.
//!
//! ## Engine-facing API (what `palmier-engine` consumes)
//! ```ignore
//! let source = FrameSource::new(resolver);            // resolver: media_ref → URL
//! source.set_playhead("clipA", current_source_frame); // keep the hot window centered
//! let res = source.request_frame("clipA", 120, SeekMode::Exact, active_layers)?;
//! // res.frame: DecodedFrame (CPU planes); res.pending: precise decode queued?
//! source.prefetch("clipA", 121)?;                     // warm ahead of the playhead
//! let stats = source.cache_stats();                   // frame_count / ram_bytes / hits
//! ```

pub mod cache;
pub mod decoder;
pub mod frame;
pub mod seek;
pub mod source;

pub use cache::{CacheStats, FrameCache, FrameKey, DEFAULT_RAM_CEILING_BYTES};
pub use decoder::{DecodeError, Decoder, HwDecodeStatus, HwKind};
pub use frame::{DecodedFrame, PixelLayout, Plane};
pub use seek::{
    interactive_tolerance_frames, interactive_tolerance_secs, ScrubThrottle, SeekMode,
    SCRUB_THROTTLE, SCRUB_TOLERANCE_CAP_SECS,
};
pub use source::{DecoderPool, FrameResult, FrameSource, UrlResolver};
