// Editor Toolbar — the strip above the timeline (E12-S9).
//
// Ported 1:1 in behavior from the macOS reference `Toolbar/ToolbarView.swift`. A
// horizontal strip with the reference groups + exact bindings (Cmd→Ctrl):
//
//   [ Undo (Ctrl+Z)  Redo (Ctrl+Shift+Z) ] | [ Pointer (V)  Razor (C) ] |
//   [ Split @ Playhead (Ctrl+K)  Trim Start (Q)  Trim End (W) ] | [ Add Text (T) ]
//   ──── spacer ────  [ −  〔log-mapped zoom slider〕  + ]
//
// Strict layering (FOUNDATION §4): the toolbar never touches engines. It drives the
// SAME seams the timeline gestures use — `EditController.dispatch`/`undo`/`redo` for
// edits + undo/redo, the `TimelineStore` for tool/zoom/playhead viewport state, and
// `editorEdit("add_texts", …)` (bridge.ts) for the Insert action. Tool mode is a
// CONTROLLED value owned here and pushed to `<TimelineEditor tool=… onToolChange=…>`
// so the toolbar and the keyboard V/C shortcuts stay in sync.
//
// Tokens: `toolbarHeight = 38` (Layout), IconSize/Spacing from `design-tokens.md`,
// accent `#F29933` for the active tool + slider tint.

import { useCallback, useMemo } from "react";
import type { CSSProperties, JSX } from "react";

import type { EditController } from "./controller";
import type { TimelineStore } from "./store";
import { useTimelineStore } from "./store";
import type { ToolMode } from "./TimelineEditor";
import type { ClipView, TimelineView } from "./types";
import { endFrame } from "./geometry";
import { editorEdit } from "./bridge";
import { Theme } from "./theme";

// ── Layout / token constants (design-tokens.md §Layout / §Scale) ─────────────
const TOOLBAR_HEIGHT = 38; // Layout.toolbarHeight
const ICON_BUTTON = 24; // reference toolbar button frame (24×24)
const ICON_FONT = 13; // FontSize.md
const ACCENT = "#F29933"; // active tool + zoom-slider tint

// Zoom scale range (reference `Utilities/Constants.swift` enum Zoom). zoomScale is
// the same unit as `pixelsPerFrame`, so the slider drives `store.setZoom`.
const ZOOM_MIN = 0.05; // Zoom.min — slider floor (min_zoom_scale)
const ZOOM_MAX = 40.0; // Zoom.max — slider ceiling (max_zoom_scale)

export interface ToolbarProps {
  store: TimelineStore;
  controller: EditController;
  /** Controlled tool mode (owned by the parent, shared with TimelineEditor). */
  tool: ToolMode;
  onToolChange: (tool: ToolMode) => void;
  className?: string;
  style?: CSSProperties;
}

const stripStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 10, // Spacing.md
  height: TOOLBAR_HEIGHT,
  minHeight: TOOLBAR_HEIGHT,
  padding: "0 10px", // .padding(.horizontal, Spacing.md)
  background: Theme.background.surface,
  borderBottom: `1px solid ${Theme.border.subtle}`,
  boxSizing: "border-box",
  userSelect: "none",
};

const groupStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 10, // Spacing.md
};

const dividerStyle: CSSProperties = {
  width: 1,
  height: 20, // Spacing.xl
  background: Theme.border.subtle,
};

function buttonStyle(opts: {
  active?: boolean;
  disabled?: boolean;
}): CSSProperties {
  return {
    display: "inline-flex",
    alignItems: "center",
    justifyContent: "center",
    width: ICON_BUTTON,
    height: ICON_BUTTON,
    padding: 0,
    border: "none",
    borderRadius: 4, // Radius.xsSm
    background: opts.active ? "rgba(242, 153, 51, 0.18)" : "transparent",
    color: opts.disabled
      ? Theme.text.muted
      : opts.active
        ? ACCENT
        : Theme.text.secondary,
    cursor: opts.disabled ? "default" : "pointer",
    opacity: opts.disabled ? 0.5 : 1,
    fontSize: ICON_FONT,
    lineHeight: 1,
  };
}

