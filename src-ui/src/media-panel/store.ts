// Media-panel external store (E4-S8) — mirrors the editor's self-contained store.
//
// The spec allows Zustand "if needed"; to avoid touching the shared pnpm lockfile
// (owned by the app-shell worker — scope guard) this is a tiny self-contained store
// exposing the same shape a Zustand store would (`getState`/`setState`/`subscribe`)
// plus a React `useMediaStore` hook on `useSyncExternalStore`. If Zustand is later
// added project-wide, swap for `create(...)` with no change to consumers.
//
// The store holds the `MediaSnapshot` (fixture today; `get_media` Tauri command at
// Epic 7) plus all panel view state the reference keeps on the `editor.*` signals:
// active tab, current folder, sort/filter/view mode, thumbnail size, selection,
// search query/results, generation jobs, and the index-status pill state.

import { useSyncExternalStore } from "react";
import {
  isTerminalJob,
  type FilterableType,
  type GenJob,
  type IndexStatus,
  type MediaFolderView,
  type MediaPanelItemKey,
  type MediaSnapshot,
  type PanelTab,
  type SearchResults,
  type SortMode,
  type ViewMode,
} from "./types";
import type { FilterState } from "./logic";
import { THUMBNAIL_SIZE } from "./types";

export interface MediaPanelState {
  snapshot: MediaSnapshot;
  // --- rail / navigation ---
  tab: PanelTab;
  currentFolderId: string | null;
  // --- toolbar ---
  sort: SortMode;
  viewMode: ViewMode;
  thumbnailSize: number;
  filter: FilterState;
  collapsedSections: Set<string | null>; // grouped-view collapse, by folderId
  // --- selection / focus (E4-S11) ---
  selection: Set<MediaPanelItemKey>;
  focusedKey: MediaPanelItemKey | null;
  // --- search (E4-S10) ---
  searchResults: SearchResults | null;
  indexStatus: IndexStatus;
  // --- generation (E4-S11) ---
  jobs: GenJob[];
}

export interface MediaPanelStore {
  getState: () => MediaPanelState;
  setState: (partial: Partial<MediaPanelState>) => void;
  subscribe: (listener: () => void) => () => void;

  // rail / nav
  setTab: (tab: PanelTab) => void;
  openFolder: (folderId: string | null) => void;
  navigateUp: (parentOf: (id: string) => string | null) => void;

  // toolbar
  setSort: (sort: SortMode) => void;
  setViewMode: (mode: ViewMode) => void;
  setThumbnailSize: (size: number) => void;
  toggleTypeFilter: (type: FilterableType) => void;
  setFilterAI: (on: boolean) => void;
  setQuery: (query: string) => void;
  toggleSectionCollapsed: (folderId: string | null) => void;

  // selection
  setSelection: (keys: Iterable<MediaPanelItemKey>) => void;
  toggleSelection: (key: MediaPanelItemKey, additive: boolean) => void;
  setFocused: (key: MediaPanelItemKey | null) => void;
  clearSelection: () => void;

  // snapshot mutation (local; Tauri commands replace these at E7)
  setSnapshot: (snapshot: MediaSnapshot) => void;
  addFolder: (folder: MediaFolderView) => void;
  renameFolder: (id: string, name: string) => void;
  renameAsset: (id: string, name: string) => void;

  // search
  setSearchResults: (results: SearchResults | null) => void;
  setIndexStatus: (status: IndexStatus) => void;

  // generation
  setJobs: (jobs: GenJob[]) => void;
  upsertJob: (job: GenJob) => void;
  dismissJob: (id: string) => void;
  cancelJob: (id: string) => void;
}

const initialFilter: FilterState = {
  filterTypes: new Set<FilterableType>(),
  filterAI: false,
  query: "",
};

