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
import { editorEdit, inTauri } from "./bridge";
import type { ClipView } from "./types";

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
    case "setClipProperties":
      return intent.volume !== undefined ? "Set Volume" : "Set Opacity";
    case "setKeyframes":
      return intent.property === "volume" ? "Set Volume Keyframe" : "Set Opacity Keyframe";
  }
}

export class EditController {
  readonly history = new TimelineHistory();

  constructor(private store: TimelineStore) {}

  /**
   * Dispatch an edit. Returns true if it mutated the timeline.
   *
   * WIRED (Project window): inside a Tauri webview the intent is ALSO translated to a
   * mutating tool and dispatched through the shared executor via `editor_edit`
   * (`bridge.ts`). The backend emits `timeline://changed`; the Project surface
   * refetches `editor_get_timeline` and `setTimeline`s the authoritative state, which
   * reconciles the optimistic apply below. The local `applyEdit` is kept for instant
   * feedback (and as the sole path outside Tauri / for `vite dev` design work).
   *
   * Undo/redo: in Tauri, undo routes to the backend `undo` tool (the agent undo stack
   * lives in palmier-tools); the local history mirrors it for snappy UI. Outside Tauri
   * the local history IS the undo system.
   */
  dispatch(intent: EditIntent, origin: EditOrigin = "user"): boolean {
    const before = this.store.getState().timeline;
    if (!before) return false;

    // One atomic, coalesced undo step per composite edit (optimistic local apply).
    const { after } = this.history.withTimelineSwap(
      before,
      actionName(intent),
      () => ({ after: applyEdit(before, intent), result: undefined }),
      origin,
    );

    this.store.setTimeline(after);

    // Backend dispatch (Project window): translate the intent to tool calls and run
    // them through the shared executor. The `timeline://changed` refetch reconciles.
    if (inTauri()) {
      void this.dispatchToBackend(intent, before);
    }
    return true;
  }

  /**
   * Translate an `EditIntent` to one or more mutating tool calls and dispatch them
   * through the shared executor. Absolute tool args are computed from `before` (the
   * pre-edit timeline) since the tools take absolute frames/tracks, not the gesture's
   * deltas. Best-effort: a tool error is logged; the `timeline://changed` refetch (or
   * its absence) is the source of truth.
   */
  private async dispatchToBackend(
    intent: EditIntent,
    before: TimelineView,
  ): Promise<void> {
    switch (intent.kind) {
      case "split":
        await editorEdit("split_clip", {
          clipId: intent.clipId,
          atFrame: intent.atFrame,
        });
        return;

      case "deleteClips":
        // Both ripple + non-ripple deletes map to remove_clips (the backend closes
        // the gap per its own ripple semantics on linked/sync-locked tracks).
        await editorEdit("remove_clips", { clipIds: intent.clipIds });
        return;

      case "rippleDeleteRange": {
        const ranges = intent.ranges.map((r) => [r.start, r.end]);
        await editorEdit("ripple_delete_ranges", {
          trackIndex: intent.trackIndex,
          ranges,
          units: "frames",
        });
        return;
      }

      case "move": {
        // Build a move per clip: absolute toFrame = currentStart + frameDelta; toTrack
        // from the per-clip destination map. (Duplicate-move has no direct tool; it
        // falls back to the optimistic local apply only — a follow-up seam.)
        if (intent.duplicate) return;
        await editorEdit("move_clips", { moves: buildMoveClipsArgs(intent, before) });
        return;
      }

      case "trim": {
        // Resolve the clip's pre-edit geometry to convert the edge delta into absolute
        // properties. Right edge → durationFrames (+ trimEnd shrinks). Left edge →
        // startFrame moves (move_clips) AND duration/trimStart change
        // (set_clip_properties), since set_clip_properties cannot move the start.
        const clip = findClip(before, intent.clipId);
        if (!clip) return;
        if (intent.edge === "right") {
          const newDuration = Math.max(
            1,
            clip.durationFrames + intent.deltaFrames,
          );
          await editorEdit("set_clip_properties", {
            clipIds: [intent.clipId],
            durationFrames: newDuration,
          });
        } else {
          // Left trim: new start = oldStart + delta; new duration shrinks by delta;
          // trimStart advances by delta × speed (source frames consumed).
          const newStart = clip.startFrame + intent.deltaFrames;
          const newDuration = Math.max(
            1,
            clip.durationFrames - intent.deltaFrames,
          );
          const newTrimStart =
            clip.trimStartFrame +
            Math.round(intent.deltaFrames * (clip.speed || 1));
          await editorEdit("set_clip_properties", {
            clipIds: [intent.clipId],
            durationFrames: newDuration,
            trimStartFrame: Math.max(0, newTrimStart),
          });
          await editorEdit("move_clips", {
            moves: [{ clipId: intent.clipId, toFrame: Math.max(0, newStart) }],
          });
        }
        return;
      }

      case "setClipProperties": {
        // Drag-to-set a flat envelope level. Volume is linear (audio rubber band);
        // opacity is 0..1 (video opacity line). Mirrors the inspector Audio/Video tabs.
        const patch: Record<string, unknown> = { clipIds: intent.clipIds };
        if (intent.volume !== undefined) patch.volume = intent.volume;
        if (intent.opacity !== undefined) patch.opacity = intent.opacity;
        await editorEdit("set_clip_properties", patch);
        return;
      }

      case "setKeyframes": {
        // Alt-drag inserted/updated a keyframe — REPLACE the property track with the
        // merged rows. `keyframes` is the full sorted `[frame, value, interp?]` list.
        await editorEdit("set_keyframes", {
          clipId: intent.clipId,
          property: intent.property,
          keyframes: intent.keyframes.map((r) => [...r]),
        });
        return;
      }

      default:
        return;
    }
  }

