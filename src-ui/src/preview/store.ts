// Preview viewport state store (E5-S10).
//
// Same tiny self-contained store shape as the editor's (`getState`/`setState`/
// `subscribe` + a `useSyncExternalStore` hook), to avoid touching the shared pnpm
// lockfile. Holds the preview-panel view state: the open tabs + active tab, the
// per-tab playhead + duration, playing/scrubbing flags, the canvas zoom/offset, the
// crop-editing toggle, and the selected clip's id (the overlays read the selected
// clip's transform/crop from the timeline view-model the panel is given).

import { useSyncExternalStore } from "react";

import type { PreviewTab } from "./types";
import { tabId } from "./types.ts";

export interface PreviewState {
  /** Open tabs (the timeline tab is always index 0 and non-closable). */
  tabs: PreviewTab[];
  /** The active tab id. */
  activeTabId: string;
  /** Per-tab playhead frame, keyed by tab id (timeline = current_frame). */
  playheads: Record<string, number>;
  /** Per-tab duration in frames (for the scrub bar + timecode). */
  durations: Record<string, number>;
  /** Whether playback is running. */
  playing: boolean;
  /** Whether the user is mid-scrub (suppresses tick advance, reference isScrubbing). */
  scrubbing: boolean;
  /** Canvas zoom (1.0 = Fit). */
  canvasZoom: number;
  /** Canvas pan offset in px. */
  canvasOffset: { width: number; height: number };
  /** Whether the crop overlay is active (else the transform overlay). */
  cropEditing: boolean;
  /** Selected clip id (drives which clip the overlays manipulate). */
  selectedClipId: string | null;
}

export interface PreviewStore {
  getState: () => PreviewState;
  setState: (partial: Partial<PreviewState>) => void;
  subscribe: (listener: () => void) => () => void;

  // tab ops
  openTab: (tab: PreviewTab) => void;
  closeTab: (id: string) => void;
  closeAllTabs: () => void;
  selectTab: (id: string) => void;

  // playback ops
  setPlayhead: (id: string, frame: number) => void;
  setActivePlayhead: (frame: number) => void;
  setDuration: (id: string, frames: number) => void;
  setPlaying: (playing: boolean) => void;
  setScrubbing: (scrubbing: boolean) => void;

  // viewport ops
  setZoom: (zoom: number) => void;
  setOffset: (offset: { width: number; height: number }) => void;
  resetView: () => void;

  // overlay ops
  setCropEditing: (cropEditing: boolean) => void;
  setSelectedClip: (clipId: string | null) => void;
}

const TIMELINE_TAB: PreviewTab = { kind: "timeline" };
const TIMELINE_ID = tabId(TIMELINE_TAB);

function initialState(initial?: Partial<PreviewState>): PreviewState {
  return {
    tabs: [TIMELINE_TAB],
    activeTabId: TIMELINE_ID,
    playheads: { [TIMELINE_ID]: 0 },
    durations: { [TIMELINE_ID]: 0 },
    playing: false,
    scrubbing: false,
    canvasZoom: 1.0,
    canvasOffset: { width: 0, height: 0 },
    cropEditing: false,
    selectedClipId: null,
    ...initial,
  };
}

export function createPreviewStore(initial?: Partial<PreviewState>): PreviewStore {
  let state = initialState(initial);
  const listeners = new Set<() => void>();
  const emit = () => listeners.forEach((l) => l());
  const setState = (partial: Partial<PreviewState>) => {
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

    openTab: (tab) => {
      const id = tabId(tab);
      if (state.tabs.some((t) => tabId(t) === id)) {
        // already open → just activate.
        setState({ activeTabId: id });
        return;
      }
      setState({
        tabs: [...state.tabs, tab],
        activeTabId: id,
        playheads: { ...state.playheads, [id]: state.playheads[id] ?? 0 },
        durations: { ...state.durations, [id]: state.durations[id] ?? 0 },
      });
    },

    closeTab: (id) => {
      if (id === TIMELINE_ID) return; // timeline is non-closable.
      const tabs = state.tabs.filter((t) => tabId(t) !== id);
      const activeTabId = state.activeTabId === id ? TIMELINE_ID : state.activeTabId;
      const playheads = { ...state.playheads };
      const durations = { ...state.durations };
      delete playheads[id];
      delete durations[id];
      setState({ tabs, activeTabId, playheads, durations });
    },

    closeAllTabs: () => {
      setState({
        tabs: [TIMELINE_TAB],
        activeTabId: TIMELINE_ID,
        playheads: { [TIMELINE_ID]: state.playheads[TIMELINE_ID] ?? 0 },
        durations: { [TIMELINE_ID]: state.durations[TIMELINE_ID] ?? 0 },
      });
    },

    selectTab: (id) => {
      if (!state.tabs.some((t) => tabId(t) === id)) return;
      setState({ activeTabId: id, scrubbing: false });
    },

    setPlayhead: (id, frame) =>
      setState({ playheads: { ...state.playheads, [id]: Math.max(0, frame) } }),
    setActivePlayhead: (frame) =>
      setState({ playheads: { ...state.playheads, [state.activeTabId]: Math.max(0, frame) } }),
    setDuration: (id, frames) =>
      setState({ durations: { ...state.durations, [id]: Math.max(0, frames) } }),
    setPlaying: (playing) => setState({ playing }),
    setScrubbing: (scrubbing) => setState({ scrubbing }),

    setZoom: (canvasZoom) => setState({ canvasZoom }),
    setOffset: (canvasOffset) => setState({ canvasOffset }),
    resetView: () => setState({ canvasZoom: 1.0, canvasOffset: { width: 0, height: 0 } }),

    setCropEditing: (cropEditing) => setState({ cropEditing }),
    setSelectedClip: (selectedClipId) => setState({ selectedClipId }),
  };
}

/** React hook selecting from a preview store (mirrors `useTimelineStore`). */
export function usePreviewStore<T>(store: PreviewStore, selector: (s: PreviewState) => T): T {
  return useSyncExternalStore(
    store.subscribe,
    () => selector(store.getState()),
    () => selector(store.getState()),
  );
}

export { TIMELINE_ID };
