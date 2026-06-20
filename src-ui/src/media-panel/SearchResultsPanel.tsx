// Search-results panel (E4-S10 scaffold; E11-S11 Moments/Spoken UI + navigation).
// Three sections — Moments (visual frame grid), Spoken (transcript rows), Files
// (name matches). Shown only when the query is non-empty. Moments/Spoken are
// collapsible; empty → "No matches". Ported from `MediaTab/MediaTab+Search.swift`.
//
// The Moments/Spoken data is fed by the Epic 11 search backend (E11-S6 visual +
// E11-S8 spoken, via the `search_media` command — see `search.ts`). The Files
// section always works (local name filter). Moment/Spoken thumbnails call the
// E4-S3 `thumbnail(media_ref, source_seconds, max_size)` command (`momentThumbnail`,
// 240px / 1s tolerance) and fall back to a type-colored placeholder until it lands.
//
// Parity targets (search.md §"UI result navigation" + the reference):
//   - Moment card: thumbnail at hit.time; name label (1 line); timecode
//     `shotStart–shotEnd` shown for video only. Range = [shotStart,
//     max(shotEnd, shotStart+0.1)]. Tap → selectMediaAsset(asset,
//     atSourceFrame: secondsToFrame(range.lowerBound, fps)). Draggable payload =
//     plain asset for stills, else moment segment over the range.
//   - Spoken row: thumbnail at hit.start; transcript text (3 lines); `name ·
//     timecode`. Range = [start, max(end, start+0.1)]. Tap → seek to start.
//     Draggable with the range segment.

import { useEffect, useState } from "react";
import type { CSSProperties } from "react";
import { Spacing, Theme, typeColor } from "./theme";
import { formatTimecode } from "./search";
import { assetUri, momentUri } from "./drag";
import { momentThumbnail } from "./media-actions";
import type {
  MediaAssetView,
  SearchResults,
  SpokenHit,
  VisualHit,
} from "./types";

export interface SearchResultsPanelProps {
  results: SearchResults;
  assetsById: ReadonlyMap<string, MediaAssetView>;
  /** Tap a Moment hit → select asset at `shotStart` (controller converts to frame). */
  onSelectMoment?: (hit: VisualHit) => void;
  /** Tap a Spoken hit → select asset / seek to `start`. */
  onSelectSpoken?: (hit: SpokenHit) => void;
  onSelectFile?: (asset: MediaAssetView) => void;
}

/** Inclusive source range for a moment: `[shotStart, max(shotEnd, shotStart+0.1)]`. */
function momentRange(shotStart: number, shotEnd: number): [number, number] {
  return [shotStart, Math.max(shotEnd, shotStart + 0.1)];
}

/** Inclusive source range for a spoken hit: `[start, max(end, start+0.1)]`. */
function spokenRange(start: number, end: number): [number, number] {
  return [start, Math.max(end, start + 0.1)];
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

/**
 * Async moment thumbnail keyed `path@time` (E4-S3 `thumbnail` command seam, 240px /
 * 1s tolerance). Stills render the asset's own thumbnail; videos seek to `time`.
 * Falls back to a type-colored placeholder until the palmier-media pipeline lands.
 */
function MomentThumbnail({
  asset,
  time,
  height,
}: {
  asset?: MediaAssetView;
  time: number;
  height?: number | string;
}) {
  const isImage = asset?.type === "image";
  // Stills show their existing thumbnail (no per-time seek); videos seek to `time`.
  const [thumb, setThumb] = useState<string | undefined>(asset?.thumbnailUrl);
  useEffect(() => {
    if (isImage) {
      setThumb(asset?.thumbnailUrl);
      return;
    }
    if (!asset?.path) return;
    let active = true;
    void momentThumbnail(asset.path, time).then((url) => {
      if (active && url) setThumb(url);
    });
    return () => {
      active = false;
    };
  }, [asset?.path, asset?.thumbnailUrl, time, isImage]);

  return (
    <div
      style={{
        height,
        aspectRatio: height === undefined ? "16 / 9" : undefined,
        width: "100%",
        background: thumb
          ? `center / cover no-repeat url(${thumb})`
          : asset
            ? typeColor(asset.type, 0.3)
            : Theme.background.prominent,
      }}
    />
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
  const isImage = asset?.type === "image";
  const [rangeStart, rangeEnd] = momentRange(hit.shotStart, hit.shotEnd);
  // Stills drag as plain assets — a source segment is meaningless for them.
  const payload = isImage
    ? assetUri(hit.assetID)
    : momentUri(hit.assetID, rangeStart, rangeEnd);

  return (
    <button
      onClick={onClick}
      draggable
      onDragStart={(e) => e.dataTransfer.setData("text/plain", payload)}
      style={{
        border: `1px solid ${Theme.border.subtle}`,
        borderRadius: 6,
        overflow: "hidden",
        background: Theme.background.raised,
        cursor: "pointer",
        padding: 0,
        display: "flex",
        flexDirection: "column",
        textAlign: "left",
      }}
    >
      <MomentThumbnail asset={asset} time={hit.time} height={68} />
      <div style={{ padding: 4 }}>
        <div
          style={{
            fontSize: 11,
            color: Theme.text.secondary,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {asset?.name ?? ""}
        </div>
        {!isImage && (
          <div
            style={{
              fontSize: 10,
              color: Theme.text.tertiary,
              fontVariantNumeric: "tabular-nums",
            }}
          >
            {formatTimecode(rangeStart)}–{formatTimecode(rangeEnd)}
          </div>
        )}
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
  const [rangeStart, rangeEnd] = spokenRange(hit.start, hit.end);
  return (
    <button
      onClick={onClick}
      draggable
      onDragStart={(e) =>
        e.dataTransfer.setData(
          "text/plain",
          momentUri(hit.assetID, rangeStart, rangeEnd),
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
          width: 64,
          height: 36,
          flexShrink: 0,
          borderRadius: 3,
          overflow: "hidden",
        }}
      >
        {/* Thumbnail seeked to the spoken hit's start time (parity). */}
        <MomentThumbnail asset={asset} time={hit.start} height={36} />
      </div>
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
        <div
          style={{
            fontSize: 10,
            color: Theme.text.muted,
            marginTop: 2,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
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
