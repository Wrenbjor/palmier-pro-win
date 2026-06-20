//! Media-path resolution + internalize-on-save (story E2-S12).
//!
//! Ports `Models/MediaResolver.swift` (`MediaResolver`, `expectedURL`,
//! `resolveURL`, `isMissing`) and `MediaAsset.toManifestEntry` /
//! `VideoProject.restoreAssetsFromManifest`. See docs/reference/project-io.md
//! "Media path resolution".
//!
//! ## The two `MediaSource` variants and their URL semantics
//!
//! - `Project { relative_path }` → `project_url.join(relative_path)`. The
//!   `relative_path` **already contains** the `media/` segment (the reference
//!   stores `media/<file>`), so we do NOT prepend it.
//! - `External { absolute_path }` → that absolute path verbatim.
//!
//! ## Internalize-on-save heuristic (`to_manifest_entry`)
//!
//! [`source_for_url`] decides how an asset's on-disk url is recorded: if the url
//! lives **under** `project_url` it becomes `Project { relative_path = <path after
//! project_url/> }` (internalized — travels with the bundle); otherwise it stays
//! `External { absolute_path }`. This is the reference `toManifestEntry` logic
//! (`url.path.hasPrefix(projectURL.path)`).
//!
//! ## Restore-on-open (`restore_entries`)
//!
//! [`restore_entries`] rebuilds the resolved url for every manifest entry,
//! **logging + skipping** missing files (never erroring — reference
//! `restoreAssetsFromManifest` `continue`s on a missing/unresolvable url) and
//! emitting a [`RestoreEvent`] per entry so the platform layer can fire the
//! downstream waveform/thumbnail/metadata regeneration hooks (Epic 4/5 own the
//! actual regen; here we only signal it).

use std::path::{Path, PathBuf};

use palmier_model::{ClipType, MediaManifest, MediaManifestEntry, MediaSource};

/// Resolves manifest asset ids → file paths against a manifest + an optional
/// project bundle path (reference `MediaResolver`).
///
/// `project_url` is `None` for an unsaved project; `Project`-sourced entries
/// cannot be resolved without it (the reference returns `nil`).
pub struct MediaResolver<'a> {
    manifest: &'a MediaManifest,
    project_url: Option<PathBuf>,
}

impl<'a> MediaResolver<'a> {
    /// Build a resolver over `manifest`, rooted at `project_url` (the `.palmier`
    /// bundle dir) when the project is saved.
    pub fn new(manifest: &'a MediaManifest, project_url: Option<impl Into<PathBuf>>) -> Self {
        MediaResolver {
            manifest,
            project_url: project_url.map(Into::into),
        }
    }

    /// The manifest entry for `asset_id` (reference `entry(for:)`).
    pub fn entry(&self, asset_id: &str) -> Option<&MediaManifestEntry> {
        self.manifest.entries.iter().find(|e| e.id == asset_id)
    }

    /// The *expected* url for `asset_id` regardless of whether the file exists
    /// (reference `expectedURL`):
    /// - external → the absolute path,
    /// - project  → `project_url.join(relative_path)` (`None` if no project_url).
    pub fn expected_url(&self, asset_id: &str) -> Option<PathBuf> {
        let entry = self.entry(asset_id)?;
        expected_url_for_source(&entry.source, self.project_url.as_deref())
    }

    /// The url for `asset_id` **only if the file exists** (reference `resolveURL`).
    pub fn resolve_url(&self, asset_id: &str) -> Option<PathBuf> {
        let url = self.expected_url(asset_id)?;
        url.exists().then_some(url)
    }

    /// Whether the backing file for `asset_id` is missing (reference `isMissing`):
    /// true when it can't be resolved OR the resolved path doesn't exist.
    pub fn is_missing(&self, asset_id: &str) -> bool {
        match self.expected_url(asset_id) {
            Some(url) => !url.exists(),
            None => true,
        }
    }

    /// Display name for `asset_id`, or `"Offline"` when unknown (reference
    /// `displayName`).
    pub fn display_name(&self, asset_id: &str) -> String {
        self.entry(asset_id)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "Offline".to_string())
    }
}

