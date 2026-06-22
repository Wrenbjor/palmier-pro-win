// Edit intent + value types for the optimistic local edit layer (E3-S10).
//
// These mirror `palmier-model`'s edit value types (`FrameRange`, `ClipShift`,
// `TimelineRangeSelection`) and define the COMMAND SEAM the controller dispatches
// through: `EditIntent`. Today `applyEdit(intent)` (in `apply.ts`) mutates the local
// `TimelineView` optimistically, mirroring the `palmier-edit` drag-state /
// orchestration semantics. When the real Tauri edit commands land (Epic 7), the seam
// is the single replacement point — see the `// TODO(E7)` in `controller.ts`.
//
// Half-open ranges `[start, end)` and ties-away rounding match the Rust crates
// (edit-engines.md lines 26, 19-21).

/** Half-open `[start, end)` frame range. `length = end - start`. */
export interface FrameRange {
  start: number;
  end: number;
}

export function rangeLength(r: FrameRange): number {
  return r.end - r.start;
}

/** `contains(frame)` is half-open: `frame >= start && frame < end`. */
export function rangeContains(r: FrameRange, frame: number): boolean {
  return frame >= r.start && frame < r.end;
}

/** A computed clip relocation: move clip `clipId` to `newStartFrame`. */
export interface ClipShift {
  clipId: string;
  newStartFrame: number;
}

/**
 * The edit commands the input controller can issue. This is the command seam:
 * the controller builds an `EditIntent` from a gesture, and `applyEdit` (local) OR a
 * future Tauri `edit` command (E7) consumes it. Keep this serde-friendly (plain data)
 * so it can cross the Tauri bridge unchanged.
 */
export type EditIntent =
  | {
      kind: "move";
      /** Clip IDs being moved (lead first). */
      clipIds: string[];
      /** Lead clip id (anchors the delta). */
      leadId: string;
      /** Timeline-frame delta applied to the lead (others ride the same delta). */
      frameDelta: number;
      /**
       * Per-clip destination track index. Clips absent from the map keep their track
       * (pinned companions). The lead's entry drives `clampedTrackDelta` upstream.
       */
      trackForClip: Record<string, number>;
      /** Alt-drag duplicate instead of move. */
      duplicate: boolean;
    }
  | {
      kind: "trim";
      clipId: string;
      edge: "left" | "right";
      /** Timeline-frame delta on the dragged edge (already clamped + snapped). */
      deltaFrames: number;
      /** Apply the same source-delta to linked partners (on unless Alt held). */
      propagateToLinked: boolean;
    }
  | {
      kind: "split";
      /** The clip under the cut (or any member of its link group). */
      clipId: string;
      /** Timeline frame to cut at. */
      atFrame: number;
    }
  | {
      kind: "rippleDeleteRange";
      /** Anchor track index the range was drawn on. */
      trackIndex: number;
      /** Ranges to remove (half-open). Merged + length-filtered downstream. */
      ranges: FrameRange[];
    }
  | {
      kind: "deleteClips";
      clipIds: string[];
      /** Ripple-close the gap (shift later clips left) vs leave a hole. */
      ripple: boolean;
    }
  | {
      kind: "setClipProperties";
      clipIds: string[];
      /** Static linear volume (audio rubber-band drag-to-set). */
      volume?: number;
      /** Static opacity 0..1 (video opacity-line drag-to-set). */
      opacity?: number;
    }
  | {
      kind: "setKeyframes";
      clipId: string;
      /** Animatable property name (`set_keyframes` wire form), e.g. "volume"/"opacity". */
      property: "volume" | "opacity";
      /**
       * The REPLACEMENT keyframe rows: `[frame, value, interp?]` (frames CLIP-RELATIVE,
       * value in the property's native units — dB for volume, 0..1 for opacity). The
       * tool REPLACES the whole track, so this carries the merged+sorted full list.
       */
      keyframes: (readonly [number, number] | readonly [number, number, string])[];
    };

/** Which undo stack an edit registers on (user vs agent — E3-S8 separation). */
export type EditOrigin = "user" | "agent";
