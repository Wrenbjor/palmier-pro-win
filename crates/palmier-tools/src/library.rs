//! LIBRARY tool bodies — `import_media`, `create_folder`, `move_to_folder`,
//! `rename_media`, `rename_folder`, `delete_media`, `delete_folder` (E7-S10;
//! reference `ToolExecutor+Import.swift` + `ToolExecutor+Folders.swift`).
//!
//! All seven mutate the **media library** (folders/assets) and — for the delete
//! cascades — the timeline, so each registers exactly **one** agent-undo step over
//! the whole [`MediaLibrary`] snapshot via
//! [`library_agent_edit`](crate::undo::library_agent_edit) (distinct from the
//! `Timeline`-only [`agent_edit`](crate::undo::agent_edit) the clip tools use).
//!
//! ## Dual-shape tools (`create_folder` / `move_to_folder` / `rename_media` /
//! `rename_folder`)
//! Each accepts **direct fields XOR `entries[]`** (never both — the XOR is enforced
//! in validation, E7-S3, and re-guarded here). The direct form returns a single id /
//! confirmation; the batch (`entries`) form returns `{ folders }` for `create_folder`
//! / an aggregate count for the others (reference `isBatch` branch).
//!
//! ## `import_media` source split (reference `importMedia`)
//! `source` sets exactly one of `path` / `bytes` / `url`:
//! - **`path`** (file or recursive directory) and **`bytes`** (≤ ~15 MB base64) are
//!   **synchronous** — they classify + probe the file through `palmier-media`'s
//!   import (Epic 4) and return the new `media_ref(s)` immediately.
//! - **`url`** (HTTPS, ≤ 1 GB) is **async** in the reference (background download);
//!   the Convex/HTTP fetch plumbing lands with Epic 9 (M3), so M2 reports that URL
//!   import is not yet wired (path/bytes are the live sources).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use palmier_media::{add_media_asset, classify_path, import_folder, ImportSummary};
use palmier_model::{ClipType, MediaLibrary, MediaSource};

use crate::editor::EditorState;
use crate::result::ToolResult;
use crate::undo::library_agent_edit;

