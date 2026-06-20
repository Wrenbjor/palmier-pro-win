// Search-results panel (E4-S10): three sections — Moments (visual frame grid),
// Spoken (transcript segments with timecodes), Files (name matches). Shown only when
// the query is non-empty. Moments/Spoken are collapsible; empty → "No matches".
//
// The Moments/Spoken data is fed by Epic 11 (`search_media`); here those arrays are
// empty (stub) so both render "No matches" until the backend lands. The Files
// section always works (local name filter). Moment thumbnails would call the E4-S3
// `thumbnail(media_ref, source_seconds, max_size)` command (TODO(E11)); here they
// render a type-colored placeholder keyed `path@time`.

import { useState } from "react";
import type { CSSProperties } from "react";
import { Spacing, Theme, typeColor } from "./theme";
import { formatTimecode } from "./search";
import { momentUri } from "./drag";
import type {
  MediaAssetView,
  SearchResults,
  SpokenHit,
  VisualHit,
} from "./types";

export interface SearchResultsPanelProps {
  results: SearchResults;
  assetsById: ReadonlyMap<string, MediaAssetView>;
  onSelectMoment?: (hit: VisualHit) => void;
  onSelectSpoken?: (hit: SpokenHit) => void;
  onSelectFile?: (asset: MediaAssetView) => void;
}

export function SearchResultsPanel({
  results,
  assetsById,
  onSelectMoment,
  onSelectSpoken,
  onSelectFile,
}: SearchResultsPanelProps) {
  const [momentsOpen, setMomentsOpen] = useState(true);
  const [spokenOpen, setSpokenOpen] = useState(true);

  return (
    <div
      style={{
        flex: 1,
        minHeight: 0,
        overflowY: "auto",
        padding: Spacing.md,
        display: "flex",
        flexDirection: "column",
        gap: Spacing.lg,
      }}
    >
      <Section
        title="Moments"
        count={results.moments.length}
        collapsible
        open={momentsOpen}
        onToggle={() => setMomentsOpen((o) => !o)}
      >
        {results.moments.length === 0 ? (
          <Empty />
        ) : (
          <div
            style={{
              display: "grid",
              gridTemplateColumns: "repeat(auto-fill, minmax(120px, 1fr))",
              gap: Spacing.sm,
            }}
          >
            {results.moments.map((hit) => {
              const asset = assetsById.get(hit.assetID);
              return (
                <MomentCard
                  key={`${hit.assetID}@${hit.time}`}
                  hit={hit}
                  asset={asset}
                  onClick={() => onSelectMoment?.(hit)}
                />
              );
            })}
          </div>
        )}
      </Section>

      <Section
        title="Spoken"
        count={results.spoken.length}
        collapsible
        open={spokenOpen}
        onToggle={() => setSpokenOpen((o) => !o)}
      >
        {results.spoken.length === 0 ? (
          <Empty />
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: Spacing.xs }}>
            {results.spoken.map((hit, i) => {
              const asset = assetsById.get(hit.assetID);
              return (
                <SpokenRow
                  key={`${hit.assetID}-${hit.start}-${i}`}
                  hit={hit}
                  asset={asset}
                  onClick={() => onSelectSpoken?.(hit)}
                />
              );
            })}
          </div>
        )}
      </Section>

      <Section title="Files" count={results.files.length}>
        {results.files.length === 0 ? (
          <Empty />
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            {results.files.map((a) => (
              <button
                key={a.id}
                onClick={() => onSelectFile?.(a)}
                style={fileRowStyle}
              >
                <span
                  style={{
                    width: 8,
                    height: 8,
                    borderRadius: 2,
                    background: typeColor(a.type),
                  }}
                />
                <span style={{ flex: 1, textAlign: "left" }}>{a.name}</span>
                <span style={{ color: Theme.text.muted, fontSize: 10 }}>
                  {a.type}
                </span>
              </button>
            ))}
          </div>
        )}
      </Section>
    </div>
  );
}

function MomentCard({
  hit,
  asset,
  onClick,
}: {
  hit: VisualHit;
  asset?: MediaAssetView;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      draggable
      onDragStart={(e) =>
        e.dataTransfer.setData(
          "text/plain",
          momentUri(hit.assetID, hit.shotStart, hit.shotEnd),
        )
      }
      style={{
        border: `1px solid ${Theme.border.subtle}`,
        borderRadius: 6,
        overflow: "hidden",
        background: Theme.background.raised,
        cursor: "pointer",
        padding: 0,
      }}
    >
      <div
        style={{
          // Moment thumbnail keyed path@time (TODO(E11): real thumbnail command).
          height: 68,
          background: asset
            ? typeColor(asset.type, 0.3)
            : Theme.background.prominent,
        }}
      />
      <div style={{ padding: 4, fontSize: 10, color: Theme.text.tertiary }}>
        {formatTimecode(hit.shotStart)}–{formatTimecode(hit.shotEnd)}
      </div>
    </button>
  );
}

function SpokenRow({
  hit,
  asset,
  onClick,
}: {
  hit: SpokenHit;
  asset?: MediaAssetView;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      draggable
      onDragStart={(e) =>
        e.dataTransfer.setData(
          "text/plain",
          momentUri(hit.assetID, hit.start, Math.max(hit.end, hit.start + 0.1)),
        )
      }
      style={{
        display: "flex",
        gap: Spacing.sm,
        textAlign: "left",
        background: Theme.background.raised,
        border: `1px solid ${Theme.border.subtle}`,
        borderRadius: 6,
        padding: Spacing.sm,
        cursor: "pointer",
      }}
    >
      <div
        style={{
          width: 48,
          height: 27,
          flexShrink: 0,
          borderRadius: 3,
          background: asset ? typeColor(asset.type, 0.3) : Theme.background.prominent,
        }}
      />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          style={{
            fontSize: 11,
            color: Theme.text.secondary,
            display: "-webkit-box",
            WebkitLineClamp: 3,
            WebkitBoxOrient: "vertical",
            overflow: "hidden",
          }}
        >
          {hit.text}
        </div>
        <div style={{ fontSize: 10, color: Theme.text.muted, marginTop: 2 }}>
          {asset?.name ?? hit.assetID} · {formatTimecode(hit.start)}
        </div>
      </div>
    </button>
  );
}

function Section({
  title,
  count,
  collapsible,
  open = true,
  onToggle,
  children,
}: {
  title: string;
  count: number;
  collapsible?: boolean;
  open?: boolean;
  onToggle?: () => void;
  children: React.ReactNode;
}) {
  return (
    <div>
      <button
        onClick={collapsible ? onToggle : undefined}
        style={{
          display: "flex",
          alignItems: "center",
          gap: Spacing.sm,
          width: "100%",
          background: "transparent",
          border: "none",
          padding: 0,
          marginBottom: Spacing.sm,
          cursor: collapsible ? "pointer" : "default",
          color: Theme.text.secondary,
          fontSize: 12,
          fontWeight: 600,
        }}
      >
        {collapsible && <span>{open ? "▾" : "▸"}</span>}
        <span>{title}</span>
        <span style={{ color: Theme.text.muted }}>({count})</span>
      </button>
      {open && children}
    </div>
  );
}

function Empty() {
  return (
    <div style={{ fontSize: 11, color: Theme.text.muted, padding: Spacing.sm }}>
      No matches
    </div>
  );
}

const fileRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
  fontSize: 12,
  color: Theme.text.primary,
  background: "transparent",
  border: "none",
  borderRadius: 4,
  padding: "5px 6px",
  cursor: "pointer",
};
