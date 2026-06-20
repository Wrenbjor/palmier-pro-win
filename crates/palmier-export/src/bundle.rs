//! E6-S7 — `.palmier` self-contained bundle export.
//!
//! Ported literally from the macOS reference
//! `Sources/PalmierPro/Export/PalmierProjectExporter.swift`
//! (`PalmierProjectExporter.export`). Collects every resolvable media reference
//! into the new bundle's `media/` directory and rewrites it to a project-relative
//! [`MediaSource::Project`], producing a portable bundle with no dangling
//! external references. See docs/reference/export.md §C.
//!
//! ## Algorithm (verbatim)
//! 1. Stage to a temp dir `palmier-export-{uuid}/`; create `media/`.
//! 2. Per manifest entry: resolve source (`External` → abs path; `Project` →
//!    `project_dir/rel`). Missing → `report.missing += {id, name}`, keep the
//!    entry dangling. **Dedup** by the standardized absolute source path.
//! 3. Copy to `media/{name}`: `Project` entries keep their `lastPathComponent`;
//!    `External` → `import-{id8}.{ext}`. **Collisions** get `-1, -2, …`
//!    (`unique_path`). Rewrite the entry source → `Project { media/{file} }`.
//!    External → `collected += id`; project-copied → `copied_internal += 1`. Sum
//!    `total_bytes`.
//! 4. Encode `project.json` / `media.json` / `generation-log.json` (serde_json);
//!    copy `thumbnail.jpg` and the `chat/` dir when present.
//! 5. Remove existing dest, create parent, **move** staging → dest.
//!
//! ## Reference filenames are load-bearing (ruling #3)
//! `project.json` / `media.json` / `generation-log.json` / `thumbnail.jpg` /
//! `chat/` — renaming any of these breaks sample import.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use palmier_model::{GenerationLog, MediaManifest, MediaSource, Timeline};

/// Reference bundle filenames + constants (`Utilities/Constants.swift` `Project`).
pub const TIMELINE_FILENAME: &str = "project.json";
pub const MANIFEST_FILENAME: &str = "media.json";
pub const GENERATION_LOG_FILENAME: &str = "generation-log.json";
pub const THUMBNAIL_FILENAME: &str = "thumbnail.jpg";
pub const MEDIA_DIRECTORY_NAME: &str = "media";
pub const CHAT_DIR_NAME: &str = "chat";
pub const BUNDLE_EXTENSION: &str = "palmier";

/// An entry whose source file couldn't be found (reference `Report.Missing`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing {
    pub id: String,
    pub name: String,
}

/// The result of a bundle export (reference `PalmierProjectExporter.Report`,
/// mapped 1:1 — FR-23).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// Entry ids that were `External` and are now bundled.
    pub collected: Vec<String>,
    /// Already-internal (`Project`) media files copied across.
    pub copied_internal: i64,
    /// Entries whose source file couldn't be found.
    pub missing: Vec<Missing>,
    /// Total bytes copied into the new bundle.
    pub total_bytes: i64,
}

