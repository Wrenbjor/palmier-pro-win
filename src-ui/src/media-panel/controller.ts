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

  // --- Search (E10 local + E11 backend seam) ---------------------------------

  /**
   * Run a query: update the live name filter (always works) and the result panel.
   * The Files section is computed locally; Moments/Spoken come from the (stubbed)
   * Epic 11 `search_media` via the debounced scheduler.
   */
  search(query: string): void {
    this.store.setQuery(query);
    this.cancelSearch?.();

    const assets = this.store.getState().snapshot.assets;
    if (query.trim().length === 0) {
      this.store.setSearchResults(null);
      return;
    }
    // Immediate Files section; empty Moments/Spoken until the debounce resolves.
    this.store.setSearchResults(assembleResults(assets, query, [], []));

    this.cancelSearch = scheduleMomentSearch(
      query,
      momentSearchCandidates(assets),
      (r) => {
        // TODO(E11): `r` is currently always empty (stub). Real visual/spoken hits
        // arrive from `invoke('search_media', { query, scope: 'both' })`.
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
