// Shared export trigger + UI state machine (E12 menu wiring).
//
// The editor Toolbar's Export button AND the File → Export menu item must run the
// SAME export flow (native Save dialog → `export_video` → live progress → reveal).
// Rather than duplicate the state machine in two places, it lives here as a hook the
// Project surface owns: the Toolbar renders its state, and the menu handler calls the
// same `runExport`. One in-flight export at a time, shared.

import { useCallback, useEffect, useRef, useState } from "react";

import { exportVideo, onExportProgress } from "./export";
import { revealInExplorer } from "../media-panel/media-actions";

/** The editor Export button's UI state machine. */
export type ExportUiState =
  | { kind: "idle" }
  | { kind: "running"; frame: number; total: number }
  | { kind: "done"; outputPath: string }
  | { kind: "error"; message: string };

/** The shared export controller returned by {@link useExport}. */
export interface ExportController {
  state: ExportUiState;
  /** Run the export (native Save dialog → render → progress). No-op if already running. */
  runExport: () => Promise<void>;
  /** Reveal the last exported file in Explorer (only meaningful in the `done` state). */
  revealExport: () => void;
  /** Dismiss the done/error status back to idle. */
  dismissExport: () => void;
}

/**
 * Owns the export state machine. Mount once on the Project surface; share the returned
 * controller between the Toolbar (renders `state`, calls `runExport`/`revealExport`/
 * `dismissExport`) and the File → Export menu handler (calls `runExport`).
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

  const runExport = useCallback(async () => {
    if (running.current) return; // single in-flight export
    running.current = true;
    setState({ kind: "running", frame: 0, total: 0 });
    // Subscribe to per-frame progress for the duration of this export.
    const unlisten = await onExportProgress((p) => {
      if (!mounted.current) return;
      setState((prev) =>
        prev.kind === "running"
          ? { kind: "running", frame: p.frame, total: p.total }
          : prev,
      );
    });
    try {
      const result = await exportVideo(); // no path ⇒ backend opens Save dialog
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
