//! Media-library **folder tree** + folder/asset CRUD with cycle-guarded moves
//! (story E4-S6).
//!
//! Port of `Editor/ViewModel/EditorViewModel+Folders.swift` (`createFolder` /
//! `renameFolder` / `deleteFolders` / `moveAssetsToFolder` /
//! `moveFoldersToFolder` / `applyParentChanges`) and its private
//! `MediaFolderIndex` (`isDescendant`, `path`, `idsIncludingDescendants`). See
//! `docs/reference/media-panel.md` §"Folder model & moves" and
//! `_bmad-output/implementation-artifacts/epic-04-media-panel.md` (E4-S6).
//!
//! ## What lives here vs. where undo lives
//!
//! This module is **pure**: every mutation operates on a [`MediaLibrary`] value
//! (the snapshot unit = `Timeline` + `MediaManifest` + the runtime `MediaAsset`
//! catalog) and returns whether it **changed** anything. The reference threads
//! its `UndoManager` through each op (`registerUndo` + `setActionName`); we
//! deliberately do **not** — `palmier-history` is generic over a `Clone +
//! PartialEq` state, so the snapshot-undo is composed at the orchestration layer
//! by running these mutations inside `History::with_user_swap` (see
//! `palmier-project::media_library`). That keeps this crate free of any history
//! dependency while reproducing the reference's snapshot-restore semantics
//! exactly (the whole-`MediaLibrary` before/after swap == the reference's
//! `mediaLibraryUndoSnapshot` / `restoreMediaLibraryUndoSnapshot`).
//!
//! ## The three cycle guards (load-bearing)
//!
//! [`MediaLibrary::move_folders_to_folder`] reproduces the reference's three
//! rejections exactly, per folder being moved:
//! 1. **no-op** — the folder is already a child of the target (`parent == target`).
//! 2. **into a descendant** — the target is `self` or a descendant of the folder
//!    (`is_descendant(target, of: folder)`), which would orphan the subtree.
//! 3. **into itself** — `folder.id == target` (a special case of #2, guarded
//!    explicitly to match the reference's `id == parentFolderId` line).

use std::collections::{HashMap, HashSet};

use crate::manifest::{MediaFolder, MediaManifest};
use crate::media_asset::MediaAsset;
use crate::timeline::Timeline;

/// The full media-library state that a folder/asset/import mutation reads and
/// writes — and the unit a snapshot-undo restores.
///
/// This bundles the three pieces the reference's `mediaLibraryUndoSnapshot`
/// captures that folder ops touch: the [`Timeline`] (clips referencing deleted
/// assets are pruned), the persisted [`MediaManifest`] (folders + entries), and
/// the runtime [`MediaAsset`] catalog (the in-memory list the panel renders).
/// Selection/preview-tab state from the reference snapshot is UI state owned by
/// `src-ui/media-panel`, not modeled here.
///
/// `Clone + PartialEq` so `palmier-history`'s `with_user_swap` can snapshot it
/// and detect no-ops exactly like the reference `guard before != after`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaLibrary {
    /// The timeline — folder deletes prune clips that reference deleted assets.
    pub timeline: Timeline,
    /// The persisted media manifest (`media.json`): folder tree + asset entries.
    pub manifest: MediaManifest,
    /// The runtime asset catalog the panel browses (the reference `mediaAssets`).
    pub assets: Vec<MediaAsset>,
}

impl MediaLibrary {
    /// An empty library (empty timeline, fresh manifest, no assets).
    pub fn new() -> Self {
        MediaLibrary::default()
    }

    // ---- reads -------------------------------------------------------------

    /// The folder with `id`, if present (reference `folder(id:)`).
    pub fn folder(&self, id: &str) -> Option<&MediaFolder> {
        self.manifest.folders.iter().find(|f| f.id == id)
    }

    /// Direct subfolders of `parent_folder_id` (`None` = top level), sorted
    /// case-insensitively by name (reference `subfolders(of:)`).
    pub fn subfolders(&self, parent_folder_id: Option<&str>) -> Vec<&MediaFolder> {
        let mut out: Vec<&MediaFolder> = self
            .manifest
            .folders
            .iter()
            .filter(|f| f.parent_id.as_deref() == parent_folder_id)
            .collect();
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out
    }

    /// The root→folder path for `folder_id` (reference `folderPath(for:)`):
    /// ancestor chain ending at the folder, root first. Empty for `None` or an
    /// unknown id. Cycle-safe (a corrupt parent cycle terminates).
    pub fn folder_path(&self, folder_id: Option<&str>) -> Vec<&MediaFolder> {
        self.index().path(folder_id)
    }

