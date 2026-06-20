//! Folder/asset move + delete orchestration with **snapshot undo** (story E4-S6).
//!
//! [`palmier_model::MediaLibrary`] holds the pure mutations (cycle-guarded
//! moves, the delete cascade); this module threads each one through
//! [`palmier_history::History`] so it registers **one named undo step** on the
//! **user** stack — the seam the reference's `EditorViewModel+Folders.swift`
//! gets from `UndoManager` (`registerUndo` + `setActionName`).
//!
//! The reference snapshots the whole media-library state
//! (`mediaLibraryUndoSnapshot`) and restores it on undo
//! (`restoreMediaLibraryUndoSnapshot`). We reproduce that exactly with
//! `History::with_user_swap`: it clones `MediaLibrary` before/after, registers a
//! whole-state swap, and — like the reference `guard before != after` — registers
//! **nothing** when the op made no change (a rejected cycle-move, a no-op rename).
//!
//! ## The action names (parity)
//!
//! Each entry point sets the reference action name verbatim:
//! `"New Folder"` / `"Rename Folder"` / `"Delete Folder"` / `"Move to Folder"` /
//! `"Move Folder"`. (Import's `"Import Media"` is in `palmier-media`'s import
//! orchestration, story E4-S7, which reuses [`MediaLibraryHistory`].)
//!
//! Because `MediaLibrary: Clone + PartialEq`, this is the whole integration — no
//! per-field inverse bookkeeping. The history crate stays generic; this module is
//! the only place that names the concrete state.

use std::collections::HashSet;

use palmier_history::History;
use palmier_model::MediaLibrary;

/// Reference action-name constants (`EditorViewModel+Folders.swift` /
/// `+MediaLibrary.swift` `setActionName(...)`). Kept public so the import
/// orchestration (E4-S7, `palmier-media`) and the Tauri command layer name
/// identical steps.
pub mod action_name {
    /// `createFolder` → `setActionName("New Folder")`.
    pub const NEW_FOLDER: &str = "New Folder";
    /// `renameFolder` → `setActionName("Rename Folder")`.
    pub const RENAME_FOLDER: &str = "Rename Folder";
    /// `deleteFolders` → `setActionName("Delete Folder")`.
    pub const DELETE_FOLDER: &str = "Delete Folder";
    /// `moveAssetsToFolder` → `setActionName("Move to Folder")`.
    pub const MOVE_TO_FOLDER: &str = "Move to Folder";
    /// `moveFoldersToFolder` → `setActionName("Move Folder")`.
    pub const MOVE_FOLDER: &str = "Move Folder";
    /// `importFinderItems` → `setActionName("Import Media")`.
    pub const IMPORT_MEDIA: &str = "Import Media";
}

/// A [`MediaLibrary`] paired with its undo [`History`] — the document-level
/// handle the panel mutates. Every folder/asset op goes through here so it lands
/// as one reversible, named user-undo step.
///
/// The `History<MediaLibrary>` is the **user** stack the reference's folder ops
/// register on (the agent stack is for MCP tool edits, Epic 7).
#[derive(Default)]
pub struct MediaLibraryHistory {
    /// The live library state (timeline + manifest + asset catalog).
    pub library: MediaLibrary,
    /// The two-stack undo history over the whole library snapshot.
    pub history: History<MediaLibrary>,
}

impl MediaLibraryHistory {
    /// A fresh, empty library + history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wrap an existing library (e.g. one loaded from a bundle) with a new,
    /// empty history.
    pub fn with_library(library: MediaLibrary) -> Self {
        MediaLibraryHistory {
            library,
            history: History::new(),
        }
    }

    /// Create a folder named `name` under `parent_folder_id`, as one `"New
    /// Folder"` undo step. Returns the new folder id.
    ///
    /// Creation always changes state, so it always registers an entry (the
    /// reference unconditionally `registerUndo { deleteFolders([id]) }`).
    pub fn create_folder(
        &mut self,
        name: impl Into<String>,
        parent_folder_id: Option<String>,
    ) -> String {
        let name = name.into();
        let mut new_id = String::new();
        let id_ref = &mut new_id;
        self.history.with_user_swap(
            action_name::NEW_FOLDER,
            &mut self.library,
            move |lib| {
                *id_ref = lib.create_folder(name, parent_folder_id);
            },
        );
        new_id
    }

    /// Rename folder `id` → `name` as one `"Rename Folder"` step. Registers
    /// nothing if the folder is missing or the name is unchanged. Returns whether
    /// it changed.
    pub fn rename_folder(&mut self, id: &str, name: impl Into<String>) -> bool {
        let name = name.into();
        let mut changed = false;
        let changed_ref = &mut changed;
        self.history
            .with_user_swap(action_name::RENAME_FOLDER, &mut self.library, move |lib| {
                *changed_ref = lib.rename_folder(id, name);
            });
        changed
    }

    /// Delete `folder_ids` + descendants + assets + referencing clips (the
    /// cascade) as one `"Delete Folder"` step. Returns whether it changed.
    pub fn delete_folders(&mut self, folder_ids: &HashSet<String>) -> bool {
        let mut changed = false;
        let changed_ref = &mut changed;
        self.history
            .with_user_swap(action_name::DELETE_FOLDER, &mut self.library, move |lib| {
                *changed_ref = lib.delete_folders(folder_ids);
            });
        changed
    }

    /// Reparent `asset_ids` onto `folder_id` as one `"Move to Folder"` step.
    /// Returns whether it changed (skips assets already in the target).
    pub fn move_assets_to_folder(
        &mut self,
        asset_ids: &HashSet<String>,
        folder_id: Option<&str>,
    ) -> bool {
        let mut changed = false;
        let changed_ref = &mut changed;
        self.history
            .with_user_swap(action_name::MOVE_TO_FOLDER, &mut self.library, move |lib| {
                *changed_ref = lib.move_assets_to_folder(asset_ids, folder_id);
            });
        changed
    }

