// TimelineCanvas — the React component the app shell mounts (E3-S9).
//
// Renders the 2D timeline (tracks/clips/ruler/playhead/rubber bands/fades) onto a
// `<canvas>` via the immediate-mode `renderTimeline` draw loop. It is DPR-aware,
// resizes with its container, and redraws whenever the timeline or viewport state
// changes.
//
// Data source: a `TimelineView` is supplied EITHER via the `timeline` prop OR via a
// `store` (the external store in `store.ts`). When neither is given, it falls back to
// the built-in fixture so it renders standalone (useful for the smoke test / preview).
// The real `get_timeline` Tauri command (Epic 7) will populate the store; nothing in
// this component assumes that command exists.
//
// Interaction here is intentionally minimal — the full input controller (tool modes,
// drag/trim/split, marquee, snap) is E3-S10. This component provides only:
//   • click-to-select / shift-click additive select (selection persists by clip ID),
//   • click on the ruler to seek the playhead.
// Those exist so the canvas demonstrably "re-renders selection by clip ID across
// state changes" (the E3-S9 acceptance test) without pulling in E3-S10 scope.

import { useCallback, useEffect, useMemo, useRef } from "react";
import type { CSSProperties, JSX, PointerEvent } from "react";
import type { TimelineView } from "./types";
import { renderTimeline } from "./renderer";
import { clipRect, frameAt, makeLayout } from "./geometry";
import { Layout } from "./theme";
import {
  type TimelineStore,
  createTimelineStore,
  useTimelineStore,
} from "./store";
import { makeFixtureTimeline } from "./fixture";

export interface TimelineCanvasProps {
  /** Explicit timeline data. Overrides the store's timeline when provided. */
  timeline?: TimelineView;
  /**
   * External store driving viewport + (optionally) timeline state. When omitted, an
   * internal store is created and seeded from `timeline` / the fixture.
   */
  store?: TimelineStore;
  /** Fired when the selection changes (clip IDs). For the app shell / inspector. */
  onSelectionChange?: (selectedIds: string[]) => void;
  /** Fired when the playhead is moved by a ruler click (timeline frame). */
  onSeek?: (frame: number) => void;
  className?: string;
  style?: CSSProperties;
}

const containerStyle: CSSProperties = {
  position: "relative",
  width: "100%",
  height: "100%",
  minHeight: 240,
  overflow: "hidden",
  background: "#0a0a0a",
};

const canvasStyle: CSSProperties = {
  display: "block",
  width: "100%",
  height: "100%",
  cursor: "default",
};

export function TimelineCanvas(props: TimelineCanvasProps): JSX.Element {
  const { timeline, onSelectionChange, onSeek, className, style } = props;

  // Use the provided store, or create a stable internal one seeded with data.
  const internalStore = useMemo<TimelineStore>(
    () =>
      props.store ??
      createTimelineStore({ timeline: timeline ?? makeFixtureTimeline() }),
    // Intentionally create once; prop-timeline changes are synced in an effect below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );
  const store = props.store ?? internalStore;

  // Keep the store's timeline in sync when the `timeline` prop changes.
  useEffect(() => {
    if (timeline) store.setTimeline(timeline);
  }, [timeline, store]);

  const state = useTimelineStore(store, (s) => s);
  const activeTimeline = timeline ?? state.timeline ?? makeFixtureTimeline();
  const viewport = state.viewport;

  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sizeRef = useRef({ w: 0, h: 0 });

  // --- Draw ---
  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const { w, h } = sizeRef.current;
    if (w === 0 || h === 0) return;
    const dpr = window.devicePixelRatio || 1;

    // Resize the backing store to device pixels; draw in CSS px.
    const targetW = Math.round(w * dpr);
    const targetH = Math.round(h * dpr);
    if (canvas.width !== targetW) canvas.width = targetW;
    if (canvas.height !== targetH) canvas.height = targetH;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);
    renderTimeline(ctx, {
      timeline: activeTimeline,
      viewport,
      width: w,
      height: h,
    });
  }, [activeTimeline, viewport]);

  // Redraw on state changes.
  useEffect(() => {
    draw();
  }, [draw]);

  // --- Resize handling ---
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const onResize = () => {
      const rect = el.getBoundingClientRect();
      sizeRef.current = { w: rect.width, h: rect.height };
      draw();
    };
    onResize();
    const ro = new ResizeObserver(onResize);
    ro.observe(el);
    return () => ro.disconnect();
  }, [draw]);

  // --- Hit testing (minimal: click select + ruler seek) ---
  const handlePointerDown = useCallback(
    (e: PointerEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      if (!canvas) return;
      const rect = canvas.getBoundingClientRect();
      const px = e.clientX - rect.left + viewport.scrollX;
      const py = e.clientY - rect.top;

      const layout = makeLayout(
        viewport.pixelsPerFrame,
        activeTimeline.tracks.map((t) => t.displayHeight),
      );

      // Ruler click → seek.
      if (py <= Layout.rulerHeight) {
        const frame = frameAt(layout, px);
        store.setPlayhead(frame);
        onSeek?.(frame);
        return;
      }

      // Clip hit test (front-most: iterate tracks/clips, last match wins on overlap).
      let hitId: string | null = null;
      for (let ti = 0; ti < activeTimeline.tracks.length; ti++) {
        for (const clip of activeTimeline.tracks[ti].clips) {
          const r = clipRect(layout, clip, ti);
          if (px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h) {
            hitId = clip.id;
          }
        }
      }

      const additive = e.shiftKey || e.ctrlKey || e.metaKey;
      if (hitId) {
        store.toggleSelection(hitId, additive);
      } else if (!additive) {
        store.setSelection([]);
      }
      onSelectionChange?.([...store.getState().viewport.selectedClipIds]);
    },
    [activeTimeline, viewport.scrollX, viewport.pixelsPerFrame, store, onSeek, onSelectionChange],
  );

  return (
    <div ref={containerRef} className={className} style={{ ...containerStyle, ...style }}>
      <canvas
        ref={canvasRef}
        style={canvasStyle}
        onPointerDown={handlePointerDown}
        role="img"
        aria-label="Timeline"
      />
    </div>
  );
}

export default TimelineCanvas;