  /** Ctrl+Z — restore the previous user state. Returns true if something was undone. */
  undo(origin: EditOrigin = "user"): boolean {
    // In Tauri, the agent undo stack lives backend-side; route undo there too so a
    // UI undo and an agent undo share one timeline. The refetch reconciles the store.
    if (inTauri() && origin === "user") {
      void editorEdit("undo", {});
    }
    const restored = this.history.undoTop(origin);
    if (!restored) return false;
    this.store.setTimeline(restored);
    return true;
  }

  /** Ctrl+Shift+Z — re-apply the next user state. */
  redo(origin: EditOrigin = "user"): boolean {
    // The tool surface has no redo (the reference's agent stack is undo-only); redo
    // stays local-only for now (a follow-up seam if palmier-history grows redo).
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

/** One `move_clips` move entry (wire shape consumed by the `move_clips` tool). */
export interface MoveClipsArg {
  clipId: string;
  /** Absolute destination frame = current start + frameDelta. */
  toFrame: number;
  /** Absolute destination track (omitted when the clip keeps its row). */
  toTrack?: number;
}

/**
 * Translate a `move` intent into the absolute `move_clips` tool args, resolving each
 * clip's pre-edit start from `before`. Pure + exported so the input controller and the
 * parity checks share ONE implementation (no drift between what the UI dispatches and
 * what the test asserts).
 */
export function buildMoveClipsArgs(
  intent: Extract<EditIntent, { kind: "move" }>,
  before: TimelineView,
): MoveClipsArg[] {
  const startById = new Map<string, number>();
  for (const t of before.tracks) {
    for (const c of t.clips) startById.set(c.id, c.startFrame);
  }
  return intent.clipIds.map((clipId) => {
    const cur = startById.get(clipId) ?? 0;
    const m: MoveClipsArg = { clipId, toFrame: cur + intent.frameDelta };
    const toTrack = intent.trackForClip[clipId];
    if (toTrack !== undefined) m.toTrack = toTrack;
    return m;
  });
}

/** Find a clip by id across all tracks of a timeline (backend-arg geometry). */
function findClip(timeline: TimelineView, id: string): ClipView | null {
  for (const track of timeline.tracks) {
    const c = track.clips.find((cc) => cc.id === id);
    if (c) return c;
  }
  return null;
}
