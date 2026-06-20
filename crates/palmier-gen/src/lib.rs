//! # palmier-gen
//!
//! Convex generation client, job lifecycle, and the generation queue
//! (FOUNDATION §4, §6.11). Submits jobs and subscribes for status via Convex
//! (HTTP + WebSocket); those deps are added per-story, not in this skeleton.

/// Placeholder for the generation subsystem.
pub fn placeholder() -> &'static str {
    "palmier-gen"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-gen");
    }
}
