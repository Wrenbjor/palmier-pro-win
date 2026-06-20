// Drag-state machine + clamping as pure transitions (E3-S10, mirrors palmier-edit::drag).
//
// FRONTEND mirror of `palmier-edit::drag` (E3-S7), ported from the macOS
// `Timeline/DragState.swift` + `Timeline/TimelineInputController.swift`. These are PURE
// functions driven by pointer events — no canvas, no React — so the move/trim/marquee
// behavior is testable in isolation. The interactive component (`TimelineEditor.tsx`)
// calls these; the Rust crate is authoritative once E7 lands.
//
// Modifier mapping (edit-engines.md line 190): macOS Cmd → Ctrl, Option → Alt.
// Sub-mode hit by localX: ≤4 trimLeft, ≥width-4 trimRight, else moveClip. Alt body grab
// → duplicate. Two move-snap probes per dragged clip (both edges). Marquee threshold 3.

import type { ClipType, ClipView, TimelineView } from "./types";
import {
  type Rect,
  type TimelineLayout,
  clipRect,
  endFrame,
  frameAt,
} from "./geometry";
import { Layout, Trim } from "./theme";
import { isCompatible } from "./apply";

/** Pointer-event modifiers (already mapped: Cmd→ctrl, Option→alt). */
export interface Modifiers {
  shift: boolean;
  ctrl: boolean;
  alt: boolean;
}

export type DragState =
  | { kind: "idle" }
  | { kind: "scrubPlayhead" }
  | {
      kind: "moveClip";
      leadId: string;
      clipIds: string[];
      /** Lead's original start frame. */
      leadOriginalFrame: number;
      /** Min original start frame across movers (floor for frameDelta). */
      minOriginalFrame: number;
      originStartFrame: number;
      originY: number;
      duplicate: boolean;
      /** Per-clip original track index (for pinned-companion logic on drop). */
      originalTrackOf: Record<string, number>;
    }
  | {
      kind: "trimLeft" | "trimRight";
      clipId: string;
      originalStartFrame: number;
      originalEndFrame: number;
    }
  | {
      kind: "marquee";
      originX: number;
      originY: number;
      baseSelection: string[];
    }
  | {
      kind: "timelineRange";
      anchorFrame: number;
    };

export type SubMode = "trimLeft" | "trimRight" | "moveClip";

/** Decide the drag sub-mode inside a clip from localX (handle width 4). */
export function subModeForLocalX(localX: number, clipWidth: number): SubMode {
  if (localX <= Trim.handleWidth) return "trimLeft";
  if (localX >= clipWidth - Trim.handleWidth) return "trimRight";
  return "moveClip";
}

/** Front-most clip hit at content-space (px, py). Last match wins on overlap. */
export function hitTestClip(
  timeline: TimelineView,
  layout: TimelineLayout,
  px: number,
  py: number,
): { clip: ClipView; trackIndex: number; rect: Rect } | null {
  let found: { clip: ClipView; trackIndex: number; rect: Rect } | null = null;
  for (let ti = 0; ti < timeline.tracks.length; ti++) {
    for (const clip of timeline.tracks[ti].clips) {
      const r = clipRect(layout, clip, ti);
      if (px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h) {
        found = { clip, trackIndex: ti, rect: r };
      }
    }
  }
  return found;
}

/**
 * Move-drag snap probes: TWO offsets per dragged clip — its start offset and
 * `start + durationFrames`, relative to the lead's original frame — so any edge of any
 * selected clip can snap (edit-engines.md lines 145-147).
 */
export function moveProbeOffsets(
  movers: ClipView[],
  leadOriginalFrame: number,
): number[] {
  const offsets: number[] = [];
  for (const clip of movers) {
    const startOffset = clip.startFrame - leadOriginalFrame;
    offsets.push(startOffset);
    offsets.push(startOffset + clip.durationFrames);
  }
  return offsets;
}

/** frame_delta = max(-minOriginalFrame, deltaFrames) — no clip before frame 0. */
export function clampFrameDelta(deltaFrames: number, minOriginalFrame: number): number {
  return Math.max(-minOriginalFrame, deltaFrames);
}

/**
 * Step a track delta toward 0 until ALL movers land on a type-compatible destination
 * track (or run out of tracks). Returns the clamped delta.
 * `moverTypes`/`moverTracks` are aligned arrays of each mover's type + source track.
 */
