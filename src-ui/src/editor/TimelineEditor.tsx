// TimelineEditor — the interactive input controller layered on TimelineCanvas (E3-S10).
//
// Drives the timeline like Premiere: Pointer (V) / Razor (C) tools; click / shift-add /
// ctrl-toggle / marquee selection; drag-to-move (cross-track when compatible); trim
// left/right via 4px edge handles; split (razor click / Ctrl+K at playhead); live sticky
// snap; ruler-click seek + J/K/L / Space / Home/End / Shift+arrow transport; and
// Ctrl+Z / Ctrl+Shift+Z undo/redo on the USER stack. Edits dispatch through
// `EditController` (the command seam) and apply optimistically against the store.
//
// Rendering: it composes the read-only `<TimelineCanvas>` for the timeline itself and
// draws interaction OVERLAYS (snap indicator, marquee, drag ghost, razor preview) on a
// second absolutely-positioned canvas so the E3-S9 `renderer.ts` is untouched.
//
// Strict layering note (FOUNDATION §4): in production the frontend never touches the
// engines directly — it dispatches `EditIntent`s. Today those intents are applied by the
// local `applyEdit`; E7 reroutes the SAME intents through Tauri (see controller.ts).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, JSX, KeyboardEvent, PointerEvent } from "react";
import type { TimelineView } from "./types";
import { TimelineCanvas } from "./TimelineCanvas";
import {
  type TimelineStore,
  createTimelineStore,
  useTimelineStore,
} from "./store";
import { makeFixtureTimeline } from "./fixture";
import { EditController } from "./controller";
import {
  type Rect,
  type TimelineLayout,
  clipRect,
  endFrame,
  frameAt,
  makeLayout,
  xForFrame,
} from "./geometry";
import { Layout, Snap, Theme, rgba } from "./theme";
import {
  type DragState,
  type Modifiers,
  clampFrameDelta,
  clampedTrackDelta,
  expandToLinkGroup,
  hitTestClip,
  marqueeRect,
  marqueeSelect,
  moveProbeOffsets,
  pinnedCompanions,
  subModeForLocalX,
} from "./drag";
import {
  type SnapResult,
  type SnapState,
  collectTargets,
  findSnap,
  makeSnapState,
} from "./snap";

export type ToolMode = "pointer" | "razor";

export interface TimelineEditorProps {
  timeline?: TimelineView;
  store?: TimelineStore;
  controller?: EditController;
  onSelectionChange?: (selectedIds: string[]) => void;
  onSeek?: (frame: number) => void;
  /**
   * Controlled tool mode. When provided, the editor uses this value instead of its
   * internal state (so the Toolbar — E12-S9 — can drive Pointer/Razor and reflect the
   * keyboard `V`/`C` shortcuts). Keyboard shortcuts still fire `onToolChange` so the
   * controlling parent can update; leave undefined for uncontrolled (internal) state.
   */
  tool?: ToolMode;
  onToolChange?: (tool: ToolMode) => void;
  className?: string;
  style?: CSSProperties;
}

const containerStyle: CSSProperties = {
  position: "relative",
  width: "100%",
  height: "100%",
  minHeight: 240,
  outline: "none",
};

const overlayStyle: CSSProperties = {
  position: "absolute",
  top: 0,
  left: 0,
  // `<canvas>` is a REPLACED element: with width/height:auto it keeps its intrinsic
  // 300x150 size and `inset:0` does NOT stretch it (unlike non-replaced elements).
  // That left the interactive overlay covering only the top-left 300x150 px — presses
  // anywhere outside it fell through to the read-only base canvas, whose minimal
  // handler clears selection / acts like a marquee instead of moving the clip. Force
  // an explicit 100% box so the overlay actually covers the whole timeline.
  width: "100%",
  height: "100%",
  pointerEvents: "none",
};

/** Modifiers from a pointer/keyboard event (Cmd→ctrl, Option→alt mapping). */
function modsOf(e: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean; altKey: boolean }): Modifiers {
  return { shift: e.shiftKey, ctrl: e.ctrlKey || e.metaKey, alt: e.altKey };
}

