// Tauri command/event bridge for the Project editor (the read/edit seam).
//
// Thin typed wrappers over the `editor_*` commands in
// `crates/palmier-tauri/src/commands.rs` and the `timeline://changed` event the
// backend emits after any mutation (UI `editor_edit` OR an agent/MCP tool dispatch).
// The Project surface reads the shared `EditorState` through these and refetches on
// the event — it never touches `palmier-tools`/`palmier-engine` directly (FOUNDATION
// §4 strict layering), and it never polls.
//
// Every call degrades gracefully outside a Tauri webview (plain `vite dev`) so the
// editor renders against the fixture for design work — reads return undefined and
// `editorEdit` is a no-op.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** The backend event refetched on (mirrors Rust `TIMELINE_CHANGED_EVENT`). */
export const TIMELINE_CHANGED_EVENT = "timeline://changed";

/** True when running inside a Tauri webview (vs plain `vite dev`). */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Read the shaped timeline JSON (reference `get_timeline`). `window` optionally scopes
 * the read to `[startFrame, endFrame)`. Returns the raw wire object (`adaptTimeline`
 * maps it), or undefined outside Tauri / on error.
 */
export async function getTimeline(window?: {
  startFrame?: number;
  endFrame?: number;
}): Promise<unknown | undefined> {
  if (!inTauri()) return undefined;
  try {
    return await invoke<unknown>("editor_get_timeline", { args: window ?? null });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("[editor] editor_get_timeline failed:", err);
    return undefined;
  }
}

/**
 * Read the media-library JSON (reference `get_media`). Returns the raw wire object
 * (`adaptMedia` maps it), or undefined outside Tauri / on error.
 */
export async function getMedia(): Promise<unknown | undefined> {
  if (!inTauri()) return undefined;
  try {
    return await invoke<unknown>("editor_get_media");
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("[editor] editor_get_media failed:", err);
    return undefined;
  }
}

/** The outcome of an `editorEdit` dispatch. */
export interface EditResult {
  ok: boolean;
  /** The tool's echoed result JSON on success (often null — refetch for state). */
  value?: unknown;
  /** The tool error string on failure (surfaced to the caller). */
  error?: string;
}

/**
 * Dispatch a mutating tool through the SHARED executor (the same owner the MCP server
 * + in-app agent use). `name` is the tool wire name (`add_clips`, `move_clips`,
 * `split_clip`, `remove_clips`, `set_clip_properties`, `ripple_delete_ranges`,
 * `undo`, `import_media`, `create_folder`, …); `args` its inputSchema-shaped arguments.
 *
 * On success the backend emits `timeline://changed` so every window refetches — the
 * caller should NOT mutate local state optimistically; await this then refetch.
 * Outside Tauri it is a no-op success (the fixture-backed editor has nothing to edit).
 */
export async function editorEdit(
  name: string,
  args: Record<string, unknown>,
): Promise<EditResult> {
  if (!inTauri()) return { ok: true };
  try {
    const value = await invoke<unknown>("editor_edit", { name, args });
    return { ok: true, value };
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error(`[editor] editor_edit '${name}' failed:`, err);
    return { ok: false, error: String(err) };
  }
}

/** The outcome of an `importMedia` call (mirrors the Rust `editor_import_media`). */
export interface ImportResult {
  /** Number of assets imported (0 when the user cancelled the native dialog). */
  imported: number;
  /** Per-asset confirmation echoes from the `import_media` tool. */
  assets: unknown[];
}

/**
 * Import media into the shared library (File → Import Media / panel drop / Import
 * button). With no `paths`, the backend opens a NATIVE multi-select file dialog
 * (cancel ⇒ `{ imported: 0 }`, a no-op). With `paths` (the absolute paths of an OS
 * file-drop), it imports those directly.
 *
 * The backend imports each path through the SAME shared executor the agent/MCP use,
 * then emits `timeline://changed` so the Project surface refetches the media library
 * automatically — the caller need not refetch. Outside Tauri it is a no-op.
 *
 * `folderId` optionally targets a media-library folder (the media panel passes its
 * current folder so a drop lands where the user is browsing).
 */
export async function importMedia(
  paths?: string[],
  folderId?: string,
): Promise<ImportResult> {
  if (!inTauri()) return { imported: 0, assets: [] };
  try {
    return await invoke<ImportResult>("editor_import_media", {
      paths: paths ?? null,
      folderId: folderId ?? null,
    });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("[editor] editor_import_media failed:", err);
    return { imported: 0, assets: [] };
  }
}

/**
 * Repoint a missing asset's source to a new on-disk path (the Media panel's Relink
 * affordance) via `editor_relink_media`. Persists into the shared library so the
 * repointed path survives save/reload; the backend emits `timeline://changed` so the
 * panel refetches. Outside Tauri it is a no-op. Errors are logged, not thrown.
 */
export async function relinkMedia(assetId: string, newPath: string): Promise<void> {
  if (!inTauri()) return;
  try {
    await invoke("editor_relink_media", { assetId, newPath });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("[editor] editor_relink_media failed:", err);
  }
}

/**
 * Reparent media-library folders (the panel's in-panel folder drag) via
 * `editor_move_folders`. `targetFolderId` of `undefined`/`null` moves to the project
 * root. The backend applies the same cycle guards as the frontend and persists the
 * reparent into the shared library; it emits `timeline://changed` so the panel
 * refetches. Outside Tauri it is a no-op. Errors are logged, not thrown.
 */
export async function moveFolders(
  folderIds: string[],
  targetFolderId?: string,
): Promise<void> {
  if (!inTauri() || folderIds.length === 0) return;
  try {
    await invoke("editor_move_folders", {
      folderIds,
      targetFolderId: targetFolderId ?? null,
    });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("[editor] editor_move_folders failed:", err);
  }
}

/**
 * Subscribe to `timeline://changed`; `handler` runs after any mutation (UI or
 * agent/MCP). Returns an unlisten fn (a no-op outside Tauri). The Project surface
 * uses this to refetch the timeline + media instead of polling.
 */
export async function onTimelineChanged(
  handler: () => void,
): Promise<UnlistenFn> {
  if (!inTauri()) return () => {};
  return listen<unknown>(TIMELINE_CHANGED_EVENT, () => handler());
}
