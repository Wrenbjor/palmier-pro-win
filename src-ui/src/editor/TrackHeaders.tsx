// TrackHeaders — the per-track control column in the reserved left gutter (E12).
//
// The timeline canvas (TimelineCanvas/renderer) draws tracks/clips/ruler; this is a
// DOM OVERLAY positioned over the left `Layout.trackHeaderWidth` (100px) gutter,
// rendering one control row per track aligned to that track's Y band (shared with the
// canvas via `trackHeaderBand`). DOM (not canvas) because these are buttons — the same
// reasoning the playhead/overlays use a separate absolutely-positioned layer.
//
// Each row shows the track label + type and toggles for:
//   • mute  (audio/video tracks — silences the track in preview + export),
//   • hide  (visual tracks — video/image/text/lottie — omits it from the composite),
//   • lock  (all tracks — sync-lock; the model field `syncLocked`).
//
// A toggle dispatches `setTrackProperties(trackId, patch)` (UI-only command, like
// Relink/folder-move — there is no `set_track_properties` MCP tool) with an OPTIMISTIC
// local store update; the backend emits `timeline://changed` and the Project surface
// refetches the authoritative state, reconciling the optimistic flip. Outside Tauri the
// dispatch is a no-op but the optimistic update still reflects the toggle for design work.

import type { CSSProperties, JSX } from "react";
import { useCallback } from "react";
import type { ClipType, TimelineView, TrackView } from "./types";
import { type TimelineStore, useTimelineStore } from "./store";
import { type TimelineLayout, trackHeaderBand } from "./geometry";
import { Layout, Theme } from "./theme";
import { setTrackProperties, type TrackPropertiesPatch } from "./bridge";

/** Visual tracks carry a composited layer (so a hide toggle is meaningful). */
export function isVisualType(t: ClipType): boolean {
  return t === "video" || t === "image" || t === "text" || t === "lottie";
}

/** Audio-bearing tracks (mute is meaningful: video tracks carry linked audio too). */
export function isAudioBearingType(t: ClipType): boolean {
  return t === "video" || t === "audio";
}

/** Which header toggle the user clicked. */
export type TrackToggle = "mute" | "hide" | "lock";

/**
 * The single-field patch a header toggle dispatches — the CURRENT flag is flipped, the
 * others left absent so only the clicked field changes (`setTrackProperties` maps an
 * absent field to backend `None`). Pure + exported so the parity checks assert exactly
 * what the buttons send.
 */
export function toggleTrackPatch(
  track: Pick<TrackView, "muted" | "hidden" | "syncLocked">,
  toggle: TrackToggle,
): TrackPropertiesPatch {
  switch (toggle) {
    case "mute":
      return { muted: !track.muted };
    case "hide":
      return { hidden: !track.hidden };
    case "lock":
      return { locked: !track.syncLocked };
  }
}

/** Short human label per track type (matches the timeline's visual language). */
function trackTypeLabel(t: ClipType): string {
  switch (t) {
    case "video":
      return "Video";
    case "audio":
      return "Audio";
    case "image":
      return "Image";
    case "text":
      return "Text";
    case "lottie":
      return "Lottie";
  }
}

export interface TrackHeadersProps {
  /** Drives the rendered tracks + receives optimistic toggles. */
  store: TimelineStore;
  /** Layout shared with the canvas (per-track Y bands). */
  layout: TimelineLayout;
  /** Vertical scroll applied to the timeline content (headers track it 1:1). */
  scrollY?: number;
}

/**
 * The fixed-width header column. Absolutely positioned at the timeline's left edge,
 * below the ruler. Non-interactive except for the toggle buttons (so canvas drag/seek
 * to the right is unaffected; the column itself sits over the leftmost content).
 */