    fn index(&self) -> MediaFolderIndex<'_> {
        MediaFolderIndex::new(&self.manifest.folders)
    }

    // ---- folder CRUD -------------------------------------------------------

    /// Create a folder named `name` under `parent_folder_id`, returning its new
    /// id (reference `createFolder(name:in:)`). The id is a fresh UUID string.
    pub fn create_folder(
        &mut self,
        name: impl Into<String>,
        parent_folder_id: Option<String>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.manifest.folders.push(MediaFolder {
            id: id.clone(),
            name: name.into(),
            parent_id: parent_folder_id,
        });
        id
    }

    /// Rename folder `id` to `name`. No-op (returns `false`) if the folder is
    /// missing or the name is unchanged (reference `renameFolder(id:name:)`'s
    /// `guard oldName != name`).
    pub fn rename_folder(&mut self, id: &str, name: impl Into<String>) -> bool {
        let name = name.into();
        let Some(folder) = self.manifest.folders.iter_mut().find(|f| f.id == id) else {
            return false;
        };
        if folder.name == name {
            return false;
        }
        folder.name = name;
        true
    }

    /// Delete `folder_ids` **and all descendants** + their assets + any timeline
    /// clips referencing those assets, then prune empty tracks — the reference
    /// `deleteFolders(ids:)` cascade. Returns `false` (no change) if nothing
    /// matched.
    ///
    /// This is the snapshot-undo cascade: the orchestration layer runs it inside
    /// one `with_user_swap("Delete Folder", …)` so the whole cascade reverses as
    /// one step.
    pub fn delete_folders(&mut self, folder_ids: &HashSet<String>) -> bool {
        if folder_ids.is_empty() {
            return false;
        }
        let all_folder_ids = self.index().ids_including_descendants(folder_ids);
        if !self
            .manifest
            .folders
            .iter()
            .any(|f| all_folder_ids.contains(&f.id))
        {
            return false;
        }

        // Assets living in any deleted folder.
        let asset_ids_to_delete: HashSet<String> = self
            .assets
            .iter()
            .filter(|a| {
                a.folder_id
                    .as_ref()
                    .is_some_and(|fid| all_folder_ids.contains(fid))
            })
            .map(|a| a.id.clone())
            .collect();

        // Timeline clips referencing any deleted asset.
        let mut removed_any_clip = false;
        if !asset_ids_to_delete.is_empty() {
            for track in &mut self.timeline.tracks {
                let before = track.clips.len();
                track
                    .clips
                    .retain(|c| !asset_ids_to_delete.contains(&c.media_ref));
                if track.clips.len() != before {
                    removed_any_clip = true;
                }
            }
            if removed_any_clip {
                prune_empty_tracks(&mut self.timeline);
            }
        }

        self.assets.retain(|a| !asset_ids_to_delete.contains(&a.id));
        self.manifest
            .entries
            .retain(|e| !asset_ids_to_delete.contains(&e.id));
        self.manifest
            .folders
            .retain(|f| !all_folder_ids.contains(&f.id));
        true
    }

    // ---- moves (the parent-change ops) -------------------------------------

    /// Reparent `asset_ids` onto `folder_id` (`None` = top level). Assets already
    /// in the target are skipped; returns `false` if nothing actually moved
    /// (reference `moveAssetsToFolder`).
    pub fn move_assets_to_folder(
        &mut self,
        asset_ids: &HashSet<String>,
        folder_id: Option<&str>,
    ) -> bool {
        if asset_ids.is_empty() {
            return false;
        }
        let mut changed = false;
        for id in asset_ids {
            // Skip ids that don't exist or are already in the target folder.
            match self.assets.iter().find(|a| &a.id == id) {
                Some(asset) if asset.folder_id.as_deref() == folder_id => continue,
                Some(_) => {}
                None => continue,
            }
            self.set_asset_folder_id(id, folder_id.map(str::to_owned));
            changed = true;
        }
        changed
    }

    /// Reparent `folder_ids` onto `parent_folder_id` (`None` = top level) with the
    /// **three cycle guards** (reference `moveFoldersToFolder`):
    /// reject no-op (already child), reject move into a descendant, reject move
    /// into self. Returns `false` if every requested move was rejected/no-op.
    pub fn move_folders_to_folder(
        &mut self,
        folder_ids: &HashSet<String>,
        parent_folder_id: Option<&str>,
    ) -> bool {
        if folder_ids.is_empty() {
            return false;
        }
        // Build the index once over the *current* tree (the reference builds it
        // from the pre-move snapshot — moves are validated against priors, then
        // applied together).
        let index = self.index();
        let mut accepted: Vec<String> = Vec::new();
        for id in folder_ids {
            let Some(folder) = index.folder(id) else {
                continue;
            };
            // Guard 1: no-op — already a child of the target.
            if folder.parent_id.as_deref() == parent_folder_id {
                continue;
            }
            // Guard 2: into a descendant — target is `id` or below it.
            if let Some(target) = parent_folder_id {
                if index.is_descendant(target, id) {
                    continue;
                }
            }
            // Guard 3: into itself (explicit, matches reference `id == parent`).
            if Some(id.as_str()) == parent_folder_id {
                continue;
            }
            accepted.push(id.clone());
        }
        if accepted.is_empty() {
            return false;
        }
        for id in accepted {
            self.set_folder_parent(&id, parent_folder_id.map(str::to_owned));
        }
        true
    }

    // ---- internal write helpers (keep manifest + runtime catalog in sync) ---

    /// Reference `setAssetFolderId(_:forAssetId:)`: write both the runtime asset
    /// and the manifest entry.
    pub(crate) fn set_asset_folder_id(&mut self, asset_id: &str, folder_id: Option<String>) {
        if let Some(asset) = self.assets.iter_mut().find(|a| a.id == asset_id) {
            asset.folder_id = folder_id.clone();
        }
        if let Some(entry) = self.manifest.entries.iter_mut().find(|e| e.id == asset_id) {
            entry.folder_id = folder_id;
        }
    }

    /// Reference `setFolderParent(_:forFolderId:)`.
    pub(crate) fn set_folder_parent(&mut self, folder_id: &str, parent: Option<String>) {
        if let Some(folder) = self.manifest.folders.iter_mut().find(|f| f.id == folder_id) {
            folder.parent_id = parent;
        }
    }
}

