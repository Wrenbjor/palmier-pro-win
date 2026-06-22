// Optimistic local edit application (E3-S10).
//
// This is the LOCAL stand-in for the `palmier-edit` + `palmier-history` engines until
// the Tauri `edit` / `get_timeline` commands land (Epic 7). It mirrors the reference
// orchestration semantics (`EditorViewModel+Ripple/+ClipMutations/+Linking`) closely
// enough that the timeline behaves like Premiere during a hand edit: move (cross-track,
// overwrite at destination), trim (in place, source↔timeline round-trip), split
// (clip-relative, keyframe migration, linked regroup), and ripple-delete-range.
//
// `applyEdit(timeline, intent)` is the COMMAND SEAM: it takes an immutable
// `TimelineView` + an `EditIntent` and returns a NEW `TimelineView` (structural copy,
// never a mutation of the input — so history can snapshot before/after). When E7 lands,
// `controller.ts` routes the same `EditIntent` through Tauri and replaces this call.
//
// Ported behavior + rounding (`f64::round` ties-away via `roundTiesAway`) follow
// `docs/reference/edit-engines.md`:
//   merge_ranges (touching merge, <=)            lines 54-56
//   compute_ripple_shifts_for_ranges             lines 57-60
//   compute_overwrite (clear-region)             lines 85-97
//   split_clip (clip-relative + kf migration)    lines 99-110
//   trim_values / round-trip                     lines 112-128
//   move_clips (pinned companions, overwrite)    lines 164-170
// All visual types interchangeable (ruling #12); audio its own zone. No Slip/Slide.

import type { ClipType, ClipView, KeyframeTrackView, TimelineView, TrackView } from "./types";
import { endFrame, roundTiesAway, sampleTrack } from "./geometry";
import type { ClipShift, EditIntent, FrameRange } from "./edit-types";
import { rangeLength } from "./edit-types";

// --- ID + structural-copy helpers ---

let uuidCounter = 0;
/** A frontend-local unique id for split right-fragments / regroups. */
export function localUuid(prefix = "clip"): string {
  uuidCounter += 1;
  const rand = Math.random().toString(36).slice(2, 10);
  return `${prefix}-${rand}-${uuidCounter}`;
}

function cloneClip(clip: ClipView): ClipView {
  return structuredClone(clip);
}

function cloneTimeline(t: TimelineView): TimelineView {
  return structuredClone(t);
}

const VISUAL: ReadonlySet<ClipType> = new Set<ClipType>(["video", "image", "text", "lottie"]);

/** isCompatible(a, b) = a==b || (isVisual(a) && isVisual(b)). Ruling #12. */
export function isCompatible(a: ClipType, b: ClipType): boolean {
  return a === b || (VISUAL.has(a) && VISUAL.has(b));
}

export function hasNoSourceMedia(type: ClipType): boolean {
  return type === "image" || type === "text";
}

// --- Clip lookup ---

interface ClipLoc {
  trackIndex: number;
  clipIndex: number;
  clip: ClipView;
}

function findClip(t: TimelineView, clipId: string): ClipLoc | null {
  for (let ti = 0; ti < t.tracks.length; ti++) {
    const ci = t.tracks[ti].clips.findIndex((c) => c.id === clipId);
    if (ci >= 0) return { trackIndex: ti, clipIndex: ci, clip: t.tracks[ti].clips[ci] };
  }
  return null;
}

function linkGroupMembers(t: TimelineView, linkGroupId: string | null | undefined): ClipView[] {
  if (!linkGroupId) return [];
  const out: ClipView[] = [];
  for (const track of t.tracks) {
    for (const clip of track.clips) {
      if (clip.linkGroupId === linkGroupId) out.push(clip);
    }
  }
  return out;
}

function sortTrackClips(track: TrackView): void {
  track.clips.sort((a, b) => a.startFrame - b.startFrame);
}

// =====================================================================================
// RippleEngine (pure) — edit-engines.md lines 54-64
// =====================================================================================

/** merge_ranges: sort by start; merge when `range.start <= last.end` (touching merges). */
export function mergeRanges(ranges: FrameRange[]): FrameRange[] {
  const sorted = [...ranges].sort((a, b) => a.start - b.start);
  const out: FrameRange[] = [];
  for (const r of sorted) {
    const last = out[out.length - 1];
    if (last && r.start <= last.end) {
      last.end = Math.max(last.end, r.end);
    } else {
      out.push({ start: r.start, end: r.end });
    }
  }
  return out;
}

