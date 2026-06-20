// MediaPanelController — the single command seam for media-panel side effects
// (E4-S8). Mirrors the editor's `EditController` boundary convention.
//
// Today every command runs against the local store + fixture. When the real Tauri
// media commands land they REPLACE the local apply: `loadMedia` will
// `await invoke('get_media')`, `import` will `await invoke('import', ...)`,
// `search` will `await invoke('search_media', ...)`, folder/asset moves go to
// `palmier-model` via Tauri. Each such seam is marked `// TODO(E7)` / `// TODO(E11)`.
//
// Per FOUNDATION §4 strict layering: all side effects flow through this controller
// (→ Tauri commands later); reactive state comes back via the store (← Tauri events).

import {
  assembleResults,
  fileMatches,
  momentSearchCandidates,
  scheduleMomentSearch,
} from "./search";
import type { MediaPanelStore } from "./store";
import { buildView, folderPath, type FilterState } from "./logic";
import { parsePayload } from "./drag";
import {
  copyPathsToClipboard,
  pickRelinkPath,
  readClipboardImportablePaths,
  revealInExplorer,
} from "./media-actions";
import type {
  GenJob,
  MediaFolderView,
  MediaPanelItemKey,
  MediaSnapshot,
  ViewMode,
} from "./types";

/** Generate a local UUID-ish id (replaced by backend ids when commands land). */
export function localId(prefix = "id"): string {
  const rnd = Math.random().toString(36).slice(2, 10);
  return `${prefix}-${rnd}-${Date.now().toString(36)}`;
}

export class MediaPanelController {
  private cancelSearch: (() => void) | null = null;

  constructor(private store: MediaPanelStore) {}

  // --- Load (E7 seam) ---------------------------------------------------------

  /**
   * Seed the panel with a media snapshot.
   * TODO(E7): replace with
   *   const snap = adaptMedia(await invoke('get_media'));
   *   this.store.setSnapshot(snap);
   */
  loadMedia(snapshot: MediaSnapshot): void {
    this.store.setSnapshot(snapshot);
  }

  // --- Folder ops (E6/E7 seam) ------------------------------------------------

  /**
   * Create a folder under the current folder (inline create).
   * TODO(E7): route through palmier-model `create_folder` (snapshot-undo via
   * palmier-history) instead of the local store mutation.
   */
  createFolder(name = "New Folder"): MediaFolderView {
    const folder: MediaFolderView = {
      id: localId("folder"),
      name,
      parentFolderId: this.store.getState().currentFolderId,
    };
    this.store.addFolder(folder);
    return folder;
  }

  /** Inline rename a folder. TODO(E7): `rename_folder` model op. */
  renameFolder(id: string, name: string): void {
    this.store.renameFolder(id, name);
  }

  /** Inline rename an asset. TODO(E7): `rename_asset` model op. */
  renameAsset(id: string, name: string): void {
    this.store.renameAsset(id, name);
  }

  /** Walk to the parent of the current folder (`Ctrl+Up`). */
  navigateUp(): void {
    const folders = this.store.getState().snapshot.folders;
    this.store.navigateUp((id) => {
      const f = folders.find((x) => x.id === id);
      return f ? f.parentFolderId : null;
    });
  }

  // --- Import / paste (E7 seam) ----------------------------------------------

  /**
   * Import dropped/picked/pasted files as ONE undo step.
   * TODO(E7): replace with
   *   await invoke('import', { paths, into: currentFolderId });   // "Import Media"
   *   this.loadMedia(adaptMedia(await invoke('get_media')));
   * Native drop arrives via the Tauri `tauri://drag-drop` event (wired in E4-S12);
   * the file picker via the `dialog` plugin; paste via clipboard + `arboard`.
   */
  async importPaths(_paths: string[]): Promise<void> {
    // No-op stub: with the fixture data source there is nothing to import yet.
    // The drop zone / picker call this; once Epic 7 lands it does the real work.
    return;
  }

  /**
   * Paste from the clipboard (`mediaPanelPasteRequestTick` / Ctrl+V): import any
   * file URLs on the clipboard into the current folder. Image-data paste
   * (`.png`/`.tiff`) lands with the real import flow at Epic 7.
   * TODO(E7): the import itself is still the `importPaths` stub.
   */
  async pasteFromClipboard(): Promise<void> {
    const paths = await readClipboardImportablePaths();
    if (paths.length > 0) await this.importPaths(paths);
  }

  // --- In-panel + drag-out drag-drop (E4-S12) --------------------------------

  /**
   * `resolveTextDrop` (MediaTab+Drag.swift): a text/URI payload dropped on a
   * folder/breadcrumb/section target. Splits the newline-joined payload, routes
   * folder ids → `moveFoldersToFolder` (cycle-guarded), asset ids → `moveAssetsToFolder`.
   * Moment URIs carry a segment that is meaningless for an in-panel move, so they
   * reparent the underlying asset.
   * TODO(E6/E7): route through palmier-model moves with one snapshot-undo entry
   * (palmier-history) instead of the local store mutation.
   */
  resolveTextDrop(payload: string, targetFolderId: string | null): void {
    const parts = parsePayload(payload);
    const folderIds: string[] = [];
    const assetIds: string[] = [];
    for (const p of parts) {
      if (p.kind === "folder") folderIds.push(p.id);
      else assetIds.push(p.id); // asset | moment → reparent the asset
    }
    if (folderIds.length > 0) this.store.moveFolders(folderIds, targetFolderId);
    if (assetIds.length > 0) this.store.moveAssets(assetIds, targetFolderId);
  }

