//! `ProjectRegistry` + `ProjectEntry` — the Home-window project list (story E2-S11).
//!
//! Ports `Project/ProjectRegistry.swift` (`ProjectRegistry`, `ProjectEntry`,
//! `ProjectRegistryDisk`) to a **synchronous** Rust registry. See
//! docs/reference/project-io.md "Registry" and FOUNDATION §6.1.
//!
//! ## File location (ruling #3)
//!
//! The registry is a JSON **array** of [`ProjectEntry`] at
//! `project-registry.json` under the platform registry dir:
//! - Windows: `%APPDATA%\PalmierProWin\project-registry.json`
//! - Linux:   `~/.config/palmier-pro/project-registry.json`
//!
//! ([`ProjectRegistry::default_path`] resolves it via the `dirs` crate.) The
//! filename matches the reference `Project.registryFilename`
//! (`Utilities/Constants.swift:104`), keeping interop with reference bundles.
//!
//! ## Dropping the async-load / `pendingMutations` complexity
//!
//! The reference loads the array on a background actor and replays mutations that
//! arrive mid-load (`pendingMutations`). The story's ruling: a **synchronous**
//! registry is acceptable as long as **(1) atomic full-array writes** and
//! **(2) standardized-URL dedup** are preserved (docs/reference/project-io.md
//! Port risks "Registry race"). We load eagerly in [`ProjectRegistry::load`] and
//! write the whole array atomically on every mutation, so there is no race to
//! replay.
//!
//! ## Standardized-URL dedup (ruling-aligned carry-forward)
//!
//! The reference dedup key is `URL.standardizedFileURL`. On Windows
//! `std::fs::canonicalize` would (a) FAIL for not-yet-created paths and (b) emit
//! `\\?\` verbatim prefixes — both wrong for a dedup key. We instead **lexically
//! normalize** the path ([`normalize_path`]): resolve `.`/`..` segments, unify
//! separators, and case-fold on Windows (NTFS is case-insensitive), WITHOUT
//! touching the filesystem. So `C:\a\.\b` and `C:/A/b` dedup to one entry even
//! before the bundle exists.
//!
//! ## Atomic full-array writes
//!
//! Every mutation re-serializes the entire `Vec<ProjectEntry>` and writes it via
//! [`atomic_write`] (write-temp-in-same-dir → atomic rename), mirroring the
//! reference `Data.write(options: .atomic)`. A crash mid-write leaves the prior
//! registry intact.
//!
//! ## `delete` → Recycle Bin / trash
//!
//! [`ProjectRegistry::delete`] trashes the bundle on disk (Recycle Bin on
//! Windows / XDG trash on Linux via the `trash` crate, behind the `system-trash`
//! feature) and then removes the entry — matching the reference
//! `FileManager.trashItem` then `remove`. A [`Trasher`] seam lets tests inject a
//! no-op/recording trasher so they never touch the real OS trash.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::bundle::project;
use crate::bundle::{BundleError, Result};

/// One row in the project registry (reference `ProjectEntry`).
///
/// `created_date` / `last_opened_date` serialize as **Apple reference-epoch
/// doubles** (the default-`JSONEncoder` numeric date the reference uses for the
/// registry), reusing `palmier_model`'s codec so the on-disk format matches a
/// reference-written `project-registry.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectEntry {
    /// Stable identity (reference `id: UUID`).
    pub id: Uuid,
    /// The `.palmier` bundle path (reference `url: URL`). Stored as written; the
    /// dedup key is the *normalized* form, computed on demand.
    pub url: PathBuf,
    /// When the entry was first registered.
    #[serde(
        rename = "createdDate",
        with = "palmier_model::serde_date::apple_ref_epoch"
    )]
    pub created_date: OffsetDateTime,
    /// When the project was last opened (the sort key, descending).
    #[serde(
        rename = "lastOpenedDate",
        with = "palmier_model::serde_date::apple_ref_epoch"
    )]
    pub last_opened_date: OffsetDateTime,
}

impl ProjectEntry {
    /// Display name = the bundle's filename minus its `.palmier` extension
    /// (reference `ProjectEntry.name`).
    pub fn name(&self) -> String {
        self.url
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string()
    }

    /// Whether the bundle still exists on disk (reference
    /// `ProjectEntry.isAccessible`).
    pub fn is_accessible(&self) -> bool {
        self.url.exists()
    }

    /// The lexically-normalized dedup key for this entry's url.
    fn dedup_key(&self) -> String {
        normalize_path(&self.url)
    }
}