/**
 * compute_ripple_shifts_for_ranges: for each clip (sorted by start), shift =
 * Σ length of merged ranges whose `r.end <= clip.startFrame`; emit a shift only when
 * `shift > 0` (a clip overlapping a gap is assumed already cleared).
 */
export function computeRippleShiftsForRanges(
  clips: ClipView[],
  removedRanges: FrameRange[],
): ClipShift[] {
  const merged = mergeRanges(removedRanges);
  const sorted = [...clips].sort((a, b) => a.startFrame - b.startFrame);
  const shifts: ClipShift[] = [];
  for (const clip of sorted) {
    let shift = 0;
    for (const r of merged) {
      if (r.end <= clip.startFrame) shift += rangeLength(r);
    }
    if (shift > 0) {
      shifts.push({ clipId: clip.id, newStartFrame: clip.startFrame - shift });
    }
  }
  return shifts;
}

// =====================================================================================
// OverwriteEngine (pure) — clear-region — edit-engines.md lines 85-97
// =====================================================================================

export type OverwriteAction =
  | { kind: "remove"; clipId: string }
  | { kind: "trimEnd"; clipId: string; newDuration: number }
  | { kind: "trimStart"; clipId: string; newStartFrame: number; newTrimStart: number; newDuration: number }
  | {
      kind: "split";
      clipId: string;
      leftDuration: number;
      rightStartFrame: number;
      rightTrimStart: number;
      rightDuration: number;
    };

/** compute_overwrite over one track's clips for `[regionStart, regionEnd)`. */
export function computeOverwrite(
  clips: ClipView[],
  regionStart: number,
  regionEnd: number,
): OverwriteAction[] {
  if (regionEnd <= regionStart) return [];
  const actions: OverwriteAction[] = [];
  for (const clip of clips) {
    const cs = clip.startFrame;
    const ce = endFrame(clip);
    if (ce <= regionStart || cs >= regionEnd) continue; // no overlap
    if (cs >= regionStart && ce <= regionEnd) {
      actions.push({ kind: "remove", clipId: clip.id });
    } else if (cs < regionStart && ce > regionEnd) {
      // region strictly inside → split (right fields advisory; VM re-derives)
      actions.push({
        kind: "split",
        clipId: clip.id,
        leftDuration: regionStart - cs,
        rightStartFrame: regionEnd,
        rightTrimStart: clip.trimStartFrame + roundTiesAway((regionEnd - cs) * clip.speed),
        rightDuration: ce - regionEnd,
      });
    } else if (cs < regionStart) {
      actions.push({ kind: "trimEnd", clipId: clip.id, newDuration: regionStart - cs });
    } else {
      actions.push({
        kind: "trimStart",
        clipId: clip.id,
        newStartFrame: regionEnd,
        newTrimStart: clip.trimStartFrame + roundTiesAway((regionEnd - cs) * clip.speed),
        newDuration: ce - regionEnd,
      });
    }
  }
  return actions;
}

// =====================================================================================
// Split (clip-relative + keyframe migration) — edit-engines.md lines 99-110
// =====================================================================================

function migrateTrackSplit(
  track: KeyframeTrackView | null | undefined,
  splitOffset: number,
): { left: KeyframeTrackView | null; right: KeyframeTrackView | null } {
  if (!track || track.keyframes.length === 0) return { left: null, right: null };
  const boundaryValue = sampleTrack(track, splitOffset, track.keyframes[0].value);

  const left = track.keyframes.filter((k) => k.frame <= splitOffset).map((k) => ({ ...k }));
  if (!left.some((k) => k.frame === splitOffset)) {
    left.push({ frame: splitOffset, value: boundaryValue, interpolationOut: "linear" });
    left.sort((a, b) => a.frame - b.frame);
  }

  const right = track.keyframes
    .filter((k) => k.frame >= splitOffset)
    .map((k) => ({ ...k, frame: k.frame - splitOffset }));
  if (!right.some((k) => k.frame === 0)) {
    right.unshift({ frame: 0, value: boundaryValue, interpolationOut: "linear" });
  }

  return { left: { keyframes: left }, right: { keyframes: right } };
}

/**
 * split_clip(clip, atFrame) → [left, right] or null when `at` is outside `(start, end)`.
 * Left keeps the id; right gets a new UUID. Volume + opacity/position/scale/crop tracks
 * migrate (keep / re-base / boundary-insert).
 */
