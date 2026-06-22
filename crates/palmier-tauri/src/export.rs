//! Timeline → video file EXPORT — wires the proven `palmier-export` render loop
//! (E6-S5) to a Tauri command + the editor UI.
//!
//! ## What this is
//! The render engine already EXISTS and is proven: `palmier_export::video::export_video`
//! (offscreen wgpu composite → FFmpeg HW encode (NVENC/QSV/AMF/MediaFoundation) or
//! `prores_ks` → optional AAC mux → finalize, with cancellation + per-frame progress).
//! It is behind the `gpu-export` cargo feature. This module is the **wiring only** — it
//! does NOT change the render behavior:
//!
//! 1. snapshot the ACTIVE timeline + the `media_ref → (path, natural_size)` map out of
//!    the ONE shared `Arc<ToolExecutor>` (`AgentState.executor`) — exactly the same
//!    snapshot the robust preview path (`preview_render.rs`) takes, so the export
//!    reflects the live edit state with zero extra source of truth;
//! 2. build the `SourceResolver` (geometry, from asset natural sizes) + a
//!    `palmier_media::FrameSource` (the one-decode-owner, `media_ref → path`) the
//!    compositor pulls pixels from;
//! 3. run `export_video` on a **blocking worker** (`spawn_blocking`) so the heavy
//!    GPU/FFmpeg work never blocks the UI or other commands;
//! 4. emit per-frame progress as a Tauri event (`export://progress` → `{frame,total}`);
//! 5. return the `VideoExportOutcome` (or an error string the panel surfaces).
//!
//! ## Audio (v1)
//! This wiring exports a **video-only** file (the render's `AudioInput` is the empty
//! slice — the render path explicitly supports that, producing a file with no AAC
//! track). Wiring the pre-decoded/resampled audio mix (`preview_audio.rs` owns the
//! decode/resample/stretch stage) is a follow-up; the render side already accepts it.
//!
//! ## Save path
//! The frontend may pass `out_path`; when it is `None` the command opens the native
//! Save dialog itself (`tauri-plugin-dialog`, the SAME `app.dialog().file()` seam
//! `pick_relink_path` / `prompt_save_bundle` use — the JS dialog plugin is not an npm
//! dependency in this repo, so the dialog is driven Rust-side). The default file name
//! is `export.mp4`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use tauri_plugin_dialog::DialogExt;

use palmier_engine::SourceInfo;
// The proven render loop, imported under an alias so it doesn't collide with this
// module's `export_video` Tauri command (which wraps it).
use palmier_export::video::export_video as run_video_export;
use palmier_export::video::{
    CancelFlag, ExportFormat, ExportResolution, VideoExportConfig, VideoExportOutcome,
};
use palmier_media::FrameSource;
use palmier_model::{MediaSource, Timeline};

use crate::agent::AgentState;

/// The Tauri event the export streams per-frame progress over (the editor's export
/// flow subscribes to this to drive a progress bar).
pub const EXPORT_PROGRESS_EVENT: &str = "export://progress";

/// One `export://progress` payload: the just-encoded frame index and the total
/// frame count. The frontend renders `frame/total` as a progress bar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportProgress {
    /// Frames encoded so far (`0..=total`).
    frame: u64,
    /// Total frames to encode.
    total: u64,
}

/// The result handed back to the frontend after a successful export (camelCase to
/// match the TS wrapper). Projects [`VideoExportOutcome`] into a serializable shape.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    /// The absolute path written.
    pub output_path: String,
    /// Encode width (even-snapped).
    pub width: u32,
    /// Encode height.
    pub height: u32,
    /// Total frames encoded.
    pub frames: u64,
    /// The FFmpeg encoder used (diagnostic, e.g. `h264_nvenc` / `prores_ks`).
    pub encoder: String,
    /// Whether an audio track was muxed (v1: always false — video-only wiring).
    pub has_audio: bool,
}

impl ExportResult {
    fn from_outcome(outcome: VideoExportOutcome) -> ExportResult {
        ExportResult {
            output_path: outcome.output_path.to_string_lossy().to_string(),
            width: outcome.width,
            height: outcome.height,
            frames: outcome.frames,
            encoder: outcome.encoder.to_string(),
            has_audio: outcome.has_audio,
        }
    }
}

