// Minimal external store for timeline view state (E3-S9).
//
// The spec allows Zustand "if needed". To avoid adding a dependency that would
// touch the shared pnpm lockfile (owned by the concurrent app-shell worker), this
// is a tiny self-contained store with the same shape a Zustand store would expose:
// `getState` / `setState` / `subscribe`, plus a React `useTimelineStore` hook built
// on `useSyncExternalStore`. If Zustand is later added project-wide, this can be
// swapped for `create(...)` with no change to consumers.
//
// The store holds a `TimelineView` (fed from a fixture today; from the `get_timeline`
// Tauri command once Epic 7 lands) and the `TimelineViewport` (zoom/scroll/playhead/
// selection). Selection is a Set of clip IDs so it persists across re-renders by ID
// (FR-9).

import { useSyncExternalStore } from "react";
import type { TimelineView, TimelineViewport } from "./types";
import { Defaults } from "./theme";

export interface TimelineState {
  timeline: TimelineView | null;
  viewport: TimelineViewport;
}

export interface TimelineStore {
  getState: () => TimelineState;
  setState: (partial: Partial<TimelineState>) => void;
  setViewport: (partial: Partial<TimelineViewport>) => void;
  setTimeline: (timeline: TimelineView) => void;
  setSelection: (ids: Iterable<string>) => void;
  toggleSelection: (id: string, additive: boolean) => void;
  setPlayhead: (frame: number) => void;
  setZoom: (pixelsPerFrame: number) => void;
  subscribe: (listener: () => void) => () => void;
}

const initialViewport: TimelineViewport = {
  scrollX: 0,
  pixelsPerFrame: Defaults.pixelsPerFrame,
  playheadFrame: 0,
  selectedClipIds: new Set<string>(),
  rangeSelection: null,
};

export function createTimelineStore(
  initial?: Partial<TimelineState>,
): TimelineStore {
  let state: TimelineState = {
    timeline: initial?.timeline ?? null,
    viewport: { ...initialViewport, ...initial?.viewport },
  };
  const listeners = new Set<() => void>();
  const emit = () => listeners.forEach((l) => l());

  const setState = (partial: Partial<TimelineState>) => {
    state = { ...state, ...partial };
    emit();
  };

  return {
    getState: () => state,
    setState,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    setViewport: (partial) =>
      setState({ viewport: { ...state.viewport, ...partial } }),
    setTimeline: (timeline) => setState({ timeline }),
    setSelection: (ids) =>
      setState({
        viewport: { ...state.viewport, selectedClipIds: new Set(ids) },
      }),
    toggleSelection: (id, additive) => {
      const next = additive
        ? new Set(state.viewport.selectedClipIds)
        : new Set<string>();
      if (state.viewport.selectedClipIds.has(id) && additive) {
        next.delete(id);
      } else {
        next.add(id);
      }
      setState({ viewport: { ...state.viewport, selectedClipIds: next } });
    },
    setPlayhead: (frame) =>
      setState({ viewport: { ...state.viewport, playheadFrame: frame } }),
    setZoom: (pixelsPerFrame) =>
      setState({ viewport: { ...state.viewport, pixelsPerFrame } }),
  };
}

/** Subscribe a React component to a slice of the store. */
export function useTimelineStore<T>(
  store: TimelineStore,
  selector: (s: TimelineState) => T,
): T {
  return useSyncExternalStore(
    store.subscribe,
    () => selector(store.getState()),
    () => selector(store.getState()),
  );
}
