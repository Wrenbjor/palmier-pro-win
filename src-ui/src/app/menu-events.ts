// Main-menu event glue (E1-S3).
//
// The Rust side (`crates/palmier-tauri/src/menu.rs`) emits `menu://<command-id>`
// Tauri events for every editor-action / Help / app menu item whose handler
// lives in the frontend (or a later epic). This module subscribes to that event
// family and routes each to a handler.
//
// Handlers whose owning epic has NOT landed yet are registered as **logged
// no-ops** here: that keeps every §6.1 menu binding provably invokable now (the
// event fires and is consumed) while the real behavior arrives with its story.
// As later stories land (editor undo/split/trim, Save/Export, Settings/Help
// windows), they replace the matching no-op with the real handler.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/// Every menu command id the Rust menu can emit (mirrors `MENU_TABLE` ids in
/// `menu.rs`). Kept in sync with the Rust side; the parity test there is the
/// source of truth for the full set.
// NOTE: only the **Event-dispatched** menu ids appear here. Window/app items
// (Settings, Check for Updates, Help tabs, Feedback, Quit, fullscreen) are handled
// natively in Rust (`menu.rs` `Dispatch::Native`) and never emit `menu://<id>` — see
// E1-S4, which gave those items real windows. The set below mirrors the `Dispatch::Event`
// rows in `crates/palmier-tauri/src/menu.rs` MENU_TABLE.
export const MENU_COMMAND_IDS = [
  // Palmier Pro
  "about",
  // File
  "new",
  "open",
  "save",
  "save-as",
  "import-media",
  "export",
  // Edit
  "undo",
  "redo",
  "cut",
  "copy",
  "paste",
  "select-all",
  "split",
  "trim-start",
  "trim-end",
  "delete",
  // View
  "toggle-media-panel",
  "toggle-inspector",
  "toggle-agent-panel",
  "maximize-panel",
  "layout-default",
  "layout-media",
  "layout-vertical",
  // Help
  "tutorial",
] as const;

export type MenuCommandId = (typeof MENU_COMMAND_IDS)[number];

/// A handler for a menu command. Receives the command id so one handler can
/// cover several related commands (e.g. the three layout presets).
export type MenuHandler = (id: MenuCommandId) => void;

/// Default handler table: a logged no-op for every command. Later stories
/// override entries here (or pass `overrides` to `registerMenuHandlers`) with the
/// real behavior. The no-op is deliberate — it proves the binding is invokable.
function defaultHandlers(): Record<MenuCommandId, MenuHandler> {
  const noop =
    (id: MenuCommandId): MenuHandler =>
    () => {
      // eslint-disable-next-line no-console
      console.debug(
        `[menu] '${id}' invoked — no-op stub (owning story not yet landed)`,
      );
    };
  const table = {} as Record<MenuCommandId, MenuHandler>;
  for (const id of MENU_COMMAND_IDS) table[id] = noop(id);
  return table;
}

/// Subscribe to every `menu://<id>` event and dispatch to the handler table.
///
/// `overrides` lets a caller (or a later story's module) supply real handlers
/// for the commands it owns; everything else falls back to the logged no-op.
///
/// Returns an unlisten function that detaches all listeners (call on teardown).
export async function registerMenuHandlers(
  overrides: Partial<Record<MenuCommandId, MenuHandler>> = {},
): Promise<UnlistenFn> {
  const handlers = { ...defaultHandlers(), ...overrides };

  const unlisteners = await Promise.all(
    MENU_COMMAND_IDS.map((id) =>
      listen(`menu://${id}`, () => {
        handlers[id](id);
      }),
    ),
  );

  return () => {
    for (const un of unlisteners) un();
  };
}
