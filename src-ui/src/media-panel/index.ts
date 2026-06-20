// Public API of the media-panel module (E4-S8..S11).
//
// The app shell mounts `<MediaPanel />` — self-contained (creates its own store +
// controller from the fixture if none injected). Until the real Tauri media
// commands land (Epic 7 `get_media`/`import`, Epic 11 `search_media`, Epic 9 gen),
// it is driven by `fixture.ts` through the `MediaPanelController` command seam.

// --- The mountable panel + its tabs ---
export { MediaPanel } from "./MediaPanel";
export type { MediaPanelProps } from "./MediaPanel";
export { MediaTab } from "./MediaTab";
export type { MediaTabProps } from "./MediaTab";
export { CaptionsTab } from "./CaptionsTab";
export { MusicTab } from "./MusicTab";

// --- Sub-components (exported for reuse / tests) ---
export { MediaGrid } from "./MediaGrid";
export type { MediaGridProps } from "./MediaGrid";
export { MediaToolbar } from "./MediaToolbar";
export type { MediaToolbarProps } from "./MediaToolbar";
export { AssetTile, FolderTile } from "./MediaTile";
export type { AssetTileProps, FolderTileProps } from "./MediaTile";
export { SearchResultsPanel } from "./SearchResultsPanel";
export type { SearchResultsPanelProps } from "./SearchResultsPanel";
export { IndexStatusPill } from "./IndexStatusPill";
export type { IndexStatusPillProps } from "./IndexStatusPill";
export { GenerationPanel } from "./GenerationPanel";
export type { GenerationPanelProps } from "./GenerationPanel";

// --- Store (Zustand-shaped, self-contained) ---
export { createMediaPanelStore, useMediaStore } from "./store";
export type { MediaPanelStore, MediaPanelState } from "./store";

// --- Command seam (Tauri commands replace the local apply at E7/E9/E11) ---
export { MediaPanelController, localId } from "./controller";

// --- OS actions (E4-S12 Tauri command seam: reveal / copy-path / relink / paste) ---
export {
  revealInExplorer,
  copyPathsToClipboard,
  pickRelinkPath,
  readClipboardImportablePaths,
  momentThumbnail,
} from "./media-actions";

// --- Reveal events (backend→UI Tauri events) ---
export {
  MEDIA_PANEL_EVENTS,
  registerRevealHandlers,
  applyReveal,
  scrollKeyForFolder,
} from "./reveal-events";
export type { RevealCallbacks } from "./reveal-events";

// --- Pure logic (sort/filter/grid/grouping/nav/marquee) — reused + tested ---
export {
  passesFilters,
  sortAndFilter,
  gridDimensions,
  tileHeight,
  childFolders,
  folderPath,
  breadcrumb,
  buildView,
  moveSelection,
  marqueeRect,
  rectsIntersect,
  marqueeSelect,
  isDescendant,
  legalFolderMoves,
} from "./logic";
export type { FilterState, GridDimensions, GroupedSection, Rect } from "./logic";

// --- Search logic ---
export {
  MOMENT_SEARCH_DEBOUNCE_MS,
  fileMatches,
  assembleResults,
  momentSearchCandidates,
  scheduleMomentSearch,
  runMomentSearchStub,
  formatTimecode,
} from "./search";

// --- Drag payload URI contract ---
export {
  ASSET_SCHEME,
  FOLDER_SCHEME,
  assetUri,
  folderUri,
  momentUri,
  formatSourceSeconds,
  buildAssetDragPayload,
  parseUri,
  parsePayload,
} from "./drag";
export type { ParsedUri } from "./drag";

// --- Theme constants ---
export { Theme, Spacing, Interaction, typeColor, typeRgb, rgba } from "./theme";

// --- Fixtures (swapped for the get_media command at Epic 7) ---
export { makeFixtureSnapshot, makeFixtureJobs } from "./fixture";

// --- Types ---
export type {
  MediaType,
  FilterableType,
  MediaAssetView,
  MediaFolderView,
  MediaSnapshot,
  PanelTab,
  SortMode,
  ViewMode,
  MediaPanelItemKey,
  VisualHit,
  SpokenHit,
  SearchResults,
  IndexStatus,
  GenJob,
  GenJobStatus,
} from "./types";
export {
  FILTERABLE_TYPES,
  SORT_MODES,
  VIEW_MODES,
  THUMBNAIL_SIZE,
  folderItemKey,
  isFolderItemKey,
  folderIdFromItemKey,
  isTerminalJob,
} from "./types";

// --- Parity checks (tsc-covered; runnable via _run-parity.mts) ---
export { runMediaParityChecks } from "./parity.checks";
