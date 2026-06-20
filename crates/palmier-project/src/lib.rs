//! # palmier-project
//!
//! `.palmier` bundle I/O, project registry, autosave/dirty-tracking, sample
//! materialization, and media-path resolution (FOUNDATION §4, §5.7).
//!
//! Owns the filesystem + reqwest + tokio side of project lifecycle; reads/writes
//! the `palmier-model` shapes to/from the directory-as-document bundle.
//!
//! Skeleton stub: real bundle I/O lands per Epic 2 (`epic-02-project-io.md`).

/// Reference bundle filenames (ruling #3). Placeholder constants for the skeleton.
pub const PROJECT_FILE: &str = "project.json";
pub const MEDIA_FILE: &str = "media.json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_filenames_match_reference() {
        assert_eq!(PROJECT_FILE, "project.json");
        assert_eq!(MEDIA_FILE, "media.json");
        // Touch the model dep so the edge is exercised.
        assert_eq!(palmier_model::CRATE_NAME, "palmier-model");
    }
}
