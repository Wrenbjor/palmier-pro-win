// Pure media-tab logic: sort / filter / grid-math / grouping / breadcrumb.
//
// Framework-free and side-effect-free so it is fully covered by `tsc --noEmit`
// and the parity-check module (`parity.checks.ts`). Ported 1:1 from the macOS
// reference `MediaTab/MediaTab.swift` (`sortAndFilter`, `passesFilters`, view-mode
// bodies) and `MediaTab/MediaTab+Grids.swift` (`gridDimensions`, `computeLayout`).

import { Spacing } from "./theme";
import {
  folderItemKey,
  type FilterableType,
  type MediaAssetView,
  type MediaFolderView,
  type MediaPanelItemKey,
  type SortMode,
} from "./types";

// --- Filtering ----------------------------------------------------------------

export interface FilterState {
  /** Selected type chips; empty set = all types pass. */
  filterTypes: Set<FilterableType>;
  /** AI-generated-only toggle. */
  filterAI: boolean;
  /** Live name-search query (trimmed at compare time). */
  query: string;
}

/**
 * `passesFilters` (MediaTab.swift): type ∈ filterTypes (or empty) AND
 * (`!filterAI || asset.isGenerated`) AND name case-insensitively contains the
 * trimmed query. text/lottie never appear in chips, but still pass when no type
 * chip is selected (they are filtered out of chips, not out of results).
 */
export function passesFilters(
  asset: MediaAssetView,
  filter: FilterState,
): boolean {
  if (filter.filterTypes.size > 0) {
    // Only `video|audio|image` are chip-filterable; a chip set excludes everything
    // not explicitly selected (so text/lottie drop out when any chip is active).
    if (
      asset.type === "text" ||
      asset.type === "lottie" ||
      !filter.filterTypes.has(asset.type as FilterableType)
    ) {
      return false;
    }
  }
  if (filter.filterAI && !asset.isGenerated) return false;
  const q = filter.query.trim().toLowerCase();
  if (q.length > 0 && !asset.name.toLowerCase().includes(q)) return false;
  return true;
}

// --- Sorting ------------------------------------------------------------------

/**
 * `sortAndFilter` (MediaTab.swift): filter by `passesFilters`, then sort by mode.
 * - `dateAdded` = stable INSERTION ORDER (no sort — preserve array order, ruling
 *   #15). Do NOT sort by a stored timestamp.
 * - `name` = case-insensitive ascending.
 * - `duration` = DESCENDING (nulls/stills treated as 0).
 * - `type` = rawValue ascending (the `MediaType` string).
 * Returns a NEW array; never mutates the input.
 */
export function sortAndFilter(
  assets: readonly MediaAssetView[],
  filter: FilterState,
  sort: SortMode,
): MediaAssetView[] {
  const filtered = assets.filter((a) => passesFilters(a, filter));
  if (sort === "dateAdded") return filtered; // insertion order, no sort
  const out = filtered.slice();
  switch (sort) {
    case "name":
      out.sort((a, b) =>
        a.name.toLowerCase().localeCompare(b.name.toLowerCase()),
      );
      break;
    case "duration":
      out.sort((a, b) => (b.durationSeconds ?? 0) - (a.durationSeconds ?? 0));
      break;
    case "type":
      out.sort((a, b) => a.type.localeCompare(b.type));
      break;
  }
  return out;
}

// --- Grid math ----------------------------------------------------------------

export interface GridDimensions {
  columns: number;
  tileWidth: number;
}

/**
 * `gridDimensions(width)` (MediaTab+Grids.swift), exact math:
 *   spacing = Spacing.xl
 *   outerPadding = Spacing.md * 2
 *   usable = width - outerPadding
 *   cols = max(1, floor((usable + spacing) / (thumbnailSize + spacing)))
 *   tileWidth = max(thumbnailSize, (usable - (cols-1)*spacing) / cols)
 * Tiles render 16:9 (tileHeight = tileWidth * 9/16) — caller's concern.
 */
