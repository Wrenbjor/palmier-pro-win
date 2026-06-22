// Tauri bridge for timeline → video EXPORT (E6-S5 render loop, wired here).
//
// Thin typed wrappers over the `export_video` command in
// `crates/palmier-tauri/src/export.rs` and the `export://progress` event the backend
// emits per encoded frame. The editor's Export button drives `exportVideo()` (which
// opens a native Save dialog Rust-side when no path is given), subscribes to progress
// via `onExportProgress`, and on success offers reveal-in-explorer.
//
// Degrades gracefully outside a Tauri webview (plain `vite dev`): `exportVideo` is a
// no-op returning `null`, and the progress subscription is a no-op unlisten.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** The backend progress event name (mirrors Rust `EXPORT_PROGRESS_EVENT`). */
export const EXPORT_PROGRESS_EVENT = "export://progress";

/** True when running inside a Tauri webview (vs plain `vite dev`). */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** One `export://progress` payload (camelCase; mirrors Rust `ExportProgress`). */
export interface ExportProgress {
  /** Frames encoded so far (`0..=total`). */
  frame: number;
  /** Total frames to encode (`0` until the loop bound is known). */
  total: number;
}

/** The successful export outcome (camelCase; mirrors Rust `ExportResult`). */
export interface ExportResult {
  /** The absolute path written. */
  outputPath: string;
  /** Encode width (even-snapped). */
  width: number;
  /** Encode height. */
  height: number;
  /** Total frames encoded. */
  frames: number;
  /** The FFmpeg encoder used (e.g. `h264_nvenc` / `prores_ks`). */
  encoder: string;
  /** Whether an audio track was muxed. */
  hasAudio: boolean;
}

/**
 * Render the ACTIVE timeline to a video file. When `outPath` is omitted, the backend
 * opens a native Save dialog (the user cancelling returns `null`). The codec is chosen
 * from the extension (`.mov` ⇒ ProRes 422, else H.264 `.mp4`).
 *
 * Resolves to the {@link ExportResult} on success, `null` if cancelled / outside Tauri.
 * Rejects with the backend error string (no GPU, no HW encoder, FFmpeg error, …) so the
 * caller can surface it.
 */
export async function exportVideo(outPath?: string): Promise<ExportResult | null> {
  if (!inTauri()) return null;
  const result = await invoke<ExportResult | null>("export_video", {
    outPath: outPath ?? null,
  });
  return result ?? null;
}

/**
 * Subscribe to `export://progress`; `handler` runs on each per-frame event. Returns an
 * unlisten fn (a no-op outside Tauri). Unsubscribe when the export completes / unmounts.
 */
export async function onExportProgress(
  handler: (progress: ExportProgress) => void,
): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<ExportProgress>(EXPORT_PROGRESS_EVENT, (event) =>
    handler(event.payload),
  );
}
