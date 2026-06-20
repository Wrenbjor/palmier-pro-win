// Index-status pill (E4-S10 stub) — renders the CLIP/index model state machine:
// notInstalled→download / downloading% / preparing / indexing N/M / ready / failed.
// Driven by a stubbed status (store.indexStatus); real progress arrives from Epic
// 11's `SearchIndexCoordinator` via a Tauri event. (media-panel.md §"Search".)

import { Spacing, Theme } from "./theme";
import type { IndexStatus } from "./types";

export interface IndexStatusPillProps {
  status: IndexStatus;
  /** TODO(E11): kicks `invoke('download_search_model')`; no-op stub today. */
  onDownload?: () => void;
}

export function IndexStatusPill({ status, onDownload }: IndexStatusPillProps) {
  let label: string;
  let action: (() => void) | undefined;
  let tone = Theme.text.tertiary;

  switch (status.kind) {
    case "notInstalled":
      label = "Search model not installed";
      action = onDownload;
      break;
    case "downloading":
      label = `Downloading model ${Math.round(status.fraction * 100)}%`;
      break;
    case "preparing":
      label = "Preparing search…";
      break;
    case "indexing":
      label = `Indexing ${status.completed}/${status.total}`;
      break;
    case "ready":
      label = "Search ready";
      tone = Theme.accentTimecode;
      break;
    case "failed":
      label = `Search failed: ${status.message}`;
      tone = Theme.status.error;
      break;
  }

  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: Spacing.sm,
        fontSize: 11,
        color: tone,
        padding: `${Spacing.xs}px ${Spacing.md}px`,
        borderTop: `1px solid ${Theme.border.subtle}`,
        background: Theme.background.surface,
      }}
    >
      <span
        style={{
          width: 7,
          height: 7,
          borderRadius: "50%",
          background: tone,
          flexShrink: 0,
        }}
      />
      <span style={{ flex: 1 }}>{label}</span>
      {action && (
        <button
          onClick={action}
          style={{
            fontSize: 11,
            padding: "2px 8px",
            borderRadius: 4,
            cursor: "pointer",
            color: "#000",
            background: Theme.accent,
            border: "none",
          }}
        >
          Download
        </button>
      )}
    </div>
  );
}
