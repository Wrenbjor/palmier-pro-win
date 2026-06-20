//! Directory-as-document file-dialog seam (FR-8, story E2-S12).
//!
//! A `.palmier` bundle is a **directory** that, on macOS, the Finder presents as
//! one file (an NSDocument package). Windows has **no native package concept**, so
//! the reference open/save panel config (`canChooseDirectories=false`,
//! `treatsFilePackagesAsDirectories=false` — "the user picks the `.palmier` dir as
//! one item") must be reproduced by a **Tauri custom file dialog**
//! (docs/reference/project-io.md "macOS APIs to replace"). On Linux a `.palmier`
//! directory behaves naturally as a directory in the picker.
//!
//! ## Scope (this story = the Rust side only)
//!
//! This module implements the **Rust side** of that seam: the dialog
//! configuration ([`DirectoryDocumentDialog`]) the Tauri layer (`palmier-tauri`)
//! consumes to present `.palmier` directories as single documents, plus the
//! validation that a chosen path is a real `.palmier` bundle. The actual native
//! dialog invocation lives in the Tauri layer; the OS Explorer **shell extension**
//! (so a `.palmier` dir shows as one file in Explorer itself) is **out of scope**
//! and recorded as a documented TODO below.
//!
//! ### TODO (out of scope — Explorer shell extension)
//!
//! FR-8 mentions "both" a custom dialog and an Explorer shell extension. Per the
//! E2-S12 ruling this story ships the **Tauri custom dialog only**; a Windows
//! Explorer shell-namespace extension (making a `.palmier` directory render as a
//! single document in Explorer, double-click to open the app) is deferred. See
//! docs/reference/project-io.md Open questions "Windows directory-as-single-document
//! UX: is a shell extension required for v1". Tracked as a follow-up.

use std::path::Path;

use crate::bundle::project;

/// How the file picker should treat directories, mirroring the reference open/save
/// panel flags (`canChooseDirectories` / `treatsFilePackagesAsDirectories`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectorySelection {
    /// macOS package semantics: the user selects a `.palmier` **directory** as a
    /// single document (cannot descend into it in the picker). This is the
    /// Windows/Tauri target — the custom dialog filters to `.palmier` dirs and
    /// returns the directory itself, not its contents.
    AsSingleDocument,
    /// Plain directory chooser (used on Linux where a `.palmier` dir is just a
    /// directory; the picker descends normally).
    AsDirectory,
}

/// The configuration the Tauri layer uses to present the open/save dialog for
/// `.palmier` bundles (reference NSOpen/SavePanel config).
///
/// On Windows the platform layer builds a **custom** dialog from this (there is no
/// native package bit); on Linux it maps to a directory chooser.
#[derive(Debug, Clone)]
pub struct DirectoryDocumentDialog {
    /// The bundle extension to filter on (always `palmier`).
    pub extension: &'static str,
    /// How directories are presented/selected.
    pub selection: DirectorySelection,
    /// Whether the user may create a new bundle (Save-As) vs only open existing.
    pub allow_create: bool,
}

impl DirectoryDocumentDialog {
    /// The **open** dialog: present `.palmier` directories as single documents,
    /// open-only (no create). Matches the reference open panel
    /// (`canChooseDirectories=false`, `treatsFilePackagesAsDirectories=false`).
    pub fn open() -> Self {
        DirectoryDocumentDialog {
            extension: project::FILE_EXTENSION,
            selection: DirectorySelection::AsSingleDocument,
            allow_create: false,
        }
    }

    /// The **save / Save-As** dialog: same single-document presentation, but the
    /// user may name a new `.palmier` bundle (reference NSSavePanel).
    pub fn save() -> Self {
        DirectoryDocumentDialog {
            extension: project::FILE_EXTENSION,
            selection: DirectorySelection::AsSingleDocument,
            allow_create: true,
        }
    }

    /// The Linux variant: a plain directory chooser (a `.palmier` dir is a
    /// directory there). The platform layer calls this on non-Windows.
    pub fn open_directory_native() -> Self {
        DirectoryDocumentDialog {
            extension: project::FILE_EXTENSION,
            selection: DirectorySelection::AsDirectory,
            allow_create: false,
        }
    }