export function splitClip(clip: ClipView, atFrame: number): [ClipView, ClipView] | null {
  const start = clip.startFrame;
  const end = endFrame(clip);
  if (!(start < atFrame && atFrame < end)) return null;

  const splitOffset = atFrame - start;
  const leftSource = roundTiesAway(splitOffset * clip.speed);
  const rightSource = roundTiesAway((clip.durationFrames - splitOffset) * clip.speed);

  const left = cloneClip(clip);
  left.durationFrames = splitOffset;
  left.trimEndFrame = clip.trimEndFrame + rightSource;
  left.fadeOutFrames = 0;
  left.fadeInFrames = Math.min(left.fadeInFrames, left.durationFrames);

  const right = cloneClip(clip);
  right.id = localUuid("clip");
  right.startFrame = atFrame;
  right.durationFrames = clip.durationFrames - splitOffset;
  right.trimStartFrame = clip.trimStartFrame + leftSource;
  right.fadeInFrames = 0;
  right.fadeOutFrames = Math.min(right.fadeOutFrames, right.durationFrames);

  for (const key of ["volumeTrack", "opacityTrack", "positionTrack", "scaleTrack", "cropTrack"] as const) {
    const migrated = migrateTrackSplit(clip[key], splitOffset);
    left[key] = migrated.left;
    right[key] = migrated.right;
  }

  return [left, right];
}

// =====================================================================================
// Trim (source↔timeline round-trip + clamp) — edit-engines.md lines 112-128, 236-238
// =====================================================================================

export interface TrimClamp {
  minDelta: number;
  maxDelta: number;
}

/** Trim-drag clamp for a clip edge (timeline-frame delta). */
export function trimClamp(clip: ClipView, edge: "left" | "right"): TrimClamp {
  const noSource = hasNoSourceMedia(clip.mediaType);
  if (edge === "left") {
    const maxDelta = clip.durationFrames - 1;
    const minDelta = noSource ? -clip.startFrame : -clip.trimStartFrame;
    return { minDelta, maxDelta };
  }
  const minDelta = -(clip.durationFrames - 1);
  const maxDelta = noSource ? Number.POSITIVE_INFINITY : clip.trimEndFrame;
  return { minDelta, maxDelta };
}

/**
 * Apply a clamped, snapped timeline-frame trim delta to a clip in place, doing the
 * source↔timeline round-trip from `trimClipInternal`. Trim does NOT ripple neighbors.
 */
function applyTrim(clip: ClipView, edge: "left" | "right", deltaFrames: number): void {
  const clamp = trimClamp(clip, edge);
  const delta = Math.max(clamp.minDelta, Math.min(clamp.maxDelta, deltaFrames));
  const noSource = hasNoSourceMedia(clip.mediaType);
  const sourceDelta = roundTiesAway(delta * clip.speed);

  if (edge === "left") {
    const oldTrimStart = clip.trimStartFrame;
    const newTrimStart = noSource ? oldTrimStart + sourceDelta : Math.max(0, oldTrimStart + sourceDelta);
    const deltaStartTimeline = roundTiesAway((newTrimStart - oldTrimStart) / clip.speed);
    clip.trimStartFrame = newTrimStart;
    clip.startFrame = clip.startFrame + deltaStartTimeline;
    clip.durationFrames = clip.durationFrames - deltaStartTimeline;
  } else {
    const oldTrimEnd = clip.trimEndFrame;
    const newTrimEnd = noSource ? oldTrimEnd - sourceDelta : Math.max(0, oldTrimEnd - sourceDelta);
    const deltaEndTimeline = roundTiesAway((newTrimEnd - oldTrimEnd) / clip.speed);
    clip.trimEndFrame = newTrimEnd;
    clip.durationFrames = clip.durationFrames - deltaEndTimeline;
  }
  if (clip.durationFrames < 1) clip.durationFrames = 1;
}

// =====================================================================================
// clear_region apply (overwrite actions → mutate a track) — edit-engines.md 95-97
// =====================================================================================

function clearRegionOnTrack(track: TrackView, regionStart: number, regionEnd: number): void {
  const actions = computeOverwrite(track.clips, regionStart, regionEnd);
  for (const action of actions) {
    const idx = track.clips.findIndex((c) => c.id === action.clipId);
    if (idx < 0) continue;
    const clip = track.clips[idx];
    switch (action.kind) {
      case "remove":
        track.clips.splice(idx, 1);
        break;
      case "trimEnd": {
        // trimEnd recomputes trimEndFrame += round((oldDur - newDur) * speed).
        const oldDur = clip.durationFrames;
        clip.trimEndFrame += roundTiesAway((oldDur - action.newDuration) * clip.speed);
        clip.durationFrames = action.newDuration;
        break;
      }
      case "trimStart":
        clip.startFrame = action.newStartFrame;
        clip.trimStartFrame = action.newTrimStart;
        clip.durationFrames = action.newDuration;
        break;
      case "split": {
        // VM re-derives the right fragment via split_clip(at = regionStart); the left
        // half becomes the trim-end, then the right fragment is removed/re-split.
        const pieces = splitClip(clip, regionStart);
        if (!pieces) break;
        const [leftHalf, rightHalf] = pieces;
        track.clips.splice(idx, 1, leftHalf);
        // Re-split the right half at regionEnd to drop the covered region.
        const rightPieces = splitClip(rightHalf, regionEnd);
        if (rightPieces) {
          // keep only the far-right fragment (the part after regionEnd)
          track.clips.push(rightPieces[1]);
        }
        break;
      }
    }
  }
  sortTrackClips(track);
}