export function gridDimensions(
  width: number,
  thumbnailSize: number,
): GridDimensions {
  const spacing = Spacing.xl;
  const outerPadding = Spacing.md * 2;
  const usable = Math.max(0, width - outerPadding);
  const columns = Math.max(
    1,
    Math.floor((usable + spacing) / (thumbnailSize + spacing)),
  );
  const tileWidth = Math.max(
    thumbnailSize,
    (usable - (columns - 1) * spacing) / columns,
  );
  return { columns, tileWidth };
}

/** 16:9 tile height for a given tile width (tiles render 16:9). */
export function tileHeight(tileWidth: number): number {
  return (tileWidth * 9) / 16;
}

// --- Folder helpers -----------------------------------------------------------

/** Direct child folders of `parentId` (null = root), in array order. */
export function childFolders(
  folders: readonly MediaFolderView[],
  parentId: string | null,
): MediaFolderView[] {
  return folders.filter((f) => f.parentFolderId === parentId);
}

/**
 * The folder-path chain for a folder id, root→leaf, as folder objects.
 * Used for the breadcrumb (`[Library, ...folderPath]`) and grouped-section sort.
 * Guards against malformed (cyclic) parent chains by tracking visited ids.
 */
export function folderPath(
  folders: readonly MediaFolderView[],
  folderId: string | null,
): MediaFolderView[] {
  const byId = new Map(folders.map((f) => [f.id, f]));
  const chain: MediaFolderView[] = [];
  const seen = new Set<string>();
  let cur = folderId;
  while (cur != null) {
    if (seen.has(cur)) break; // cycle guard
    seen.add(cur);
    const f = byId.get(cur);
    if (!f) break;
    chain.unshift(f);
    cur = f.parentFolderId;
  }
  return chain;
}

/** Breadcrumb labels `[Library, ...folderPath names]`. */
export function breadcrumb(
  folders: readonly MediaFolderView[],
  folderId: string | null,
): { id: string | null; name: string }[] {
  const crumbs: { id: string | null; name: string }[] = [
    { id: null, name: "Library" },
  ];
  for (const f of folderPath(folders, folderId)) {
    crumbs.push({ id: f.id, name: f.name });
  }
  return crumbs;
}

// --- View-mode item construction ----------------------------------------------

export interface GroupedSection {
  /** Folder id, or null for the root "Library" section. */
  folderId: string | null;
  title: string;
  assets: MediaAssetView[];
}

/**
 * Build the ordered display items for a view mode. Returns the flat list of grid
 * item KEYS in render order (`mediaPanelOrderedItemIds`) plus, for `grouped`, the
 * section breakdown. Folder cell keys are `"folder-<id>"`, asset keys are raw ids.
 *
 * - `folder`: subfolders of `currentFolderId` (array order) then its assets
 *   (sorted/filtered).
 * - `flat`: every asset (sorted/filtered), no folders.
 * - `grouped`: root "Library" section + one section per folder, sections sorted
 *   by full folder path joined " / "; each section's assets sorted/filtered.
 */
