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
//! ## Landed (E2-S11): the project registry
//!
//! [`registry`] ports `Project/ProjectRegistry.swift` to a **synchronous**
//! registry at `project-registry.json` under the platform config dir
//! (`%APPDATA%\PalmierProWin\` / `~/.config/palmier-pro/`):
//! - [`ProjectRegistry`] — `register` / `remove` / `delete` (→ Recycle
//!   Bin/trash) / `update_url` (Save-As) / `sorted_entries` (newest-first), with
//!   **atomic full-array writes** and **standardized-URL dedup**.
//! - [`ProjectEntry`] — `{ id, url, created_date, last_opened_date }` (dates as
//!   apple-epoch doubles, matching reference-written registries).
//!
//! ## Landed (E2-S12): media-path resolution + autosave + directory-as-document
//!
//! - [`resolver`] — [`MediaResolver`] (`expected_url` / `resolve_url` /
//!   `is_missing`), [`source_for_url`] (internalize-on-save heuristic), and
//!   [`restore_entries`] (logs + skips missing files on open).
//! - [`document`] — [`ProjectDocument`]: dirty-tracking, `flush_if_dirty`
//!   (force-flush-on-switch), autosave debounce, and `set_path` →
//!   [`ProjectRegistry::update_url`] (Save-As/rename never orphans the entry).
//! - [`dialog`] — [`DirectoryDocumentDialog`]: the FR-8 directory-as-document
//!   dialog config the Tauri layer consumes (Rust side; Explorer shell extension
//!   left as a documented TODO).
//!
//! ## Landed (E2-S10): round-trip golden fixtures + open-speed gate
//!
//! `tests/round_trip.rs` is the §11.2 M1 import→edit→save→reopen gate and the
//! SM-1b open-speed assertion; `tests/fixtures/golden_project_*.palmier/` are the
//! committed golden bundles (consumed downstream by Epics 5/6).

pub mod bundle;
pub mod dialog;
pub mod document;
pub mod media_library;
pub mod registry;
pub mod resolver;
pub mod samples;

pub use bundle::{
    project, read_bundle, read_chat_session_files, write_bundle, BundleError, BundleSnapshot,
    LoadedBundle,
};
pub use dialog::{DialogError, DirectoryDocumentDialog, DirectorySelection};
pub use media_library::{action_name, MediaLibraryHistory};
pub use document::{ProjectDocument, DEFAULT_AUTOSAVE_DEBOUNCE};
pub use registry::{normalize_path, ProjectEntry, ProjectRegistry, SystemTrasher, Trasher};
pub use resolver::{
    expected_url_for_source, restore_entries, source_for_url, MediaResolver, RestoreEvent,
};
pub use samples::{
    safe_name, FixtureSampleBackend, HttpSampleBackend, ResolvedSample, SampleBackend, SampleChat,
    SampleDownload, SampleError, SampleProjectService, SampleSummary,
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