export function Toolbar(props: ToolbarProps): JSX.Element {
  const { store, controller, tool, onToolChange, className, style } = props;

  // Reactive viewport slices (selection / playhead / zoom) drive enable-state + zoom.
  const playheadFrame = useTimelineStore(store, (s) => s.viewport.playheadFrame);
  const selectedClipIds = useTimelineStore(
    store,
    (s) => s.viewport.selectedClipIds,
  );
  // `timeline` is re-read after every edit (the store re-emits on setTimeline), so
  // canUndo/canRedo re-derive whenever an edit lands — keeping the buttons live.
  const timeline = useTimelineStore(store, (s) => s.timeline);

  const canUndo = controller.canUndo("user");
  const canRedo = controller.canRedo("user");
  const hasSelection = selectedClipIds.size > 0;

  // ── Undo / Redo ────────────────────────────────────────────────────────────
  const undo = useCallback(() => {
    controller.undo("user");
  }, [controller]);
  const redo = useCallback(() => {
    controller.redo("user");
  }, [controller]);

  // ── Tool mode ────────────────────────────────────────────────────────────
  const selectPointer = useCallback(() => onToolChange("pointer"), [onToolChange]);
  const selectRazor = useCallback(() => onToolChange("razor"), [onToolChange]);

  // ── Split at playhead (Ctrl+K) — split each selected clip the playhead bisects.
  const splitAtPlayhead = useCallback(() => {
    if (!timeline) return;
    const at = playheadFrame;
    for (const id of selectedClipIds) {
      const clip = findClip(timeline, id);
      if (!clip) continue;
      if (at > clip.startFrame && at < endFrame(clip)) {
        controller.dispatch({ kind: "split", clipId: id, atFrame: at });
      }
    }
  }, [timeline, playheadFrame, selectedClipIds, controller]);

  // ── Trim start / end to playhead (Q / W) ─────────────────────────────────
  // Reference parity: only the portion between the clip edge and the playhead is
  // trimmed away. Mapped onto the `trim` intent (edge + frame delta on that edge).
  const trimStartToPlayhead = useCallback(() => {
    if (!timeline) return;
    const at = playheadFrame;
    for (const id of selectedClipIds) {
      const clip = findClip(timeline, id);
      if (!clip) continue;
      if (at > clip.startFrame && at < endFrame(clip)) {
        // Move the left edge right to the playhead (positive delta shrinks the start).
        controller.dispatch({
          kind: "trim",
          clipId: id,
          edge: "left",
          deltaFrames: at - clip.startFrame,
          propagateToLinked: true,
        });
      }
    }
  }, [timeline, playheadFrame, selectedClipIds, controller]);

  const trimEndToPlayhead = useCallback(() => {
    if (!timeline) return;
    const at = playheadFrame;
    for (const id of selectedClipIds) {
      const clip = findClip(timeline, id);
      if (!clip) continue;
      if (at > clip.startFrame && at < endFrame(clip)) {
        // Move the right edge left to the playhead (negative delta shrinks the end).
        controller.dispatch({
          kind: "trim",
          clipId: id,
          edge: "right",
          deltaFrames: at - endFrame(clip),
          propagateToLinked: true,
        });
      }
    }
  }, [timeline, playheadFrame, selectedClipIds, controller]);

  // ── Add Text (T) — insert a 1-second text overlay at the playhead. ─────────
  const addText = useCallback(() => {
    const fps = timeline?.fps ?? 30;
    const durationFrames = Math.max(1, Math.round(fps)); // ~1s default
    void editorEdit("add_texts", {
      entries: [
        {
          content: "Text",
          startFrame: playheadFrame,
          durationFrames,
        },
      ],
    });
    // The backend emits `timeline://changed`; the Project surface refetches.
  }, [timeline?.fps, playheadFrame]);

  // ── Zoom — log-mapped slider (uniform travel per zoom factor). ─────────────
  const pixelsPerFrame = useTimelineStore(
    store,
    (s) => s.viewport.pixelsPerFrame,
  );
  const logMin = useMemo(() => Math.log(ZOOM_MIN), []);
  const logMax = useMemo(() => Math.log(ZOOM_MAX), []);
  const sliderValue = clamp(Math.log(pixelsPerFrame), logMin, logMax);

  const setZoomFromLog = useCallback(
    (logValue: number) => {
      const scale = Math.exp(clamp(logValue, logMin, logMax));
      store.setZoom(clamp(scale, ZOOM_MIN, ZOOM_MAX));
    },
    [store, logMin, logMax],
  );

  const onSliderChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      setZoomFromLog(Number(e.target.value));
    },
    [setZoomFromLog],
  );

  const zoomOut = useCallback(() => {
    // One slider "step" out (÷ a zoom factor); log-mapped so it feels uniform.
    setZoomFromLog(Math.log(pixelsPerFrame) - ZOOM_STEP_LOG);
  }, [pixelsPerFrame, setZoomFromLog]);
  const zoomIn = useCallback(() => {
    setZoomFromLog(Math.log(pixelsPerFrame) + ZOOM_STEP_LOG);
  }, [pixelsPerFrame, setZoomFromLog]);

  return (
    <div
      className={className}
      style={{ ...stripStyle, ...style }}
      role="toolbar"
      aria-label="Editor toolbar"
    >
      {/* Undo / Redo */}
      <div style={groupStyle}>
        <button
          type="button"
          style={buttonStyle({ disabled: !canUndo })}
          disabled={!canUndo}
          onClick={undo}
          title="Undo (Ctrl+Z)"
          aria-label="Undo"
        >
          <UndoGlyph />
        </button>
        <button
          type="button"
          style={buttonStyle({ disabled: !canRedo })}
          disabled={!canRedo}
          onClick={redo}
          title="Redo (Ctrl+Shift+Z)"
          aria-label="Redo"
        >
          <RedoGlyph />
        </button>
      </div>

      <div style={dividerStyle} aria-hidden />

      {/* Tool mode */}
      <div style={groupStyle}>
        <button
          type="button"
          style={buttonStyle({ active: tool === "pointer" })}
          aria-pressed={tool === "pointer"}
          onClick={selectPointer}
          title="Pointer (V)"
          aria-label="Pointer tool"
        >
          <PointerGlyph />
        </button>
        <button
          type="button"
          style={buttonStyle({ active: tool === "razor" })}
          aria-pressed={tool === "razor"}
          onClick={selectRazor}
          title="Razor (C)"
          aria-label="Razor tool"
        >
          <RazorGlyph />
        </button>
      </div>

      <div style={dividerStyle} aria-hidden />

      {/* Clip edit — split / trim start / trim end */}
      <div style={groupStyle}>
        <button
          type="button"
          style={buttonStyle({ disabled: !hasSelection })}
          disabled={!hasSelection}
          onClick={splitAtPlayhead}
          title="Split at Playhead (Ctrl+K)"
          aria-label="Split at playhead"
        >
          <SplitGlyph />
        </button>
        <button
          type="button"
          style={{ ...buttonStyle({ disabled: !hasSelection }), fontWeight: 600, fontFamily: "monospace" }}
          disabled={!hasSelection}
          onClick={trimStartToPlayhead}
          title="Trim Start to Playhead (Q)"
          aria-label="Trim start to playhead"
        >
          [
        </button>
        <button
          type="button"
          style={{ ...buttonStyle({ disabled: !hasSelection }), fontWeight: 600, fontFamily: "monospace" }}
          disabled={!hasSelection}
          onClick={trimEndToPlayhead}
          title="Trim End to Playhead (W)"
          aria-label="Trim end to playhead"
        >
          ]
        </button>
      </div>

      <div style={dividerStyle} aria-hidden />

      {/* Insert — add text */}
      <div style={groupStyle}>
        <button
          type="button"
          style={{ ...buttonStyle({}), fontFamily: "Georgia, serif", fontWeight: 700, fontSize: 16 }}
          onClick={addText}
          title="Add Text (T)"
          aria-label="Add text"
        >
          T
        </button>
      </div>

      {/* Spacer pushes the zoom group to the right edge. */}
      <div style={{ flex: 1 }} aria-hidden />

      {/* Zoom — log-mapped slider between min/max zoom scale. */}
      <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
        <button
          type="button"
          style={buttonStyle({})}
          onClick={zoomOut}
          title="Zoom out"
          aria-label="Zoom out"
        >
          <MinusGlyph />
        </button>
        <input
          type="range"
          min={logMin}
          max={logMax}
          step={(logMax - logMin) / 200}
          value={sliderValue}
          onChange={onSliderChange}
          aria-label="Zoom"
          title="Zoom"
          style={{ width: 100, accentColor: ACCENT, cursor: "pointer" }}
        />
        <button
          type="button"
          style={buttonStyle({})}
          onClick={zoomIn}
          title="Zoom in"
          aria-label="Zoom in"
        >
          <PlusGlyph />
        </button>
      </div>
    </div>
  );
}

