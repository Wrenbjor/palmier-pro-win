//! Import **orchestration** — multi-file + recursive-folder import as ONE undo
//! step, plus the byte-exact drag-payload contract (story E4-S7).
//!
//! Port of `EditorViewModel+MediaLibrary.swift` (`importFinderItems`,
//! `importFolder`, `addMediaAsset`, `importPastedImageData`,
//! `finalizeImportedAsset`) and the `MediaTab+Drag.swift` URI-scheme contract.
//! See `docs/reference/media-panel.md` §"Import & supported extensions" +
//! §"Drag-drop" and `_bmad-output/implementation-artifacts/epic-04-media-panel.md`
//! (E4-S7).
//!
//! ## The one-undo-step rule
//!
//! [`import_finder_items`] is the heart of the story: a whole drop — every file
//! plus every recursively-mirrored folder — must collapse into a **single**
//! `"Import Media"` undo entry. The reference does this by
//! `disableUndoRegistration()` around the import loop, then registering one
//! snapshot-restore undo *iff* anything changed. We get the same effect for free
//! from [`palmier_project::MediaLibraryHistory`]: the whole import runs inside one
//! `History::with_user_swap("Import Media", …)`, which snapshots the library
//! before/after and registers exactly one entry — and **nothing** when the drop
//! imported zero assets (matching the reference `guard summary != 0`).
//!
//! ## Directory tree → folder tree, 1:1
//!
//! Folder URLs recurse through [`import_folder`]: create a `MediaFolder` named
//! after the dir, list its contents (hidden files skipped), sort by a
//! localized-standard-style compare, recurse subdirs, and import files whose
//! extension classifies via [`crate::classify_path`]. The mirror is exact — one
//! `MediaFolder` per directory, the same nesting.
//!
//! ## Internalize-vs-reference (`MediaSource`)
//!
//! The caller supplies how an imported file's on-disk url is recorded, via a
//! [`SourceResolver`] closure — normally
//! `palmier_project::source_for_url(url, project_url)` (internalize when the file
//! lives under the project bundle, else reference its absolute path). Keeping it a
//! closure lets `palmier-media` stay free of project-bundle policy while
//! reproducing the reference `toManifestEntry` decision.

use std::path::{Path, PathBuf};

use palmier_model::{ClipType, MediaAsset, MediaManifestEntry, MediaSource};
use palmier_project::media_library::action_name;
use palmier_project::MediaLibraryHistory;

use crate::clip::classify_path;
use crate::metadata::load_metadata_as;

// ======================================================================
//  Drag-payload contract (byte-exact — the cross-surface wire format)
// ======================================================================
//
// These mirror `MediaTab+Drag.swift` exactly. They are the contract between the
// panel, the timeline drop, and the agent moment-drags; any drift breaks those
// surfaces, so they are kept byte-for-byte and covered by a format test.

/// `palmier-folder://` — the folder-drag URI scheme prefix.
pub const FOLDER_DRAG_SCHEME: &str = "palmier-folder://";
/// `palmier-asset://` — the asset-drag URI scheme prefix.
pub const ASSET_DRAG_SCHEME: &str = "palmier-asset://";

/// `palmier-folder://<id>` (reference `folderDragString(forFolderId:)`).
pub fn folder_drag_string(folder_id: &str) -> String {
    format!("{FOLDER_DRAG_SCHEME}{folder_id}")
}

/// `palmier-asset://<id>` (reference `assetDragString(forAssetId:)`).
pub fn asset_drag_string(asset_id: &str) -> String {
    format!("{ASSET_DRAG_SCHEME}{asset_id}")
}

/// A search "moment": `palmier-asset://<id>#<start>-<end>`, start/end in **source
/// seconds** formatted `%.3f` (reference
/// `assetDragString(forAssetId:segment:)`). Byte-exact: `String(format:
/// "#%.3f-%.3f", lower, upper)`.
pub fn asset_drag_string_with_segment(asset_id: &str, start: f64, end: f64) -> String {
    format!("{ASSET_DRAG_SCHEME}{asset_id}#{start:.3}-{end:.3}")
}

