//! `ProjectDocument` — the NSDocument analogue: bundle path + dirty-tracking +
//! autosave + Save-As→registry linkage (story E2-S12).
//!
//! Ports the `VideoProject` (NSDocument) lifecycle the reference gets for free:
//! `autosavesInPlace`, `isDocumentEdited`/`updateChangeCount`, the
//! `AppState.showHome` "force-flush if dirty before hiding" behavior, and the
//! `fileURL.didSet → ProjectRegistry.updateURL` rename hook
//! (docs/reference/project-io.md "Autosave", "macOS APIs to replace").
//!
//! ## Dirty-tracking (reference `updateChangeCount`)
//!
//! [`ProjectDocument::mark_dirty`] sets the dirty flag; any model edit, AND a
//! chat-session change (reference `agentService.onSessionsChanged →
//! updateChangeCount(.changeDone)`), goes through it. [`save`](Self::save) and
//! [`autosave`](Self::autosave) clear it.
//!
//! ## Autosave / force-flush-on-switch (reference `AppState.showHome`)
//!
//! [`flush_if_dirty`] is the port of "switching away from a project force-flushes
//! it before hiding": it saves **only if dirty** and is what the app calls before
//! navigating Home. `autosaves_in_place` is modeled as a debounce: the document
//! records the last-dirtied instant and [`autosave_due`] reports when the debounce
//! window has elapsed, so the app's timer loop can call [`autosave`](Self::autosave)
//! without this crate owning a timer.
//!
//! ## Save-As / rename → registry (reference `fileURL.didSet`)
//!
//! [`set_path`](Self::set_path) is the `fileURL` setter: changing the bundle path
//! calls [`ProjectRegistry::update_url`] so a rename updates the entry instead of
//! orphaning it, then writes the bundle to the new location on the next save.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::bundle::{write_bundle, BundleSnapshot, Result};
use crate::registry::ProjectRegistry;

/// The default autosave debounce: how long after the last edit an
/// `autosaves_in_place` save becomes due (reference uses NSDocument's implicit
/// autosave cadence; we expose a concrete, testable window).
pub const DEFAULT_AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(2);

/// An open project document: the live bundle path, the dirty flag, and the
/// autosave debounce state. Holds no model itself — the caller supplies a
/// [`BundleSnapshot`] at save time (keeping this crate free of the editor's
/// in-memory view model).
pub struct ProjectDocument {
    /// The `.palmier` bundle path. `None` for a never-saved document.
    path: Option<PathBuf>,
    /// Whether there are unsaved changes (reference `isDocumentEdited`).
    dirty: bool,
    /// When the document was last marked dirty (drives the autosave debounce).
    last_dirtied: Option<Instant>,
    /// The autosave debounce window.
    debounce: Duration,
}

impl ProjectDocument {
    /// A new, clean document at `path` (or `None` for unsaved).
    pub fn new(path: Option<impl Into<PathBuf>>) -> Self {
        ProjectDocument {
            path: path.map(Into::into),
            dirty: false,
            last_dirtied: None,
            debounce: DEFAULT_AUTOSAVE_DEBOUNCE,
        }
    }

    /// Override the autosave debounce window (tests use a tiny one).
    pub fn with_debounce(mut self, debounce: Duration) -> Self {
        self.debounce = debounce;
        self
    }

