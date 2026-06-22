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
  indexStatusFromWire,
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
import { editorEdit, getMedia, importMedia, inTauri } from "../editor/bridge";
import { adaptMedia } from "./adapt";

/** Generate a local UUID-ish id (replaced by backend ids when commands land). */
export function localId(prefix = "id"): string {
  const rnd = Math.random().toString(36).slice(2, 10);
  return `${prefix}-${rnd}-${Date.now().toString(36)}`;
}

export class MediaPanelController {
  private cancelSearch: (() => void) | null = null;

  constructor(private store: MediaPanelStore) {}

  // --- Load (WIRED: editor_get_media) -----------------------------------------

  /**
   * Seed the panel with an explicit media snapshot (fixture / tests / design preview).
   */
  loadMedia(snapshot: MediaSnapshot): void {
    this.store.setSnapshot(snapshot);
  }

  /**
   * Refetch the media library from the shared `EditorState` and replace the snapshot
   * (reference `get_media`). Runs the real `editor_get_media` Tauri command via the
   * editor bridge + `adaptMedia`; outside Tauri it is a no-op (the fixture stands in).
   * The Project surface calls this on mount and on every `timeline://changed`.
   */
  async refreshMedia(): Promise<void> {
    if (!inTauri()) return;
    const wire = await getMedia();
    if (wire !== undefined) this.store.setSnapshot(adaptMedia(wire));
  }

  // --- Folder ops (E6/E7 seam) ------------------------------------------------

  /**
   * Create a folder under the current folder (inline create).
   *
   * WIRED: inside a Tauri webview this dispatches `create_folder` through the shared
   * executor (the `timeline://changed` refetch reconciles the authoritative folder id).
   * The local `addFolder` runs first for instant feedback (and is the sole path
   * outside Tauri). Returns the optimistic folder row.
   */
  createFolder(name = "New Folder"): MediaFolderView {
    const parentFolderId = this.store.getState().currentFolderId;
    const folder: MediaFolderView = {
      id: localId("folder"),
      name,
      parentFolderId,
    };
    this.store.addFolder(folder);
    if (inTauri()) {
      const args: Record<string, unknown> = { name };
      if (parentFolderId) args.parentFolderId = parentFolderId;
      void editorEdit("create_folder", args);
    }
    return folder;
  }

  /** Inline rename a folder (WIRED: `rename_folder`). */
  renameFolder(id: string, name: string): void {
    this.store.renameFolder(id, name);
    if (inTauri()) void editorEdit("rename_folder", { folderId: id, name });
  }

  /** Inline rename an asset (WIRED: `rename_media`). */
  renameAsset(id: string, name: string): void {
    this.store.renameAsset(id, name);
    if (inTauri()) void editorEdit("rename_media", { mediaRef: id, name });
  }

  /** Walk to the parent of the current folder (`Ctrl+Up`). */
  navigateUp(): void {
    const folders = this.store.getState().snapshot.folders;
    this.store.navigateUp((id) => {
      const f = folders.find((x) => x.id === id);
      return f ? f.parentFolderId : null;
    });
  }

  // --- Import / paste ---------------------------------------------------------

  /**
   * Import dropped/picked/pasted files into the current folder as ONE batch.
   *
   * Routes through the dedicated `editor_import_media` command (bridge `importMedia`),
   * which imports each path through the SAME shared executor `import_media` tool the
   * agent/MCP use (a directory path is imported recursively backend-side) and emits
   * `timeline://changed` so the library refetches. We also refetch here so a drop
   * without an active listener still updates. Best-effort: a failed import is logged
   * by the bridge, not fatal.
   */
  async importPaths(paths: string[]): Promise<void> {
    if (!inTauri() || paths.length === 0) return;
    const folderId = this.store.getState().currentFolderId ?? undefined;
    await importMedia(paths, folderId);
    await this.refreshMedia();
  }

  /**
   * Open the native OS file picker (no paths) and import the chosen media into the
   * current folder. Backs the panel's Import affordance / File → Import Media. The
   * backend opens a multi-select dialog Rust-side; cancel is a no-op.
   */
  async importViaDialog(): Promise<void> {
    if (!inTauri()) return;
    const folderId = this.store.getState().currentFolderId ?? undefined;
    await importMedia(undefined, folderId);
    await this.refreshMedia();
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
    if (assetIds.length > 0) {
      this.store.moveAssets(assetIds, targetFolderId);
      // WIRED: asset reparent → `move_to_folder` (folder reparent has no tool yet, so
      // it stays local-only — a follow-up seam). Omit folderId for the project root.
      if (inTauri()) {
        const args: Record<string, unknown> = { assetIds };
        if (targetFolderId) args.folderId = targetFolderId;
        void editorEdit("move_to_folder", args);
      }
    }
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
        // Surface the live visual-index status into the pill. `ready` is the steady
        // state (no affordance needed); anything else (disabled/preparing/indexing/
        // downloading/failed/model-not-installed) drives the pill's status/CTA.
        this.store.setIndexStatus(indexStatusFromWire(r.visualStatus));
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