/// Parse the folder id out of a `palmier-folder://<id>` line, or `None`
/// (reference `folderId(fromDragString:)`).
pub fn folder_id_from_drag_string(line: &str) -> Option<&str> {
    line.strip_prefix(FOLDER_DRAG_SCHEME)
}

/// Parse the asset id out of a `palmier-asset://<id>[#…]` line (the id is
/// everything up to a `#`), or `None` (reference `assetId(fromDragString:)`).
pub fn asset_id_from_drag_string(line: &str) -> Option<&str> {
    let rest = line.strip_prefix(ASSET_DRAG_SCHEME)?;
    Some(match rest.split_once('#') {
        Some((id, _)) => id,
        None => rest,
    })
}

/// Parse the `#<start>-<end>` source-second range from a moment-drag line, or
/// `None` if absent/invalid (reference `assetSegment(fromDragString:)`):
/// requires `start >= 0` and `end > start`.
pub fn asset_segment_from_drag_string(line: &str) -> Option<(f64, f64)> {
    let rest = line.strip_prefix(ASSET_DRAG_SCHEME)?;
    let (_, frag) = rest.split_once('#')?;
    // The reference splits on '-' without omitting empty subsequences and
    // requires exactly two parts — a leading '-' (negative start) yields 3 parts
    // and is rejected.
    let parts: Vec<&str> = frag.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
    let start: f64 = parts[0].parse().ok()?;
    let end: f64 = parts[1].parse().ok()?;
    if start >= 0.0 && end > start {
        Some((start, end))
    } else {
        None
    }
}

/// Build the multi-asset drag payload for a drag that started on `asset_id`:
/// when `asset_id` is part of the current selection, emit **all** selected ids
/// newline-joined (in `selected_in_order`); otherwise just the one id (reference
/// `dragPayload(for:)`).
pub fn drag_payload(asset_id: &str, selected_in_order: &[String]) -> String {
    if selected_in_order.iter().any(|id| id == asset_id) {
        selected_in_order
            .iter()
            .map(|id| asset_drag_string(id))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        asset_drag_string(asset_id)
    }
}

// ======================================================================
//  Import orchestration
// ======================================================================

/// Decides the [`MediaSource`] recorded for an imported file at `url`. Normally
/// `|url| palmier_project::source_for_url(url, project_url)`. Boxed so the import
/// API stays object-safe and free of bundle policy.
pub type SourceResolver<'a> = dyn Fn(&Path) -> MediaSource + 'a;

/// What [`import_finder_items`] reports: counts + the ids it created, so the UI
/// can select/reveal them and so the caller can fire the per-asset finalize
/// (metadata refresh + visual-cache kick) for each new asset.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportSummary {
    /// Number of assets imported (across files + recursive folders).
    pub asset_count: usize,
    /// Number of folders created (the mirrored directory tree).
    pub folder_count: usize,
    /// The ids of every asset created, in import order (for finalize + select).
    pub imported_asset_ids: Vec<String>,
    /// The ids of every folder created, in creation order.
    pub created_folder_ids: Vec<String>,
}

impl ImportSummary {
    /// Whether the import changed anything (matches the reference's
    /// `guard summary.assetCount != 0 || summary.folderCount != 0`).
    pub fn is_empty(&self) -> bool {
        self.asset_count == 0 && self.folder_count == 0
    }
}

