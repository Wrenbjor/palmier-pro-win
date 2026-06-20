// Media-tab toolbar (E4-S9): sort (4 modes #15), filter chips (video/audio/image +
// AI toggle), view-mode switch (folder/flat/grouped), thumbnail-size slider 80–200,
// new-folder, and the name-search field that drives the search panel (E4-S10).

import type { CSSProperties } from "react";
import { Spacing, Theme } from "./theme";
import {
  FILTERABLE_TYPES,
  SORT_MODES,
  THUMBNAIL_SIZE,
  VIEW_MODES,
  type FilterableType,
  type SortMode,
  type ViewMode,
} from "./types";
import type { FilterState } from "./logic";

const SORT_LABELS: Record<SortMode, string> = {
  name: "Name",
  dateAdded: "Date Added",
  duration: "Duration",
  type: "Type",
};
const VIEW_LABELS: Record<ViewMode, string> = {
  folder: "Folders",
  flat: "Flat",
  grouped: "Grouped",
};

export interface MediaToolbarProps {
  sort: SortMode;
  viewMode: ViewMode;
  thumbnailSize: number;
  filter: FilterState;
  onSort: (s: SortMode) => void;
  onViewMode: (v: ViewMode) => void;
  onThumbnailSize: (n: number) => void;
  onToggleType: (t: FilterableType) => void;
  onFilterAI: (on: boolean) => void;
  onQuery: (q: string) => void;
  onNewFolder: () => void;
}

export function MediaToolbar(props: MediaToolbarProps) {
  const { filter } = props;
  return (
    <div style={barStyle}>
      {/* row 1: search + new folder */}
      <div style={{ display: "flex", gap: Spacing.sm, alignItems: "center" }}>
        <input
          placeholder="Search media…"
          value={filter.query}
          onChange={(e) => props.onQuery(e.target.value)}
          style={{
            flex: 1,
            fontSize: 12,
            color: Theme.text.primary,
            background: Theme.background.base,
            border: `1px solid ${Theme.border.primary}`,
            borderRadius: 6,
            padding: "5px 8px",
          }}
        />
        <button
          title="New Folder (Ctrl+Shift+N)"
          onClick={props.onNewFolder}
          style={iconButtonStyle}
        >
          + Folder
        </button>
      </div>

      {/* row 2: sort + view */}
      <div style={{ display: "flex", gap: Spacing.sm, alignItems: "center" }}>
        <Segmented
          options={SORT_MODES.map((s) => ({ value: s, label: SORT_LABELS[s] }))}
          value={props.sort}
          onChange={(v) => props.onSort(v as SortMode)}
        />
        <div style={{ flex: 1 }} />
        <Segmented
          options={VIEW_MODES.map((v) => ({ value: v, label: VIEW_LABELS[v] }))}
          value={props.viewMode}
          onChange={(v) => props.onViewMode(v as ViewMode)}
        />
      </div>

      {/* row 3: filter chips + AI toggle + size slider */}
      <div style={{ display: "flex", gap: Spacing.sm, alignItems: "center" }}>
        {FILTERABLE_TYPES.map((t) => (
          <Chip
            key={t}
            label={t}
            active={filter.filterTypes.has(t)}
            onClick={() => props.onToggleType(t)}
          />
        ))}
        <Chip
          label="AI"
          active={filter.filterAI}
          onClick={() => props.onFilterAI(!filter.filterAI)}
        />
        <div style={{ flex: 1 }} />
        <input
          type="range"
          min={THUMBNAIL_SIZE.min}
          max={THUMBNAIL_SIZE.max}
          value={props.thumbnailSize}
          onChange={(e) => props.onThumbnailSize(Number(e.target.value))}
          title={`Thumbnail size: ${props.thumbnailSize}px`}
          style={{ width: 110 }}
        />
      </div>
    </div>
  );
}

function Segmented({
  options,
  value,
  onChange,
}: {
  options: { value: string; label: string }[];
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <div
      style={{
        display: "inline-flex",
        background: Theme.background.base,
        border: `1px solid ${Theme.border.subtle}`,
        borderRadius: 6,
        overflow: "hidden",
      }}
    >
      {options.map((o) => (
        <button
          key={o.value}
          onClick={() => onChange(o.value)}
          style={{
            fontSize: 11,
            padding: "4px 8px",
            border: "none",
            cursor: "pointer",
            color: value === o.value ? "#000" : Theme.text.secondary,
            background: value === o.value ? Theme.accent : "transparent",
            fontWeight: value === o.value ? 600 : 400,
          }}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function Chip({
  label,
  active,
  onClick,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      style={{
        fontSize: 11,
        textTransform: "uppercase",
        letterSpacing: 0.3,
        padding: "3px 9px",
        borderRadius: 12,
        cursor: "pointer",
        border: `1px solid ${active ? Theme.accentTimecode : Theme.border.primary}`,
        color: active ? "#000" : Theme.text.secondary,
        background: active ? Theme.accentTimecode : "transparent",
        fontWeight: active ? 600 : 400,
      }}
    >
      {label}
    </button>
  );
}

const barStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: Spacing.sm,
  padding: Spacing.md,
  borderBottom: `1px solid ${Theme.border.subtle}`,
  background: Theme.background.surface,
};

const iconButtonStyle: CSSProperties = {
  fontSize: 11,
  padding: "5px 8px",
  borderRadius: 6,
  cursor: "pointer",
  color: Theme.text.primary,
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.primary}`,
  whiteSpace: "nowrap",
};
