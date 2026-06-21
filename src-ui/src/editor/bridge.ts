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