/// The expected url for a `MediaSource` given an optional project root.
pub fn expected_url_for_source(source: &MediaSource, project_url: Option<&Path>) -> Option<PathBuf> {
    match source {
        MediaSource::External { absolute_path } => Some(PathBuf::from(absolute_path)),
        MediaSource::Project { relative_path } => {
            project_url.map(|base| base.join(relative_path))
        }
    }
}

/// Decide how an asset's on-disk `url` should be recorded in the manifest — the
/// **internalize-on-save heuristic** (reference `MediaAsset.toManifestEntry`).
///
/// If `url` is under `project_url`, return `Project { relative_path }` (the path
/// after `project_url/`, with forward slashes so the stored value is
/// platform-stable and matches the reference `media/<file>` spelling); otherwise
/// `External { absolute_path }`.
pub fn source_for_url(url: &Path, project_url: Option<&Path>) -> MediaSource {
    // Let-chain (edition 2024): internalize only when `url` is under the project.
    if let Some(base) = project_url
        && let Ok(rel) = url.strip_prefix(base)
    {
        // Store with `/` separators regardless of platform so a Windows-saved
        // bundle resolves on Linux and matches the reference `media/<file>`.
        let rel_str = rel
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .collect::<Vec<_>>()
            .join("/");
        return MediaSource::Project {
            relative_path: rel_str,
        };
    }
    MediaSource::External {
        absolute_path: url.to_string_lossy().to_string(),
    }
}

/// What [`restore_entries`] reports per manifest entry on open — the seam the
/// platform layer turns into the actual regeneration hooks (Epic 4/5).
#[derive(Debug, Clone, PartialEq)]
pub enum RestoreEvent {
    /// The asset's file resolved and exists; `kind` drives which regen hook fires
    /// (audio/video → waveform; video → thumbnails; image → thumbnail).
    Restored {
        asset_id: String,
        url: PathBuf,
        kind: ClipType,
    },
    /// The url could not be resolved (e.g. `Project` source with no project_url).
    Unresolvable { asset_id: String },
    /// The url resolved but the file is missing on disk — logged + skipped
    /// (reference `restoreAssetsFromManifest` warns and `continue`s).
    Missing { asset_id: String, url: PathBuf },
}

