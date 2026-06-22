// Export panel — the format + resolution chooser the spec requires (FOUNDATION §385:
// `Export | Ctrl+E | Open export panel`). A modal dialog over the Project surface.
//
// The panel OWNS only the selection (format + resolution); the run/progress state
// stays in `useExport` (single in-flight export). On confirm it hands an
// `ExportRequest` to `runExport`:
//   - a video format → `export_video` (native Save dialog filtered to the codec's
//     container, then the proven render loop + per-frame progress);
//   - Premiere XML → `export_timeline_xml` (native `.xml` Save dialog, instant write).
//
// Styling matches the editor's dark surface + the #F29933 accent (theme.ts tokens),
// consistent with the Toolbar. No JS dialog plugin in this repo, so the native Save
// dialog is opened backend-side by the invoked command (the panel just closes on
// confirm and lets the shared controller drive it).

import { useCallback, useEffect, useState } from "react";
import type { CSSProperties, JSX } from "react";

import { Theme } from "./theme";
import type { ExportRequest } from "./useExport";
import type { VideoFormat, ResolutionPreset } from "./export";

/** The accent (AppTheme; matches the Toolbar's #F29933). */
const ACCENT = "#F29933";

/**
 * A selectable export format. `video` formats carry a {@link VideoFormat} codec; the
 * `xml` format is the Premiere/FCP7 XMEML path. `extension` is the container the Save
 * dialog filters to (informational in the label; the backend applies the real filter).
 */
export type FormatChoice =
  | { kind: "video"; id: VideoFormat; label: string; extension: "mp4" | "mov" }
  | { kind: "xml"; id: "xml"; label: string; extension: "xml" };

/**
 * The format options, in display order (spec §423 video containers + the Premiere XML
 * export from §638). Exported (with {@link RESOLUTION_OPTIONS}) so the editor checks can
 * assert each format maps to the right command + extension without a DOM.
 */
export const FORMAT_OPTIONS: readonly FormatChoice[] = [
  { kind: "video", id: "h264", label: "H.264 (.mp4)", extension: "mp4" },
  { kind: "video", id: "h265", label: "H.265 (.mp4)", extension: "mp4" },
  { kind: "video", id: "prores422", label: "ProRes 422 (.mov)", extension: "mov" },
  { kind: "xml", id: "xml", label: "Premiere XML (.xml)", extension: "xml" },
] as const;

/** A selectable resolution preset (`Source` keeps the timeline's own dimensions). */
export interface ResolutionChoice {
  id: ResolutionPreset;
  label: string;
}

/** The resolution options, in display order. `source` is the default (native scale). */
export const RESOLUTION_OPTIONS: readonly ResolutionChoice[] = [
  { id: "source", label: "Source" },
  { id: "1080p", label: "1080p" },
  { id: "720p", label: "720p" },
] as const;

/**
 * Translate the panel's selection into the {@link ExportRequest} the controller runs.
 * Pure — the editor checks assert the format→request mapping here. A `video` format
 * carries the chosen codec + resolution; `xml` ignores resolution (the emitter writes
 * the timeline's own dimensions).
 */
export function buildExportRequest(
  format: FormatChoice,
  resolution: ResolutionPreset,
): ExportRequest {
  return format.kind === "xml"
    ? { kind: "xml" }
    : { kind: "video", format: format.id, resolution };
}

export interface ExportPanelProps {
  /** Run the chosen export (the shared `useExport` controller's `runExport`). */
  onExport: (request: ExportRequest) => void;
  /** Close the panel without exporting. */
  onClose: () => void;
}

/**
 * The Export panel dialog. Pick a format (video codec or Premiere XML) and — for video
 * — a resolution; Export hands an {@link ExportRequest} to the controller and closes.
 * Resolution is disabled for the XML format (it carries the timeline's own dimensions).
 */
export function ExportPanel({ onExport, onClose }: ExportPanelProps): JSX.Element {
  const [formatId, setFormatId] = useState<FormatChoice["id"]>("h264");
  const [resolution, setResolution] = useState<ResolutionPreset>("source");

  const format =
    FORMAT_OPTIONS.find((f) => f.id === formatId) ?? FORMAT_OPTIONS[0];
  const isXml = format.kind === "xml";

  // Esc closes the panel (standard dialog affordance).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const confirm = useCallback(() => {
    onExport(buildExportRequest(format, resolution));
    onClose();
  }, [onExport, onClose, format, resolution]);

  return (
    <div
      style={backdropStyle}
      onMouseDown={(e) => {
        // Click on the backdrop (not the dialog) dismisses.
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Export"
        style={dialogStyle}
      >
        <h2 style={titleStyle}>Export</h2>

        {/* Format */}
        <label style={labelStyle} htmlFor="export-format">
          Format
        </label>
        <select
          id="export-format"
          value={formatId}
          onChange={(e) => setFormatId(e.target.value as FormatChoice["id"])}
          style={selectStyle}
        >
          {FORMAT_OPTIONS.map((f) => (
            <option key={f.id} value={f.id}>
              {f.label}
            </option>
          ))}
        </select>

        {/* Resolution (video only — XML carries the timeline's own dimensions). */}
        <label
          style={{ ...labelStyle, opacity: isXml ? 0.4 : 1 }}
          htmlFor="export-resolution"
        >
          Resolution
        </label>
        <select
          id="export-resolution"
          value={resolution}
          onChange={(e) => setResolution(e.target.value as ResolutionPreset)}
          disabled={isXml}
          style={{ ...selectStyle, opacity: isXml ? 0.4 : 1 }}
        >
          {RESOLUTION_OPTIONS.map((r) => (
            <option key={r.id} value={r.id}>
              {r.label}
            </option>
          ))}
        </select>

        {/* Actions */}
        <div style={actionsStyle}>
          <button type="button" style={cancelButtonStyle} onClick={onClose}>
            Cancel
          </button>
          <button type="button" style={exportButtonStyle} onClick={confirm}>
            Export
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Styles (editor dark surface + #F29933 accent, matching the Toolbar) ─────────

const backdropStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0, 0, 0, 0.55)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 50,
};

const dialogStyle: CSSProperties = {
  width: 320,
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.primary}`,
  borderRadius: 8,
  padding: 20,
  display: "flex",
  flexDirection: "column",
  gap: 8,
  color: Theme.text.primary,
  boxShadow: "0 12px 40px rgba(0, 0, 0, 0.5)",
};

const titleStyle: CSSProperties = {
  margin: "0 0 8px 0",
  fontSize: 15,
  fontWeight: 600,
  color: Theme.text.primary,
};

const labelStyle: CSSProperties = {
  fontSize: 11,
  color: Theme.text.tertiary,
  marginTop: 4,
};

const selectStyle: CSSProperties = {
  height: 30,
  background: Theme.background.surface,
  color: Theme.text.primary,
  border: `1px solid ${Theme.border.subtle}`,
  borderRadius: 4,
  padding: "0 8px",
  fontSize: 13,
};

const actionsStyle: CSSProperties = {
  display: "flex",
  justifyContent: "flex-end",
  gap: 8,
  marginTop: 16,
};

const baseButton: CSSProperties = {
  height: 30,
  padding: "0 14px",
  borderRadius: 4,
  fontSize: 13,
  cursor: "pointer",
  border: "none",
};

const cancelButtonStyle: CSSProperties = {
  ...baseButton,
  background: "transparent",
  color: Theme.text.secondary,
  border: `1px solid ${Theme.border.subtle}`,
};

const exportButtonStyle: CSSProperties = {
  ...baseButton,
  background: ACCENT,
  color: "#1a1206",
  fontWeight: 600,
};
