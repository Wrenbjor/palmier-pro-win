// Index-status pill — renders the CLIP/index model state machine:
// notInstalled→set up / downloading% / preparing / indexing N/M / ready / failed.
// The status comes from the live `search_media` visual.status (search.ts
// `indexStatusFromWire`); there is NO dedicated `download_search_model` command yet,
// so the "Set up" CTA probes the search backend (controller.setUpSearchModel), which
// nudges Epic 11's `SearchIndexCoordinator` to load/download the model on demand and
// drives the pill from the reported status — an honest action, not a no-op.
// (media-panel.md §"Search".)

import { Spacing, Theme } from "./theme";
import type { IndexStatus } from "./types";

export interface IndexStatusPillProps {
  status: IndexStatus;
  /**
   * Set up the visual-search model (the `notInstalled` CTA). Wired to
   * `controller.setUpSearchModel`, which probes `search_media` to trigger the
   * coordinator's model load and reflects the returned status here. When unset the
   * CTA is hidden (the pill is then a pure status read-out).
   */
  onDownload?: () => void;
}

export function IndexStatusPill({ status, onDownload }: IndexStatusPillProps) {
  let label: string;
  let action: (() => void) | undefined;
  let tone = Theme.text.tertiary;

  switch (status.kind) {
    case "notInstalled":
      label = "Visual search not set up";
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
          Set up
        </button>
      )}
    </div>
  );
}
