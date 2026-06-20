//! # palmier-engine
//!
//! Composition graph, wgpu compositor, and playback transport (FOUNDATION §4, §6.5).
//! Builds a per-frame `CompositionFrame` from the `palmier-model` timeline, decodes
//! source frames via `palmier-media`, and renders via wgpu. Heavy GPU/audio deps
//! (wgpu, cpal, rubato) are added per-story, not in this skeleton.
//!
//! ## Landed stories
//! - **E5-S6** — [`audio`] mixer: symphonia decode → rubato resample 48 kHz → speed
//!   time-stretch → per-frame volume/fade envelope → sum → cpal. Presentation-agnostic
//!   (no wgpu); the live cpal device sits behind the `audio-device` feature.
//! - **E5-S3 / E5-S4** — [`composition`] graph: per-frame [`CompositionFrame`] build
//!   from the timeline (z-order, overlap precedence, clip→source-frame mapping) +
//!   per-layer transform/opacity/crop sampling (smoothstep parity, fade fold).
//!   Presentation-agnostic descriptors — GPU textures/device are deferred to E5-S8.
//! - **E5-S5** — [`preview`] model: the render-ready [`RenderFrame`] (the
//!   [`CompositionFrame`] finalized with [`Canvas`] geometry + a [`QualityTarget`])
//!   that E5-S8 consumes, plus the [`PreviewTab`] model (always-present `.timeline`
//!   tab + closable `.media_asset` tabs) with per-tab playhead state.
//! - **E5-S7** — [`transport`] loop: the [`Transport`] play/pause/toggle/seek/step
//!   state machine, a reactive `current_frame` over [`TransportEvent`]s, the
//!   fake-clock-testable playback clock, and the two-tier structural-vs-property
//!   rebuild (risk #8). Reuses `palmier-media`'s `SeekMode`/tolerance/throttle.
//! - **E5-S8** — [`compositor`]: the wgpu textured-quad compositor. Pure helpers
//!   (Mat3→clip-space matrix, premultiplied YUV/RGBA upload, 1.5 GB VRAM LRU texture
//!   cache) are always compiled + unit-tested; the GPU `Compositor` (device +
//!   pipeline + on-screen/headless present) sits behind the `wgpu-compositor`
//!   feature. Premultiplied-alpha blend, black opaque floor, BT.709. The present
//!   integration into the real Tauri window lives in `palmier-tauri`'s preview
//!   module (S-2 plan A1). Text + Lottie layers are stubbed (deferred to E5-S9).

pub mod audio;
pub mod composition;
pub mod compositor;
pub mod preview;
pub mod transport;

pub use composition::{
    build_frame, refresh_visuals, CompositionFrame, CropRect, FrameRef, LayerRender, Mat3,
    SourceInfo, SourceResolver, VisualLayer,
};
// E5-S8 wgpu compositor. Pure geometry/pixel/cache helpers are always available; the
// GPU `Compositor` itself is behind the `wgpu-compositor` feature.
pub use compositor::{
    crop_uv_rect, decoded_to_rgba, layer_clip_matrix, CanvasSize, RgbaImage, TexCacheStats, TexKey,
    TextureCache, DEFAULT_VRAM_CEILING_BYTES,
};
#[cfg(feature = "wgpu-compositor")]
pub use compositor::{Compositor, CompositorError};
pub use preview::{Canvas, PreviewTab, PreviewTabState, QualityTarget, RenderFrame};
pub use transport::{
    active_video_layer_count, Clock, ManualClock, Transport, TransportEvent, WallClock,
};

// `SeekMode` is owned by `palmier-media` (E5-S2, the decode owner); re-export it so
// the transport's callers speak one seek vocabulary.
pub use palmier_media::SeekMode;