export default Toolbar;

// One "step" of the +/- zoom buttons in log space (≈ ×1.5 per click, like the
// reference magnify sensitivity). Travel is uniform because the axis is log-mapped.
const ZOOM_STEP_LOG = Math.log(1.5);

// ── small pure helpers ───────────────────────────────────────────────────────

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

function findClip(timeline: TimelineView, id: string): ClipView | null {
  for (const track of timeline.tracks) {
    const c = track.clips.find((cc) => cc.id === id);
    if (c) return c;
  }
  return null;
}

// ── Inline glyphs (SVG, currentColor) — no icon-font dependency. ─────────────
const SVG: CSSProperties = { display: "block" };

function UndoGlyph(): JSX.Element {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={SVG} aria-hidden>
      <path d="M9 14L4 9l5-5" />
      <path d="M4 9h11a5 5 0 0 1 0 10h-1" />
    </svg>
  );
}

function RedoGlyph(): JSX.Element {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={SVG} aria-hidden>
      <path d="M15 14l5-5-5-5" />
      <path d="M20 9H9a5 5 0 0 0 0 10h1" />
    </svg>
  );
}

function PointerGlyph(): JSX.Element {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" strokeWidth="1" strokeLinejoin="round" style={SVG} aria-hidden>
      <path d="M5 3l14 9-6 1 3.5 6-2.5 1.3-3.5-6L7 18z" />
    </svg>
  );
}