/// Import `urls` (files and/or directories) into `into` (`None` = top level) as
/// **one** `"Import Media"` undo step.
///
/// - directories recurse via [`import_folder`] (directory tree → folder tree 1:1),
/// - files classify via [`classify_path`] (the two-gate ClipType + Lottie sniff);
///   unsupported / non-Lottie-JSON files are **skipped** (the caller emits the
///   `mediaPanelToast`),
/// - each imported file's [`MediaSource`] is decided by `source_resolver`,
/// - metadata is loaded inline ([`load_metadata_as`]) so the manifest entry
///   carries duration/dimensions/fps/has-audio at import time.
///
/// Returns the [`ImportSummary`]; if nothing imported, **no** undo entry is
/// registered (the `with_user_swap` no-op guard).
pub fn import_finder_items(
    doc: &mut MediaLibraryHistory,
    urls: &[PathBuf],
    into: Option<&str>,
    source_resolver: &SourceResolver<'_>,
) -> ImportSummary {
    let mut summary = ImportSummary::default();
    let summary_ref = &mut summary;
    let into_owned = into.map(str::to_owned);

    doc.history.with_user_swap(
        action_name::IMPORT_MEDIA,
        &mut doc.library,
        move |lib| {
            for url in urls {
                if url.is_dir() {
                    import_folder(lib, url, into_owned.as_deref(), source_resolver, summary_ref);
                } else {
                    add_media_asset(lib, url, into_owned.as_deref(), source_resolver, summary_ref);
                }
            }
        },
    );

    summary
}

/// Mirror a directory tree into media folders (reference `importFolder(at:into:)`).
///
/// Creates a `MediaFolder` named after `url`'s final component under
/// `parent_folder_id`, lists `url`'s contents with hidden files skipped, sorts
/// them by a localized-standard-style compare, then recurses subdirs and imports
/// classifiable files into the new folder.
pub fn import_folder(
    lib: &mut palmier_model::MediaLibrary,
    url: &Path,
    parent_folder_id: Option<&str>,
    source_resolver: &SourceResolver<'_>,
    summary: &mut ImportSummary,
) {
    let Ok(read_dir) = std::fs::read_dir(url) else {
        return;
    };

    let folder_name = url
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Folder")
        .to_owned();
    let folder_id = lib.create_folder(folder_name, parent_folder_id.map(str::to_owned));
    summary.folder_count += 1;
    summary.created_folder_ids.push(folder_id.clone());

    // Collect, skip hidden, then sort by localized-standard-style compare.
    let mut entries: Vec<PathBuf> = read_dir
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| !is_hidden(p))
        .collect();
    entries.sort_by(|a, b| localized_standard_compare(file_name_str(a), file_name_str(b)));

    for entry in entries {
        if entry.is_dir() {
            import_folder(lib, &entry, Some(&folder_id), source_resolver, summary);
        } else if classify_path(&entry).is_some() {
            add_media_asset(lib, &entry, Some(&folder_id), source_resolver, summary);
        }
    }
}

/// Import one file (reference `addMediaAsset(from:folderId:)`). Classifies via
/// [`classify_path`] (extension gate + Lottie sniff); an unsupported or
/// non-Lottie-JSON file is **skipped** (returns without touching `lib`). On
/// success, appends the runtime asset + manifest entry, populated with probed
/// metadata, and records the new id in `summary`. Returns the new asset id.
pub fn add_media_asset(
    lib: &mut palmier_model::MediaLibrary,
    url: &Path,
    folder_id: Option<&str>,
    source_resolver: &SourceResolver<'_>,
    summary: &mut ImportSummary,
) -> Option<String> {
    let clip_type = classify_path(url)?;

    // Reference name = url.deletingPathExtension().lastPathComponent.
    let name = url
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_owned();

    let source = source_resolver(url);
    let id = uuid::Uuid::new_v4().to_string();

    // Probe metadata inline so the manifest entry is complete at import time
    // (the reference does this async in `finalizeImportedAsset`; doing it here
    // keeps the entry self-consistent and the import deterministic for undo).
    let meta = load_metadata_as(url, clip_type).ok();
    let duration = meta.and_then(|m| m.duration).unwrap_or(0.0);
    let width = meta.and_then(|m| m.width).map(|w| w as i32);
    let height = meta.and_then(|m| m.height).map(|h| h as i32);
    let fps = meta.and_then(|m| m.fps);
    // Audio presence: audio files always have audio; video reflects the probe;
    // image/text/lottie never do. (Reference `MediaAsset.init` defaults
    // hasAudio = (type == .video) then loadMetadata corrects it.)
    let has_audio = match clip_type {
        ClipType::Audio => true,
        ClipType::Video => meta.map(|m| m.has_audio).unwrap_or(false),
        _ => false,
    };

    let mut asset = MediaAsset::new(id.clone(), name.clone(), clip_type, source.clone(), duration);
    asset.source_width = width;
    asset.source_height = height;
    asset.source_fps = fps;
    asset.has_audio = has_audio;
    asset.folder_id = folder_id.map(str::to_owned);
    lib.assets.push(asset);

    lib.manifest.entries.push(MediaManifestEntry {
        id: id.clone(),
        name,
        asset_type: clip_type,
        source,
        duration,
        generation_input: None,
        source_width: width,
        source_height: height,
        source_fps: fps,
        has_audio: Some(has_audio),
        folder_id: folder_id.map(str::to_owned),
        cached_remote_url: None,
        cached_remote_url_expires_at: None,
    });

    summary.asset_count += 1;
    summary.imported_asset_ids.push(id.clone());
    Some(id)
}

