// OS-level media-panel actions (E4-S12) — the Tauri command seam for Reveal in
// Explorer / Copy Path / Relink / clipboard paste. Mirrors `src-ui/app/api.ts`:
// thin typed wrappers over `@tauri-apps/api` `invoke` that DEGRADE GRACEFULLY
// outside a Tauri webview (plain `vite dev` / a browser) so the panel still renders.
//
// Reference → Windows/Linux mapping (media-panel.md §"macOS APIs to replace"):
//   NSWorkspace.activateFileViewerSelecting (Reveal in Finder)
//     → Rust `reveal_in_explorer` (Windows `explorer /select,<path>`; Linux
//       `tauri-plugin-opener` reveal-item-in-dir / xdg-open parent).
//   NSPasteboard Copy-Path → Rust `copy_paths_to_clipboard` (newline-joined).
//   NSOpenPanel Relink → the `dialog` plugin's open picker (E1-S7 pattern).
//   NSPasteboard paste → `read_clipboard_importable_paths` (file URLs the paste
//     menu imports; image-data paste lands with the real import at Epic 7).
//
// The Rust side of these commands lives in `crates/palmier-tauri/src/media.rs`; the
// dialog relink reuses the already-wired `tauri-plugin-dialog`.

import { invoke } from "@tauri-apps/api/core";

/** True when running inside a Tauri webview (vs plain `vite dev` / a browser). */
export function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function tryInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T | undefined> {
  if (!inTauri()) return undefined;
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error(`[media-actions] invoke '${cmd}' failed:`, err);
    return undefined;
  }
}

/**
 * Reveal a file in the OS file manager, selecting it (Windows Explorer
 * `/select,`; Linux file-manager show-item / parent open). No-op outside Tauri.
 */
export const revealInExplorer = (path: string) =>
  tryInvoke<void>("reveal_in_explorer", { path });

/**
 * Copy one or more absolute paths to the system clipboard, newline-joined
 * (reference Copy-Path writes newline-joined paths). No-op outside Tauri.
 */
export const copyPathsToClipboard = (paths: string[]) =>
  tryInvoke<void>("copy_paths_to_clipboard", { paths });

/**
 * Open the OS file picker to repoint a missing asset (Relink). Returns the chosen
 * absolute path, or `null`/`undefined` on cancel / outside Tauri. The `name` seeds
 * the dialog title so the user knows which asset they are relinking.
 */
export const pickRelinkPath = (name: string) =>
  tryInvoke<string | null>("pick_relink_path", { name });

/**
 * Read importable file paths off the clipboard for paste (file-URL branch of the
 * reference `handleClipboardPaste`). Returns `[]` outside Tauri. Image-data paste
 * (`.png`/`.tiff` → written + imported) lands with the real import flow at Epic 7.
 */
export const readClipboardImportablePaths = async (): Promise<string[]> =>
  (await tryInvoke<string[]>("read_clipboard_importable_paths")) ?? [];

/**
 * The async "moment" thumbnail command seam (E4-S3 `thumbnail(media_ref,
 * source_seconds, max_size)`), keyed `path@time`, consumed by the search panel's
 * `MomentThumbnail`. Returns a data-URL, or `undefined` until the E4-S3 pipeline +
 * Epic 11 search land. No-op outside Tauri.
 *
 * Parity (MediaTab+Search.swift `MomentThumbnail.thumbnail`): the reference
 * `AVAssetImageGenerator` uses `maximumSize = 240×240` and a 1s tolerance
 * before/after the requested time (so the decoder snaps to the nearest sync frame).
 * `maxSize` defaults to 240 to match; the 1s seek tolerance is the FFmpeg backend's
 * responsibility (palmier-media `thumbnail` command).
 */
export const momentThumbnail = (
  path: string,
  sourceSeconds: number,
  maxSize = 240,
) =>
  tryInvoke<string | null>("thumbnail", {
    mediaRef: path,
    sourceSeconds,
    maxSize,
  });
