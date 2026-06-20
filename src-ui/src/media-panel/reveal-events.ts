// External reveal entry points (E4-S8) — backend→UI Tauri events the panel reacts
// to (media-panel.md §"Selection / navigation"). The macOS reference drives these
// off `editor.*` `@Observable` signals; on Tauri they are events emitted by Rust.
//
//   mediaPanelRevealAssetId    — select + scroll to an asset (e.g. agent reveal)
//   mediaPanelOpenFolderId     — drill into a folder
//   mediaPanelScrollTarget     — scroll a given item key into view
//   mediaPanelPasteRequestTick — request a clipboard paste (menu/shortcut)
//   mediaPanelShowMediaTabTick — force the rail to the Media tab
//
// `registerRevealHandlers` wires the events to controller/store actions and returns
// an unlisten fn. Outside a Tauri webview (plain `vite dev`) the event API is
// unavailable, so registration degrades gracefully (logs + no-op), mirroring the
// app-shell's `registerMenuHandlers` pattern in `App.tsx`.

import { folderItemKey } from "./types";
import type { MediaPanelStore } from "./store";

export const MEDIA_PANEL_EVENTS = {
  revealAssetId: "mediaPanelRevealAssetId",
  openFolderId: "mediaPanelOpenFolderId",
  scrollTarget: "mediaPanelScrollTarget",
  pasteRequestTick: "mediaPanelPasteRequestTick",
  showMediaTabTick: "mediaPanelShowMediaTabTick",
} as const;

export interface RevealCallbacks {
  /** Scroll a given item key into view (the grid component provides this). */
  onScrollTo?: (key: string) => void;
  /** A paste was requested (controller.importPaths after reading the clipboard). */
  onPasteRequest?: () => void;
}

type TauriEventApi = {
  listen: <T>(
    event: string,
    handler: (e: { payload: T }) => void,
  ) => Promise<() => void>;
};

/**
 * Apply a reveal event to the store/controller directly (no Tauri). Exposed so the
 * behavior is unit-testable and so callers can synthesize reveals (e.g. from the
 * agent panel in-process) without round-tripping through Tauri.
 */
export function applyReveal(
  store: MediaPanelStore,
  cbs: RevealCallbacks,
  event: { kind: keyof typeof MEDIA_PANEL_EVENTS; payload?: string },
): void {
  switch (event.kind) {
    case "showMediaTabTick":
      store.setTab("media");
      break;
    case "openFolderId":
      if (event.payload != null) {
        store.setTab("media");
        store.openFolder(event.payload);
      }
      break;
    case "revealAssetId":
      if (event.payload != null) {
        store.setTab("media");
        store.setSelection([event.payload]);
        store.setFocused(event.payload);
        cbs.onScrollTo?.(event.payload);
      }
      break;
    case "scrollTarget":
      if (event.payload != null) cbs.onScrollTo?.(event.payload);
      break;
    case "pasteRequestTick":
      cbs.onPasteRequest?.();
      break;
  }
}

/**
 * Subscribe the panel to the backend→UI reveal events. Returns an unlisten fn.
 * Pass the Tauri `event` module's `listen` (dynamic-imported by the caller) so this
 * module stays free of a hard `@tauri-apps/api` import at module load.
 */
export async function registerRevealHandlers(
  api: TauriEventApi,
  store: MediaPanelStore,
  cbs: RevealCallbacks = {},
): Promise<() => void> {
  const unlisteners: Array<() => void> = [];
  const bind = async (
    eventName: string,
    kind: keyof typeof MEDIA_PANEL_EVENTS,
  ) => {
    const un = await api.listen<string | undefined>(eventName, (e) => {
      applyReveal(store, cbs, { kind, payload: e.payload });
    });
    unlisteners.push(un);
  };

  await bind(MEDIA_PANEL_EVENTS.revealAssetId, "revealAssetId");
  await bind(MEDIA_PANEL_EVENTS.openFolderId, "openFolderId");
  await bind(MEDIA_PANEL_EVENTS.scrollTarget, "scrollTarget");
  await bind(MEDIA_PANEL_EVENTS.pasteRequestTick, "pasteRequestTick");
  await bind(MEDIA_PANEL_EVENTS.showMediaTabTick, "showMediaTabTick");

  return () => {
    for (const un of unlisteners) un();
  };
}

/** Scroll-target key for a folder cell (asset cells use the raw id). */
export function scrollKeyForFolder(folderId: string): string {
  return folderItemKey(folderId);
}
