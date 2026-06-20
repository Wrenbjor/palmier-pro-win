//! Composition graph (E5-S3 build + E5-S4 sampling) — FOUNDATION §6.5.
//!
//! The pure, **presentation-agnostic** assembly that turns a `palmier-model`
//! [`Timeline`](palmier_model::Timeline) + a frame index into a
//! [`CompositionFrame`]: the ordered, bottom→top stack of [`LayerRender`]
//! descriptors the wgpu compositor (E5-S8) renders.
//!
//! ## Module layout
//! - [`mat3`] — the pure `Mat3` affine (no wgpu dep; E5-S8 converts to a GPU matrix).
//! - [`types`] — the descriptor types: [`CompositionFrame`], [`LayerRender`],
//!   [`VisualLayer`], [`FrameRef`], [`CropRect`].
//! - [`sampler`] (E5-S4) — per-frame transform / opacity / crop sampling, the
//!   verbatim `affineTransform` + `emitCrop` + `emitOpacity` port, with 8-segment
//!   smoothstep parity guaranteed by sampling the model's true smoothstep curve.
//! - [`build`] (E5-S3) — the [`build::build_frame`] assembly (z-order, overlap
//!   precedence, clip→source-frame mapping) + the [`build::refresh_visuals`]
//!   fast path (risk #8).
//!
//! ## What is deferred to E5-S8 (wgpu compositor)
//!
//! This wave owns the **descriptors only**. It does NOT create a wgpu device,
//! upload textures, or fetch decoded pixels — a [`FrameRef`] is a `(media_ref,
//! source_frame)` handle that E5-S8 resolves through `palmier-media`'s
//! `FrameSource`. Stills/Lottie are first-class layers (no `.mov` bake, #22). The
//! `has_alpha` flag on a [`VisualLayer`] is set by E5-S8 from the decoded frame's
//! pixfmt; the build leaves it `false`.

pub mod build;
pub mod mat3;
pub mod sampler;
pub mod types;

pub use build::{build_frame, refresh_visuals, source_frame_for, SourceResolver};
pub use mat3::Mat3;
pub use sampler::{affine_transform, crop_rect, layer_opacity, layer_transform, SourceInfo};
pub use types::{CompositionFrame, CropRect, FrameRef, LayerRender, TextLayer, VisualLayer};
