//! Preview model — E5-S5.
//!
//! The seam between the composition graph (E5-S3/S4) and the GPU present (E5-S8),
//! plus the preview-tab model the viewport (E5-S10) and the transport (E5-S7) share.
//!
//! - [`render_frame`] — [`RenderFrame`], the render-ready frame description E5-S8
//!   consumes: a [`CompositionFrame`](crate::composition::CompositionFrame) finalized
//!   with [`Canvas`] geometry + a [`QualityTarget`].
//! - [`tab`] — [`PreviewTab`] (the always-present `.timeline` tab + closable
//!   `.media_asset` tabs) and the per-tab [`PreviewTabState`] playhead, ported from
//!   the reference `PreviewTab.swift`.
//!
//! Presentation-agnostic: descriptors + identity only — no wgpu, no Tauri.

pub mod render_frame;
pub mod tab;

pub use render_frame::{Canvas, QualityTarget, RenderFrame};
pub use tab::{PreviewTab, PreviewTabState, TIMELINE_TAB_ID};