// ---- helpers ----------------------------------------------------------------

fn file_name_str(p: &Path) -> &str {
    p.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// Skip hidden files/dirs (dot-prefixed) — the reference uses
/// `.skipsHiddenFiles`.
fn is_hidden(p: &Path) -> bool {
    file_name_str(p).starts_with('.')
}

/// A localized-standard-style comparison (reference `localizedStandardCompare`):
/// case-insensitive with **numeric** runs ordered by value (so `clip2` < `clip10`).
/// A faithful-enough port for the deterministic import ordering the recursion
/// relies on.
fn localized_standard_compare(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    // Compare full numeric runs by value (ignoring leading zeros).
                    let na = take_number(&mut ai);
                    let nb = take_number(&mut bi);
                    let trimmed_a = na.trim_start_matches('0');
                    let trimmed_b = nb.trim_start_matches('0');
                    let ord = trimmed_a
                        .len()
                        .cmp(&trimmed_b.len())
                        .then_with(|| trimmed_a.cmp(trimmed_b));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                } else {
                    let la = ca.to_ascii_lowercase();
                    let lb = cb.to_ascii_lowercase();
                    let ord = la.cmp(&lb);
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

fn take_number(it: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut s = String::new();
    while let Some(&c) = it.peek() {
        if c.is_ascii_digit() {
            s.push(c);
            it.next();
        } else {
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ---- drag-payload format (byte-exact) ----------------------------------

    #[test]
    fn drag_payload_format_is_byte_exact() {
        assert_eq!(folder_drag_string("F1"), "palmier-folder://F1");
        assert_eq!(asset_drag_string("A1"), "palmier-asset://A1");
        // The %.3f moment format — three decimals, always.
        assert_eq!(
            asset_drag_string_with_segment("A1", 1.5, 3.25),
            "palmier-asset://A1#1.500-3.250"
        );
        assert_eq!(
            asset_drag_string_with_segment("A1", 0.0, 12.0),
            "palmier-asset://A1#0.000-12.000"
        );
        // Rounding to 3 decimals matches Swift String(format:"%.3f").
        assert_eq!(
            asset_drag_string_with_segment("x", 1.23456, 2.0),
            "palmier-asset://x#1.235-2.000"
        );
    }

    #[test]
    fn drag_payload_roundtrips_id_and_segment() {
        let line = asset_drag_string_with_segment("asset-7", 2.0, 5.5);
        assert_eq!(asset_id_from_drag_string(&line), Some("asset-7"));
        assert_eq!(asset_segment_from_drag_string(&line), Some((2.0, 5.5)));

        // Plain asset line: id parses, no segment.
        let plain = asset_drag_string("asset-7");
        assert_eq!(asset_id_from_drag_string(&plain), Some("asset-7"));
        assert_eq!(asset_segment_from_drag_string(&plain), None);

        // Folder line: only folder id parses.
        let f = folder_drag_string("f3");
        assert_eq!(folder_id_from_drag_string(&f), Some("f3"));
        assert_eq!(asset_id_from_drag_string(&f), None);

        // Invalid segment (end <= start) rejected.
        assert_eq!(
            asset_segment_from_drag_string("palmier-asset://a#5.0-5.0"),
            None
        );
        // Negative start rejected (3-part split).
        assert_eq!(
            asset_segment_from_drag_string("palmier-asset://a#-1.0-2.0"),
            None
        );
    }

    #[test]
    fn drag_payload_multi_select_newline_joins() {
        let selected = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        // Dragging an in-selection asset emits all selected ids, newline-joined.
        assert_eq!(
            drag_payload("b", &selected),
            "palmier-asset://a\npalmier-asset://b\npalmier-asset://c"
        );
        // Dragging an asset NOT in the selection emits just that one.
        assert_eq!(drag_payload("z", &selected), "palmier-asset://z");
    }

    // ---- import orchestration ----------------------------------------------

    fn scratch() -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("palmier-e4s7-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// Test resolver: always External(absolute path) — no bundle internalize.
    fn external_source(url: &Path) -> MediaSource {
        MediaSource::External {
            absolute_path: url.to_string_lossy().to_string(),
        }
    }

    #[test]
    fn recursive_folder_maps_to_hierarchy_one_undo_step() {
        let root = scratch();
        // root/
        //   a.png
        //   sub/
        //     b.png
        //     ignored.txt   (unsupported → skipped)
        //   .hidden.png      (hidden → skipped)
        fs::write(root.join("a.png"), tiny_png()).unwrap();
        let sub = root.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("b.png"), tiny_png()).unwrap();
        fs::write(sub.join("ignored.txt"), b"nope").unwrap();
        fs::write(root.join(".hidden.png"), tiny_png()).unwrap();

        let mut doc = MediaLibraryHistory::new();
        let urls = vec![root.clone()];
        let summary = import_finder_items(&mut doc, &urls, None, &external_source);

        // 2 folders (root + sub), 2 assets (a.png, b.png); hidden + txt skipped.
        assert_eq!(summary.folder_count, 2, "root + sub mirrored");
        assert_eq!(summary.asset_count, 2, "a.png + b.png; hidden/txt skipped");
        assert_eq!(doc.library.manifest.folders.len(), 2);
        assert_eq!(doc.library.assets.len(), 2);

        // Hierarchy: the root folder is top-level, sub is its child.
        let root_folder = doc
            .library
            .manifest
            .folders
            .iter()
            .find(|f| f.parent_id.is_none())
            .unwrap();
        let sub_folder = doc
            .library
            .manifest
            .folders
            .iter()
            .find(|f| f.parent_id.is_some())
            .unwrap();
        assert_eq!(sub_folder.parent_id.as_deref(), Some(root_folder.id.as_str()));
        assert_eq!(root_folder.name, root.file_name().unwrap().to_str().unwrap());
        assert_eq!(sub_folder.name, "sub");

        // b.png landed in sub; a.png in root.
        let b = doc.library.assets.iter().find(|a| a.name == "b").unwrap();
        assert_eq!(b.folder_id.as_deref(), Some(sub_folder.id.as_str()));
        let a = doc.library.assets.iter().find(|a| a.name == "a").unwrap();
        assert_eq!(a.folder_id.as_deref(), Some(root_folder.id.as_str()));

        // EXACTLY ONE undo step for the whole multi-item import.
        assert_eq!(doc.history.user_undo_len(), 1);
        assert_eq!(
            doc.history.current_undo_action_name(),
            Some(action_name::IMPORT_MEDIA)
        );

        // Undo fully reverses the import in one step.
        assert!(doc.undo());
        assert!(doc.library.manifest.folders.is_empty());
        assert!(doc.library.assets.is_empty());
        assert!(doc.library.manifest.entries.is_empty());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn multi_file_import_is_one_undo_step() {
        let root = scratch();
        fs::write(root.join("one.png"), tiny_png()).unwrap();
        fs::write(root.join("two.png"), tiny_png()).unwrap();
        fs::write(root.join("bad.xyz"), b"nope").unwrap();

        let mut doc = MediaLibraryHistory::new();
        let urls = vec![
            root.join("one.png"),
            root.join("two.png"),
            root.join("bad.xyz"), // unsupported → skipped
        ];
        let summary = import_finder_items(&mut doc, &urls, None, &external_source);

        assert_eq!(summary.asset_count, 2);
        assert_eq!(summary.folder_count, 0);
        assert_eq!(summary.imported_asset_ids.len(), 2);
        // One undo entry for the batch.
        assert_eq!(doc.history.user_undo_len(), 1);

        assert!(doc.undo());
        assert!(doc.library.assets.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn import_of_only_unsupported_files_registers_no_undo_step() {
        let root = scratch();
        fs::write(root.join("a.txt"), b"x").unwrap();
        fs::write(root.join("b.bin"), b"y").unwrap();

        let mut doc = MediaLibraryHistory::new();
        let urls = vec![root.join("a.txt"), root.join("b.bin")];
        let summary = import_finder_items(&mut doc, &urls, None, &external_source);

        assert!(summary.is_empty());
        // Nothing imported → no undo entry registered (reference guard).
        assert_eq!(doc.history.user_undo_len(), 0);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn import_into_target_folder_parents_top_level_files() {
        let root = scratch();
        fs::write(root.join("x.png"), tiny_png()).unwrap();

        let mut doc = MediaLibraryHistory::new();
        // Pre-create a destination folder (its own undo step).
        let dest = doc.create_folder("Dest", None);

        let urls = vec![root.join("x.png")];
        import_finder_items(&mut doc, &urls, Some(&dest), &external_source);

        let x = doc.library.assets.iter().find(|a| a.name == "x").unwrap();
        assert_eq!(x.folder_id.as_deref(), Some(dest.as_str()));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn non_lottie_json_is_skipped_real_lottie_imported() {
        let root = scratch();
        fs::write(root.join("plain.json"), b"{}").unwrap();
        fs::write(
            root.join("anim.json"),
            br#"{"v":"5.7.4","fr":30,"ip":0,"op":60,"w":256,"h":256,"layers":[]}"#,
        )
        .unwrap();

        let mut doc = MediaLibraryHistory::new();
        let urls = vec![root.join("plain.json"), root.join("anim.json")];
        let summary = import_finder_items(&mut doc, &urls, None, &external_source);

        // Only the real Lottie imports; the plain {} json is refused.
        assert_eq!(summary.asset_count, 1);
        let a = &doc.library.assets[0];
        assert_eq!(a.name, "anim");
        assert_eq!(a.asset_type, ClipType::Lottie);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn localized_standard_compare_orders_numerically() {
        use std::cmp::Ordering;
        assert_eq!(
            localized_standard_compare("clip2.mp4", "clip10.mp4"),
            Ordering::Less,
            "numeric run clip2 < clip10"
        );
        assert_eq!(
            localized_standard_compare("A.png", "a.png"),
            Ordering::Equal,
            "case-insensitive"
        );
        assert_eq!(
            localized_standard_compare("b.png", "a.png"),
            Ordering::Greater
        );
    }

    /// A minimal valid 1x1 PNG so the `image` crate can probe dimensions.
    fn tiny_png() -> Vec<u8> {
        // 1x1 transparent PNG.
        const B64: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89\x00\x00\x00\nIDATx\x9cc\x00\x01\x00\x00\x05\x00\x01\r\n-\xb4\x00\x00\x00\x00IEND\xaeB`\x82";
        B64.to_vec()
    }
}