    /// The current bundle path, if saved.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Whether the document has unsaved changes (reference `isDocumentEdited`).
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark the document dirty (reference `updateChangeCount(.changeDone)`). Any
    /// model edit OR a chat-session change calls this. Records the dirty instant
    /// for the autosave debounce.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.last_dirtied = Some(Instant::now());
    }

    /// A chat-session change marks the document dirty so it autosaves on the next
    /// save (chat persists **on save**, ruling #4). Distinct method name for
    /// call-site clarity; behavior is `mark_dirty`.
    pub fn mark_chat_changed(&mut self) {
        self.mark_dirty();
    }

    /// Whether an `autosaves_in_place` save is due: dirty AND the debounce window
    /// has elapsed since the last edit. The app's timer loop polls this.
    pub fn autosave_due(&self) -> bool {
        self.dirty
            && self
                .last_dirtied
                .is_some_and(|t| t.elapsed() >= self.debounce)
    }

    /// Force-flush the document **if dirty** (reference `AppState.showHome`:
    /// autosave-before-hide). Called before navigating Home / switching projects.
    /// A clean document is a no-op (returns `false`); a dirty one is saved
    /// (returns `true`).
    pub fn flush_if_dirty(&mut self, snapshot: &BundleSnapshot) -> Result<bool> {
        if !self.dirty {
            return Ok(false);
        }
        self.save(snapshot)?;
        Ok(true)
    }

    /// Save the bundle to the current path and clear the dirty flag. Errors if the
    /// document was never given a path (an unsaved document must Save-As first).
    pub fn save(&mut self, snapshot: &BundleSnapshot) -> Result<()> {
        let path = self.path.clone().ok_or_else(|| {
            crate::bundle::BundleError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cannot save a document with no path (use Save-As / set_path first)",
            ))
        })?;
        write_bundle(&path, snapshot)?;
        self.dirty = false;
        self.last_dirtied = None;
        Ok(())
    }

    /// Autosave: the same as [`save`](Self::save) but a no-op when clean (the
    /// debounce-timer entry point).
    pub fn autosave(&mut self, snapshot: &BundleSnapshot) -> Result<bool> {
        self.flush_if_dirty(snapshot)
    }

    /// Change the bundle path (reference `fileURL.didSet`), updating the registry
    /// so a Save-As / rename **moves** the entry instead of orphaning it.
    ///
    /// - On first assignment (was `None`) the registry is left to `register`
    ///   (called by the open/create flow); only an actual *change* of an existing
    ///   path drives `update_url` (mirroring the reference, which calls
    ///   `updateURL` only when `oldValue` exists and differs).
    /// - Marks the document dirty so the next save writes the bundle to the new
    ///   location.
    pub fn set_path(
        &mut self,
        new_path: impl Into<PathBuf>,
        registry: &mut ProjectRegistry,
    ) -> Result<()> {
        let new_path = new_path.into();
        match &self.path {
            Some(old) if *old != new_path => {
                registry.update_url(old, &new_path)?;
                self.mark_dirty();
            }
            None => {
                // First path assignment: the create/open flow registers it; we
                // just adopt it without a spurious update_url. Mark dirty so it
                // gets written.
                self.mark_dirty();
            }
            // Same path → no-op (no registry churn).
            Some(_) => {}
        }
        self.path = Some(new_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{project, read_bundle};
    use palmier_model::{ClipType, Timeline, Track};
    use uuid::Uuid;

    fn scratch() -> PathBuf {
        let p = std::env::temp_dir().join(format!("palmier-e2s12-doc-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn snap() -> BundleSnapshot {
        let mut t = Timeline::new();
        t.tracks.push(Track::new(ClipType::Video));
        BundleSnapshot::new(t)
    }

    #[test]
    fn dirty_document_saved_on_switch_clean_one_skipped() {
        let dir = scratch();
        let bundle = dir.join("Doc.palmier");
        let mut doc = ProjectDocument::new(Some(bundle.clone()));

        // Clean → flush is a no-op, nothing written.
        assert!(!doc.flush_if_dirty(&snap()).unwrap());
        assert!(!bundle.exists());

        // Dirty → flush saves and clears the flag.
        doc.mark_dirty();
        assert!(doc.is_dirty());
        assert!(doc.flush_if_dirty(&snap()).unwrap());
        assert!(!doc.is_dirty());
        assert!(read_bundle(&bundle).is_ok(), "flush must have written the bundle");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn chat_change_marks_dirty() {
        let mut doc = ProjectDocument::new(Some(PathBuf::from("X.palmier")));
        assert!(!doc.is_dirty());
        doc.mark_chat_changed();
        assert!(doc.is_dirty(), "a chat change dirties the document (ruling #4)");
    }

    #[test]
    fn autosave_due_respects_debounce() {
        let mut doc = ProjectDocument::new(Some(PathBuf::from("X.palmier")))
            .with_debounce(Duration::from_millis(30));
        assert!(!doc.autosave_due(), "clean → not due");
        doc.mark_dirty();
        assert!(!doc.autosave_due(), "just-dirtied → debounce not elapsed");
        std::thread::sleep(Duration::from_millis(45));
        assert!(doc.autosave_due(), "after debounce window → due");
    }

    #[test]
    fn set_path_rename_updates_registry_and_dirties() {
        let dir = scratch();
        let mut registry = ProjectRegistry::with_path(dir.join(project::REGISTRY_FILE));
        let old = dir.join("Old.palmier");
        let new = dir.join("New.palmier");

        // The open flow registered the original path.
        registry.register(&old).unwrap();
        let id = registry.entries()[0].id;

        let mut doc = ProjectDocument::new(Some(old.clone()));
        doc.set_path(&new, &mut registry).unwrap();

        // Registry entry MOVED (same id, new name), not orphaned.
        assert_eq!(registry.entries().len(), 1);
        assert_eq!(registry.entries()[0].id, id);
        assert_eq!(registry.entries()[0].name(), "New");
        // Document adopted the new path and is dirty (needs a write to the new loc).
        assert_eq!(doc.path(), Some(new.as_path()));
        assert!(doc.is_dirty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_path_same_path_is_noop() {
        let dir = scratch();
        let mut registry = ProjectRegistry::with_path(dir.join(project::REGISTRY_FILE));
        let p = dir.join("Same.palmier");
        registry.register(&p).unwrap();
        let opened = registry.entries()[0].last_opened_date;

        let mut doc = ProjectDocument::new(Some(p.clone()));
        doc.set_path(&p, &mut registry).unwrap();
        // No registry churn (last_opened unchanged), document stays clean.
        assert_eq!(registry.entries()[0].last_opened_date, opened);
        assert!(!doc.is_dirty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_without_path_errors() {
        let mut doc = ProjectDocument::new(None::<PathBuf>);
        doc.mark_dirty();
        assert!(doc.save(&snap()).is_err(), "no path → save must error");
    }
}