export function TimelineEditor(props: TimelineEditorProps): JSX.Element {
  const { onSelectionChange, onSeek, onToolChange, className, style } = props;

  const internalStore = useMemo<TimelineStore>(
    () => props.store ?? createTimelineStore({ timeline: props.timeline ?? makeFixtureTimeline() }),
    // create once
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );
  const store = props.store ?? internalStore;

  const controller = useMemo<EditController>(
    () => props.controller ?? new EditController(store),
    [props.controller, store],
  );

  useEffect(() => {
    if (props.timeline) store.setTimeline(props.timeline);
  }, [props.timeline, store]);

  const state = useTimelineStore(store, (s) => s);
  const timeline = props.timeline ?? state.timeline ?? makeFixtureTimeline();
  const viewport = state.viewport;

  const [internalTool, setInternalTool] = useState<ToolMode>("pointer");
  // Controlled when `props.tool` is provided (Toolbar drives it); else internal.
  const tool = props.tool ?? internalTool;
  const setTool = useCallback(
    (next: ToolMode) => {
      if (props.tool === undefined) setInternalTool(next);
    },
    [props.tool],
  );
  const rootRef = useRef<HTMLDivElement | null>(null);
  const overlayRef = useRef<HTMLCanvasElement | null>(null);
  const sizeRef = useRef({ w: 0, h: 0 });

  // Live drag state + transient overlay primitives (refs so pointermove doesn't churn React).
  const dragRef = useRef<DragState>({ kind: "idle" });
  const snapStateRef = useRef<SnapState>(makeSnapState());
  const [overlay, setOverlay] = useState<{
    snapX: number | null;
    marquee: Rect | null;
    ghost: { rect: Rect } | null;
    razorX: number | null;
  }>({ snapX: null, marquee: null, ghost: null, razorX: null });

  const layout = useMemo<TimelineLayout>(
    () => makeLayout(viewport.pixelsPerFrame, timeline.tracks.map((t) => t.displayHeight)),
    [viewport.pixelsPerFrame, timeline],
  );

  const emitSelection = useCallback(() => {
    onSelectionChange?.([...store.getState().viewport.selectedClipIds]);
  }, [onSelectionChange, store]);

  // ---------- Overlay drawing ----------
  const drawOverlay = useCallback(() => {
    const canvas = overlayRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const { w, h } = sizeRef.current;
    if (w === 0 || h === 0) return;
    const dpr = window.devicePixelRatio || 1;
    const tw = Math.round(w * dpr);
    const th = Math.round(h * dpr);
    if (canvas.width !== tw) canvas.width = tw;
    if (canvas.height !== th) canvas.height = th;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const scrollX = viewport.scrollX;

    // Drag ghost.
    if (overlay.ghost) {
      const g = overlay.ghost.rect;
      ctx.fillStyle = rgba(255, 255, 255, 0.12);
      ctx.strokeStyle = Theme.selectionStroke;
      ctx.lineWidth = 1;
      ctx.fillRect(g.x - scrollX, g.y, g.w, g.h);
      ctx.strokeRect(g.x - scrollX, g.y, g.w, g.h);
    }

    // Snap indicator (vertical guide).
    if (overlay.snapX !== null) {
      const x = overlay.snapX - scrollX;
      ctx.strokeStyle = rgba(120, 200, 255, 0.9);
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(x + 0.5, layout.rulerHeight);
      ctx.lineTo(x + 0.5, h);
      ctx.stroke();
    }

    // Razor preview.
    if (overlay.razorX !== null) {
      const x = overlay.razorX - scrollX;
      ctx.strokeStyle = rgba(255, 220, 120, 0.95);
      ctx.lineWidth = 1;
      ctx.setLineDash([4, 3]);
      ctx.beginPath();
      ctx.moveTo(x + 0.5, layout.rulerHeight);
      ctx.lineTo(x + 0.5, h);
      ctx.stroke();
      ctx.setLineDash([]);
    }

    // Marquee.
    if (overlay.marquee) {
      const m = overlay.marquee;
      ctx.fillStyle = rgba(120, 200, 255, 0.12);
      ctx.strokeStyle = rgba(120, 200, 255, 0.8);
      ctx.lineWidth = 1;
      ctx.fillRect(m.x - scrollX, m.y, m.w, m.h);
      ctx.strokeRect(m.x - scrollX, m.y, m.w, m.h);
    }
  }, [overlay, viewport.scrollX, layout.rulerHeight]);

  useEffect(() => {
    drawOverlay();
  }, [drawOverlay]);

  useEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const onResize = () => {
      const rect = el.getBoundingClientRect();
      sizeRef.current = { w: rect.width, h: rect.height };
      drawOverlay();
    };
    onResize();
    const ro = new ResizeObserver(onResize);
    ro.observe(el);
    return () => ro.disconnect();
  }, [drawOverlay]);

  // ---------- Pointer math ----------
  const pointAt = useCallback(
    (e: PointerEvent): { px: number; py: number } => {
      const canvas = overlayRef.current;
      if (!canvas) return { px: 0, py: 0 };
      const rect = canvas.getBoundingClientRect();
      return {
        px: e.clientX - rect.left + viewport.scrollX,
        py: e.clientY - rect.top,
      };
    },
    [viewport.scrollX],
  );

  // ---------- mouseDown ----------
  const handlePointerDown = useCallback(
    (e: PointerEvent<HTMLCanvasElement>) => {
      rootRef.current?.focus();
      const { px, py } = pointAt(e);
      const mods = modsOf(e);
      snapStateRef.current = makeSnapState();

      // Ruler band.
      if (py <= Layout.rulerHeight) {
        if (mods.shift) {
          const anchor = frameAt(layout, px);
          dragRef.current = { kind: "timelineRange", anchorFrame: anchor };
          store.setViewport({ rangeSelection: { startFrame: anchor, endFrame: anchor } });
        } else {
          const frame = frameAt(layout, px);
          store.setPlayhead(frame);
          onSeek?.(frame);
          dragRef.current = { kind: "scrubPlayhead" };
        }
        (e.target as Element).setPointerCapture?.(e.pointerId);
        return;
      }

      const hit = hitTestClip(timeline, layout, px, py);

      // Razor tool: split at frame on mousedown over a clip.
      if (tool === "razor") {
        if (hit) {
          const at = frameAt(layout, px);
          controller.dispatch({ kind: "split", clipId: hit.clip.id, atFrame: at });
          setOverlay((o) => ({ ...o, razorX: null }));
        }
        return;
      }

      if (!hit) {
        // Empty space → begin marquee (clears selection unless shift).
        const base = mods.shift ? [...store.getState().viewport.selectedClipIds] : [];
        if (!mods.shift) store.setSelection([]);
        dragRef.current = { kind: "marquee", originX: px, originY: py, baseSelection: base };
        (e.target as Element).setPointerCapture?.(e.pointerId);
        emitSelection();
        return;
      }

      // --- Pointer tool over a clip ---
      const localX = px - hit.rect.x;
      const sub = subModeForLocalX(localX, hit.rect.w);
      const linkedOn = !mods.alt;

      // Selection bookkeeping.
      const selected = store.getState().viewport.selectedClipIds;
      if (mods.shift || mods.ctrl) {
        // toggle membership (expand to link group when linkedOn)
        const ids = linkedOn ? expandToLinkGroup(timeline, [hit.clip.id]) : [hit.clip.id];
        const next = new Set(selected);
        const allSelected = ids.every((id) => next.has(id));
        for (const id of ids) {
          if (allSelected) next.delete(id);
          else next.add(id);
        }
        store.setSelection(next);
      } else if (!selected.has(hit.clip.id)) {
        const ids = linkedOn ? expandToLinkGroup(timeline, [hit.clip.id]) : [hit.clip.id];
        store.setSelection(ids);
      }
      emitSelection();

      // Begin a trim or move drag.
      const movers = [...store.getState().viewport.selectedClipIds]
        .map((id) => {
          for (let ti = 0; ti < timeline.tracks.length; ti++) {
            const c = timeline.tracks[ti].clips.find((cc) => cc.id === id);
            if (c) return { clip: c, trackIndex: ti };
          }
          return null;
        })
        .filter((m): m is { clip: typeof timeline.tracks[0]["clips"][0]; trackIndex: number } => m !== null);

      if (sub === "trimLeft" || sub === "trimRight") {
        dragRef.current = {
          kind: sub,
          clipId: hit.clip.id,
          originalStartFrame: hit.clip.startFrame,
          originalEndFrame: endFrame(hit.clip),
        };
      } else {
        const originalTrackOf: Record<string, number> = {};
        let minFrame = Number.POSITIVE_INFINITY;
        for (const m of movers) {
          originalTrackOf[m.clip.id] = m.trackIndex;
          minFrame = Math.min(minFrame, m.clip.startFrame);
        }
        // Grab offset: how far INTO the clip the press landed. The cursor frame minus
        // this offset gives the lead's candidate start, so the clip follows the
        // pointer by displacement (reference parity) rather than teleporting its start
        // to the cursor.
        const grabFrame = frameAt(layout, px);
        dragRef.current = {
          kind: "moveClip",
          leadId: hit.clip.id,
          clipIds: movers.map((m) => m.clip.id),
          leadOriginalFrame: hit.clip.startFrame,
          grabOffsetFrames: grabFrame - hit.clip.startFrame,
          minOriginalFrame: Number.isFinite(minFrame) ? minFrame : hit.clip.startFrame,
          originStartFrame: hit.clip.startFrame,
          originY: py,
          duplicate: mods.alt,
          originalTrackOf,
        };
      }
      (e.target as Element).setPointerCapture?.(e.pointerId);
    },
    [pointAt, layout, timeline, tool, store, controller, onSeek, emitSelection],
  );

  // ---------- mouseMove ----------
  const handlePointerMove = useCallback(
    (e: PointerEvent<HTMLCanvasElement>) => {
      const { px, py } = pointAt(e);

      // Razor preview (hover) — uses its own snap state.
      if (tool === "razor" && dragRef.current.kind === "idle") {
        const hit = hitTestClip(timeline, layout, px, py);
        if (hit) {
          const frame = frameAt(layout, px);
          setOverlay((o) => ({ ...o, razorX: xForFrame(layout, frame) }));
        } else {
          setOverlay((o) => ({ ...o, razorX: null }));
        }
        return;
      }

      const drag = dragRef.current;
      switch (drag.kind) {
        case "scrubPlayhead": {
          const frame = frameAt(layout, px);
          store.setPlayhead(frame);
          onSeek?.(frame);
          break;
        }
        case "timelineRange": {
          const frame = frameAt(layout, px);
          store.setViewport({
            rangeSelection: { startFrame: drag.anchorFrame, endFrame: frame },
          });
          break;
        }
        case "marquee": {
          const rect = marqueeRect(drag.originX, drag.originY, px, py);
          const ids = marqueeSelect(timeline, layout, rect, drag.baseSelection, true);
          store.setSelection(ids);
          setOverlay((o) => ({ ...o, marquee: rect }));
          emitSelection();
          break;
        }
        case "moveClip": {
          // Cursor frame minus the grab offset = lead's candidate start (so the clip
          // tracks the pointer by displacement, not start-snaps to the cursor).
          const candidateFrame = frameAt(layout, px) - drag.grabOffsetFrames;
          const rawDelta = candidateFrame - drag.leadOriginalFrame;
          // Live snap with two probes per mover.
          const movers = drag.clipIds
            .map((id) => findClipView(timeline, id))
            .filter((c): c is NonNullable<typeof c> => c !== null);
          const probeOffsets = moveProbeOffsets(movers, drag.leadOriginalFrame);
          const targets = collectTargets(
            timeline,
            viewport.playheadFrame,
            new Set(drag.clipIds),
            true,
          );
          const snap: SnapResult | null = findSnap(
            drag.leadOriginalFrame + rawDelta,
            probeOffsets,
            targets,
            snapStateRef.current,
            Snap.thresholdPixels,
            viewport.pixelsPerFrame,
          );
          const snappedDelta = snap
            ? snap.frame - snap.probeOffset - drag.leadOriginalFrame
            : rawDelta;
          const frameDelta = clampFrameDelta(snappedDelta, drag.minOriginalFrame);
          // Drag ghost for the lead.
          const lead = findClipView(timeline, drag.leadId);
          if (lead) {
            const trackIndex = drag.originalTrackOf[drag.leadId] ?? 0;
            const r = clipRect(layout, lead, trackIndex);
            setOverlay((o) => ({
              ...o,
              ghost: { rect: { ...r, x: xForFrame(layout, Math.max(0, lead.startFrame + frameDelta)) } },
              snapX: snap ? snap.x : null,
            }));
          }
          break;
        }
        case "trimLeft":
        case "trimRight": {
          const clip = findClipView(timeline, drag.clipId);
          if (!clip) break;
          const candidate = frameAt(layout, px);
          const targets = collectTargets(timeline, viewport.playheadFrame, new Set([drag.clipId]), true);
          const snap = findSnap(
            candidate,
            [0],
            targets,
            snapStateRef.current,
            Snap.thresholdPixels,
            viewport.pixelsPerFrame,
          );
          const edgeFrame = snap ? snap.frame : candidate;
          setOverlay((o) => ({ ...o, snapX: snap ? snap.x : null, razorX: xForFrame(layout, edgeFrame) }));
          break;
        }
        default:
          break;
      }
    },
    [pointAt, tool, timeline, layout, store, onSeek, viewport.playheadFrame, viewport.pixelsPerFrame, emitSelection],
  );

  // ---------- mouseUp (commit) ----------
  const handlePointerUp = useCallback(
    (e: PointerEvent<HTMLCanvasElement>) => {
      const { px } = pointAt(e);
      const drag = dragRef.current;
      const mods = modsOf(e);

      switch (drag.kind) {
        case "moveClip": {
          const candidateFrame = frameAt(layout, px) - drag.grabOffsetFrames;
          const rawDelta = candidateFrame - drag.leadOriginalFrame;
          const movers = drag.clipIds
            .map((id) => findClipView(timeline, id))
            .filter((c): c is NonNullable<typeof c> => c !== null);
          const probeOffsets = moveProbeOffsets(movers, drag.leadOriginalFrame);
          const targets = collectTargets(timeline, viewport.playheadFrame, new Set(drag.clipIds), true);
          const snap = findSnap(
            drag.leadOriginalFrame + rawDelta,
            probeOffsets,
            targets,
            snapStateRef.current,
            Snap.thresholdPixels,
            viewport.pixelsPerFrame,
          );
          const snappedDelta = snap ? snap.frame - snap.probeOffset - drag.leadOriginalFrame : rawDelta;
          const frameDelta = clampFrameDelta(snappedDelta, drag.minOriginalFrame);

          // Cross-track delta from pointer Y vs lead's original track.
          const py = e.clientY - (overlayRef.current?.getBoundingClientRect().top ?? 0);
          const destTrackTypes = timeline.tracks.map((t) => t.type);
          const moverTypes = movers.map((m) => m.mediaType);
          const moverTracks = movers.map((m) => drag.originalTrackOf[m.id] ?? 0);
          // Which track row is the pointer over?
          let pointerTrack = drag.originalTrackOf[drag.leadId] ?? 0;
          for (let ti = 0; ti < layout.cumulativeY.length; ti++) {
            if (py >= layout.cumulativeY[ti] && py < layout.cumulativeY[ti] + layout.trackHeights[ti]) {
              pointerTrack = ti;
              break;
            }
          }
          const rawTrackDelta = pointerTrack - (drag.originalTrackOf[drag.leadId] ?? 0);
          const trackDelta = clampedTrackDelta(rawTrackDelta, moverTypes, moverTracks, destTrackTypes);

          const lead = findClipView(timeline, drag.leadId);
          const pinned = pinnedCompanions(
            movers,
            drag.leadId,
            lead?.linkGroupId ?? null,
            destTrackTypes[(drag.originalTrackOf[drag.leadId] ?? 0) + trackDelta] ?? "video",
          );
          const trackForClip: Record<string, number> = {};
          for (const m of movers) {
            const src = drag.originalTrackOf[m.id] ?? 0;
            trackForClip[m.id] = pinned.has(m.id) ? src : src + trackDelta;
          }

          if (frameDelta !== 0 || trackDelta !== 0 || drag.duplicate) {
            controller.dispatch({
              kind: "move",
              clipIds: drag.clipIds,
              leadId: drag.leadId,
              frameDelta,
              trackForClip,
              duplicate: drag.duplicate,
            });
          }
          break;
        }
        case "trimLeft":
        case "trimRight": {
          const clip = findClipView(timeline, drag.clipId);
          if (clip) {
            const candidate = frameAt(layout, px);
            const targets = collectTargets(timeline, viewport.playheadFrame, new Set([drag.clipId]), true);
            const snap = findSnap(candidate, [0], targets, snapStateRef.current, Snap.thresholdPixels, viewport.pixelsPerFrame);
            const edgeFrame = snap ? snap.frame : candidate;
            const deltaFrames =
              drag.kind === "trimLeft"
                ? edgeFrame - drag.originalStartFrame
                : edgeFrame - drag.originalEndFrame;
            if (deltaFrames !== 0) {
              controller.dispatch({
                kind: "trim",
                clipId: drag.clipId,
                edge: drag.kind === "trimLeft" ? "left" : "right",
                deltaFrames,
                propagateToLinked: !mods.alt,
              });
            }
          }
          break;
        }
        default:
          break;
      }

      dragRef.current = { kind: "idle" };
      setOverlay({ snapX: null, marquee: null, ghost: null, razorX: null });
      (e.target as Element).releasePointerCapture?.(e.pointerId);
    },
    [pointAt, layout, timeline, controller, viewport.playheadFrame, viewport.pixelsPerFrame],
  );

  // ---------- Keyboard (tools, transport, edit menu, undo) ----------
  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLDivElement>) => {
      const mods = modsOf(e);
      const totalFrames = timelineTotalFrames(timeline);

      // Undo / redo.
      if (mods.ctrl && (e.key === "z" || e.key === "Z")) {
        e.preventDefault();
        if (mods.shift) controller.redo();
        else controller.undo();
        return;
      }
      if (mods.ctrl && (e.key === "y" || e.key === "Y")) {
        e.preventDefault();
        controller.redo();
        return;
      }

      // Edit-menu shortcuts.
      if (mods.ctrl && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        // Split at playhead: find the clip under the playhead on a selected track (or
        // any clip the playhead bisects).
        const at = viewport.playheadFrame;
        const target = clipUnderFrame(timeline, at, store.getState().viewport.selectedClipIds);
        if (target) controller.dispatch({ kind: "split", clipId: target, atFrame: at });
        return;
      }

      // Delete selected.
      if (!mods.ctrl && (e.key === "Delete" || e.key === "Backspace")) {
        e.preventDefault();
        const sel = [...store.getState().viewport.selectedClipIds];
        if (sel.length > 0) {
          controller.dispatch({ kind: "deleteClips", clipIds: sel, ripple: false });
          store.setSelection([]);
          emitSelection();
        } else if (viewport.rangeSelection) {
          const r = viewport.rangeSelection;
          const trackIndex = firstTrackIndexInRange(timeline, r.startFrame, r.endFrame);
          controller.dispatch({
            kind: "rippleDeleteRange",
            trackIndex,
            ranges: [{ start: Math.min(r.startFrame, r.endFrame), end: Math.max(r.startFrame, r.endFrame) }],
          });
          store.setViewport({ rangeSelection: null });
        }
        return;
      }

      // Tool modes.
      if (!mods.ctrl && (e.key === "v" || e.key === "V")) {
        setTool("pointer");
        onToolChange?.("pointer");
        return;
      }
      if (!mods.ctrl && (e.key === "c" || e.key === "C")) {
        setTool("razor");
        onToolChange?.("razor");
        return;
      }

      // Transport: J / K / L scrub (timeline-side seek; preview is Epic 5).
      const step = 1;
      if (e.key === " ") {
        e.preventDefault();
        return; // play/pause toggles preview transport in Epic 5 (no-op here).
      }
      if (e.key === "Home") {
        e.preventDefault();
        store.setPlayhead(0);
        onSeek?.(0);
        return;
      }
      if (e.key === "End") {
        e.preventDefault();
        store.setPlayhead(totalFrames);
        onSeek?.(totalFrames);
        return;
      }
      if (e.key === "l" || e.key === "L") {
        store.setPlayhead(Math.min(totalFrames, viewport.playheadFrame + 10));
        return;
      }
      if (e.key === "j" || e.key === "J") {
        store.setPlayhead(Math.max(0, viewport.playheadFrame - 10));
        return;
      }
      if (e.key === "k" || e.key === "K") {
        return; // K halts shuttle in Epic 5 preview; no-op on the timeline seek.
      }
      if (e.key === "ArrowRight") {
        e.preventDefault();
        const delta = mods.shift ? 10 : step;
        store.setPlayhead(Math.min(totalFrames, viewport.playheadFrame + delta));
        onSeek?.(store.getState().viewport.playheadFrame);
        return;
      }
      if (e.key === "ArrowLeft") {
        e.preventDefault();
        const delta = mods.shift ? 10 : step;
        store.setPlayhead(Math.max(0, viewport.playheadFrame - delta));
        onSeek?.(store.getState().viewport.playheadFrame);
        return;
      }
    },
    [timeline, controller, viewport.playheadFrame, viewport.rangeSelection, store, onSeek, onToolChange, emitSelection],
  );

  const cursor = tool === "razor" ? "crosshair" : "default";

  return (
    <div
      ref={rootRef}
      className={className}
      style={{ ...containerStyle, ...style }}
      tabIndex={0}
      role="application"
      aria-label="Timeline editor"
      onKeyDown={handleKeyDown}
    >
      <TimelineCanvas store={store} />
      <canvas
        ref={overlayRef}
        style={{ ...overlayStyle, cursor, pointerEvents: "auto" }}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        aria-hidden
      />
    </div>
  );
}