function RazorGlyph(): JSX.Element {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={SVG} aria-hidden>
      <circle cx="6" cy="6" r="3" />
      <circle cx="6" cy="18" r="3" />
      <line x1="20" y1="4" x2="8.12" y2="15.88" />
      <line x1="14.47" y1="14.48" x2="20" y2="20" />
      <line x1="8.12" y1="8.12" x2="12" y2="12" />
    </svg>
  );
}

function SplitGlyph(): JSX.Element {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={SVG} aria-hidden>
      <rect x="3" y="6" width="7" height="12" rx="1" />
      <rect x="14" y="6" width="7" height="12" rx="1" />
      <line x1="12" y1="3" x2="12" y2="21" strokeDasharray="2 2" />
    </svg>
  );
}

function MinusGlyph(): JSX.Element {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={SVG} aria-hidden>
      <circle cx="11" cy="11" r="7" />
      <line x1="16" y1="16" x2="21" y2="21" />
      <line x1="8" y1="11" x2="14" y2="11" />
    </svg>
  );
}

function PlusGlyph(): JSX.Element {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" style={SVG} aria-hidden>
      <circle cx="11" cy="11" r="7" />
      <line x1="16" y1="16" x2="21" y2="21" />
      <line x1="11" y1="8" x2="11" y2="14" />
      <line x1="8" y1="11" x2="14" y2="11" />
    </svg>
  );
}
