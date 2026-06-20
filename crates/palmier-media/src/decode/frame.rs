//! Decoded-frame value types handed from `palmier-media` to `palmier-engine`.
//!
//! The decode pipeline (story E5-S2) produces **CPU-side decoded frames** —
//! YUV or RGBA planes plus the metadata the compositor needs. The GPU-texture
//! upload is deliberately **out of scope** here (it lands with the wgpu
//! compositor, E5-S8): the one-decode-owner contract (Glossary, FOUNDATION §4)
//! says `palmier-media` owns the decode + cache and hands frames to the engine,
//! which owns wgpu. So a [`DecodedFrame`] is a plain heap buffer the engine
//! later uploads — never a `wgpu::Texture`.

use std::sync::Arc;

/// Pixel layout of a [`DecodedFrame`]'s planes.
///
/// The reference forces a single BT.709 working space downstream (risk #5); the
/// decoder preserves the codec's native layout here and tags it, so the engine
/// can upload YUV directly (a YUV→RGB shader) or consume the RGBA fallback. We
/// keep the **planar** native formats (no swscale on the hot path when the
/// engine can sample YUV) plus an interleaved RGBA escape hatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelLayout {
    /// Planar Y, U, V (4:2:0). Three planes; U/V are half-resolution.
    Yuv420P,
    /// Planar Y, U, V (4:2:2). Three planes; U/V are half-width, full-height.
    Yuv422P,
    /// Planar Y, U, V (4:4:4). Three planes, all full-resolution.
    Yuv444P,
    /// Interleaved RGBA, 8 bits/channel, one plane (the universal fallback and
    /// the layout images/Lottie decode to). Premultiplied per `has_alpha`/risk #3
    /// downstream on upload.
    Rgba8,
}

impl PixelLayout {
    /// Number of populated planes for this layout.
    pub fn plane_count(self) -> usize {
        match self {
            PixelLayout::Rgba8 => 1,
            PixelLayout::Yuv420P | PixelLayout::Yuv422P | PixelLayout::Yuv444P => 3,
        }
    }

    /// True for the planar YUV layouts (engine uploads three textures + a
    /// YUV→RGB conversion); false for the interleaved RGBA fallback.
    pub fn is_planar_yuv(self) -> bool {
        !matches!(self, PixelLayout::Rgba8)
    }
}

/// One decoded plane: tightly-or-strided pixel bytes plus the row stride used to
/// pack them. For planar YUV the chroma planes are sub-sampled, so each plane
/// carries its own `width`/`height`/`stride`.
#[derive(Debug, Clone)]
pub struct Plane {
    /// Tightly-or-strided pixel bytes for this plane.
    pub bytes: Vec<u8>,
    /// Bytes per row (may exceed `width * bytes_per_pixel` due to alignment).
    pub stride: usize,
    /// Plane width in pixels (luma = frame width; chroma may be sub-sampled).
    pub width: u32,
    /// Plane height in pixels.
    pub height: u32,
}

impl Plane {
    /// Heap footprint of this plane's pixel buffer, in bytes.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}

/// A single decoded video frame addressable by `(media_ref, source_frame)`.
///
/// Cheap to clone — the planes live behind an [`Arc`] so the [`FrameCache`] and
/// the engine can share one allocation. `palmier-engine` consumes this and
/// uploads the planes to GPU textures (E5-S8); `palmier-media` never touches
/// wgpu.
///
/// [`FrameCache`]: crate::decode::FrameCache
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// Pixel layout of [`planes`](DecodedFrame::planes).
    pub layout: PixelLayout,
    /// Frame width in pixels (the coded/display width).
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Whether the source carries a real alpha channel — taken from the **codec
    /// / pixfmt** alpha flag only, never container capability (risk #3). The
    /// engine premultiplies on upload when this is set.
    pub has_alpha: bool,
    /// The decoded planes (1 for RGBA, 3 for planar YUV) behind a shared `Arc`.
    pub planes: Arc<Vec<Plane>>,
    /// Source frame index this frame represents (the cache key's frame component).
    pub source_frame: u64,
}

impl DecodedFrame {
    /// Total system-RAM footprint of this frame's decoded planes, in bytes.
    /// Used by the [`FrameCache`] to enforce the 512 MB RAM ceiling
    /// (FOUNDATION §6.5).
    ///
    /// [`FrameCache`]: crate::decode::FrameCache
    pub fn ram_bytes(&self) -> usize {
        self.planes.iter().map(Plane::byte_len).sum()
    }
}