export function TrackHeaders(props: TrackHeadersProps): JSX.Element {
  const { store, layout, scrollY = 0 } = props;
  const timeline = useTimelineStore(store, (s) => s.timeline) as
    | TimelineView
    | undefined;

  const applyPatch = useCallback(
    (track: TrackView, patch: TrackPropertiesPatch) => {
      // Optimistic local update so the toggle reflects immediately (and so it works
      // outside Tauri for design). The `timeline://changed` refetch reconciles.
      const current = store.getState().timeline;
      if (current) {
        const next: TimelineView = {
          ...current,
          tracks: current.tracks.map((t) =>
            t.id === track.id
              ? {
                  ...t,
                  muted: patch.muted ?? t.muted,
                  hidden: patch.hidden ?? t.hidden,
                  syncLocked: patch.locked ?? t.syncLocked,
                }
              : t,
          ),
        };
        store.setTimeline(next);
      }
      void setTrackProperties(track.id, patch);
    },
    [store],
  );

  if (!timeline) return <div aria-hidden style={hiddenStyle} />;

  return (
    <div style={columnStyle} aria-label="Track headers">
      {timeline.tracks.map((track, i) => {
        const band = trackHeaderBand(layout, i);
        const rowStyle: CSSProperties = {
          position: "absolute",
          left: 0,
          top: band.y - scrollY,
          width: Layout.trackHeaderWidth,
          height: band.h,
          boxSizing: "border-box",
          display: "flex",
          flexDirection: "column",
          justifyContent: "center",
          gap: 3,
          padding: "0 6px",
          background: Theme.background.prominent,
          borderRight: `1px solid ${Theme.border.primary}`,
          borderBottom: `1px solid ${Theme.border.subtle}`,
          opacity: track.hidden ? 0.55 : 1,
        };
        const showMute = isAudioBearingType(track.type);
        const showHide = isVisualType(track.type);
        return (
          <div key={track.id} style={rowStyle}>
            <div style={labelStyle} title={trackTypeLabel(track.type)}>
              {trackTypeLabel(track.type)}
            </div>
            <div style={buttonRowStyle}>
              {showMute && (
                <HeaderToggle
                  label={track.muted ? "Unmute track" : "Mute track"}
                  active={track.muted}
                  onClick={() => applyPatch(track, toggleTrackPatch(track, "mute"))}
                  glyph={track.muted ? "M̸" : "M"}
                  data-action="mute"
                />
              )}
              {showHide && (
                <HeaderToggle
                  label={track.hidden ? "Show track" : "Hide track"}
                  active={track.hidden}
                  onClick={() => applyPatch(track, toggleTrackPatch(track, "hide"))}
                  glyph={track.hidden ? "\u{1F441}̸" : "\u{1F441}"}
                  data-action="hide"
                />
              )}
              <HeaderToggle
                label={track.syncLocked ? "Unlock track" : "Lock track"}
                active={track.syncLocked}
                onClick={() => applyPatch(track, toggleTrackPatch(track, "lock"))}
                glyph={track.syncLocked ? "\u{1F512}" : "\u{1F513}"}
                data-action="lock"
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}

interface HeaderToggleProps {
  label: string;
  active: boolean;
  glyph: string;
  onClick: () => void;
  "data-action": string;
}

function HeaderToggle(props: HeaderToggleProps): JSX.Element {
  const { label, active, glyph, onClick } = props;
  const style: CSSProperties = {
    appearance: "none",
    border: `1px solid ${active ? Theme.border.primary : Theme.border.subtle}`,
    borderRadius: 3,
    background: active ? Theme.background.base : "transparent",
    color: active ? Theme.text.primary : Theme.text.muted,
    font: "inherit",
    fontSize: 11,
    lineHeight: 1,
    padding: "2px 5px",
    cursor: "pointer",
  };
  return (
    <button
      type="button"
      style={style}
      onClick={onClick}
      aria-pressed={active}
      aria-label={label}
      title={label}
      data-action={props["data-action"]}
    >
      {glyph}
    </button>
  );
}

const columnStyle: CSSProperties = {
  // Spans the full timeline container so a row's `top: band.y` (canvas track Y,
  // measured from the container top) aligns 1:1 with the canvas lane. The ruler band
  // (top rulerHeight px) is left clear — the first track's band.y is already below it.
  position: "absolute",
  left: 0,
  top: 0,
  width: Layout.trackHeaderWidth,
  bottom: 0,
  overflow: "hidden",
  pointerEvents: "none",
  zIndex: 2,
};

const hiddenStyle: CSSProperties = { display: "none" };

const labelStyle: CSSProperties = {
  color: Theme.text.secondary,
  fontSize: 11,
  fontWeight: 600,
  whiteSpace: "nowrap",
  overflow: "hidden",
  textOverflow: "ellipsis",
};

const buttonRowStyle: CSSProperties = {
  display: "flex",
  gap: 4,
  pointerEvents: "auto",
};

export default TrackHeaders;