// =====================================================================================
// applyEdit — the command seam (orchestration over the pure pieces)
// =====================================================================================

/**
 * Apply an `EditIntent` to a `TimelineView`, returning a NEW timeline (the input is
 * never mutated). This is the optimistic local path; the same intent will route through
 * Tauri once E7 lands (see controller.ts `// TODO(E7)`).
 */
export function applyEdit(timeline: TimelineView, intent: EditIntent): TimelineView {
  const t = cloneTimeline(timeline);
  switch (intent.kind) {
    case "move":
      applyMove(t, intent);
      break;
    case "trim":
      applyTrimIntent(t, intent);
      break;
    case "split":
      applySplit(t, intent.clipId, intent.atFrame);
      break;
    case "rippleDeleteRange":
      applyRippleDeleteRange(t, intent.trackIndex, intent.ranges);
      break;
    case "deleteClips":
      applyDeleteClips(t, intent.clipIds, intent.ripple);
      break;
    case "setClipProperties":
      applySetClipProperties(t, intent);
      break;
    case "setKeyframes":
      applySetKeyframes(t, intent);
      break;
  }
  return t;
}

/**
 * Set static clip scalars (volume linear / opacity 0..1). Setting a scalar CLEARS that
 * property's keyframe track (matches the backend `set_clip_properties`, properties.rs
 * §259 "Setting a scalar clears that property's keyframe track").
 */
function applySetClipProperties(
  t: TimelineView,
  intent: Extract<EditIntent, { kind: "setClipProperties" }>,
): void {
  for (const id of intent.clipIds) {
    const loc = findClip(t, id);
    if (!loc) continue;
    if (intent.volume !== undefined) {
      loc.clip.volume = intent.volume;
      loc.clip.volumeTrack = null;
    }
    if (intent.opacity !== undefined) {
      loc.clip.opacity = intent.opacity;
      loc.clip.opacityTrack = null;
    }
  }
}

/**
 * Replace a clip's keyframe track for `property` with the given rows (REPLACE semantics
 * mirroring the backend `set_keyframes`). Rows are `[frame, value, interp?]`; an empty
 * list clears the track.
 */
function applySetKeyframes(
  t: TimelineView,
  intent: Extract<EditIntent, { kind: "setKeyframes" }>,
): void {
  const loc = findClip(t, intent.clipId);
  if (!loc) return;
  const kfs = intent.keyframes
    .map((row) => ({
      frame: row[0],
      value: row[1],
      interpolationOut: (row[2] as "linear" | "hold" | "smooth" | undefined) ?? "smooth",
    }))
    .sort((a, b) => a.frame - b.frame);
  const track: KeyframeTrackView | null = kfs.length > 0 ? { keyframes: kfs } : null;
  if (intent.property === "volume") loc.clip.volumeTrack = track;
  else loc.clip.opacityTrack = track;
}

function applyMove(t: TimelineView, intent: Extract<EditIntent, { kind: "move" }>): void {
  // Pull movers off their source tracks (snapshot them first).
  const movers: { clip: ClipView; destTrack: number; sourceTrack: number }[] = [];
  for (const id of intent.clipIds) {
    const loc = findClip(t, id);
    if (!loc) continue;
    const destTrack = intent.trackForClip[id] ?? loc.trackIndex;
    movers.push({ clip: loc.clip, destTrack, sourceTrack: loc.trackIndex });
  }
  if (movers.length === 0) return;

  if (!intent.duplicate) {
    // Remove originals.
    for (const m of movers) {
      const track = t.tracks[m.sourceTrack];
      const idx = track.clips.findIndex((c) => c.id === m.clip.id);
      if (idx >= 0) track.clips.splice(idx, 1);
    }
  }

  // Compute each mover's new frame and drop (overwrite destination first).
  for (const m of movers) {
    const placed = intent.duplicate ? { ...cloneClip(m.clip), id: localUuid("clip") } : cloneClip(m.clip);
    const newStart = Math.max(0, m.clip.startFrame + intent.frameDelta);
    placed.startFrame = newStart;
    const destTrack = t.tracks[m.destTrack];
    if (!destTrack) continue;
    clearRegionOnTrack(destTrack, newStart, newStart + placed.durationFrames);
    destTrack.clips.push(placed);
    sortTrackClips(destTrack);
  }
}

