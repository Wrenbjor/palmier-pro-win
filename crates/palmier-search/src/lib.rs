//! # palmier-search
//!
//! Visual frame index (SigLIP2/CLIP embeddings) + transcript full-text search
//! (FOUNDATION §4, §6.10). Embeds frames/queries via `candle` or `ort`; those
//! heavy deps are added per-story, not in this skeleton.

/// Placeholder for the search subsystem.
pub fn placeholder() -> &'static str {
    "palmier-search"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-search");
    }
}