/// Abstraction over "move a path to the OS trash", so tests can avoid the real
/// Recycle Bin / XDG trash. The default [`SystemTrasher`] performs the real move
/// when the `system-trash` feature is on.
pub trait Trasher {
    /// Move `path` to the platform trash. Returns `Ok(())` on success **or** when
    /// the path does not exist (matching the reference `trashIfPresent`, which
    /// returns `true` when the file is already gone).
    fn trash(&self, path: &Path) -> std::io::Result<()>;
}

/// The real Recycle-Bin / XDG-trash trasher (reference `FileManager.trashItem`).
///
/// With `system-trash` enabled it delegates to the `trash` crate; without it,
/// the move is a no-op error so a misbuilt binary fails loudly rather than
/// silently hard-deleting.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTrasher;

impl Trasher for SystemTrasher {
    fn trash(&self, path: &Path) -> std::io::Result<()> {
        if !path.exists() {
            // Reference `trashIfPresent`: already gone ⇒ success.
            return Ok(());
        }
        #[cfg(feature = "system-trash")]
        {
            trash::delete(path).map_err(|e| std::io::Error::other(e.to_string()))
        }
        #[cfg(not(feature = "system-trash"))]
        {
            let _ = path;
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "system-trash feature disabled: refusing to delete bundle",
            ))
        }
    }
}

/// The project registry: an ordered, deduped list of [`ProjectEntry`] backed by
/// `project-registry.json`. Synchronous; every mutation writes the whole array
/// atomically.
///
/// Construct with [`ProjectRegistry::load`] (real disk) or
/// [`ProjectRegistry::with_path`] (a chosen path — used by tests). The trasher is
/// [`SystemTrasher`] by default; [`ProjectRegistry::with_trasher`] injects a
/// custom one for tests.
pub struct ProjectRegistry {
    path: PathBuf,
    entries: Vec<ProjectEntry>,
    trasher: Box<dyn Trasher + Send + Sync>,
}

impl ProjectRegistry {
    /// The default registry path: `<config_dir>/PalmierProWin/project-registry.json`
    /// on Windows (`%APPDATA%`), `~/.config/palmier-pro/project-registry.json` on
    /// Linux. Returns `None` only if the OS has no config dir.
    pub fn default_path() -> Option<PathBuf> {
        let base = dirs::config_dir()?;
        // `dirs::config_dir()` = `%APPDATA%` on Windows, `~/.config` on Linux.
        // Append the app folder so we live at `…\PalmierProWin\` / `…/palmier-pro/`.
        #[cfg(windows)]
        let dir = base.join("PalmierProWin");
        #[cfg(not(windows))]
        let dir = base.join("palmier-pro");
        Some(dir.join(project::REGISTRY_FILE))
    }

    /// Load the registry from [`default_path`](Self::default_path). A missing or
    /// unreadable file yields an empty registry (reference `loadEntries` returns
    /// `[]` on any error). Errors only if no OS config dir exists.
    pub fn load() -> Result<Self> {
        let path = Self::default_path().ok_or_else(|| {
            BundleError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no OS config directory for the project registry",
            ))
        })?;
        Ok(Self::with_path(path))
    }

    /// Build a registry backed by `path`, loading existing entries if present.
    /// A missing/corrupt file ⇒ empty (lenient, like the reference).
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = load_entries(&path);
        ProjectRegistry {
            path,
            entries,
            trasher: Box::new(SystemTrasher),
        }
    }

    /// Override the trasher (tests inject a recording/no-op one).
    pub fn with_trasher(mut self, trasher: Box<dyn Trasher + Send + Sync>) -> Self {
        self.trasher = trasher;
        self
    }

    /// The backing file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// All entries **sorted newest-first** by `last_opened_date` descending
    /// (reference `sortedEntries`).
    pub fn sorted_entries(&self) -> Vec<ProjectEntry> {
        let mut v = self.entries.clone();
        v.sort_by(|a, b| b.last_opened_date.cmp(&a.last_opened_date));
        v
    }

    /// The raw (unsorted) entries, as stored.
    pub fn entries(&self) -> &[ProjectEntry] {
        &self.entries
    }

    // ---- Mutations (each persists the whole array atomically) ----

    /// Register a project at `url` (reference `register`).
    ///
    /// Standardizes the url to its [dedup key](normalize_path); if an entry with
    /// the same key exists, **bumps its `last_opened_date`**, else appends a new
    /// entry (`new UUID`, `created = last_opened = now`).
    pub fn register(&mut self, url: impl AsRef<Path>) -> Result<()> {
        let url = url.as_ref().to_path_buf();
        let key = normalize_path(&url);
        let now = OffsetDateTime::now_utc();
        if let Some(e) = self.entries.iter_mut().find(|e| e.dedup_key() == key) {
            e.last_opened_date = now;
        } else {
            self.entries.push(ProjectEntry {
                id: Uuid::new_v4(),
                url,
                created_date: now,
                last_opened_date: now,
            });
        }
        self.save()
    }

    /// Remove the entry for `url` **without touching disk** (reference `remove`).
    pub fn remove(&mut self, url: impl AsRef<Path>) -> Result<()> {
        let key = normalize_path(url.as_ref());
        self.entries.retain(|e| e.dedup_key() != key);
        self.save()
    }

    /// Trash the bundle on disk, then remove the entry (reference `delete`).
    ///
    /// The bundle is moved to the Recycle Bin / XDG trash via the [`Trasher`]; on
    /// trash success (which includes "already gone") the entry is removed and the
    /// array re-saved. A trash failure leaves the entry intact and returns the
    /// error (reference: `guard … trashIfPresent else return`).
    pub fn delete(&mut self, url: impl AsRef<Path>) -> Result<()> {
        let url = url.as_ref();
        self.trasher.trash(url).map_err(BundleError::Io)?;
        self.remove(url)
    }

    /// Rewrite an entry's url after a Save-As / rename and bump `last_opened`
    /// (reference `updateURL`). A no-op if no entry matches `old` (the reference
    /// guards on `firstIndex`).
    pub fn update_url(&mut self, old: impl AsRef<Path>, new: impl AsRef<Path>) -> Result<()> {
        let old_key = normalize_path(old.as_ref());
        let new_url = new.as_ref().to_path_buf();
        let now = OffsetDateTime::now_utc();
        if let Some(e) = self.entries.iter_mut().find(|e| e.dedup_key() == old_key) {
            e.url = new_url;
            e.last_opened_date = now;
        }
        self.save()
    }

    /// Re-serialize the whole array and write it atomically (reference
    /// `saveEntries` + `Data.write(.atomic)`).
    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.entries).map_err(|e| BundleError::WriteUnknown {
            file: "project-registry.json",
            detail: e.to_string(),
        })?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        atomic_write(&self.path, &bytes)?;
        Ok(())
    }
}

