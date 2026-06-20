// User + agent undo stacks over timeline snapshots (E3-S10, mirrors palmier-history).
//
// The FRONTEND mirror of `palmier-history` (E3-S8). It implements the reference's
// `withTimelineSwap` pattern (whole-`Timeline` before/after snapshot, atomic) with two
// SEPARATE stacks — User and Agent — so Ctrl+Z/Ctrl+Shift+Z drive ONLY the user stack
// and the agent's undo never tangles with the user's (FOUNDATION §1.5/§6.14;
// edit-engines.md lines 193-196). Each group carries an `actionName`; the agent `undo`
// tool (Epic 7) will refuse unless `currentUndoActionName()` matches — so we expose it
// now (carry-forward note in the epic, E3-S8 acceptance).
//
// Until E7 lands the snapshot IS the timeline JSON (cheap structural clone); when the
// real history crate is authoritative, `controller.ts` swaps these calls for Tauri
// `undo`/`redo` commands — the stack semantics here match so the seam is mechanical.

import type { TimelineView } from "./types";
import type { EditOrigin } from "./edit-types";

interface UndoEntry {
  /** Timeline state BEFORE the edit (restored on undo). */
  before: TimelineView;
  /** Timeline state AFTER the edit (restored on redo). */
  after: TimelineView;
  actionName: string;
}

interface Stack {
  undo: UndoEntry[];
  redo: UndoEntry[];
}

function emptyStack(): Stack {
  return { undo: [], redo: [] };
}

export class TimelineHistory {
  private user = emptyStack();
  private agent = emptyStack();
  /** Depth counter so nested swaps coalesce into ONE user-visible entry. */
  private nestDepth = 0;

  private stackFor(origin: EditOrigin): Stack {
    return origin === "agent" ? this.agent : this.user;
  }

  /**
   * Register a before/after swap. Returns false (no-op) when called nested — the
   * outermost `withTimelineSwap` owns the single coalesced entry (edit-engines.md
   * lines 248-249).
   */
  register(
    before: TimelineView,
    after: TimelineView,
    actionName: string,
    origin: EditOrigin = "user",
  ): boolean {
    if (this.nestDepth > 0) return false;
    const stack = this.stackFor(origin);
    stack.undo.push({
      before: structuredClone(before),
      after: structuredClone(after),
      actionName,
    });
    stack.redo.length = 0; // a new edit invalidates redo
    return true;
  }

  /**
   * Run `fn` as one atomic, coalesced edit: nested `withTimelineSwap` calls inside `fn`
   * do not register their own entries. Use for composite orchestration (move = pull +
   * clear-region + drop) so it is a single undo step.
   */
  withTimelineSwap<T>(
    before: TimelineView,
    actionName: string,
    fn: () => { after: TimelineView; result: T },
    origin: EditOrigin = "user",
  ): { after: TimelineView; result: T } {
    const outermost = this.nestDepth === 0;
    this.nestDepth += 1;
    try {
      const { after, result } = fn();
      if (outermost) {
        // temporarily exit nesting so register() actually records.
        this.nestDepth -= 1;
        this.register(before, after, actionName, origin);
        this.nestDepth += 1;
      }
      return { after, result };
    } finally {
      this.nestDepth -= 1;
    }
  }

  /** Undo the top of a stack. Returns the timeline to restore, or null if empty. */
  undoTop(origin: EditOrigin = "user"): TimelineView | null {
    const stack = this.stackFor(origin);
    const entry = stack.undo.pop();
    if (!entry) return null;
    stack.redo.push(entry);
    return structuredClone(entry.before);
  }

  /** Redo the top of a stack. Returns the timeline to restore, or null if empty. */
  redoTop(origin: EditOrigin = "user"): TimelineView | null {
    const stack = this.stackFor(origin);
    const entry = stack.redo.pop();
    if (!entry) return null;
    stack.undo.push(entry);
    return structuredClone(entry.after);
  }

  canUndo(origin: EditOrigin = "user"): boolean {
    return this.stackFor(origin).undo.length > 0;
  }

  canRedo(origin: EditOrigin = "user"): boolean {
    return this.stackFor(origin).redo.length > 0;
  }

  /**
   * The action name of the last-pushed undo group on a stack (or null). Epic 7's agent
   * `undo` tool refuses unless this matches the name it pushed — exposed now per the
   * E3-S8 carry-forward note.
   */
  currentUndoActionName(origin: EditOrigin = "user"): string | null {
    const stack = this.stackFor(origin);
    return stack.undo[stack.undo.length - 1]?.actionName ?? null;
  }

  clear(): void {
    this.user = emptyStack();
    this.agent = emptyStack();
    this.nestDepth = 0;
  }
}