/// Walk every manifest entry and produce a [`RestoreEvent`] for each, **never
/// erroring** on a missing/unresolvable file (reference
/// `restoreAssetsFromManifest`). The caller fires the downstream waveform/
/// thumbnail/metadata regeneration for each `Restored` event.
pub fn restore_entries(manifest: &MediaManifest, project_url: Option<&Path>) -> Vec<RestoreEvent> {
    manifest
        .entries
        .iter()
        .map(|entry| match expected_url_for_source(&entry.source, project_url) {
            None => RestoreEvent::Unresolvable {
                asset_id: entry.id.clone(),
            },
            Some(url) if url.exists() => RestoreEvent::Restored {
                asset_id: entry.id.clone(),
                url,
                kind: entry.asset_type,
            },
            Some(url) => RestoreEvent::Missing {
                asset_id: entry.id.clone(),
                url,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn scratch() -> PathBuf {
        let p = std::env::temp_dir().join(format!("palmier-e2s12-res-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn manifest_with(entries: Vec<MediaManifestEntry>) -> MediaManifest {
        let mut m = MediaManifest::new();
        m.entries = entries;
        m
    }

    fn entry(id: &str, source: MediaSource, kind: ClipType) -> MediaManifestEntry {
        MediaManifestEntry {
            id: id.into(),
            name: format!("{id} name"),
            asset_type: kind,
            source,
            duration: 1.0,
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

    #[test]
    fn expected_url_for_both_variants() {
        let proj = if cfg!(windows) {
            PathBuf::from(r"C:\proj\My.palmier")
        } else {
            PathBuf::from("/proj/My.palmier")
        };
        // project source → project_url + relative (relative already has media/).
        let p = expected_url_for_source(
            &MediaSource::Project {
                relative_path: "media/clip.mov".into(),
            },
            Some(&proj),
        )
        .unwrap();
        assert_eq!(p, proj.join("media/clip.mov"));

        // external source → absolute path verbatim, project_url irrelevant.
        let abs = if cfg!(windows) {
            r"D:\ext\v.mov"
        } else {
            "/ext/v.mov"
        };
        let e = expected_url_for_source(
            &MediaSource::External {
                absolute_path: abs.into(),
            },
            Some(&proj),
        )
        .unwrap();
        assert_eq!(e, PathBuf::from(abs));

        // project source with NO project_url → None (reference returns nil).
        assert!(expected_url_for_source(
            &MediaSource::Project {
                relative_path: "media/x.mov".into()
            },
            None
        )
        .is_none());
    }

    #[test]
    fn source_for_url_internalizes_paths_under_project() {
        let proj = if cfg!(windows) {
            PathBuf::from(r"C:\proj\My.palmier")
        } else {
            PathBuf::from("/proj/My.palmier")
        };
        // Under the project → Project, with `/`-joined relative incl. media/.
        let under = proj.join("media").join("clip.mov");
        match source_for_url(&under, Some(&proj)) {
            MediaSource::Project { relative_path } => assert_eq!(relative_path, "media/clip.mov"),
            other => panic!("expected Project, got {other:?}"),
        }

        // Outside the project → External (absolute path).
        let outside = if cfg!(windows) {
            PathBuf::from(r"D:\elsewhere\v.mov")
        } else {
            PathBuf::from("/elsewhere/v.mov")
        };
        match source_for_url(&outside, Some(&proj)) {
            MediaSource::External { absolute_path } => {
                assert_eq!(absolute_path, outside.to_string_lossy());
            }
            other => panic!("expected External, got {other:?}"),
        }

        // No project_url → always External.
        assert!(matches!(
            source_for_url(&under, None),
            MediaSource::External { .. }
        ));
    }

    #[test]
    fn restore_logs_and_skips_missing_files() {
        let proj = scratch();
        // One real file under media/, one project entry whose file is missing,
        // one external missing, one unresolvable (project source, no project_url).
        let media = proj.join("media");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::write(media.join("present.mov"), b"x").unwrap();

        let m = manifest_with(vec![
            entry(
                "present",
                MediaSource::Project {
                    relative_path: "media/present.mov".into(),
                },
                ClipType::Video,
            ),
            entry(
                "gone",
                MediaSource::Project {
                    relative_path: "media/gone.mov".into(),
                },
                ClipType::Audio,
            ),
        ]);

        let events = restore_entries(&m, Some(&proj));
        assert_eq!(events.len(), 2, "every entry yields an event; none errors");
        assert!(matches!(
            &events[0],
            RestoreEvent::Restored { asset_id, kind, .. }
                if asset_id == "present" && *kind == ClipType::Video
        ));
        assert!(matches!(
            &events[1],
            RestoreEvent::Missing { asset_id, .. } if asset_id == "gone"
        ));

        // With no project_url, the Project-sourced entries are Unresolvable (not
        // an error).
        let unresolved = restore_entries(&m, None);
        assert!(unresolved
            .iter()
            .all(|e| matches!(e, RestoreEvent::Unresolvable { .. })));

        std::fs::remove_dir_all(&proj).unwrap();
    }

    #[test]
    fn resolver_resolve_vs_expected_and_missing() {
        let proj = scratch();
        let media = proj.join("media");
        std::fs::create_dir_all(&media).unwrap();
        std::fs::write(media.join("a.mov"), b"x").unwrap();

        let m = manifest_with(vec![
            entry(
                "a",
                MediaSource::Project {
                    relative_path: "media/a.mov".into(),
                },
                ClipType::Video,
            ),
            entry(
                "b",
                MediaSource::Project {
                    relative_path: "media/b.mov".into(),
                },
                ClipType::Video,
            ),
        ]);
        let r = MediaResolver::new(&m, Some(proj.clone()));

        // a exists: expected == resolve, not missing.
        assert!(r.resolve_url("a").is_some());
        assert_eq!(r.expected_url("a"), r.resolve_url("a"));
        assert!(!r.is_missing("a"));

        // b is expected-but-missing: expected Some, resolve None, missing true.
        assert!(r.expected_url("b").is_some());
        assert!(r.resolve_url("b").is_none());
        assert!(r.is_missing("b"));

        // unknown id: missing, display name "Offline".
        assert!(r.is_missing("zzz"));
        assert_eq!(r.display_name("zzz"), "Offline");
        assert_eq!(r.display_name("a"), "a name");

        std::fs::remove_dir_all(&proj).unwrap();
    }
}
