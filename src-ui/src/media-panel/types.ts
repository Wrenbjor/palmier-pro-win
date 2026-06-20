// Media-panel view-model types (E4-S8..S11).
//
// These are the FRONTEND view types for the left-dock Media Panel — a TS mirror
// of the Rust `palmier-model` `MediaAsset` / `MediaFolder` shapes, carrying only
// what the panel needs to browse/sort/filter/render. The real Tauri media commands
// (`get_media`, `search_media`, `import`) do not exist yet (Epic 7 / Epic 11), so
// the panel is fed a `MediaSnapshot` from a fixture (see `fixture.ts`). When the
// commands land, the adapter that turns the serde payload into a `MediaSnapshot`
// is the only thing that changes.
//
// Naming follows `docs/reference/media-panel.md` and the macOS reference
// (`Models/MediaAsset.swift`, `Models/MediaFolder`, `MediaTab/MediaTab.swift`).
// IDs are UUID strings (reference parity).

/**
 * Clip / media kinds — same union the editor uses (`editor/types.ts`).
 * Filterable chips are `video | audio | image` only; `text`/`lottie` are excluded
 * from the filter chips (reference parity, media-panel.md §"Sorting / filtering").
 */
export type MediaType = "video" | "image" | "text" | "lottie" | "audio";

/** The three filterable chip types (text/lottie excluded). */
export type FilterableType = "video" | "audio" | "image";
export const FILTERABLE_TYPES: readonly FilterableType[] = [
  "video",
  "audio",
  "image",
];

/**
 * A media asset as the panel needs to render it. Mirrors the render-relevant
 * subset of `palmier-model::MediaAsset`. `durationSeconds` is null for stills.
 * `isGenerated` drives the AI-generated filter toggle + the AI badge (#21 badge).
 */
export interface MediaAssetView {
  id: string;
  name: string;
  /** Absolute source path on disk (used by Reveal/Copy-Path; opaque to layout). */
  path: string;
  type: MediaType;
  /** Folder this asset lives in; null = root ("Library"). */
  folderId: string | null;
  /** Seconds; null for images/text. Drives `duration` sort + tile duration badge. */
  durationSeconds: number | null;
  /** AI-generated (Palmier gen) flag — drives the AI filter + badge. */
  isGenerated: boolean;
  /** True once the source file is missing (drives the Relink affordance). */
  missing?: boolean;
  /**
   * A thumbnail data-URL (decoded RGBA → data URL handed to the webview, per
   * media-panel.md §"macOS APIs to replace": NSImage → data URL). Optional; when
   * absent the tile shows a type-colored placeholder. TODO(E4-S3/E4-S5): real
   * sprite-sheet / image-thumb pipeline fills this.
   */
  thumbnailUrl?: string;
}

/** A media folder. Mirrors `palmier-model::MediaFolder { id, name, parentFolderId }`. */
export interface MediaFolderView {
  id: string;
  name: string;
  parentFolderId: string | null;
}

/** The whole media library snapshot the panel renders (fixture today, command later). */
export interface MediaSnapshot {
  assets: MediaAssetView[];
  folders: MediaFolderView[];
}

/** The three secondary surfaces on the rail. */
export type PanelTab = "media" | "captions" | "music";

/** Media-tab sort modes — 4 modes per ruling #15 (FOUNDATION §6.2 lists only 3). */
export type SortMode = "name" | "dateAdded" | "duration" | "type";
export const SORT_MODES: readonly SortMode[] = [
  "name",
  "dateAdded",
  "duration",
  "type",
];

/** Media-tab view modes. */
export type ViewMode = "folder" | "flat" | "grouped";
export const VIEW_MODES: readonly ViewMode[] = ["folder", "flat", "grouped"];

/** Thumbnail-size presets (slider range 80–200), media-panel.md §"Sorting/filtering". */
export const THUMBNAIL_SIZE = {
  min: 80,
  max: 200,
  presets: { small: 80, medium: 110, large: 150, xlarge: 200 },
} as const;

/**
 * Item key for grid cells. Folder cells are `"folder-<id>"`; asset cells are the
 * raw asset id (`MediaPanelItemKey`, media-panel.md §"Sorting / filtering"). Used
 * for keyboard arrow nav + scroll-to-reveal.
 */
export type MediaPanelItemKey = string;
export function folderItemKey(folderId: string): MediaPanelItemKey {
  return `folder-${folderId}`;
}
export function isFolderItemKey(key: MediaPanelItemKey): boolean {
  return key.startsWith("folder-");
}
export function folderIdFromItemKey(key: MediaPanelItemKey): string | null {
  return key.startsWith("folder-") ? key.slice("folder-".length) : null;
}

// --- Search result types (E4-S10) — mirrors palmier-search Hit shapes ---------

/** Visual ("Moments") hit. `time`/`shotStart`/`shotEnd` are source seconds. */
export interface VisualHit {
  assetID: string;
  time: number;
  shotStart: number;
  shotEnd: number;
  score: number;
}

/** Spoken ("transcript") hit. `start`/`end` are source seconds. */
export interface SpokenHit {
  assetID: string;
  start: number;
  end: number;
  text: string;
}

/** The three result sections the search panel renders. */
export interface SearchResults {
  /** Visual hits (frame grid). Fed by Epic 11 `search_media` (stub returns []). */
  moments: VisualHit[];
  /** Spoken hits (transcript segments w/ timecodes). Epic 11 (stub returns []). */
  spoken: SpokenHit[];
  /** File-name matches — always works with no backend (E4-S10). */
  files: MediaAssetView[];
}

// --- Index-status pill (E4-S10 stub) ------------------------------------------

/** CLIP/index model state machine (media-panel.md §"Search" / search.md loader states). */
export type IndexStatus =
  | { kind: "notInstalled" }
  | { kind: "downloading"; fraction: number }
  | { kind: "preparing" }
  | { kind: "indexing"; completed: number; total: number }
  | { kind: "ready" }
  | { kind: "failed"; message: string };

// --- Generation jobs (E4-S11) -------------------------------------------------

/** A generation job surfaced as a card below the media list (E4-S11). */
export type GenJobStatus =
  | { kind: "queued" }
  | { kind: "running"; progress: number }
  | { kind: "succeeded"; assetId?: string }
  | { kind: "failed"; message: string }
  | { kind: "cancelled" };

export interface GenJob {
  id: string;
  prompt: string;
  model: string;
  /** Thumbnail data-URL once a preview frame exists. */
  thumbnailUrl?: string;
  status: GenJobStatus;
  /** Insertion order for stable sort (newest first). */
  createdAt: number;
}

/** A job is terminal when it can no longer change without user action. */
export function isTerminalJob(job: GenJob): boolean {
  return (
    job.status.kind === "succeeded" ||
    job.status.kind === "failed" ||
    job.status.kind === "cancelled"
  );
}