/// Load `Vec<ProjectEntry>` from `path`; any read/decode failure ⇒ empty vec
/// (reference `loadEntries`: `try?` everything, fall back to `[]`).
fn load_entries(path: &Path) -> Vec<ProjectEntry> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Write `bytes` to `path` atomically: write a sibling temp file then rename it
/// over `path` (same dir ⇒ same filesystem ⇒ atomic rename). On Windows
/// `fs::rename` replaces an existing file, so no remove-first dance is needed for
/// a regular file.
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("registry"),
        Uuid::new_v4()
    ));
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Lexically normalize a path for use as the dedup key (reference
/// `URL.standardizedFileURL`), WITHOUT hitting the filesystem (so it works for
/// not-yet-created bundles, unlike `canonicalize`).
///
/// - resolves `.` (drop) and `..` (pop) segments,
/// - normalizes separators to the platform's (`PathBuf` does this),
/// - lowercases on Windows (NTFS is case-insensitive) so `C:\A` == `C:\a`,
/// - strips a single trailing separator.
///
/// This is intentionally pure-lexical: two strings that denote the same path
/// after `.`/`..` resolution dedup, but symlinks are NOT resolved (the reference
/// `standardizedFileURL` also does not resolve symlinks — that's
/// `resolvingSymlinksInPath`).
pub fn normalize_path(path: &Path) -> String {
    let mut out: Vec<Component> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop the last normal component, but never past a root/prefix.
                match out.last() {
                    Some(Component::Normal(_)) => {
                        out.pop();
                    }
                    _ => out.push(comp),
                }
            }
            other => out.push(other),
        }
    }
    let rebuilt: PathBuf = out.iter().collect();
    let s = rebuilt.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        s.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// Scratch dir under the OS temp dir (no `tempfile` dep).
    fn scratch() -> PathBuf {
        let p = std::env::temp_dir().join(format!("palmier-e2s11-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// A trasher that records what it was asked to trash and never touches disk.
    #[derive(Default)]
    struct RecordingTrasher {
        trashed: Mutex<Vec<PathBuf>>,
    }
    impl Trasher for RecordingTrasher {
        fn trash(&self, path: &Path) -> std::io::Result<()> {
            self.trashed.lock().unwrap().push(path.to_path_buf());
            Ok(())
        }
    }

    fn reg(dir: &Path) -> ProjectRegistry {
        ProjectRegistry::with_path(dir.join(project::REGISTRY_FILE))
    }

    #[test]
    fn register_dedups_on_normalized_path_and_bumps_last_opened() {
        let dir = scratch();
        let mut r = reg(&dir);

        // Two paths that differ only by `.`/`..`/separators/case normalize to one.
        let a = dir.join("Proj.palmier");
        // Build a messy equivalent: <dir>/sub/../Proj.palmier with mixed case.
        let messy = dir.join("sub").join("..").join("Proj.palmier");

        r.register(&a).unwrap();
        let first_opened = r.entries()[0].last_opened_date;
        assert_eq!(r.entries().len(), 1);

        // Ensure a strictly-later timestamp so the bump is observable.
        std::thread::sleep(std::time::Duration::from_millis(5));
        r.register(&messy).unwrap();
        assert_eq!(r.entries().len(), 1, "messy path must dedup to one entry");
        assert!(
            r.entries()[0].last_opened_date >= first_opened,
            "register on an existing path bumps last_opened_date"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sorted_entries_newest_first() {
        let dir = scratch();
        let mut r = reg(&dir);
        r.register(dir.join("A.palmier")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        r.register(dir.join("B.palmier")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        r.register(dir.join("C.palmier")).unwrap();

        let sorted = r.sorted_entries();
        let names: Vec<String> = sorted.iter().map(|e| e.name()).collect();
        assert_eq!(names, vec!["C", "B", "A"], "newest last_opened first");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn update_url_moves_entry_not_orphan() {
        let dir = scratch();
        let mut r = reg(&dir);
        let old = dir.join("Old.palmier");
        let new = dir.join("Renamed.palmier");
        r.register(&old).unwrap();
        let id = r.entries()[0].id;

        r.update_url(&old, &new).unwrap();
        assert_eq!(r.entries().len(), 1, "update_url must not add an entry");
        assert_eq!(r.entries()[0].id, id, "same entry (id preserved)");
        assert_eq!(r.entries()[0].name(), "Renamed");

        // The old key no longer resolves; a fresh register of `old` would add a
        // SECOND entry (proving the entry truly moved, not orphaned-and-kept).
        r.update_url(&old, dir.join("Nope.palmier")).unwrap();
        assert_eq!(r.entries()[0].name(), "Renamed", "no-op when old missing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn delete_trashes_then_removes_entry() {
        let dir = scratch();
        // Create a real bundle dir so a real trash WOULD have something, but we
        // inject a recording trasher so nothing actually moves.
        let bundle = dir.join("Doomed.palmier");
        std::fs::create_dir_all(&bundle).unwrap();

        let recorder = Box::new(RecordingTrasher::default());
        let mut r = reg(&dir).with_trasher(recorder);
        r.register(&bundle).unwrap();
        assert_eq!(r.entries().len(), 1);

        r.delete(&bundle).unwrap();
        assert_eq!(r.entries().len(), 0, "delete removes the entry");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remove_deletes_entry_only() {
        let dir = scratch();
        let bundle = dir.join("Keep.palmier");
        std::fs::create_dir_all(&bundle).unwrap();
        let mut r = reg(&dir);
        r.register(&bundle).unwrap();
        r.remove(&bundle).unwrap();
        assert_eq!(r.entries().len(), 0);
        assert!(bundle.is_dir(), "remove must NOT touch the bundle on disk");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn on_disk_json_is_well_formed_array_after_each_mutation() {
        let dir = scratch();
        let mut r = reg(&dir);
        let path = dir.join(project::REGISTRY_FILE);

        r.register(dir.join("A.palmier")).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(v.is_array() && v.as_array().unwrap().len() == 1);

        r.register(dir.join("B.palmier")).unwrap();
        let v2: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v2.as_array().unwrap().len(), 2);

        // Reloading from disk yields the same set (round-trip through the file).
        let r2 = reg(&dir);
        assert_eq!(r2.entries().len(), 2);

        // Dates serialize as JSON numbers (apple-epoch), matching the reference.
        let first = &v2.as_array().unwrap()[0];
        assert!(first["createdDate"].is_number(), "dates are numeric: {first}");
        assert!(first["lastOpenedDate"].is_number());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn normalize_path_resolves_dot_and_dotdot() {
        let base = if cfg!(windows) { r"C:\a\b" } else { "/a/b" };
        let p1 = PathBuf::from(base).join(".").join("c.palmier");
        let p2 = PathBuf::from(base).join("x").join("..").join("c.palmier");
        assert_eq!(normalize_path(&p1), normalize_path(&p2));
    }

    #[test]
    fn entry_name_strips_extension() {
        let e = ProjectEntry {
            id: Uuid::new_v4(),
            url: PathBuf::from(if cfg!(windows) {
                r"C:\proj\My Movie.palmier"
            } else {
                "/proj/My Movie.palmier"
            }),
            created_date: OffsetDateTime::now_utc(),
            last_opened_date: OffsetDateTime::now_utc(),
        };
        assert_eq!(e.name(), "My Movie");
    }

    #[test]
    fn missing_registry_file_loads_empty() {
        let dir = scratch();
        let r = reg(&dir);
        assert!(r.entries().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