/// Snapshot of everything the export render needs out of the shared `EditorState`,
/// taken under the executor lock so the render itself runs lock-free. Mirrors the
/// `preview_render::snapshot` so export and preview composite the SAME live state.
struct ExportSnapshot {
    timeline: Timeline,
    /// `media_ref → absolute source path` (the decode owner's resolver map).
    urls: HashMap<String, PathBuf>,
    /// `media_ref → natural size` (falls back to the canvas when unknown).
    sizes: HashMap<String, (f64, f64)>,
}

/// Pull the live timeline + media-path/size maps out of the shared executor — the
/// SAME read the preview path uses (one source of truth for the live edit state).
fn snapshot(agent: &AgentState) -> ExportSnapshot {
    agent.executor.with_state_ref(|state| {
        let timeline = state.library.timeline.clone();
        let mut urls = HashMap::new();
        let mut sizes = HashMap::new();
        for asset in &state.library.assets {
            if let Some(path) = asset_path(&asset.source) {
                urls.insert(asset.id.clone(), path);
            }
            if let (Some(w), Some(h)) = (asset.source_width, asset.source_height) {
                if w > 0 && h > 0 {
                    sizes.insert(asset.id.clone(), (w as f64, h as f64));
                }
            }
        }
        ExportSnapshot {
            timeline,
            urls,
            sizes,
        }
    })
}

/// Resolve a [`MediaSource`] to an absolute path for decoding (same logic as
/// `preview_render::asset_path`): `External` uses its absolute path; `Project` uses
/// its relative path as-is. Empty path ⇒ `None` (the layer is skipped, never a crash).
fn asset_path(source: &MediaSource) -> Option<PathBuf> {
    let raw = match source {
        MediaSource::External { absolute_path } => absolute_path,
        MediaSource::Project { relative_path } => relative_path,
    };
    if raw.is_empty() {
        None
    } else {
        Some(PathBuf::from(raw))
    }
}

/// Pick the output format from the file extension (`.mov` → ProRes 422, anything
/// else → H.264 MP4). Keeps the command's surface tiny — the user picks the codec by
/// choosing the extension in the save dialog, defaulting to H.264 `.mp4`.
fn format_for_path(path: &std::path::Path) -> ExportFormat {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("mov") => ExportFormat::ProRes422,
        _ => ExportFormat::H264,
    }
}

/// Open the native Save dialog for a video file (reference `NSSavePanel`), defaulting
/// the file name to `export.mp4` with an `.mp4` / `.mov` filter. Returns the chosen
/// path, or `None` on cancel. Driven Rust-side (the JS dialog plugin is not an npm dep
/// here), the SAME `app.dialog().file()` seam `pick_relink_path` uses.
fn prompt_export_path<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    app.dialog()
        .file()
        .set_title("Export Video")
        .set_file_name("export.mp4")
        .add_filter("MP4 Video", &["mp4"])
        .add_filter("QuickTime (ProRes)", &["mov"])
        .blocking_save_file()
        .and_then(|p| p.into_path().ok())
}

/// `export_video` — render the ACTIVE timeline to a video file.
///
/// `out_path` is the destination; when `None` the native Save dialog is opened (the
/// user cancelling it returns `Ok(None)` — not an error). The codec is chosen from the
/// extension (`.mov` ⇒ ProRes 422, else H.264 MP4). Resolution defaults to the 1080p
/// short-side preset and output fps defaults to the project fps. Progress streams over
/// [`EXPORT_PROGRESS_EVENT`].
///
/// The heavy GPU/FFmpeg work runs on a blocking worker via
/// [`tauri::async_runtime::spawn_blocking`] so it never blocks the UI / other commands.
/// Returns the [`ExportResult`] on success, `Ok(None)` if the dialog was cancelled, or
/// an `Err(String)` the panel surfaces (no GPU, no HW encoder, FFmpeg error, …).
#[tauri::command]
pub async fn export_video<R: Runtime>(
    app: AppHandle<R>,
    agent: State<'_, AgentState>,
    out_path: Option<String>,
) -> Result<Option<ExportResult>, String> {
    // Resolve the destination — either the explicit path or a Save dialog. The dialog
    // is blocking; run it on a blocking worker so the async runtime stays free.
    let output_path = match out_path {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => {
            let app_for_dialog = app.clone();
            let picked =
                tauri::async_runtime::spawn_blocking(move || prompt_export_path(&app_for_dialog))
                    .await
                    .map_err(|e| format!("export save dialog task failed: {e}"))?;
            match picked {
                Some(p) => p,
                None => return Ok(None), // user cancelled — not an error
            }
        }
    };

    // Snapshot the live edit state on the async thread (cheap clone under the lock).
    let snap = snapshot(&agent);

    // Build the export config. The codec comes from the extension; resolution defaults
    // to the proven 1080p short-side preset; output fps `0` ⇒ use the project fps.
    let format = format_for_path(&output_path);
    let config = VideoExportConfig {
        format,
        resolution: ExportResolution::P1080,
        output_path: output_path.clone(),
        output_fps: 0, // use the project fps
    };

    let app_for_render = app.clone();
    let cancel = CancelFlag::new();

    // Offload the heavy composite + encode + mux to a blocking worker.
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        run_export(&snap, &config, &app_for_render, &cancel)
    })
    .await
    .map_err(|e| format!("export task failed: {e}"))?;

    outcome.map(|o| Some(ExportResult::from_outcome(o)))
}