export function clampedTrackDelta(
  rawDelta: number,
  moverTypes: ClipType[],
  moverTracks: number[],
  destTrackTypes: ClipType[],
): number {
  const trackCount = destTrackTypes.length;
  const step = rawDelta === 0 ? 0 : rawDelta > 0 ? -1 : 1;
  let delta = rawDelta;
  // Walk toward zero; accept the first delta where every mover fits.
  for (;;) {
    let allFit = true;
    for (let i = 0; i < moverTracks.length; i++) {
      const dest = moverTracks[i] + delta;
      if (dest < 0 || dest >= trackCount) {
        allFit = false;
        break;
      }
      if (!isCompatible(moverTypes[i], destTrackTypes[dest])) {
        allFit = false;
        break;
      }
    }
    if (allFit) return delta;
    if (delta === 0 || step === 0) return 0;
    delta += step;
  }
}

/**
 * Pinned companions: linked partners of the lead, OR co-selected clips whose type is
 * incompatible with the lead's DESTINATION track type. These hold their own row on a
 * cross-track move (edit-engines.md lines 167-170, 230-231).
 */
export function pinnedCompanions(
  movers: ClipView[],
  leadId: string,
  leadLinkGroupId: string | null | undefined,
  destTypeOfLead: ClipType,
): Set<string> {
  const pinned = new Set<string>();
  for (const clip of movers) {
    if (clip.id === leadId) continue;
    const linked = !!leadLinkGroupId && clip.linkGroupId === leadLinkGroupId;
    const incompatible = !isCompatible(clip.mediaType, destTypeOfLead);
    if (linked || incompatible) pinned.add(clip.id);
  }
  return pinned;
}

/** Marquee rect (min/max of origin↔point). */
export function marqueeRect(originX: number, originY: number, px: number, py: number): Rect {
  return {
    x: Math.min(originX, px),
    y: Math.min(originY, py),
    w: Math.abs(px - originX),
    h: Math.abs(py - originY),
  };
}

/** A marquee cancels gap selection once it exceeds dragThreshold (3). */
export function marqueeExceedsThreshold(rect: Rect): boolean {
  return rect.w > Layout.dragThreshold || rect.h > Layout.dragThreshold;
}

function rectsIntersect(a: Rect, b: Rect): boolean {
  return (
    a.x < b.x + b.w &&
    a.x + a.w > b.x &&
    a.y < b.y + b.h &&
    a.y + a.h > b.y
  );
}

/**
 * Marquee selection = baseSelection ∪ {clips whose clipRect intersects the marquee},
 * optionally expanded to link groups (unless Alt).
 */
export function marqueeSelect(
  timeline: TimelineView,
  layout: TimelineLayout,
  rect: Rect,
  baseSelection: string[],
  expandLinks: boolean,
): string[] {
  const selected = new Set(baseSelection);
  const linkGroupsToExpand = new Set<string>();
  for (let ti = 0; ti < timeline.tracks.length; ti++) {
    for (const clip of timeline.tracks[ti].clips) {
      const r = clipRect(layout, clip, ti);
      if (rectsIntersect(rect, r)) {
        selected.add(clip.id);
        if (expandLinks && clip.linkGroupId) linkGroupsToExpand.add(clip.linkGroupId);
      }
    }
  }
  if (expandLinks && linkGroupsToExpand.size > 0) {
    for (const track of timeline.tracks) {
      for (const clip of track.clips) {
        if (clip.linkGroupId && linkGroupsToExpand.has(clip.linkGroupId)) {
          selected.add(clip.id);
        }
      }
    }
  }
  return [...selected];
}

/** Expand a set of clip ids to include every member of their link groups. */
export function expandToLinkGroup(timeline: TimelineView, ids: Iterable<string>): string[] {
  const seed = new Set(ids);
  const groups = new Set<string>();
  for (const track of timeline.tracks) {
    for (const clip of track.clips) {
      if (seed.has(clip.id) && clip.linkGroupId) groups.add(clip.linkGroupId);
    }
  }
  if (groups.size === 0) return [...seed];
  for (const track of timeline.tracks) {
    for (const clip of track.clips) {
      if (clip.linkGroupId && groups.has(clip.linkGroupId)) seed.add(clip.id);
    }
  }
  return [...seed];
}

/** Convenience: the timeline frame at a content-space x. */
export function frameAtX(layout: TimelineLayout, x: number): number {
  return frameAt(layout, x);
}

/** endFrame re-export so the controller imports drag math from one place. */
export { endFrame };