export function createMediaPanelStore(
  initial?: Partial<MediaPanelState>,
): MediaPanelStore {
  let state: MediaPanelState = {
    snapshot: initial?.snapshot ?? { assets: [], folders: [] },
    tab: initial?.tab ?? "media",
    currentFolderId: initial?.currentFolderId ?? null,
    sort: initial?.sort ?? "dateAdded",
    viewMode: initial?.viewMode ?? "folder",
    thumbnailSize: initial?.thumbnailSize ?? THUMBNAIL_SIZE.presets.medium,
    filter: initial?.filter ?? { ...initialFilter, filterTypes: new Set() },
    collapsedSections: initial?.collapsedSections ?? new Set<string | null>(),
    selection: initial?.selection ?? new Set<MediaPanelItemKey>(),
    focusedKey: initial?.focusedKey ?? null,
    searchResults: initial?.searchResults ?? null,
    indexStatus: initial?.indexStatus ?? { kind: "notInstalled" },
    jobs: initial?.jobs ?? [],
  };

  const listeners = new Set<() => void>();
  const emit = () => listeners.forEach((l) => l());
  const setState = (partial: Partial<MediaPanelState>) => {
    state = { ...state, ...partial };
    emit();
  };
  const setFilter = (partial: Partial<FilterState>) =>
    setState({ filter: { ...state.filter, ...partial } });

  return {
    getState: () => state,
    setState,
    subscribe: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },

    setTab: (tab) => setState({ tab }),
    openFolder: (folderId) =>
      setState({
        currentFolderId: folderId,
        selection: new Set(),
        focusedKey: null,
      }),
    navigateUp: (parentOf) => {
      const cur = state.currentFolderId;
      if (cur == null) return;
      setState({
        currentFolderId: parentOf(cur),
        selection: new Set(),
        focusedKey: null,
      });
    },

    setSort: (sort) => setState({ sort }),
    setViewMode: (viewMode) => setState({ viewMode }),
    setThumbnailSize: (size) =>
      setState({
        thumbnailSize: Math.max(
          THUMBNAIL_SIZE.min,
          Math.min(THUMBNAIL_SIZE.max, size),
        ),
      }),
    toggleTypeFilter: (type) => {
      const next = new Set(state.filter.filterTypes);
      if (next.has(type)) next.delete(type);
      else next.add(type);
      setFilter({ filterTypes: next });
    },
    setFilterAI: (on) => setFilter({ filterAI: on }),
    setQuery: (query) => setFilter({ query }),
    toggleSectionCollapsed: (folderId) => {
      const next = new Set(state.collapsedSections);
      if (next.has(folderId)) next.delete(folderId);
      else next.add(folderId);
      setState({ collapsedSections: next });
    },

    setSelection: (keys) => setState({ selection: new Set(keys) }),
    toggleSelection: (key, additive) => {
      const next = additive
        ? new Set(state.selection)
        : new Set<MediaPanelItemKey>();
      if (state.selection.has(key) && additive) next.delete(key);
      else next.add(key);
      setState({ selection: next, focusedKey: key });
    },
    setFocused: (key) => setState({ focusedKey: key }),
    clearSelection: () => setState({ selection: new Set(), focusedKey: null }),

    setSnapshot: (snapshot) => setState({ snapshot }),
    addFolder: (folder) =>
      setState({
        snapshot: {
          ...state.snapshot,
          folders: [...state.snapshot.folders, folder],
        },
      }),
    renameFolder: (id, name) =>
      setState({
        snapshot: {
          ...state.snapshot,
          folders: state.snapshot.folders.map((f) =>
            f.id === id ? { ...f, name } : f,
          ),
        },
      }),
    renameAsset: (id, name) =>
      setState({
        snapshot: {
          ...state.snapshot,
          assets: state.snapshot.assets.map((a) =>
            a.id === id ? { ...a, name } : a,
          ),
        },
      }),

    setSearchResults: (searchResults) => setState({ searchResults }),
    setIndexStatus: (indexStatus) => setState({ indexStatus }),

    setJobs: (jobs) => setState({ jobs }),
    upsertJob: (job) => {
      const idx = state.jobs.findIndex((j) => j.id === job.id);
      const jobs =
        idx >= 0
          ? state.jobs.map((j) => (j.id === job.id ? job : j))
          : [...state.jobs, job];
      setState({ jobs });
    },
    dismissJob: (id) => {
      // Failed/terminal jobs persist until explicitly dismissed (E4-S11).
      setState({ jobs: state.jobs.filter((j) => j.id !== id) });
    },
    cancelJob: (id) =>
      setState({
        jobs: state.jobs.map((j) =>
          j.id === id && !isTerminalJob(j)
            ? { ...j, status: { kind: "cancelled" } }
            : j,
        ),
      }),
  };
}

/** Subscribe a React component to a slice of the store. */
export function useMediaStore<T>(
  store: MediaPanelStore,
  selector: (s: MediaPanelState) => T,
): T {
  return useSyncExternalStore(
    store.subscribe,
    () => selector(store.getState()),
    () => selector(store.getState()),
  );
}
