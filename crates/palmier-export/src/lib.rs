//! # palmier-export
//!
//! Video export (H.264/H.265/ProRes), FCP7 XMEML emitter, and self-contained
//! `.palmier` project export (FOUNDATION §4, §6.12). Drives the same composition
//! path as the engine, then muxes via FFmpeg (added per-story).
//!
//! Skeleton stub: real export pipeline + XMEML emitter land per the export epic.

/// Placeholder for the export subsystem.
pub fn placeholder() -> &'static str {
    "palmier-export"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-export");
    }
}
