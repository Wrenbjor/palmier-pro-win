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

pub mod audio;
pub mod composition;

pub use composition::{
    build_frame, refresh_visuals, CompositionFrame, CropRect, FrameRef, LayerRender, Mat3,
    SourceInfo, SourceResolver, VisualLayer,
};

/// Placeholder for the transport engine (transport lands in E5-S7; the wgpu
/// compositor + present in E5-S8).
pub fn placeholder() -> &'static str {
    "palmier-engine"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-engine");
    }
}
