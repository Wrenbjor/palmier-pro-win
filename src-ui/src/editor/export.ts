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

/** A video codec the Export panel can pick (mirrors Rust `ExportFormat`). */
export type VideoFormat = "h264" | "h265" | "prores422";

/** A resolution preset the Export panel can pick (keyed off the project canvas). */
export type ResolutionPreset = "source" | "1080p" | "720p";

/** Options for {@link exportVideo} (the panel's explicit format + resolution). */
export interface ExportVideoOptions {
  /** Destination path. Omit ⇒ the backend opens a native Save dialog. */
  outPath?: string;
  /** Explicit codec. Omit ⇒ backend derives it from the extension (legacy). */
  format?: VideoFormat;
  /** Resolution preset. Omit ⇒ backend defaults to "source" (native scale). */
  resolution?: ResolutionPreset;
}

/**
 * Render the ACTIVE timeline to a video file. When `outPath` is omitted, the backend
 * opens a native Save dialog (the user cancelling returns `null`). `format` is the
 * EXPLICIT codec the panel chose; when omitted the backend derives it from the
 * extension (`.mov` ⇒ ProRes 422, else H.264 `.mp4`). `resolution` is the panel's
 * preset (keyed off the project canvas); omitted ⇒ "source" (native scale).
 *
 * Resolves to the {@link ExportResult} on success, `null` if cancelled / outside Tauri.
 * Rejects with the backend error string (no GPU, no HW encoder, FFmpeg error, …) so the
 * caller can surface it.
 */
export async function exportVideo(
  options: ExportVideoOptions = {},
): Promise<ExportResult | null> {
  if (!inTauri()) return null;
  const result = await invoke<ExportResult | null>("export_video", {
    outPath: options.outPath ?? null,
    format: options.format ?? null,
    resolution: options.resolution ?? null,
  });
  return result ?? null;
}

/**
 * Export the ACTIVE timeline as an FCP7/Premiere XMEML 4 document. Wraps the
 * `export_timeline_xml` command (the proven, golden-tested emitter). When `outPath` is
 * omitted the backend opens a native Save dialog (`.xml` filter; cancel ⇒ `null`).
 * Instant — there is no per-frame progress. Resolves to the absolute path written (for
 * reveal-in-explorer), or `null` if cancelled / outside Tauri. Rejects with the backend
 * error string on a write failure.
 */
export async function exportTimelineXml(outPath?: string): Promise<string | null> {
  if (!inTauri()) return null;
  const written = await invoke<string | null>("export_timeline_xml", {
    outPath: outPath ?? null,
  });
  return written ?? null;
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
