//! # palmier-model
//!
//! Core data model: `Timeline`, `Track`, `Clip`, `Keyframe`, `MediaAsset` and the
//! pure sampling / geometry / computed-property math over them (FOUNDATION §4, §5).
//!
//! This crate is the data foundation every later epic consumes. It holds only pure
//! serde shapes and pure functions — no filesystem, no async, no GPU.
//!
//! Skeleton stub: real shapes land per Epic 2 (`epic-02-project-io.md`).

/// Placeholder marker for the model crate. Replaced by the real `Timeline` and
/// friends in Epic 2.
pub const CRATE_NAME: &str = "palmier-model";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE_NAME, "palmier-model");
    }
}