/// Errors from the bundle export (filesystem + encode failures).
#[derive(Debug)]
pub enum ExportError {
    Io(io::Error),
    Encode(serde_json::Error),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Io(e) => write!(f, "io error: {e}"),
            ExportError::Encode(e) => write!(f, "encode error: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}

impl From<io::Error> for ExportError {
    fn from(e: io::Error) -> Self {
        ExportError::Io(e)
    }
}
impl From<serde_json::Error> for ExportError {
    fn from(e: serde_json::Error) -> Self {
        ExportError::Encode(e)
    }
}

/// Write a self-contained `.palmier` bundle at `dest`.
///
/// `source_project_dir` is the existing bundle directory used to resolve
/// `Project` media sources and to carry across `thumbnail.jpg` / `chat/`
/// (`None` when exporting a never-saved project — those carry-across steps are
/// then skipped, matching the reference's optional `sourceProjectURL`).
///
/// `temp_root` is the directory to stage under (the caller supplies it — the
/// reference uses `FileManager.temporaryDirectory`; injecting it keeps this
/// testable and avoids a global temp dependency).
pub fn export_palmier_project(
    timeline: &Timeline,
    manifest: &MediaManifest,
    generation_log: &GenerationLog,
    source_project_dir: Option<&Path>,
    dest: &Path,
    temp_root: &Path,
) -> Result<Report, ExportError> {
    // E11-S6 follow-up: pause visual-search indexing for the duration of this
    // export run (reference: `ExportService.exportPalmierProject` sets
    // `isExporting = true` → `SearchIndexCoordinator.exportDidBegin()`, with the
    // matching end in its `defer`). The RAII guard ends the pause on drop —
    // including any early `?` return or panic — so the counter can never get
    // stuck and wedge indexing.
    let _export_pause = palmier_search::ExportPauseGuard::begin();

    let staging = temp_root.join(format!("palmier-export-{}", uuid::Uuid::new_v4()));
    let media_dir = staging.join(MEDIA_DIRECTORY_NAME);
    fs::create_dir_all(&media_dir)?;

    // RAII-ish staging cleanup on any early return.
    let result = export_inner(
        timeline,
        manifest,
        generation_log,
        source_project_dir,
        dest,
        &staging,
        &media_dir,
    );
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn export_inner(
    timeline: &Timeline,
    manifest: &MediaManifest,
    generation_log: &GenerationLog,
    source_project_dir: Option<&Path>,
    dest: &Path,
    staging: &Path,
    media_dir: &Path,
) -> Result<Report, ExportError> {
    let mut report = Report::default();
    let mut new_entries = manifest.entries.clone();
    // Dedup: standardized absolute source path → "media/<file>".
    let mut relative_by_source: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for entry in new_entries.iter_mut() {
        let src = source_path(&entry.source, source_project_dir);

        // Missing source → record, keep the (dangling) reference, continue.
        let Some(src_path) = src.filter(|p| p.exists()) else {
            report.missing.push(Missing {
                id: entry.id.clone(),
                name: entry.name.clone(),
            });
            continue;
        };

        let key = standardized_path(&src_path);
        let relative_path = if let Some(existing) = relative_by_source.get(&key) {
            existing.clone()
        } else {
            let preferred = preferred_filename(entry, &src_path);
            let dest_file = unique_path(media_dir, &preferred);
            fs::copy(&src_path, &dest_file)?;
            let rel = format!(
                "{}/{}",
                MEDIA_DIRECTORY_NAME,
                dest_file.file_name().and_then(|s| s.to_str()).unwrap_or(&preferred)
            );
            relative_by_source.insert(key, rel.clone());
            report.total_bytes += file_size(&dest_file);
            if matches!(entry.source, MediaSource::Project { .. }) {
                report.copied_internal += 1;
            }
            rel
        };

        if matches!(entry.source, MediaSource::External { .. }) {
            report.collected.push(entry.id.clone());
        }
        entry.source = MediaSource::Project {
            relative_path,
        };
    }

    let mut new_manifest = manifest.clone();
    new_manifest.entries = new_entries;

    // Encode the three JSON documents (serde_json, matching JSONEncoder shapes).
    write_json(&staging.join(TIMELINE_FILENAME), timeline)?;
    write_json(&staging.join(MANIFEST_FILENAME), &new_manifest)?;
    write_json(&staging.join(GENERATION_LOG_FILENAME), generation_log)?;

    // Carry across non-media bundle contents when a source dir is present.
    if let Some(src_dir) = source_project_dir {
        copy_if_present(&src_dir.join(THUMBNAIL_FILENAME), &staging.join(THUMBNAIL_FILENAME))?;
        copy_dir_if_present(&src_dir.join(CHAT_DIR_NAME), &staging.join(CHAT_DIR_NAME))?;
    }

    // Remove existing dest, create parent, move staging → dest.
    if dest.exists() {
        remove_path(dest)?;
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    move_dir(staging, dest)?;
    Ok(report)
}

/// Resolve a [`MediaSource`] to an absolute path (reference `sourceURL(for:)`):
/// `External` → its path; `Project` → `project_dir/rel` (requires a dir).
fn source_path(source: &MediaSource, project_dir: Option<&Path>) -> Option<PathBuf> {
    match source {
        MediaSource::External { absolute_path } => Some(PathBuf::from(absolute_path)),
        MediaSource::Project { relative_path } => project_dir.map(|d| d.join(relative_path)),
    }
}

/// Preferred destination filename (reference `filename(for:sourceURL:)`):
/// `Project` entries keep the source's `lastPathComponent`; `External` →
/// `import-{id8}.{ext}` (no ext → bare `import-{id8}`).
fn preferred_filename(entry: &palmier_model::MediaManifestEntry, src: &Path) -> String {
    match entry.source {
        MediaSource::Project { .. } => src
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&entry.name)
            .to_string(),
        MediaSource::External { .. } => {
            let id8: String = entry.id.chars().take(8).collect();
            let base = format!("import-{id8}");
            match src.extension().and_then(|s| s.to_str()) {
                Some(ext) if !ext.is_empty() => format!("{base}.{ext}"),
                _ => base,
            }
        }
    }
}

/// A path in `dir` that doesn't yet exist: `preferred`, else
/// `{stem}-1.{ext}`, `{stem}-2.{ext}`, … (reference `uniqueURL`).
fn unique_path(dir: &Path, preferred: &str) -> PathBuf {
    let candidate = dir.join(preferred);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(preferred);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(preferred);
    let ext = p.extension().and_then(|s| s.to_str());
    let mut n = 1;
    loop {
        let name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem}-{n}.{ext}"),
            _ => format!("{stem}-{n}"),
        };
        let url = dir.join(&name);
        if !url.exists() {
            return url;
        }
        n += 1;
    }
}

/// A canonicalized absolute path key for dedup (reference
/// `srcURL.standardizedFileURL.path`). Falls back to the lexical path when the
/// file can't be canonicalized.
fn standardized_path(p: &Path) -> String {
    fs::canonicalize(p)
        .ok()
        .map(|c| c.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string_lossy().to_string())
}

fn file_size(p: &Path) -> i64 {
    fs::metadata(p).map(|m| m.len() as i64).unwrap_or(0)
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), ExportError> {
    let bytes = serde_json::to_vec(value)?;
    fs::write(path, bytes)?;
    Ok(())
}

fn copy_if_present(src: &Path, dest: &Path) -> Result<(), ExportError> {
    if src.exists() {
        fs::copy(src, dest)?;
    }
    Ok(())
}

fn copy_dir_if_present(src: &Path, dest: &Path) -> Result<(), ExportError> {
    if src.exists() && src.is_dir() {
        copy_dir_recursive(src, dest)?;
    }
    Ok(())
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<(), ExportError> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn remove_path(p: &Path) -> Result<(), ExportError> {
    if p.is_dir() {
        fs::remove_dir_all(p)?;
    } else {
        fs::remove_file(p)?;
    }
    Ok(())
}

/// Move a directory `from` → `to`. Tries a rename first (atomic, same volume);
/// falls back to copy-then-remove across volumes (reference `moveItem`).
fn move_dir(from: &Path, to: &Path) -> Result<(), ExportError> {
    match fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            copy_dir_recursive(from, to)?;
            fs::remove_dir_all(from)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::{ClipType, MediaManifestEntry};

    fn entry(id: &str, name: &str, source: MediaSource) -> MediaManifestEntry {
        MediaManifestEntry {
            id: id.into(),
            name: name.into(),
            asset_type: ClipType::Video,
            source,
            duration: 4.0,
            generation_input: None,
            source_width: None,
            source_height: None,
            source_fps: None,
            has_audio: None,
            folder_id: None,
            cached_remote_url: None,
            cached_remote_url_expires_at: None,
        }
    }

    /// A unique temp dir under the OS temp root for this test process.
    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("palmier-test-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn preferred_filename_rules() {
        // External → import-{id8}.{ext}.
        let ext_entry = entry(
            "abcdef0123456789",
            "x",
            MediaSource::External {
                absolute_path: "/a/clip.mov".into(),
            },
        );
        assert_eq!(
            preferred_filename(&ext_entry, Path::new("/a/clip.mov")),
            "import-abcdef01.mov"
        );
        // Project → keep lastPathComponent.
        let proj_entry = entry(
            "p1",
            "x",
            MediaSource::Project {
                relative_path: "media/internal.mov".into(),
            },
        );
        assert_eq!(
            preferred_filename(&proj_entry, Path::new("/proj/media/internal.mov")),
            "internal.mov"
        );
    }

    #[test]
    fn unique_path_appends_suffix_on_collision() {
        let dir = temp_dir("unique");
        // First is free.
        let p0 = unique_path(&dir, "a.mov");
        assert_eq!(p0.file_name().unwrap().to_str().unwrap(), "a.mov");
        // Create it, then the next is a-1.mov.
        fs::write(&p0, b"x").unwrap();
        let p1 = unique_path(&dir, "a.mov");
        assert_eq!(p1.file_name().unwrap().to_str().unwrap(), "a-1.mov");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn export_collects_external_renames_and_reports() {
        let work = temp_dir("export");
        let src_proj = work.join("source.palmier");
        let src_media = src_proj.join("media");
        fs::create_dir_all(&src_media).unwrap();

        // One external media file (outside the project).
        let external_file = work.join("My External Clip.mov");
        fs::write(&external_file, b"EXTERNAL-DATA").unwrap();
        // One project-internal media file.
        let internal_file = src_media.join("internal.mov");
        fs::write(&internal_file, b"INTERNAL").unwrap();
        // A thumbnail and a chat dir to carry across.
        fs::write(src_proj.join(THUMBNAIL_FILENAME), b"JPEG").unwrap();
        fs::create_dir_all(src_proj.join(CHAT_DIR_NAME)).unwrap();
        fs::write(src_proj.join(CHAT_DIR_NAME).join("s1.json"), b"{}").unwrap();

        let mut manifest = MediaManifest::new();
        manifest.entries.push(entry(
            "ext-id-12345678abc",
            "My External Clip.mov",
            MediaSource::External {
                absolute_path: external_file.to_string_lossy().to_string(),
            },
        ));
        manifest.entries.push(entry(
            "proj-id",
            "internal.mov",
            MediaSource::Project {
                relative_path: "media/internal.mov".into(),
            },
        ));
        // A missing external ref (file does not exist).
        manifest.entries.push(entry(
            "missing-id",
            "gone.mov",
            MediaSource::External {
                absolute_path: work.join("does-not-exist.mov").to_string_lossy().to_string(),
            },
        ));

        let timeline = Timeline::new();
        let log = GenerationLog::new();
        let dest = work.join("out.palmier");
        let temp_root = temp_dir("staging");

        let report = export_palmier_project(
            &timeline,
            &manifest,
            &log,
            Some(&src_proj),
            &dest,
            &temp_root,
        )
        .unwrap();

        // Report fields.
        assert_eq!(report.collected, vec!["ext-id-12345678abc".to_string()]);
        assert_eq!(report.copied_internal, 1);
        assert_eq!(report.missing, vec![Missing {
            id: "missing-id".into(),
            name: "gone.mov".into(),
        }]);
        assert!(report.total_bytes > 0);

        // media/ has both copied files: external renamed import-{id8}.mov,
        // project keeps its name.
        let media = dest.join("media");
        assert!(media.join("import-ext-id-1.mov").exists(), "external renamed by id8");
        assert!(media.join("internal.mov").exists(), "project keeps name");

        // media.json rewrites both present entries to Project { relative_path },
        // and keeps the missing one dangling (still External).
        let media_json = fs::read_to_string(dest.join(MANIFEST_FILENAME)).unwrap();
        let rewritten: MediaManifest = serde_json::from_str(&media_json).unwrap();
        let ext = rewritten.entries.iter().find(|e| e.id == "ext-id-12345678abc").unwrap();
        assert!(matches!(&ext.source, MediaSource::Project { relative_path } if relative_path == "media/import-ext-id-1.mov"));
        let proj = rewritten.entries.iter().find(|e| e.id == "proj-id").unwrap();
        assert!(matches!(&proj.source, MediaSource::Project { relative_path } if relative_path == "media/internal.mov"));
        let miss = rewritten.entries.iter().find(|e| e.id == "missing-id").unwrap();
        assert!(matches!(miss.source, MediaSource::External { .. }), "missing stays dangling");

        // Carried-across thumbnail + chat dir.
        assert!(dest.join(THUMBNAIL_FILENAME).exists());
        assert!(dest.join(CHAT_DIR_NAME).join("s1.json").exists());

        // project.json + generation-log.json present.
        assert!(dest.join(TIMELINE_FILENAME).exists());
        assert!(dest.join(GENERATION_LOG_FILENAME).exists());

        fs::remove_dir_all(&work).unwrap();
        let _ = fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn export_dedups_same_source_to_one_copy() {
        let work = temp_dir("dedup");
        let shared = work.join("shared.mov");
        fs::write(&shared, b"SHARED").unwrap();

        let mut manifest = MediaManifest::new();
        // Two entries pointing at the SAME external file.
        for id in ["a-aaaaaaaa-1", "b-bbbbbbbb-2"] {
            manifest.entries.push(entry(
                id,
                "shared.mov",
                MediaSource::External {
                    absolute_path: shared.to_string_lossy().to_string(),
                },
            ));
        }
        let dest = work.join("out.palmier");
        let temp_root = temp_dir("dedupstage");
        let report = export_palmier_project(
            &Timeline::new(),
            &manifest,
            &GenerationLog::new(),
            None,
            &dest,
            &temp_root,
        )
        .unwrap();

        // Both ids collected, but only ONE file copied (dedup by source path).
        assert_eq!(report.collected.len(), 2);
        let media = dest.join("media");
        let count = fs::read_dir(&media).unwrap().count();
        assert_eq!(count, 1, "dedup: one physical copy for two entries");

        fs::remove_dir_all(&work).unwrap();
        let _ = fs::remove_dir_all(&temp_root);
    }
}
