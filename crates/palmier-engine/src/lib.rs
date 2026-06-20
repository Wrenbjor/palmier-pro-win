//! # palmier-engine
//!
//! Composition graph, wgpu compositor, and playback transport (FOUNDATION §4, §6.5).
//! Builds a per-frame `CompositionFrame` from the `palmier-model` timeline, decodes
//! source frames via `palmier-media`, and renders via wgpu. Heavy GPU/audio deps
//! (wgpu, cpal, rubato) are added per-story, not in this skeleton.

/// Placeholder for the composition + transport engine.
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
