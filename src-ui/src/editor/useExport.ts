// Shared export trigger + UI state machine (E12 menu wiring).
//
// The editor Toolbar's Export button AND the File → Export menu item open the Export
// PANEL (format + resolution selection); the panel then drives the SAME export flow
// through this hook. Rather than duplicate the run/progress state machine in two
// places, it lives here as a hook the Project surface owns: the Toolbar renders its
// state, the menu handler + Toolbar button open the panel, and the panel's confirm
// calls `runExport`. One in-flight export at a time, shared between video (per-frame
// progress) and the instant Premiere-XML path.

import { useCallback, useEffect, useRef, useState } from "react";

import {
  exportVideo,
  exportTimelineXml,
  onExportProgress,
  type VideoFormat,
  type ResolutionPreset,
} from "./export";
import { revealInExplorer } from "../media-panel/media-actions";

/** The editor Export button's UI state machine. */
export type ExportUiState =
  | { kind: "idle" }
  | { kind: "running"; frame: number; total: number }
  | { kind: "done"; outputPath: string }
  | { kind: "error"; message: string };

/**
 * What the Export panel asks the controller to run. A `video` request carries the
 * panel's explicit codec + resolution preset (the backend opens the Save dialog,
 * filtered by the codec's container); an `xml` request runs the instant Premiere-XML
 * export (the backend opens an `.xml` Save dialog). Both reuse the single in-flight slot.
 */
export type ExportRequest =
  | { kind: "video"; format: VideoFormat; resolution: ResolutionPreset }
  | { kind: "xml" };

/** The shared export controller returned by {@link useExport}. */
export interface ExportController {
  state: ExportUiState;
  /**
   * Run an export (the panel's chosen request). Video → native Save dialog → render →
   * per-frame progress; XML → native Save dialog → instant write. No-op if already
   * running.
   */
  runExport: (request: ExportRequest) => Promise<void>;
  /** Reveal the last exported file in Explorer (only meaningful in the `done` state). */
  revealExport: () => void;
  /** Dismiss the done/error status back to idle. */
  dismissExport: () => void;
}

/**
 * Owns the export state machine. Mount once on the Project surface; share the returned
 * controller between the Toolbar (renders `state`, calls `revealExport`/`dismissExport`),
 * the File → Export menu handler + Toolbar Export button (which open the panel), and the
 * Export panel's confirm (calls `runExport`).
 */
export function useExport(): ExportController {
  const [state, setState] = useState<ExportUiState>({ kind: "idle" });
  // Track mount so an in-flight progress event after unmount doesn't setState.
  const mounted = useRef(true);
  // Guard against concurrent runs even if React batches a stale `state` read.
  const running = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const runExport = useCallback(async (request: ExportRequest) => {
    if (running.current) return; // single in-flight export
    running.current = true;

    // The Premiere-XML path is instant (no per-frame render), so it skips the progress
    // subscription and just runs the write → done/idle.
    if (request.kind === "xml") {
      setState({ kind: "running", frame: 0, total: 0 });
      try {
        const written = await exportTimelineXml(); // backend opens the .xml Save dialog
        if (!mounted.current) return;
        if (written === null) {
          // Cancelled (dialog dismissed) or outside Tauri — back to idle.
          setState({ kind: "idle" });
        } else {
          setState({ kind: "done", outputPath: written });
        }
      } catch (err) {
        if (mounted.current) setState({ kind: "error", message: String(err) });
        // eslint-disable-next-line no-console
        console.error("[editor] export_timeline_xml failed:", err);
      } finally {
        running.current = false;
      }
      return;
    }

    // Video path — subscribe to per-frame progress for the duration of this export.
    setState({ kind: "running", frame: 0, total: 0 });
    const unlisten = await onExportProgress((p) => {
      if (!mounted.current) return;
      setState((prev) =>
        prev.kind === "running"
          ? { kind: "running", frame: p.frame, total: p.total }
          : prev,
      );
    });
    try {
      // No path ⇒ backend opens the Save dialog; the panel's chosen codec + resolution
      // are threaded through so the encode matches the user's selection.
      const result = await exportVideo({
        format: request.format,
        resolution: request.resolution,
      });
      if (!mounted.current) return;
      if (result === null) {
        // Cancelled (dialog dismissed) or outside Tauri — back to idle.
        setState({ kind: "idle" });
      } else {
        setState({ kind: "done", outputPath: result.outputPath });
      }
    } catch (err) {
      if (mounted.current) {
        setState({ kind: "error", message: String(err) });
      }
      // eslint-disable-next-line no-console
      console.error("[editor] export_video failed:", err);
    } finally {
      running.current = false;
      unlisten();
    }
  }, []);

  const revealExport = useCallback(() => {
    setState((prev) => {
      if (prev.kind === "done") void revealInExplorer(prev.outputPath);
      return prev;
    });
  }, []);

  const dismissExport = useCallback(() => setState({ kind: "idle" }), []);

  return { state, runExport, revealExport, dismissExport };
}