// ---------- small pure helpers ----------

function findClipView(timeline: TimelineView, id: string) {
  for (const track of timeline.tracks) {
    const c = track.clips.find((cc) => cc.id === id);
    if (c) return c;
  }
  return null;
}

function timelineTotalFrames(timeline: TimelineView): number {
  let max = 0;
  for (const track of timeline.tracks) {
    for (const clip of track.clips) max = Math.max(max, endFrame(clip));
  }
  return max;
}

/** Clip the playhead bisects — prefer a selected one, else the first overlapped. */
function clipUnderFrame(
  timeline: TimelineView,
  frame: number,
  selected: ReadonlySet<string>,
): string | null {
  let fallback: string | null = null;
  for (const track of timeline.tracks) {
    for (const clip of track.clips) {
      if (clip.startFrame < frame && frame < endFrame(clip)) {
        if (selected.has(clip.id)) return clip.id;
        if (fallback === null) fallback = clip.id;
      }
    }
  }
  return fallback;
}

function firstTrackIndexInRange(timeline: TimelineView, start: number, end: number): number {
  const lo = Math.min(start, end);
  const hi = Math.max(start, end);
  for (let ti = 0; ti < timeline.tracks.length; ti++) {
    for (const clip of timeline.tracks[ti].clips) {
      if (clip.startFrame < hi && endFrame(clip) > lo) return ti;
    }
  }
  return 0;
}

export default TimelineEditor;