/// A [`palmier_media::SourceResolver`] that records every imported file by its
/// absolute path (reference `source_for_url` when there is no project bundle to
/// internalize into — M2's `EditorState` has no on-disk project, so imports are
/// referenced in place).
fn external_source_resolver() -> impl Fn(&Path) -> MediaSource {
    |p: &Path| MediaSource::External {
        absolute_path: p.to_string_lossy().into_owned(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// import_media
// ─────────────────────────────────────────────────────────────────────────────

/// Max base64 length for inline `bytes` (reference `importBytesMaxBase64Length` =
/// 15 MiB of base64 ≈ 11 MB binary).
const IMPORT_BYTES_MAX_BASE64: usize = 15 * 1024 * 1024;

/// `import_media` (`source { exactly one of url | path | bytes; mime_type required
/// for bytes }`, optional `name`, `folder_id`): import an asset, returning a new
/// `media_ref`. path/bytes are synchronous (Epic 4 import); url is async (Epic 9).
/// Reference `importMedia`.
pub fn import_media(state: &mut EditorState, args: &Value) -> ToolResult {
    let source = match args.get("source").and_then(Value::as_object) {
        Some(s) => s,
        None => return ToolResult::error("Missing required 'source' object"),
    };
    let url = source.get("url").and_then(Value::as_str);
    let path = source.get("path").and_then(Value::as_str);
    let bytes = source.get("bytes").and_then(Value::as_str);
    let mime_type = source.get("mimeType").and_then(Value::as_str);

    let set = [url, path, bytes].iter().filter(|o| o.is_some()).count();
    if set != 1 {
        return ToolResult::error(format!(
            "source must set exactly one of 'url', 'path', or 'bytes' (got {set})"
        ));
    }

    // Resolve the optional target folder (must exist when set).
    let folder_id = match resolve_folder_id(&state.library, args) {
        Ok(f) => f,
        Err(msg) => return ToolResult::error(msg),
    };
    let name = args.get("name").and_then(Value::as_str).map(str::to_string);

    if let Some(path) = path {
        return import_from_path(state, path, name, folder_id);
    }
    if let Some(bytes) = bytes {
        let Some(mime) = mime_type else {
            return ToolResult::error("source.mimeType is required when source.bytes is set");
        };
        return import_from_bytes(state, bytes, mime, name, folder_id);
    }
    // url: async download plumbing lands with Epic 9 (M3).
    let _ = url;
    ToolResult::error(
        "import_media: URL import is not yet available in this build (the background \
         download plumbing lands in a later milestone). Use source.path or source.bytes.",
    )
}

fn import_from_path(
    state: &mut EditorState,
    path: &str,
    name: Option<String>,
    folder_id: Option<String>,
) -> ToolResult {
    let file = PathBuf::from(path);
    if !file.exists() {
        return ToolResult::error(format!("File not found: {path}"));
    }
    let resolver = external_source_resolver();
    let is_dir = file.is_dir();
    library_agent_edit(state, "Import Media", move |lib| {
        let mut summary = ImportSummary::default();
        if is_dir {
            import_folder(lib, &file, folder_id.as_deref(), &resolver, &mut summary);
            if summary.asset_count == 0 {
                return Err(format!("No supported media found in folder: {}", file.display()));
            }
            Ok(ToolResult::ok(format!(
                "Imported {} file(s) into {} folder(s) from '{}', mirroring its structure. \
                 Available now in get_media / list_folders.",
                summary.asset_count,
                summary.folder_count,
                file.file_name().and_then(|n| n.to_str()).unwrap_or("folder")
            )))
        } else {
            if classify_path(&file).is_none() {
                let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
                return Err(format!(
                    "Unsupported file extension '.{ext}'. Supported: mov/mp4/m4v, mp3/wav/aac/m4a, \
                     png/jpg/jpeg/tiff/heic, json (Lottie)."
                ));
            }
            let id = match add_media_asset(lib, &file, folder_id.as_deref(), &resolver, &mut summary) {
                Some(id) => id,
                None => return Err(format!("Failed to import file: {path}")),
            };
            apply_name(lib, &id, name.as_deref());
            let ty = lib.assets.iter().find(|a| a.id == id).map(|a| a.asset_type);
            let ty_str = ty.map(clip_type_str).unwrap_or("media");
            Ok(ToolResult::ok(format!(
                "Imported (id: {id}, type: {ty_str}) from path. Available now in get_media."
            )))
        }
    })
}

fn import_from_bytes(
    state: &mut EditorState,
    base64: &str,
    mime: &str,
    name: Option<String>,
    folder_id: Option<String>,
) -> ToolResult {
    if base64.len() > IMPORT_BYTES_MAX_BASE64 {
        return ToolResult::error(format!(
            "source.bytes is too large ({} chars; max {IMPORT_BYTES_MAX_BASE64}). \
             Use source.url or source.path for larger files.",
            base64.len()
        ));
    }
    let Some(ext) = file_extension_for_mime(mime) else {
        return ToolResult::error(format!(
            "Unsupported mimeType '{mime}'. Accepted: video/mp4, video/quicktime, audio/mpeg, \
             audio/wav, audio/aac, audio/mp4, image/png, image/jpeg, image/tiff, image/heic."
        ));
    };
    let data = match base64_decode(base64) {
        Some(d) if !d.is_empty() => d,
        _ => return ToolResult::error("source.bytes is not valid non-empty base64"),
    };

    // Write to a temp file the importer can probe (M2 EditorState has no project
    // media dir; the reference writes into the project's media/ folder).
    let dir = std::env::temp_dir().join("palmier-import");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return ToolResult::error(format!("Failed to prepare import directory: {e}"));
    }
    let filename = format!("imported-{}.{ext}", &crate::clips::new_uuid()[..8]);
    let dest = dir.join(filename);
    if let Err(e) = std::fs::write(&dest, &data) {
        return ToolResult::error(format!("Failed to write bytes to disk: {e}"));
    }

    let resolver = external_source_resolver();
    let byte_len = data.len();
    library_agent_edit(state, "Import Media", move |lib| {
        let mut summary = ImportSummary::default();
        let id = match add_media_asset(lib, &dest, folder_id.as_deref(), &resolver, &mut summary) {
            Some(id) => id,
            None => {
                let _ = std::fs::remove_file(&dest);
                return Err("Failed to register imported asset".to_string());
            }
        };
        apply_name(lib, &id, name.as_deref());
        let ty = lib.assets.iter().find(|a| a.id == id).map(|a| a.asset_type);
        let ty_str = ty.map(clip_type_str).unwrap_or("media");
        Ok(ToolResult::ok(format!(
            "Imported (id: {id}, type: {ty_str}, {byte_len} bytes). Available now in get_media."
        )))
    })
}

/// Apply an optional display name override to the runtime asset + manifest entry
/// (reference `applyImportMetadata`).
fn apply_name(lib: &mut MediaLibrary, id: &str, name: Option<&str>) {
    let Some(name) = name else { return };
    if let Some(asset) = lib.assets.iter_mut().find(|a| a.id == id) {
        asset.name = name.to_string();
    }
    if let Some(entry) = lib.manifest.entries.iter_mut().find(|e| e.id == id) {
        entry.name = name.to_string();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// relink_media (UI Relink — repoint a missing asset's source path)
// ─────────────────────────────────────────────────────────────────────────────

/// `relink_media` (`mediaRef`, `newPath`): repoint an asset's source to a new
/// absolute on-disk path as ONE undo step. There is no reference MCP tool for this
/// (it backs the panel's Relink affordance only — `MediaTab+Drag.swift`'s relink),
/// so it lives behind the dedicated `editor_relink_media` command rather than the
/// 30-tool dispatch. Writes the new [`MediaSource::External`] onto both the runtime
/// asset and the manifest entry so the repointed path survives save/reload.
pub fn relink_media(state: &mut EditorState, media_ref: &str, new_path: &str) -> ToolResult {
    if new_path.is_empty() {
        return ToolResult::error("relink_media: newPath is required");
    }
    if !state.library.assets.iter().any(|a| a.id == media_ref) {
        return ToolResult::error(format!("Media asset not found: {media_ref}"));
    }
    let media_ref = media_ref.to_string();
    let new_path = new_path.to_string();
    library_agent_edit(state, "Relink Media", move |lib| {
        let source = MediaSource::External { absolute_path: new_path.clone() };
        if let Some(asset) = lib.assets.iter_mut().find(|a| a.id == media_ref) {
            asset.source = source.clone();
        }
        if let Some(entry) = lib.manifest.entries.iter_mut().find(|e| e.id == media_ref) {
            entry.source = source;
        }
        Ok(ToolResult::ok(format!("Relinked {media_ref} to '{new_path}'")))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// move_folders (UI in-panel folder reparent — cycle-guarded)
// ─────────────────────────────────────────────────────────────────────────────

/// `move_folders` (`folderIds[]`, `targetFolderId?`): reparent folders onto
/// `target_folder_id` (`None` = root) as ONE undo step, applying the model's three
/// cycle guards (reject no-op / into-descendant / into-self — see
/// [`MediaLibrary::move_folders_to_folder`]). The reference `move_to_folder` tool
/// only reparents ASSETS, so folder-into-folder moves have no MCP tool; this backs
/// the panel's in-panel folder drag (`MediaTab+Drag.swift`) behind the dedicated
/// `editor_move_folders` command so the reparent survives save/reload.
pub fn move_folders(
    state: &mut EditorState,
    folder_ids: &[String],
    target_folder_id: Option<&str>,
) -> ToolResult {
    if folder_ids.is_empty() {
        return ToolResult::error("move_folders: folderIds is required");
    }
    for id in folder_ids {
        if state.library.folder(id).is_none() {
            return ToolResult::error(format!("folderId not found: {id}"));
        }
    }
    if let Some(target) = target_folder_id {
        if state.library.folder(target).is_none() {
            return ToolResult::error(format!("targetFolderId not found: {target}"));
        }
    }
    let set: HashSet<String> = folder_ids.iter().cloned().collect();
    let target = target_folder_id.map(str::to_string);
    let n = folder_ids.len();
    library_agent_edit(state, "Move Folder", move |lib| {
        // The model self-guards cycles (matching the frontend `legalFolderMoves`);
        // a fully-rejected batch leaves the library unchanged (no undo step).
        lib.move_folders_to_folder(&set, target.as_deref());
        Ok(ToolResult::ok(format!("Moved {n} folder(s)")))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// create_folder (dual-shape)
// ─────────────────────────────────────────────────────────────────────────────

struct CreateSpec {
    name: String,
    parent_folder_id: Option<String>,
}

/// `create_folder` (`name` + `parent_folder_id?` XOR `entries[]`): create folder(s)
/// as ONE undo step. Direct form returns the single folder; the `entries` form
/// returns `{ folders }`. Reference `createFolder`.
pub fn create_folder(state: &mut EditorState, args: &Value) -> ToolResult {
    let (specs, is_batch) = match parse_create_specs(&state.library, args) {
        Ok(v) => v,
        Err(msg) => return ToolResult::error(msg),
    };
    let action = if specs.len() == 1 { "New Folder" } else { "New Folders" };
    library_agent_edit(state, action, move |lib| {
        let mut folders: Vec<Value> = Vec::with_capacity(specs.len());
        for spec in &specs {
            let id = lib.create_folder(spec.name.clone(), spec.parent_folder_id.clone());
            let mut obj = serde_json::Map::new();
            obj.insert("id".into(), json!(id));
            obj.insert("name".into(), json!(spec.name));
            if let Some(p) = &spec.parent_folder_id {
                obj.insert("parentFolderId".into(), json!(p));
            }
            folders.push(Value::Object(obj));
        }
        let body = if !is_batch {
            folders.into_iter().next().unwrap_or(json!({}))
        } else {
            json!({ "folders": folders })
        };
        Ok(ToolResult::ok(serde_json::to_string(&body).unwrap_or_else(|_| "{}".into())))
    })
}

fn parse_create_specs(
    lib: &MediaLibrary,
    args: &Value,
) -> Result<(Vec<CreateSpec>, bool), String> {
    if let Some(entries) = entry_objects(args)? {
        let mut specs = Vec::with_capacity(entries.len());
        for (idx, entry) in entries.iter().enumerate() {
            let name = require_string(entry, "name", &format!("entries[{idx}]"))?;
            let parent = parent_folder_id(lib, entry, &format!("entries[{idx}]"))?;
            specs.push(CreateSpec { name, parent_folder_id: parent });
        }
        return Ok((specs, true));
    }
    let name = require_string(args.as_object().unwrap_or(&serde_json::Map::new()), "name", "create_folder")?;
    let parent = parent_folder_id(lib, args.as_object().unwrap(), "create_folder")?;
    Ok((vec![CreateSpec { name, parent_folder_id: parent }], false))
}

// ─────────────────────────────────────────────────────────────────────────────
// move_to_folder (dual-shape)
// ─────────────────────────────────────────────────────────────────────────────

struct MoveSpec {
    asset_ids: Vec<String>,
    folder_id: Option<String>,
}

/// `move_to_folder` (`asset_ids[]` + `folder_id?` XOR `entries[]`): reparent assets
/// as ONE undo step; omitting `folder_id` moves to root. Reference `moveToFolder`.
pub fn move_to_folder(state: &mut EditorState, args: &Value) -> ToolResult {
    let (specs, is_batch) = match parse_move_specs(&state.library, args) {
        Ok(v) => v,
        Err(msg) => return ToolResult::error(msg),
    };
    let action = if specs.len() == 1 { "Move to Folder" } else { "Move to Folder" };
    let count: usize = specs.iter().map(|s| s.asset_ids.len()).sum();
    let ops = specs.len();
    library_agent_edit(state, action, move |lib| {
        for spec in &specs {
            let set: HashSet<String> = spec.asset_ids.iter().cloned().collect();
            lib.move_assets_to_folder(&set, spec.folder_id.as_deref());
        }
        let msg = if !is_batch {
            let dest = specs[0]
                .folder_id
                .as_ref()
                .map(|f| format!(" to folder {f}"))
                .unwrap_or_else(|| " to root".into());
            format!("Moved {} asset(s){dest}", specs[0].asset_ids.len())
        } else {
            format!("Moved {count} asset(s) across {ops} folder operation(s)")
        };
        Ok(ToolResult::ok(msg))
    })
}

fn parse_move_specs(lib: &MediaLibrary, args: &Value) -> Result<(Vec<MoveSpec>, bool), String> {
    if let Some(entries) = entry_objects(args)? {
        let mut specs = Vec::with_capacity(entries.len());
        for (idx, entry) in entries.iter().enumerate() {
            let path = format!("entries[{idx}]");
            let asset_ids = valid_asset_ids(lib, entry, &path)?;
            let folder_id = resolve_folder_id_obj(lib, entry, &path)?;
            specs.push(MoveSpec { asset_ids, folder_id });
        }
        return Ok((specs, true));
    }
    let obj = args.as_object().ok_or("move_to_folder: arguments must be an object")?;
    let asset_ids = valid_asset_ids(lib, obj, "move_to_folder")?;
    let folder_id = resolve_folder_id_obj(lib, obj, "move_to_folder")?;
    Ok((vec![MoveSpec { asset_ids, folder_id }], false))
}

// ─────────────────────────────────────────────────────────────────────────────
// rename_media (dual-shape)
// ─────────────────────────────────────────────────────────────────────────────

struct RenameMediaSpec {
    media_ref: String,
    name: String,
}

/// `rename_media` (`media_ref`, `name` XOR `entries[]`): rename asset(s) as ONE undo
/// step. Reference `renameMedia`.
pub fn rename_media(state: &mut EditorState, args: &Value) -> ToolResult {
    let (specs, is_batch) = match parse_rename_media_specs(&state.library, args) {
        Ok(v) => v,
        Err(msg) => return ToolResult::error(msg),
    };
    let action = if specs.len() == 1 { "Rename Asset" } else { "Rename Assets" };
    let n = specs.len();
    library_agent_edit(state, action, move |lib| {
        for spec in &specs {
            rename_media_asset(lib, &spec.media_ref, &spec.name);
        }
        let msg = if !is_batch {
            format!("Renamed {} to '{}'", specs[0].media_ref, specs[0].name)
        } else {
            format!("Renamed {n} media asset(s)")
        };
        Ok(ToolResult::ok(msg))
    })
}

/// Rename an asset id → `name` on both the runtime catalog + manifest entry
/// (reference `renameMediaAsset(id:name:)`).
fn rename_media_asset(lib: &mut MediaLibrary, id: &str, name: &str) {
    if let Some(asset) = lib.assets.iter_mut().find(|a| a.id == id) {
        asset.name = name.to_string();
    }
    if let Some(entry) = lib.manifest.entries.iter_mut().find(|e| e.id == id) {
        entry.name = name.to_string();
    }
}

fn parse_rename_media_specs(
    lib: &MediaLibrary,
    args: &Value,
) -> Result<(Vec<RenameMediaSpec>, bool), String> {
    if let Some(entries) = entry_objects(args)? {
        let mut specs = Vec::with_capacity(entries.len());
        for (idx, entry) in entries.iter().enumerate() {
            let path = format!("entries[{idx}]");
            let media_ref = require_string(entry, "mediaRef", &path)?;
            let name = require_string(entry, "name", &path)?;
            if !lib.assets.iter().any(|a| a.id == media_ref) {
                return Err(format!("{path}: media asset not found: {media_ref}"));
            }
            specs.push(RenameMediaSpec { media_ref, name });
        }
        return Ok((specs, true));
    }
    let obj = args.as_object().ok_or("rename_media: arguments must be an object")?;
    let media_ref = require_string(obj, "mediaRef", "rename_media")?;
    let name = require_string(obj, "name", "rename_media")?;
    if !lib.assets.iter().any(|a| a.id == media_ref) {
        return Err(format!("Media asset not found: {media_ref}"));
    }
    Ok((vec![RenameMediaSpec { media_ref, name }], false))
}

// ─────────────────────────────────────────────────────────────────────────────
// rename_folder (dual-shape)
// ─────────────────────────────────────────────────────────────────────────────

struct RenameFolderSpec {
    folder_id: String,
    name: String,
}

/// `rename_folder` (`folder_id`, `name` XOR `entries[]`): rename folder(s) as ONE
/// undo step. Reference `renameFolder`.
pub fn rename_folder(state: &mut EditorState, args: &Value) -> ToolResult {
    let (specs, is_batch) = match parse_rename_folder_specs(&state.library, args) {
        Ok(v) => v,
        Err(msg) => return ToolResult::error(msg),
    };
    let action = if specs.len() == 1 { "Rename Folder" } else { "Rename Folders" };
    let n = specs.len();
    library_agent_edit(state, action, move |lib| {
        for spec in &specs {
            lib.rename_folder(&spec.folder_id, spec.name.clone());
        }
        let msg = if !is_batch {
            format!("Renamed folder {} to '{}'", specs[0].folder_id, specs[0].name)
        } else {
            format!("Renamed {n} folder(s)")
        };
        Ok(ToolResult::ok(msg))
    })
}

fn parse_rename_folder_specs(
    lib: &MediaLibrary,
    args: &Value,
) -> Result<(Vec<RenameFolderSpec>, bool), String> {
    if let Some(entries) = entry_objects(args)? {
        let mut specs = Vec::with_capacity(entries.len());
        for (idx, entry) in entries.iter().enumerate() {
            let path = format!("entries[{idx}]");
            let folder_id = require_string(entry, "folderId", &path)?;
            let name = require_string(entry, "name", &path)?;
            if lib.folder(&folder_id).is_none() {
                return Err(format!("{path}: folderId not found: {folder_id}"));
            }
            specs.push(RenameFolderSpec { folder_id, name });
        }
        return Ok((specs, true));
    }
    let obj = args.as_object().ok_or("rename_folder: arguments must be an object")?;
    let folder_id = require_string(obj, "folderId", "rename_folder")?;
    let name = require_string(obj, "name", "rename_folder")?;
    if lib.folder(&folder_id).is_none() {
        return Err(format!("folderId not found: {folder_id}"));
    }
    Ok((vec![RenameFolderSpec { folder_id, name }], false))
}

// ─────────────────────────────────────────────────────────────────────────────
// delete_media / delete_folder
// ─────────────────────────────────────────────────────────────────────────────

/// `delete_media` (`asset_ids[]`): remove assets + any referencing timeline clips in
/// the SAME undo step. Reference `deleteMedia`.
pub fn delete_media(state: &mut EditorState, args: &Value) -> ToolResult {
    let asset_ids: Vec<String> = match args.get("assetIds").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => {
            arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        }
        _ => return ToolResult::error("assetIds is required"),
    };
    for id in &asset_ids {
        if !state.library.assets.iter().any(|a| &a.id == id) {
            return ToolResult::error(format!("Media asset not found: {id}"));
        }
    }
    library_agent_edit(state, "Delete Asset", move |lib| {
        delete_media_assets(lib, &asset_ids);
        Ok(ToolResult::ok(format!(
            "Deleted {} asset(s). Any clips referencing them were removed from the timeline.",
            asset_ids.len()
        )))
    })
}

/// Remove assets by id + their manifest entries + any referencing timeline clips,
/// pruning empty tracks (reference `deleteMediaAssets(ids:)`).
fn delete_media_assets(lib: &mut MediaLibrary, ids: &[String]) {
    let set: HashSet<&String> = ids.iter().collect();
    for track in &mut lib.timeline.tracks {
        track.clips.retain(|c| !set.contains(&c.media_ref));
    }
    lib.timeline.tracks.retain(|t| !t.clips.is_empty());
    lib.assets.retain(|a| !set.contains(&a.id));
    lib.manifest.entries.retain(|e| !set.contains(&e.id));
}

/// `delete_folder` (`folder_ids[]`): recursively delete folders + descendants +
/// their assets + referencing clips in the SAME undo step. Reference `deleteFolder`.
pub fn delete_folder(state: &mut EditorState, args: &Value) -> ToolResult {
    let folder_ids: Vec<String> = match args.get("folderIds").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => {
            arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        }
        _ => return ToolResult::error("folderIds is required"),
    };
    for id in &folder_ids {
        if state.library.folder(id).is_none() {
            return ToolResult::error(format!("folderId not found: {id}"));
        }
    }
    library_agent_edit(state, "Delete Folder", move |lib| {
        let set: HashSet<String> = folder_ids.iter().cloned().collect();
        lib.delete_folders(&set);
        Ok(ToolResult::ok(format!(
            "Deleted {} folder(s) with their contents. Any clips referencing deleted assets \
             were removed from the timeline.",
            folder_ids.len()
        )))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// shared parse helpers
// ─────────────────────────────────────────────────────────────────────────────

/// `entries[]` as a list of objects, or `None` if the key is absent. Errors if
/// present-but-empty or a non-object element (reference `entryObjects`).
fn entry_objects(args: &Value) -> Result<Option<Vec<serde_json::Map<String, Value>>>, String> {
    let Some(raw) = args.get("entries") else { return Ok(None) };
    let arr = match raw.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return Err("Missing or empty 'entries' array".to_string()),
    };
    let mut out = Vec::with_capacity(arr.len());
    for (idx, el) in arr.iter().enumerate() {
        match el.as_object() {
            Some(o) => out.push(o.clone()),
            None => return Err(format!("entries[{idx}] must be an object")),
        }
    }
    Ok(Some(out))
}

fn require_string(obj: &serde_json::Map<String, Value>, key: &str, path: &str) -> Result<String, String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{path}: missing required field '{key}'"))
}

/// Resolve `parentFolderId` on an object: `None` if absent; error if it names a
/// folder that doesn't exist (reference `parentFolderId`).
fn parent_folder_id(
    lib: &MediaLibrary,
    obj: &serde_json::Map<String, Value>,
    path: &str,
) -> Result<Option<String>, String> {
    let Some(id) = obj.get("parentFolderId").and_then(Value::as_str) else {
        return Ok(None);
    };
    if lib.folder(id).is_none() {
        return Err(format!("{path}: parentFolderId not found: {id}"));
    }
    Ok(Some(id.to_string()))
}

/// Resolve a top-level `folderId` arg (reference `resolveFolderId`): `None` if
/// absent (→ root); error if it names a missing folder.
fn resolve_folder_id(lib: &MediaLibrary, args: &Value) -> Result<Option<String>, String> {
    let obj = args.as_object().ok_or("arguments must be an object")?;
    resolve_folder_id_obj(lib, obj, "import_media")
}

fn resolve_folder_id_obj(
    lib: &MediaLibrary,
    obj: &serde_json::Map<String, Value>,
    path: &str,
) -> Result<Option<String>, String> {
    let Some(id) = obj.get("folderId").and_then(Value::as_str) else {
        return Ok(None);
    };
    if lib.folder(id).is_none() {
        return Err(format!("{path}: folderId not found: {id}"));
    }
    Ok(Some(id.to_string()))
}

/// A non-empty `assetIds[]` whose ids all exist (reference `validAssetIds`).
fn valid_asset_ids(
    lib: &MediaLibrary,
    obj: &serde_json::Map<String, Value>,
    path: &str,
) -> Result<Vec<String>, String> {
    let ids: Vec<String> = obj
        .get("assetIds")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if ids.is_empty() {
        return Err(format!("{path}: assetIds is required"));
    }
    for id in &ids {
        if !lib.assets.iter().any(|a| &a.id == id) {
            return Err(format!("{path}: media asset not found: {id}"));
        }
    }
    Ok(ids)
}

// ─────────────────────────────────────────────────────────────────────────────
// mime + base64 + clip-type helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Map a MIME type to the file extension used to classify the import (reference
/// `fileExtension(forMime:)`).
fn file_extension_for_mime(mime: &str) -> Option<&'static str> {
    Some(match mime.to_ascii_lowercase().as_str() {
        "video/mp4" | "video/mpeg4" => "mp4",
        "video/quicktime" => "mov",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/aac" => "aac",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" => "m4a",
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/tiff" => "tiff",
        "image/heic" | "image/heif" => "heic",
        "application/json" | "application/vnd.lottie+json" => "json",
        _ => return None,
    })
}

fn clip_type_str(t: ClipType) -> &'static str {
    match t {
        ClipType::Video => "video",
        ClipType::Audio => "audio",
        ClipType::Image => "image",
        ClipType::Text => "text",
        ClipType::Lottie => "lottie",
    }
}

/// Decode standard base64 (RFC 4648, `+/` alphabet, optional `=` padding),
/// ignoring ASCII whitespace. Returns `None` on any invalid character.
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in input.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue;
        }
        let v = val(c)?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::EditorState;
    use palmier_model::{ClipType, MediaAsset};

    fn editor_with_asset(id: &str, path: &str) -> EditorState {
        let mut lib = MediaLibrary::new();
        let asset = MediaAsset::new(
            id,
            id,
            ClipType::Video,
            MediaSource::External { absolute_path: path.to_string() },
            1.0,
        );
        lib.manifest.entries.push(palmier_model::MediaManifestEntry {
            id: asset.id.clone(),
            name: asset.name.clone(),
            asset_type: asset.asset_type,
            source: asset.source.clone(),
            duration: asset.duration_seconds,
            generation_input: None,
            source_width: None,
            source_height: None,
            source_fps: None,
            has_audio: Some(asset.has_audio),
            folder_id: None,
            cached_remote_url: None,
            cached_remote_url_expires_at: None,
        });
        lib.assets.push(asset);
        EditorState::with_library(lib)
    }

    #[test]
    fn relink_media_repoints_source_on_asset_and_manifest_and_pushes_one_step() {
        let mut state = editor_with_asset("a1", "/old/clip.mov");
        let r = relink_media(&mut state, "a1", "/new/clip.mov");
        assert!(!r.is_error, "relink should succeed: {:?}", r.content);
        // Both the runtime asset and the manifest entry carry the new External path.
        let asset = state.library.assets.iter().find(|a| a.id == "a1").unwrap();
        assert_eq!(
            asset.source,
            MediaSource::External { absolute_path: "/new/clip.mov".into() }
        );
        let entry = state.library.manifest.entries.iter().find(|e| e.id == "a1").unwrap();
        assert_eq!(
            entry.source,
            MediaSource::External { absolute_path: "/new/clip.mov".into() }
        );
        // Exactly one library agent-undo step (the snapshot the panel relink relies on).
        assert_eq!(state.lib_history.agent_undo_len(), 1);
    }

    #[test]
    fn relink_media_rejects_unknown_asset_and_empty_path() {
        let mut state = editor_with_asset("a1", "/old/clip.mov");
        assert!(relink_media(&mut state, "nope", "/x.mov").is_error);
        assert!(relink_media(&mut state, "a1", "").is_error);
        // No change registered on the rejection paths.
        assert_eq!(state.lib_history.agent_undo_len(), 0);
    }

    #[test]
    fn move_folders_reparents_with_cycle_guard_and_one_step() {
        let mut lib = MediaLibrary::new();
        let a = lib.create_folder("A", None);
        let b = lib.create_folder("B", None);
        let mut state = EditorState::with_library(lib);

        // B → A is a valid reparent (one undo step, B.parent == A).
        let r = move_folders(&mut state, &[b.clone()], Some(&a));
        assert!(!r.is_error, "{:?}", r.content);
        assert_eq!(state.library.folder(&b).unwrap().parent_id.as_deref(), Some(a.as_str()));
        assert_eq!(state.lib_history.agent_undo_len(), 1);

        // Moving A into its descendant B is rejected by the model guard → no change,
        // no new undo step (the fully-rejected batch leaves the library untouched).
        let r2 = move_folders(&mut state, &[a.clone()], Some(&b));
        assert!(!r2.is_error, "tool reports ok even when the guard rejects the move");
        assert_eq!(state.library.folder(&a).unwrap().parent_id, None);
        assert_eq!(state.lib_history.agent_undo_len(), 1, "no step for a no-op move");
    }

    #[test]
    fn move_folders_rejects_unknown_ids() {
        let mut state = EditorState::with_library(MediaLibrary::new());
        assert!(move_folders(&mut state, &[], None).is_error);
        assert!(move_folders(&mut state, &["nope".into()], None).is_error);
    }

    #[test]
    fn base64_roundtrip_known_vectors() {
        // "foobar" → "Zm9vYmFy"; padding + whitespace tolerated.
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm9v\nYmFy").unwrap(), b"foobar");
        assert!(base64_decode("****").is_none());
    }

    #[test]
    fn mime_maps_cover_the_accepted_set() {
        assert_eq!(file_extension_for_mime("video/mp4"), Some("mp4"));
        assert_eq!(file_extension_for_mime("image/png"), Some("png"));
        assert_eq!(file_extension_for_mime("audio/wav"), Some("wav"));
        assert_eq!(file_extension_for_mime("application/x-bogus"), None);
    }
}