  /**
   * `handleProviderDrop` (MediaTab+Drag.swift): a native drop. A file-URL provider
   * (OS files / external) imports; a text/URI provider (in-panel drag) routes moves.
   * `files` are absolute paths from the Tauri `tauri://drag-drop` event; `text` is
   * the in-panel drag payload from `dataTransfer`.
   */
  async handleProviderDrop(
    drop: { files?: string[]; text?: string },
    targetFolderId: string | null,
  ): Promise<void> {
    if (drop.files && drop.files.length > 0) {
      await this.importPaths(drop.files);
      return;
    }
    if (drop.text && drop.text.trim().length > 0) {
      this.resolveTextDrop(drop.text, targetFolderId);
    }
  }

  // --- OS actions (E4-S12): Reveal / Copy-Path / Relink ----------------------

  /** Reveal an asset in Explorer/Finder (Tauri `opener`). No-op outside Tauri. */
  async revealAsset(assetId: string): Promise<void> {
    const a = this.store.getState().snapshot.assets.find((x) => x.id === assetId);
    if (a?.path) await revealInExplorer(a.path);
  }

  /**
   * Copy the path(s) of the given asset to the clipboard, newline-joined. If the
   * asset is part of the current selection, copies every selected asset's path
   * (reference Copy-Path on a multi-selection).
   */
  async copyAssetPath(assetId: string): Promise<void> {
    const s = this.store.getState();
    const sel = s.selection;
    const ids =
      sel.has(assetId) && sel.size > 1 ? Array.from(sel) : [assetId];
    const byId = new Map(s.snapshot.assets.map((a) => [a.id, a]));
    const paths = ids
      .map((id) => byId.get(id)?.path)
      .filter((p): p is string => !!p);
    if (paths.length > 0) await copyPathsToClipboard(paths);
  }

  /**
   * Relink a missing asset: open the OS picker (Tauri `dialog`) to repoint the
   * source file, then update the asset path + clear its missing flag.
   * TODO(E7): persist the repointed path through palmier-project.
   */
  async relinkAsset(assetId: string): Promise<void> {
    const a = this.store.getState().snapshot.assets.find((x) => x.id === assetId);
    if (!a) return;
    const picked = await pickRelinkPath(a.name);
    if (picked) this.store.relinkAsset(assetId, picked);
  }

  // --- Search (E10 local + E11 backend seam) ---------------------------------

  /**
   * Run a query: update the live name filter (always works) and the result panel.
   * The Files section is computed locally; Moments/Spoken come from the Epic 11
   * search backend (E11-S6 visual + E11-S8 spoken via the `search_media` command)
   * through the debounced scheduler (`scheduleMomentSearch`, 250ms).
   *
   * `search_media` wiring is live in `search.ts` (`runVisualSearch` /
   * `runSpokenSearch`); outside Tauri or before E11-S10 lands those return [] so
   * Moments/Spoken render "No matches" while Files keeps working.
   */
  search(query: string): void {
    this.store.setQuery(query);
    this.cancelSearch?.();

    const assets = this.store.getState().snapshot.assets;
    if (query.trim().length === 0) {
      this.store.setSearchResults(null);
      return;
    }
    // Immediate Files section; Moments/Spoken fill in once the debounce resolves.
    this.store.setSearchResults(assembleResults(assets, query, [], []));

    this.cancelSearch = scheduleMomentSearch(
      query,
      momentSearchCandidates(assets),
      (r) => {
        this.store.setSearchResults(
          assembleResults(assets, query, r.moments, r.spoken),
        );
      },
    );
  }

  /** File-name matches helper (exposed for tests / the Files section). */
  fileMatches(query: string) {
    return fileMatches(this.store.getState().snapshot.assets, query);
  }

  /**
   * `previewMoment` → `selectMediaAsset(asset, atSourceFrame:)`
   * (MediaTab+Search.swift): tapping a moment / spoken hit selects the underlying
   * asset and focuses it. `atSourceFrame` is the hit's source time converted to an
   * integer frame via `secondsToFrame(range.lowerBound, fps)` at the call site —
   * the reference scrubs the preview to that source frame.
   * TODO(E5/E6): seek the preview/inspector to `atSourceFrame` once the preview
   * surface owns a source-time cursor; today it selects + focuses the asset.
   */
  selectMediaAtSource(assetId: string, _atSourceFrame: number): void {
    this.store.setSelection([assetId]);
    this.store.setFocused(assetId);
  }

  // --- Selection / view derivation -------------------------------------------

  /**
   * Build the ordered grid item keys + grouped sections for the current view.
   * Published as `mediaPanelOrderedItemIds` for keyboard nav / scroll-to-reveal.
   */
  currentView(mode?: ViewMode): {
    orderedKeys: MediaPanelItemKey[];
    sections: ReturnType<typeof buildView>["sections"];
  } {
    const s = this.store.getState();
    return buildView(
      mode ?? s.viewMode,
      s.snapshot,
      s.currentFolderId,
      s.filter as FilterState,
      s.sort,
    );
  }

  /** The breadcrumb path objects for the current folder. */
  breadcrumbPath() {
    const s = this.store.getState();
    return folderPath(s.snapshot.folders, s.currentFolderId);
  }

  // --- Generation jobs (E9 seam) ---------------------------------------------

  /** Seed/replace the job list (fixture today; Tauri events at Epic 9). */
  setJobs(jobs: GenJob[]): void {
    this.store.setJobs(jobs);
  }

  /**
   * Cancel a running/queued job.
   * TODO(E9): `await invoke('cancel_generation', { id })`; the cancelled status
   * then arrives via a Tauri event instead of this optimistic local update.
   */
  cancelJob(id: string): void {
    this.store.cancelJob(id);
  }

  /** Dismiss a terminal (failed/cancelled/succeeded) job card. */
  dismissJob(id: string): void {
    this.store.dismissJob(id);
  }
}