export function buildView(
  mode: "folder" | "flat" | "grouped",
  snapshot: { assets: MediaAssetView[]; folders: MediaFolderView[] },
  currentFolderId: string | null,
  filter: FilterState,
  sort: SortMode,
): { orderedKeys: MediaPanelItemKey[]; sections: GroupedSection[] } {
  const { assets, folders } = snapshot;

  if (mode === "flat") {
    const sorted = sortAndFilter(assets, filter, sort);
    return { orderedKeys: sorted.map((a) => a.id), sections: [] };
  }

  if (mode === "folder") {
    const subfolders = childFolders(folders, currentFolderId);
    const here = assets.filter((a) => a.folderId === currentFolderId);
    const sorted = sortAndFilter(here, filter, sort);
    const keys: MediaPanelItemKey[] = [
      ...subfolders.map((f) => folderItemKey(f.id)),
      ...sorted.map((a) => a.id),
    ];
    return { orderedKeys: keys, sections: [] };
  }

  // grouped
  const pathLabel = (folderId: string | null): string =>
    folderId == null
      ? "Library"
      : folderPath(folders, folderId)
          .map((f) => f.name)
          .join(" / ");

  // Bucket assets by folderId once.
  const buckets = new Map<string | null, MediaAssetView[]>();
  for (const a of assets) {
    const arr = buckets.get(a.folderId) ?? [];
    arr.push(a);
    buckets.set(a.folderId, arr);
  }
  // Root section first, then folder sections sorted by full path.
  const folderIds = folders.map((f) => f.id);
  folderIds.sort((a, b) => pathLabel(a).localeCompare(pathLabel(b)));
  const orderedFolderIds: (string | null)[] = [null, ...folderIds];

  const sections: GroupedSection[] = [];
  const orderedKeys: MediaPanelItemKey[] = [];
  for (const fid of orderedFolderIds) {
    const bucket = buckets.get(fid) ?? [];
    const sorted = sortAndFilter(bucket, filter, sort);
    if (sorted.length === 0 && fid !== null) continue; // skip empty folder sections
    sections.push({ folderId: fid, title: pathLabel(fid), assets: sorted });
    orderedKeys.push(...sorted.map((a) => a.id));
  }
  return { orderedKeys, sections };
}

// --- Keyboard arrow navigation ------------------------------------------------

/**
 * `moveMediaSelection` (MediaTab.swift): move the focused item by arrow direction
 * over the ordered item-key grid given the column count. Returns the new focused
 * key, or the current one at edges (clamped, no wrap).
 */
export function moveSelection(
  orderedKeys: readonly MediaPanelItemKey[],
  current: MediaPanelItemKey | null,
  direction: "left" | "right" | "up" | "down",
  columnCount: number,
): MediaPanelItemKey | null {
  if (orderedKeys.length === 0) return null;
  const cols = Math.max(1, columnCount);
  const idx = current == null ? -1 : orderedKeys.indexOf(current);
  if (idx < 0) return orderedKeys[0];
  let next = idx;
  switch (direction) {
    case "left":
      next = idx - 1;
      break;
    case "right":
      next = idx + 1;
      break;
    case "up":
      next = idx - cols;
      break;
    case "down":
      next = idx + cols;
      break;
  }
  if (next < 0 || next >= orderedKeys.length) return orderedKeys[idx];
  return orderedKeys[next];
}

// --- Marquee selection --------------------------------------------------------

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Build a normalized rect from two corner points (drag start + current). */
export function marqueeRect(
  ax: number,
  ay: number,
  bx: number,
  by: number,
): Rect {
  return {
    x: Math.min(ax, bx),
    y: Math.min(ay, by),
    w: Math.abs(ax - bx),
    h: Math.abs(ay - by),
  };
}

/** Axis-aligned rect intersection test. */
export function rectsIntersect(a: Rect, b: Rect): boolean {
  return (
    a.x < b.x + b.w &&
    a.x + a.w > b.x &&
    a.y < b.y + b.h &&
    a.y + a.h > b.y
  );
}

/**
 * Intersect a marquee rect against reported cell frames (`assetFrames`) to build
 * the selection of item keys whose frame overlaps the marquee. `additive` (shift)
 * unions with the base selection.
 */
export function marqueeSelect(
  marquee: Rect,
  frames: ReadonlyMap<MediaPanelItemKey, Rect>,
  base: ReadonlySet<MediaPanelItemKey>,
  additive: boolean,
): Set<MediaPanelItemKey> {
  const out = additive ? new Set(base) : new Set<MediaPanelItemKey>();
  for (const [key, frame] of frames) {
    if (rectsIntersect(marquee, frame)) out.add(key);
  }
  return out;
}
