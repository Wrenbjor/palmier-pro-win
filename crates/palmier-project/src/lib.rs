//! # palmier-project
//!
//! `.palmier` bundle I/O, project registry, autosave/dirty-tracking, sample
//! materialization, and media-path resolution (FOUNDATION §4, §5.7).
//!
//! Owns the filesystem side of project lifecycle; reads/writes the `palmier-model`
//! shapes to/from the directory-as-document bundle.
//!
//! ## Landed (E2-S9): the bundle reader / writer
//!
//! [`bundle`] ports `Project/VideoProject.swift` read/save with the reference's
//! exact required/optional/soft-error severities and an atomic whole-directory
//! save (NSDocument safe-save):
//! - [`read_bundle`] — `project.json` required (missing/corrupt = hard
//!   [`BundleError::Corrupt`]); `media.json` decode-fail = hard; `generation-log.json`
//!   fail = soft/ignored.
//! - [`write_bundle`] — write-to-sibling-temp-then-atomic-swap so a crash never
//!   leaves a half-written bundle.
//! - Reference bundle filenames per ruling #3 ([`bundle::project`]):
//!   `project.json` / `media.json` / `generation-log.json` / `thumbnail.jpg` /
//!   `media/` / `chat/` — **not** FOUNDATION §5.7's `timeline.json` etc.
//!
//! Later Epic 2 stories add the project registry (E2-S11), media-path resolution +
//! autosave + the directory-as-document dialog (E2-S12), and the round-trip golden
//! fixtures (E2-S10).

pub mod bundle;

pub use bundle::{
    project, read_bundle, read_chat_session_files, write_bundle, BundleError, BundleSnapshot,
    LoadedBundle,
};

/// Reference bundle filenames (ruling #3). Re-exported from [`bundle::project`]
/// for convenience / backward compatibility with the crate skeleton.
pub const PROJECT_FILE: &str = bundle::project::TIMELINE_FILE;
pub const MEDIA_FILE: &str = bundle::project::MANIFEST_FILE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_filenames_match_reference() {
        assert_eq!(PROJECT_FILE, "project.json");
        assert_eq!(MEDIA_FILE, "media.json");
        assert_eq!(bundle::project::GENERATION_LOG_FILE, "generation-log.json");
        assert_eq!(bundle::project::CHAT_DIR, "chat");
        assert_eq!(bundle::project::MEDIA_DIR, "media");
        // Touch the model dep so the edge is exercised.
        assert_eq!(palmier_model::CRATE_NAME, "palmier-model");
    }
}