    /// Reparent `folder_ids` onto `parent_folder_id` (cycle-guarded) as one
    /// `"Move Folder"` step. Returns whether it changed (false if every move was
    /// rejected by a cycle guard / no-op).
    pub fn move_folders_to_folder(
        &mut self,
        folder_ids: &HashSet<String>,
        parent_folder_id: Option<&str>,
    ) -> bool {
        let mut changed = false;
        let changed_ref = &mut changed;
        self.history
            .with_user_swap(action_name::MOVE_FOLDER, &mut self.library, move |lib| {
                *changed_ref = lib.move_folders_to_folder(folder_ids, parent_folder_id);
            });
        changed
    }

    /// Undo the most recent user step, restoring the prior library snapshot.
    pub fn undo(&mut self) -> bool {
        self.history.undo(&mut self.library)
    }

    /// Redo the most recently undone user step.
    pub fn redo(&mut self) -> bool {
        self.history.redo(&mut self.library)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::{ClipType, MediaAsset, MediaManifestEntry, MediaSource};

    fn add_asset(lib: &mut MediaLibrary, id: &str, folder: Option<&str>) {
        let mut a = MediaAsset::new(
            id,
            id,
            ClipType::Video,
            MediaSource::External {
                absolute_path: format!("/x/{id}.mov"),
            },
            1.0,
        );
        a.folder_id = folder.map(str::to_owned);
        lib.assets.push(a.clone());
        lib.manifest.entries.push(MediaManifestEntry {
            id: a.id.clone(),
            name: a.name.clone(),
            asset_type: a.asset_type,
            source: a.source.clone(),
            duration: a.duration_seconds,
            generation_input: None,
            source_width: None,
            source_height: None,
            source_fps: None,
            has_audio: Some(a.has_audio),
            folder_id: a.folder_id.clone(),
            cached_remote_url: None,
            cached_remote_url_expires_at: None,
        });
    }

    #[test]
    fn create_then_undo_removes_folder() {
        let mut doc = MediaLibraryHistory::new();
        let id = doc.create_folder("Footage", None);
        assert_eq!(doc.library.manifest.folders.len(), 1);
        assert_eq!(
            doc.history.current_undo_action_name(),
            Some(action_name::NEW_FOLDER)
        );

        assert!(doc.undo());
        assert_eq!(doc.library.manifest.folders.len(), 0);
        // Redo recreates it.
        assert!(doc.redo());
        assert_eq!(doc.library.manifest.folders.len(), 1);
        assert_eq!(doc.library.folder(&id).unwrap().name, "Footage");
    }

    #[test]
    fn move_undo_restores_prior_parents() {
        let mut doc = MediaLibraryHistory::new();
        let a = doc.create_folder("A", None);
        let b = doc.create_folder("B", None);
        // Move B under A.
        let ids: HashSet<String> = [b.clone()].into_iter().collect();
        assert!(doc.move_folders_to_folder(&ids, Some(&a)));
        assert_eq!(doc.library.folder(&b).unwrap().parent_id, Some(a.clone()));
        assert_eq!(
            doc.history.current_undo_action_name(),
            Some(action_name::MOVE_FOLDER)
        );

        // Undo restores B to the root (its prior parent).
        assert!(doc.undo());
        assert_eq!(doc.library.folder(&b).unwrap().parent_id, None);
    }

    #[test]
    fn rejected_cycle_move_registers_no_undo_step() {
        let mut doc = MediaLibraryHistory::new();
        let a = doc.create_folder("A", None);
        let b = doc.create_folder("B", Some(a.clone()));
        let undo_len_before = doc.history.user_undo_len();

        // Move A into its descendant B → rejected, no state change, no entry.
        let ids: HashSet<String> = [a.clone()].into_iter().collect();
        assert!(!doc.move_folders_to_folder(&ids, Some(&b)));
        assert_eq!(doc.history.user_undo_len(), undo_len_before);
        // The most recent step is still the folder creation, not a phantom move.
        assert_eq!(
            doc.history.current_undo_action_name(),
            Some(action_name::NEW_FOLDER)
        );
    }

    #[test]
    fn delete_cascade_is_one_undo_step() {
        let mut doc = MediaLibraryHistory::new();
        let a = doc.create_folder("A", None);
        let _b = doc.create_folder("B", Some(a.clone()));
        add_asset(&mut doc.library, "in_a", Some(&a));

        let len_before = doc.history.user_undo_len();
        let ids: HashSet<String> = [a.clone()].into_iter().collect();
        assert!(doc.delete_folders(&ids));
        // Exactly one entry was registered for the whole cascade.
        assert_eq!(doc.history.user_undo_len(), len_before + 1);
        assert!(doc.library.assets.is_empty());

        // Undo brings everything back in one step.
        assert!(doc.undo());
        assert_eq!(doc.library.manifest.folders.len(), 2);
        assert_eq!(doc.library.assets.len(), 1);
    }

    #[test]
    fn move_assets_undo_restores_folder_ids() {
        let mut doc = MediaLibraryHistory::new();
        let f = doc.create_folder("F", None);
        add_asset(&mut doc.library, "a1", None);

        let ids: HashSet<String> = ["a1".into()].into_iter().collect();
        assert!(doc.move_assets_to_folder(&ids, Some(&f)));
        assert_eq!(
            doc.library.assets[0].folder_id,
            Some(f.clone()),
            "asset moved into F"
        );
        assert!(doc.undo());
        assert_eq!(doc.library.assets[0].folder_id, None, "undo restores root");
    }
}
