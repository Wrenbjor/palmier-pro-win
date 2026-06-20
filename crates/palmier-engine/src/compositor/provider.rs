//! The frame-resolution seam the compositor draws through — E5-S8.
//!
//! The compositor must turn a layer's [`FrameRef`](crate::FrameRef) into decoded
//! pixels. In production that is `palmier-media`'s
//! [`FrameSource`](palmier_media::FrameSource) (the one-decode-owner; the engine
//! never opens FFmpeg). We abstract it behind a tiny [`FrameProvider`] trait for two
//! reasons:
//! 1. it keeps the compositor's GPU code from hard-depending on the concrete
//!    `FrameSource` shape, and
//! 2. it lets the headless smoke test inject a **pre-seeded** provider so the
//!    textured-quad draw is exercised without a real media file (we can't seed
//!    `FrameSource`'s private cache from outside `palmier-media`).
//!
//! The blanket impl wires `FrameSource` in for free, so callers pass their existing
//! `FrameSource` unchanged.

use palmier_media::{DecodedFrame, FrameSource, SeekMode};

/// Resolves a `(media_ref, source_frame)` to a decoded CPU frame for the compositor.
pub trait FrameProvider {
    /// Error type surfaced when a frame can't be produced (offline source, decode
    /// failure). The compositor skips the layer on `Err` rather than failing the
    /// whole frame.
    type Error: std::fmt::Debug;

    /// Resolve the decoded frame at `(media_ref, source_frame)`. `active_layers`
    /// feeds the scrub tolerance for `InteractiveScrub` (the compositor passes the
    /// frame's layer count); the compositor itself always asks `Exact` for a
    /// precise present.
    fn provide_frame(
        &self,
        media_ref: &str,
        source_frame: u64,
        mode: SeekMode,
        active_layers: u32,
    ) -> Result<DecodedFrame, Self::Error>;
}

impl FrameProvider for FrameSource {
    type Error = palmier_media::DecodeError;

    fn provide_frame(
        &self,
        media_ref: &str,
        source_frame: u64,
        mode: SeekMode,
        active_layers: u32,
    ) -> Result<DecodedFrame, Self::Error> {
        self.request_frame(media_ref, source_frame, mode, active_layers)
            .map(|res| res.frame)
    }
}