/// The Tauri-free render core: build the geometry resolver + the `FrameSource` decode
/// owner from the snapshot, then run the proven `export_video` render loop, emitting
/// progress. Returns the outcome or an error string.
fn run_export<R: Runtime>(
    snap: &ExportSnapshot,
    config: &VideoExportConfig,
    app: &AppHandle<R>,
    cancel: &CancelFlag,
) -> Result<VideoExportOutcome, String> {
    // The decode owner (one-decode-owner contract): a FrameSource over a resolver that
    // maps each media_ref to its absolute path (offline refs ⇒ None ⇒ skipped layer).
    let urls = snap.urls.clone();
    let resolver: palmier_media::UrlResolver =
        Arc::new(move |media_ref: &str| urls.get(media_ref).cloned());
    let frames = FrameSource::new(resolver);

    // The geometry resolver: each media_ref's natural size (falls back to the canvas),
    // exactly as the preview path builds it. A closure is a `SourceResolver`.
    let sizes = snap.sizes.clone();
    let canvas_size = (
        snap.timeline.width.max(1) as f64,
        snap.timeline.height.max(1) as f64,
    );
    let geometry = move |media_ref: &str| -> Option<SourceInfo> {
        let nat = sizes.get(media_ref).copied().unwrap_or(canvas_size);
        Some(SourceInfo::upright(nat))
    };

    // The total frame count for the progress event (the render reports a fraction).
    let total = snap.timeline.total_frames().max(0) as u64;

    // Emit an initial 0/total so the UI can show the bar immediately.
    emit_progress(app, 0, total);

    // Run the proven render loop. The progress callback turns the 0..=1 fraction back
    // into (frame,total) for the event. Audio is empty (video-only wiring, v1).
    let app_for_cb = app.clone();
    let outcome = run_video_export(
        &snap.timeline,
        &geometry,
        &frames,
        &Vec::new(), // no audio (v1: video-only)
        config,
        |fraction| {
            let frame = (fraction * total as f64).round() as u64;
            emit_progress(&app_for_cb, frame.min(total), total);
        },
        cancel,
    )
    .map_err(|e| e.to_string())?;

    // Final 100% (defensive — the per-frame callback already reached total).
    emit_progress(app, outcome.frames, outcome.frames);
    Ok(outcome)
}

/// Emit one `export://progress` event. Logged-but-non-fatal on failure.
fn emit_progress<R: Runtime>(app: &AppHandle<R>, frame: u64, total: u64) {
    if let Err(err) = app.emit(EXPORT_PROGRESS_EVENT, ExportProgress { frame, total }) {
        tracing::warn!(target: "export", error = %err, "failed to emit export://progress");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_is_chosen_from_extension() {
        assert_eq!(format_for_path(std::path::Path::new("a.mov")), ExportFormat::ProRes422);
        assert_eq!(format_for_path(std::path::Path::new("a.MOV")), ExportFormat::ProRes422);
        assert_eq!(format_for_path(std::path::Path::new("a.mp4")), ExportFormat::H264);
        // Unknown / no extension defaults to H.264 MP4.
        assert_eq!(format_for_path(std::path::Path::new("a.mkv")), ExportFormat::H264);
        assert_eq!(format_for_path(std::path::Path::new("noext")), ExportFormat::H264);
    }

    #[test]
    fn asset_path_resolves_external_and_skips_empty() {
        let ext = MediaSource::External {
            absolute_path: "/clip.mp4".into(),
        };
        assert_eq!(asset_path(&ext), Some(PathBuf::from("/clip.mp4")));
        let empty = MediaSource::External {
            absolute_path: String::new(),
        };
        assert_eq!(asset_path(&empty), None);
    }
}