    /// Whether the picker should present `.palmier` dirs as single (non-descendable)
    /// documents (the Windows/Tauri custom-dialog behavior).
    pub fn presents_directory_as_document(&self) -> bool {
        self.selection == DirectorySelection::AsSingleDocument
    }

    /// Validate that a user-chosen path is a usable `.palmier` bundle selection:
    /// it must have the `.palmier` extension (by name; the dir need not yet exist
    /// for Save-As). For an **open** dialog the path must additionally be an
    /// existing directory.
    pub fn validate_selection(&self, path: &Path) -> std::result::Result<(), DialogError> {
        let has_ext = path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case(self.extension));
        if !has_ext {
            return Err(DialogError::WrongExtension);
        }
        if !self.allow_create {
            // Open: the bundle must exist and be a directory (package = dir).
            if !path.exists() {
                return Err(DialogError::NotFound);
            }
            if !path.is_dir() {
                return Err(DialogError::NotADirectory);
            }
        }
        Ok(())
    }
}

/// Why a chosen dialog path is not a valid `.palmier` bundle selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogError {
    /// The path does not end in `.palmier`.
    WrongExtension,
    /// An open-dialog selection that does not exist on disk.
    NotFound,
    /// An open-dialog selection that exists but is a file, not a bundle directory.
    NotADirectory,
}

impl std::fmt::Display for DialogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DialogError::WrongExtension => write!(f, "not a .palmier bundle (wrong extension)"),
            DialogError::NotFound => write!(f, "bundle does not exist"),
            DialogError::NotADirectory => write!(f, "bundle path is a file, not a directory"),
        }
    }
}

impl std::error::Error for DialogError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn scratch() -> PathBuf {
        let p = std::env::temp_dir().join(format!("palmier-e2s12-dlg-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn open_dialog_presents_directory_as_single_document() {
        let d = DirectoryDocumentDialog::open();
        // The reference open-panel semantics: package = one selectable item.
        assert!(d.presents_directory_as_document());
        assert_eq!(d.selection, DirectorySelection::AsSingleDocument);
        assert!(!d.allow_create);
        assert_eq!(d.extension, "palmier");
    }

    #[test]
    fn linux_native_variant_is_plain_directory() {
        let d = DirectoryDocumentDialog::open_directory_native();
        assert!(!d.presents_directory_as_document());
        assert_eq!(d.selection, DirectorySelection::AsDirectory);
    }

    #[test]
    fn validate_open_requires_existing_palmier_dir() {
        let root = scratch();
        let bundle = root.join("Real.palmier");
        std::fs::create_dir_all(&bundle).unwrap();

        let open = DirectoryDocumentDialog::open();
        // A real .palmier directory validates.
        assert!(open.validate_selection(&bundle).is_ok());
        // Wrong extension is rejected.
        assert_eq!(
            open.validate_selection(&root.join("file.txt")).unwrap_err(),
            DialogError::WrongExtension
        );
        // A .palmier name that doesn't exist is rejected for OPEN.
        assert_eq!(
            open.validate_selection(&root.join("Missing.palmier"))
                .unwrap_err(),
            DialogError::NotFound
        );
        // A .palmier path that is a FILE (not a dir) is rejected.
        let as_file = root.join("AsFile.palmier");
        std::fs::write(&as_file, b"x").unwrap();
        assert_eq!(
            open.validate_selection(&as_file).unwrap_err(),
            DialogError::NotADirectory
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn validate_save_allows_nonexistent_palmier_path() {
        let root = scratch();
        let save = DirectoryDocumentDialog::save();
        assert!(save.allow_create);
        // Save-As may name a not-yet-created bundle.
        assert!(save.validate_selection(&root.join("New.palmier")).is_ok());
        // Still must have the right extension.
        assert_eq!(
            save.validate_selection(&root.join("New.txt")).unwrap_err(),
            DialogError::WrongExtension
        );
        std::fs::remove_dir_all(&root).unwrap();
    }
}
