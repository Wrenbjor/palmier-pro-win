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

pub mod audio;

/// Placeholder for the composition + transport engine (compositor/transport land in
/// E5-S4/S5/S7/S8).
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
