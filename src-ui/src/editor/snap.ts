// Snap target collection + sticky snap finder (E3-S10, mirrors palmier-edit::snap).
//
// This is the FRONTEND mirror of `palmier-edit::snap` (E3-S5) — ported 1:1 from the
// macOS `Timeline/SnapEngine.swift` (`collectTargets`, `findSnap`) with the ONE side
// effect (`NSHapticFeedbackManager.perform(.alignment)`) STRIPPED, per
// `docs/reference/edit-engines.md` lines 32, 130-141, 183-185. The Rust crate is the
// source of truth once the Tauri bridge lands; this copy lets the input controller
// drive live snap before then and keeps the math unit-testable for parity.
//
// Constants (Constants.swift / theme.ts): base threshold 8px, playhead ×1.5, sticky
// ×1.5 (ruling #10 — NOT FOUNDATION's 2.5), pixelsPerFrame default 4.0.

import type { TimelineView } from "./types";
import { endFrame } from "./geometry";
import { Snap } from "./theme";

/** A snap target — a clip edge or the playhead, as a frame value. */
export interface SnapTarget {
  frame: number;
  kind: "clipEdge" | "playhead";
}

/** Mutable sticky-snap state carried across pointer-move events. */
export interface SnapState {
  /** The frame currently held via stickiness, or null. */
  currentlySnappedTo: number | null;
  /** Which probe offset produced the current snap. */
  currentProbeOffset: number;
}

/** Result of a successful snap. */
export interface SnapResult {
  /** The target frame snapped to. */
  frame: number;
  /** Which probe offset matched (so the caller can back out the lead delta). */
  probeOffset: number;
  /** Indicator x in content space (`frame * pixelsPerFrame`). */
  x: number;
}

export function makeSnapState(): SnapState {
  return { currentlySnappedTo: null, currentProbeOffset: 0 };
}

/**
 * collectTargets — every non-excluded clip's start AND end frame as `clipEdge`,
 * plus the playhead as `playhead` when `includePlayhead`.
 * (edit-engines.md lines 131-132)
 */
export function collectTargets(
  timeline: TimelineView,
  playheadFrame: number,
  excludeClipIds: ReadonlySet<string>,
  includePlayhead: boolean,
): SnapTarget[] {
  const targets: SnapTarget[] = [];
  for (const track of timeline.tracks) {
    for (const clip of track.clips) {
      if (excludeClipIds.has(clip.id)) continue;
      targets.push({ frame: clip.startFrame, kind: "clipEdge" });
      targets.push({ frame: endFrame(clip), kind: "clipEdge" });
    }
  }
  if (includePlayhead) {
    targets.push({ frame: playheadFrame, kind: "playhead" });
  }
  return targets;
}

/**
 * findSnap — sticky snap finder with NO side effects (edit-engines.md lines 133-141).
 *
 * `position` is the candidate frame (e.g. the dragged lead's proposed frame).
 * `probeOffsets` are added to `position` to form probe positions (move drags supply
 * two offsets per participant — both edges; trims supply `[0]`).
 *
 * Sticky: if a snap is held and a probe is within `baseFrameThreshold * 1.5` of it
 * AND the target still exists → return the held snap. Else clear and search.
 * Playhead targets use a `* 1.5` wider catch radius (priority via radius).
 */
export function findSnap(
  position: number,
  probeOffsets: number[],
  targets: SnapTarget[],
  state: SnapState,
  baseThresholdPx: number,
  pixelsPerFrame: number,
): SnapResult | null {
  if (pixelsPerFrame <= 0) return null;
  const baseFrameThreshold = baseThresholdPx / pixelsPerFrame;
  const stickyThreshold = baseFrameThreshold * Snap.stickyMultiplier;

  // --- Sticky: hold the existing snap if a probe is still within sticky range. ---
  if (state.currentlySnappedTo !== null) {
    const held = state.currentlySnappedTo;
    const stillExists = targets.some((t) => t.frame === held);
    if (stillExists) {
      for (const offset of probeOffsets) {
        const probePos = position + offset;
        if (Math.abs(probePos - held) <= stickyThreshold) {
          state.currentProbeOffset = offset;
          return { frame: held, probeOffset: offset, x: held * pixelsPerFrame };
        }
      }
    }
    // Probe left the sticky radius (or target gone) → release.
    state.currentlySnappedTo = null;
  }

  // --- Find the closest target within threshold across all probe offsets. ---
  let best: SnapResult | null = null;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const offset of probeOffsets) {
    const probePos = position + offset;
    for (const target of targets) {
      const threshold =
        target.kind === "playhead"
          ? baseFrameThreshold * Snap.playheadMultiplier
          : baseFrameThreshold;
      const distance = Math.abs(probePos - target.frame);
      if (distance <= threshold && distance < bestDistance) {
        bestDistance = distance;
        best = {
          frame: target.frame,
          probeOffset: offset,
          x: target.frame * pixelsPerFrame,
        };
      }
    }
  }

  if (best) {
    state.currentlySnappedTo = best.frame;
    state.currentProbeOffset = best.probeOffset;
  }
  return best;
}