/// Prune tracks that have become empty (reference `pruneEmptyTracks`). The model
/// keeps it minimal — a track with no clips is removed. The edit layer (Epic 3)
/// owns the richer prune policy; folder-delete needs only the empty-track drop.
fn prune_empty_tracks(timeline: &mut Timeline) {
    timeline.tracks.retain(|t| !t.clips.is_empty());
}

/// Cached lookup tables for folder path + descendant traversal — a 1:1 port of
/// the reference's private `MediaFolderIndex`. Built per-operation over a
/// `&[MediaFolder]` slice (cheap; folder counts are small).
pub struct MediaFolderIndex<'a> {
    by_id: HashMap<&'a str, &'a MediaFolder>,
    children_by_parent: HashMap<Option<&'a str>, Vec<&'a MediaFolder>>,
}

impl<'a> MediaFolderIndex<'a> {
    /// Index `folders` for O(1) id lookup + grouped children.
    pub fn new(folders: &'a [MediaFolder]) -> Self {
        let mut by_id = HashMap::with_capacity(folders.len());
        let mut children_by_parent: HashMap<Option<&'a str>, Vec<&'a MediaFolder>> = HashMap::new();
        for folder in folders {
            by_id.insert(folder.id.as_str(), folder);
            children_by_parent
                .entry(folder.parent_id.as_deref())
                .or_default()
                .push(folder);
        }
        MediaFolderIndex {
            by_id,
            children_by_parent,
        }
    }

