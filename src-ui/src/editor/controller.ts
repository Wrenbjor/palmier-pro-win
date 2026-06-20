// Edit controller: the command seam wiring gestures → edits → store + history (E3-S10).
//
// This is the single place edits are dispatched. The interactive component
// (`TimelineEditor.tsx`) calls `EditController.dispatch(intent)`; the controller:
//   1. snapshots the current timeline (history `before`),
//   2. applies the edit OPTIMISTICALLY against the local store via `applyEdit`,
//   3. registers the before/after on the user (or agent) undo stack.
//
// COMMAND SEAM — this is the boundary the epic asks for. `applyEdit(intent)` is the
// local optimistic apply. When the real Tauri edit commands land (Epic 7) they REPLACE
// the local apply: the controller will `await invoke('edit', { intent })`, then refresh
// via `get_timeline`, instead of calling `applyEdit` + writing the store directly. The
// `EditIntent` shape is already serde-friendly so it crosses the bridge unchanged.

import type { TimelineView } from "./types";
import type { EditIntent, EditOrigin } from "./edit-types";
import { applyEdit } from "./apply";
import { TimelineHistory } from "./history";
import type { TimelineStore } from "./store";

/** Human-readable undo-group names per intent kind (drives currentUndoActionName). */
function actionName(intent: EditIntent): string {
  switch (intent.kind) {
    case "move":
      return intent.duplicate ? "Duplicate Clips" : "Move Clips";
    case "trim":
      return intent.edge === "left" ? "Trim Clip Start" : "Trim Clip End";
    case "split":
      return "Split Clip";
    case "rippleDeleteRange":
      return "Ripple Delete Range";
    case "deleteClips":
      return intent.ripple ? "Ripple Delete" : "Delete Clips";
  }
}

export class EditController {
  readonly history = new TimelineHistory();

  constructor(private store: TimelineStore) {}

  /**
   * Dispatch an edit. Returns true if it mutated the timeline.
   *
   * TODO(E7): route through Tauri get_timeline/edit commands. Replace the
   * `applyEdit` + `setTimeline` below with:
   *   await invoke('edit', { intent });
   *   const next = adaptTimeline(await invoke('get_timeline'));
   *   this.store.setTimeline(next);
   * and let `palmier-history` own undo (call invoke('undo')/invoke('redo')). The
   * before/after snapshotting here is the local stand-in for that crate.
   */
  dispatch(intent: EditIntent, origin: EditOrigin = "user"): boolean {
    const before = this.store.getState().timeline;
    if (!before) return false;

    // One atomic, coalesced undo step per composite edit.
    const { after } = this.history.withTimelineSwap(
      before,
      actionName(intent),
      () => ({ after: applyEdit(before, intent), result: undefined }),
      origin,
    );

    this.store.setTimeline(after);
    return true;
  }

  /** Ctrl+Z — restore the previous user state. Returns true if something was undone. */
  undo(origin: EditOrigin = "user"): boolean {
    const restored = this.history.undoTop(origin);
    if (!restored) return false;
    this.store.setTimeline(restored);
    return true;
  }

  /** Ctrl+Shift+Z — re-apply the next user state. */
  redo(origin: EditOrigin = "user"): boolean {
    const restored = this.history.redoTop(origin);
    if (!restored) return false;
    this.store.setTimeline(restored);
    return true;
  }

  canUndo(origin: EditOrigin = "user"): boolean {
    return this.history.canUndo(origin);
  }

  canRedo(origin: EditOrigin = "user"): boolean {
    return this.history.canRedo(origin);
  }

  /** Exposed for Epic 7 / SM-4 agent-undo refuse-after-user-edit enforcement. */
  currentUndoActionName(origin: EditOrigin = "user"): string | null {
    return this.history.currentUndoActionName(origin);
  }

  /** Snapshot accessor used by tests / verification. */
  snapshot(): TimelineView | null {
    return this.store.getState().timeline;
  }
}