function applyTrimIntent(t: TimelineView, intent: Extract<EditIntent, { kind: "trim" }>): void {
  const loc = findClip(t, intent.clipId);
  if (!loc) return;
  applyTrim(loc.clip, intent.edge, intent.deltaFrames);
  sortTrackClips(t.tracks[loc.trackIndex]);

  if (intent.propagateToLinked && loc.clip.linkGroupId) {
    const partners = linkGroupMembers(t, loc.clip.linkGroupId).filter((c) => c.id !== loc.clip.id);
    for (const partner of partners) {
      applyTrim(partner, intent.edge, intent.deltaFrames);
    }
    for (const track of t.tracks) sortTrackClips(track);
  }
}

function applySplit(t: TimelineView, clipId: string, atFrame: number): void {
  const loc = findClip(t, clipId);
  if (!loc) return;
  const group = loc.clip.linkGroupId;
  // Split every member of the link group at the same atFrame; regroup right halves
  // under a fresh link_group_id (edit-engines.md line 110).
  const members = group ? linkGroupMembers(t, group) : [loc.clip];
  const newLinkId = members.length > 1 ? localUuid("link") : null;
  for (const member of members) {
    const mloc = findClip(t, member.id);
    if (!mloc) continue;
    const pieces = splitClip(mloc.clip, atFrame);
    if (!pieces) continue;
    const [leftHalf, rightHalf] = pieces;
    if (newLinkId) rightHalf.linkGroupId = newLinkId;
    const track = t.tracks[mloc.trackIndex];
    track.clips.splice(mloc.clipIndex, 1, leftHalf, rightHalf);
    sortTrackClips(track);
  }
}

function applyRippleDeleteRange(t: TimelineView, trackIndex: number, ranges: FrameRange[]): void {
  const merged = mergeRanges(ranges.filter((r) => rangeLength(r) > 0));
  if (merged.length === 0) return;
  const anchor = t.tracks[trackIndex];
  if (!anchor) return;

  // Tracks holding linked partners of any clip a range overlaps are also cleared.
  const linkedGroups = new Set<string>();
  for (const clip of anchor.clips) {
    if (!clip.linkGroupId) continue;
    for (const r of merged) {
      if (r.start < endFrame(clip) && r.end > clip.startFrame) {
        linkedGroups.add(clip.linkGroupId);
      }
    }
  }
  const clearTrackIdx = new Set<number>([trackIndex]);
  for (let ti = 0; ti < t.tracks.length; ti++) {
    for (const clip of t.tracks[ti].clips) {
      if (clip.linkGroupId && linkedGroups.has(clip.linkGroupId)) clearTrackIdx.add(ti);
    }
  }

  // For each cleared track: clear each merged range; then for cleared OR sync-locked
  // tracks apply ripple shifts.
  for (let ti = 0; ti < t.tracks.length; ti++) {
    const track = t.tracks[ti];
    const isCleared = clearTrackIdx.has(ti);
    if (isCleared) {
      for (const r of merged) clearRegionOnTrack(track, r.start, r.end);
    }
    if (isCleared || track.syncLocked) {
      const shifts = computeRippleShiftsForRanges(track.clips, merged);
      applyShifts(track, shifts);
      sortTrackClips(track);
    }
  }
}

function applyDeleteClips(t: TimelineView, clipIds: string[], ripple: boolean): void {
  const ids = new Set(clipIds);
  // Per track: build removed ranges from this track's removed clips, remove them,
  // then ripple the remaining clips left by the gaps before them.
  for (const track of t.tracks) {
    const removed = track.clips.filter((c) => ids.has(c.id));
    if (removed.length === 0) continue;
    const ranges: FrameRange[] = removed.map((c) => ({ start: c.startFrame, end: endFrame(c) }));
    track.clips = track.clips.filter((c) => !ids.has(c.id));
    if (ripple) {
      const shifts = computeRippleShiftsForRanges(track.clips, ranges);
      applyShifts(track, shifts);
    }
    sortTrackClips(track);
  }
}

function applyShifts(track: TrackView, shifts: ClipShift[]): void {
  const map = new Map(shifts.map((s) => [s.clipId, s.newStartFrame]));
  for (const clip of track.clips) {
    const next = map.get(clip.id);
    if (next !== undefined) clip.startFrame = next;
  }
}
