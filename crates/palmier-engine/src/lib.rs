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

pub mod audio;
pub mod composition;
pub mod preview;
pub mod transport;

pub use composition::{
    build_frame, refresh_visuals, CompositionFrame, CropRect, FrameRef, LayerRender, Mat3,
    SourceInfo, SourceResolver, VisualLayer,
};
pub use preview::{Canvas, PreviewTab, PreviewTabState, QualityTarget, RenderFrame};
pub use transport::{
    active_video_layer_count, Clock, ManualClock, Transport, TransportEvent, WallClock,
};

// `SeekMode` is owned by `palmier-media` (E5-S2, the decode owner); re-export it so
// the transport's callers speak one seek vocabulary.
pub use palmier_media::SeekMode;
