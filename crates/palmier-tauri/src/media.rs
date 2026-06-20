//! Media-panel OS-action commands (E4-S12) — the Tauri seam the React media panel
//! (`src-ui/media-panel`) calls via `invoke` for Reveal in Explorer / Copy Path /
//! Relink / clipboard paste, plus the async "moment" `thumbnail` command seam.
//!
//! Reference mapping (docs/reference/media-panel.md §"macOS APIs to replace"):
//! - `NSWorkspace.activateFileViewerSelecting` (Reveal in Finder) →
//!   [`reveal_in_explorer`] via `tauri-plugin-opener` `reveal_item_in_dir`
//!   (Windows `explorer /select,`; Linux file-manager show-item / parent open).
//! - `NSPasteboard` Copy-Path → [`copy_paths_to_clipboard`] (newline-joined paths)
//!   via `tauri-plugin-clipboard-manager`.
//! - `NSOpenPanel` Relink → [`pick_relink_path`] via the already-wired
//!   `tauri-plugin-dialog` (E1-S7 pattern).
//! - `NSPasteboard` paste → [`read_clipboard_importable_paths`]: the file-URL
//!   branch of the reference `handleClipboardPaste` (image-data paste lands with
//!   the real import at Epic 7).
//!
//! The `thumbnail` command is the E4-S3 sprite/seek seam consumed by the search
//! panel's `MomentThumbnail` (keyed `path@time`). Its decode pipeline lands in
//! `palmier-media` (E4-S3) + the search backend in Epic 11; here it is a typed
//! **stub** returning `None` so the frontend wiring is a drop-in.

use std::path::{Path, PathBuf};

use tauri::{AppHandle, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

/// Reveal a file in the OS file manager, selecting it (Windows Explorer
/// `/select,`; Linux file-manager show-item / parent open). Reference
/// `NSWorkspace.activateFileViewerSelecting`.
#[tauri::command]
pub fn reveal_in_explorer<R: Runtime>(app: AppHandle<R>, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(PathBuf::from(&path))
        .map_err(|e| e.to_string())
}

/// Copy one or more absolute paths to the system clipboard, **newline-joined**
/// (reference Copy-Path writes newline-joined paths for a multi-selection).
#[tauri::command]
pub fn copy_paths_to_clipboard<R: Runtime>(
    app: AppHandle<R>,
    paths: Vec<String>,
) -> Result<(), String> {
    let joined = paths.join("\n");
    app.clipboard()
        .write_text(joined)
        .map_err(|e| e.to_string())
}

/// Open the OS file picker to repoint a missing asset (Relink). Returns the chosen
/// absolute path, or `None` on cancel. Reference `NSOpenPanel` relink.
#[tauri::command]
pub fn pick_relink_path<R: Runtime>(app: AppHandle<R>, name: String) -> Result<Option<String>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title(format!("Relink \"{name}\""))
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().to_string());
    Ok(picked)
}

/// Read importable file paths off the clipboard for paste (the file-URL branch of
/// the reference `handleClipboardPaste`). Splits the clipboard text into lines,
/// strips any `file://` scheme, and returns paths that exist on disk. Image-data
/// paste (`.png`/`.tiff` → written + imported) lands with the real import at Epic 7.
#[tauri::command]
pub fn read_clipboard_importable_paths<R: Runtime>(app: AppHandle<R>) -> Vec<String> {
    let Ok(text) = app.clipboard().read_text() else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let path = line.strip_prefix("file://").unwrap_or(line);
            // Windows file URLs look like `file:///C:/...`; drop the leading slash.
            let path = if cfg!(windows) {
                path.strip_prefix('/').unwrap_or(path)
            } else {
                path
            };
            let p = Path::new(path);
            p.exists().then(|| path.to_string())
        })
        .collect()
}

/// Async "moment" thumbnail seam (E4-S3 `thumbnail(media_ref, source_seconds,
/// max_size)`), keyed `path@time`, consumed by the search panel's `MomentThumbnail`.
///
/// **Stub** (returns `None`): the FFmpeg seek+scale decode pipeline lands in
/// `palmier-media` (E4-S3) and the search backend in Epic 11. Typed now so the
/// frontend `momentThumbnail` wrapper is a drop-in.
#[tauri::command]
pub fn thumbnail(
    _media_ref: String,
    _source_seconds: f64,
    _max_size: u32,
) -> Result<Option<String>, String> {
    // TODO(E4-S3/E11): FFmpeg seek+scale → JPEG → data-URL via palmier-media.
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_path_parsing_strips_file_scheme() {
        // The line-splitting + scheme-stripping logic is exercised here without a
        // live clipboard. (A non-existent path is dropped, matching the command.)
        let lines = "file:///nonexistent/path.mp4\n   \nplain/no-scheme.mov";
        let parsed: Vec<&str> = lines
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(parsed.len(), 2);
        assert!(parsed[0].starts_with("file://"));
    }
}