    /// The folder with `id`, if present.
    pub fn folder(&self, id: &str) -> Option<&'a MediaFolder> {
        self.by_id.get(id).copied()
    }

    /// Root→folder path for `folder_id` (root first). Cycle-safe via a visited
    /// set (reference `path(for:)`).
    pub fn path(&self, folder_id: Option<&str>) -> Vec<&'a MediaFolder> {
        let mut path: Vec<&'a MediaFolder> = Vec::new();
        let mut current = folder_id.map(str::to_owned);
        let mut visited: HashSet<String> = HashSet::new();
        while let Some(id) = current {
            if !visited.insert(id.clone()) {
                break;
            }
            let Some(folder) = self.by_id.get(id.as_str()).copied() else {
                break;
            };
            path.push(folder);
            current = folder.parent_id.clone();
        }
        path.reverse();
        path
    }

    /// Whether `folder_id` is `ancestor_id` itself or a descendant of it — the
    /// reference `isDescendant(folderId:of:)`. Walks up from `folder_id`;
    /// cycle-safe. (Note the reference returns `true` when `folder_id ==
    /// ancestor_id`, which is what makes the move-into-self guard fire.)
    pub fn is_descendant(&self, folder_id: &str, ancestor_id: &str) -> bool {
        let mut current: Option<String> = Some(folder_id.to_owned());
        let mut visited: HashSet<String> = HashSet::new();
        while let Some(id) = current {
            if !visited.insert(id.clone()) {
                break;
            }
            if id == ancestor_id {
                return true;
            }
            current = self.by_id.get(id.as_str()).and_then(|f| f.parent_id.clone());
        }
        false
    }

    /// `ids` plus every descendant of each (reference `idsIncludingDescendants`).
    pub fn ids_including_descendants(&self, ids: &HashSet<String>) -> HashSet<String> {
        let mut all = ids.clone();
        for id in ids {
            self.collect_descendant_ids(id, &mut all);
        }
        all
    }

    fn collect_descendant_ids(&self, folder_id: &str, ids: &mut HashSet<String>) {
        if let Some(children) = self.children_by_parent.get(&Some(folder_id)) {
            for child in children {
                if ids.insert(child.id.clone()) {
                    self.collect_descendant_ids(&child.id, ids);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{MediaManifestEntry, MediaSource};
    use crate::{Clip, ClipType, Track};

    fn lib() -> MediaLibrary {
        MediaLibrary::new()
    }

    fn add_asset(l: &mut MediaLibrary, id: &str, folder: Option<&str>) {
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
        l.assets.push(a.clone());
        l.manifest.entries.push(MediaManifestEntry {
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

    fn clip(media_ref: &str, start: i32, dur: i32) -> Clip {
        Clip::new(media_ref, start, dur)
    }

    #[test]
    fn create_and_rename_folder() {
        let mut l = lib();
        let id = l.create_folder("Footage", None);
        assert_eq!(l.manifest.folders.len(), 1);
        assert_eq!(l.folder(&id).unwrap().name, "Footage");

        assert!(l.rename_folder(&id, "B-Roll"));
        assert_eq!(l.folder(&id).unwrap().name, "B-Roll");
        // Same name → no change.
        assert!(!l.rename_folder(&id, "B-Roll"));
        // Missing id → no change.
        assert!(!l.rename_folder("nope", "X"));
    }

    #[test]
    fn move_into_self_is_rejected() {
        let mut l = lib();
        let a = l.create_folder("A", None);
        let ids: HashSet<String> = [a.clone()].into_iter().collect();
        // Moving A into A must be rejected (guard 3 / is_descendant self-case).
        assert!(!l.move_folders_to_folder(&ids, Some(&a)));
        assert_eq!(l.folder(&a).unwrap().parent_id, None);
    }

    #[test]
    fn move_into_descendant_is_rejected() {
        let mut l = lib();
        let a = l.create_folder("A", None);
        let b = l.create_folder("B", Some(a.clone()));
        let c = l.create_folder("C", Some(b.clone()));
        // Moving A into C (a descendant of A) must be rejected — would orphan B/C.
        let ids: HashSet<String> = [a.clone()].into_iter().collect();
        assert!(!l.move_folders_to_folder(&ids, Some(&c)));
        assert_eq!(l.folder(&a).unwrap().parent_id, None);

        // Moving A into B (direct child, also a descendant) likewise rejected.
        let ids2: HashSet<String> = [a.clone()].into_iter().collect();
        assert!(!l.move_folders_to_folder(&ids2, Some(&b)));
        assert_eq!(l.folder(&a).unwrap().parent_id, None);
    }

    #[test]
    fn move_no_op_already_parent_is_rejected() {
        let mut l = lib();
        let a = l.create_folder("A", None);
        let b = l.create_folder("B", Some(a.clone()));
        // B is already a child of A → no-op (guard 1).
        let ids: HashSet<String> = [b.clone()].into_iter().collect();
        assert!(!l.move_folders_to_folder(&ids, Some(&a)));
        assert_eq!(l.folder(&b).unwrap().parent_id, Some(a.clone()));
    }

    #[test]
    fn move_folder_accepts_valid_reparent() {
        let mut l = lib();
        let a = l.create_folder("A", None);
        let b = l.create_folder("B", None);
        // Move B under A — valid (A is not a descendant of B).
        let ids: HashSet<String> = [b.clone()].into_iter().collect();
        assert!(l.move_folders_to_folder(&ids, Some(&a)));
        assert_eq!(l.folder(&b).unwrap().parent_id, Some(a.clone()));
        // Move B back to root — valid.
        let ids2: HashSet<String> = [b.clone()].into_iter().collect();
        assert!(l.move_folders_to_folder(&ids2, None));
        assert_eq!(l.folder(&b).unwrap().parent_id, None);
    }

    #[test]
    fn move_assets_to_folder_reparents_and_skips_no_ops() {
        let mut l = lib();
        let f = l.create_folder("F", None);
        add_asset(&mut l, "a1", None);
        add_asset(&mut l, "a2", Some(&f));

        // a1 → F is a real move; a2 → F is a no-op (already there).
        let ids: HashSet<String> = ["a1".into(), "a2".into()].into_iter().collect();
        assert!(l.move_assets_to_folder(&ids, Some(&f)));
        assert_eq!(
            l.assets.iter().find(|a| a.id == "a1").unwrap().folder_id,
            Some(f.clone())
        );
        // Manifest entry kept in sync.
        assert_eq!(
            l.manifest
                .entries
                .iter()
                .find(|e| e.id == "a1")
                .unwrap()
                .folder_id,
            Some(f.clone())
        );

        // Moving both when both already in F → no change.
        let ids2: HashSet<String> = ["a1".into(), "a2".into()].into_iter().collect();
        assert!(!l.move_assets_to_folder(&ids2, Some(&f)));
    }

    #[test]
    fn delete_folders_cascades_to_descendants_assets_and_clips() {
        let mut l = lib();
        let a = l.create_folder("A", None);
        let b = l.create_folder("B", Some(a.clone()));
        let _other = l.create_folder("Other", None);
        add_asset(&mut l, "in_a", Some(&a));
        add_asset(&mut l, "in_b", Some(&b));
        add_asset(&mut l, "outside", None);

        // A timeline track with a clip referencing in_b plus an unrelated clip.
        let mut track = Track::new(ClipType::Video);
        track.clips.push(clip("in_b", 0, 30));
        track.clips.push(clip("outside", 30, 30));
        l.timeline.tracks.push(track);
        // A second track holding ONLY a clip referencing in_a (will go empty → pruned).
        let mut track2 = Track::new(ClipType::Video);
        track2.clips.push(clip("in_a", 0, 30));
        l.timeline.tracks.push(track2);

        let ids: HashSet<String> = [a.clone()].into_iter().collect();
        assert!(l.delete_folders(&ids));

        // A and its descendant B are gone; Other survives.
        assert!(l.folder(&a).is_none());
        assert!(l.folder(&b).is_none());
        assert_eq!(l.manifest.folders.len(), 1);

        // Assets in A/B deleted from both catalog + manifest; outside survives.
        assert!(l.assets.iter().all(|x| x.id == "outside"));
        assert!(l.manifest.entries.iter().all(|x| x.id == "outside"));

        // Clips referencing deleted assets removed; the now-empty track pruned.
        let remaining_clip_refs: Vec<&str> = l
            .timeline
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .map(|c| c.media_ref.as_str())
            .collect();
        assert_eq!(remaining_clip_refs, vec!["outside"]);
        // Track2 (only held in_a) was pruned; track1 survives.
        assert_eq!(l.timeline.tracks.len(), 1);
    }

    #[test]
    fn folder_path_and_subfolders() {
        let mut l = lib();
        let a = l.create_folder("Apple", None);
        let z = l.create_folder("zebra", Some(a.clone())); // mixed case for sort
        let b = l.create_folder("Banana", Some(a.clone()));

        // path(z) = [Apple, zebra] root-first.
        let path: Vec<&str> = l
            .folder_path(Some(&z))
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(path, vec!["Apple", "zebra"]);

        // subfolders(A) sorted case-insensitively: Banana, zebra.
        let subs: Vec<&str> = l
            .subfolders(Some(&a))
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(subs, vec!["Banana", "zebra"]);
        // top-level subfolders → just Apple.
        let top: Vec<&str> = l.subfolders(None).iter().map(|f| f.name.as_str()).collect();
        assert_eq!(top, vec!["Apple"]);
        let _ = b;
    }

    #[test]
    fn is_descendant_self_and_chain() {
        let mut l = lib();
        let a = l.create_folder("A", None);
        let b = l.create_folder("B", Some(a.clone()));
        let c = l.create_folder("C", Some(b.clone()));
        let idx = MediaFolderIndex::new(&l.manifest.folders);
        // self-case is true (this is what makes move-into-self fire).
        assert!(idx.is_descendant(&a, &a));
        // c is a descendant of a (via b).
        assert!(idx.is_descendant(&c, &a));
        assert!(idx.is_descendant(&b, &a));
        // a is NOT a descendant of c.
        assert!(!idx.is_descendant(&a, &c));
    }
}
