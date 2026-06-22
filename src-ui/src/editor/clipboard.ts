// In-memory clip clipboard for Edit → Cut / Copy / Paste (Project window).
//
// The editor has no OS-clipboard integration for clips (a clip is a structured
// timeline object, not text/an image), so Cut/Copy/Paste operate over a simple
// module-level store that survives for the lifetime of the session (a fresh module
// load = an empty clipboard). This mirrors the reference editor's in-app clip
// pasteboard rather than the system pasteboard.
//
// Copy captures a FULL spec of each selected clip — its media reference, placement
// (start/duration), source trims, owning track index + type, and every editable
// property the tools layer can re-express (volume / opacity / speed / fades +
// interpolations / static transform-ish scalars, plus the animated keyframe tracks
// for volume / opacity / rotation / position / scale / crop). Paste recreates the
// clips at the playhead via the backend (`add_clips` → `set_clip_properties` /
// `set_keyframes`) so a pasted clip matches the original as closely as the tools
// allow. Anything the tools cannot express is documented in PASTE_LIMITATIONS below.

import type {
  ClipType,
  ClipView,
  Interpolation,
  KeyframeTrackView,
  KeyframeView,
  TimelineView,
} from "./types";
import { editorEdit, getTimeline } from "./bridge";
import { adaptTimeline } from "./adapt";

/**
 * A captured copy of one clip, normalised to the playhead so a paste can offset
 * every clip in the selection by the same delta (preserving inter-clip gaps and
 * relative track placement). Frame fields are TIMELINE frames; trims are SOURCE
 * frames — matching `ClipView` semantics.
 */
export interface ClipboardClip {
  mediaRef: string;
  /** The clip's media kind (drives which property fields the paste restores). */
  mediaType: ClipType;
  /** Offset of this clip's start from the COPY anchor (min start across selection). */
  startOffsetFrames: number;
  durationFrames: number;
  trimStartFrame: number;
  trimEndFrame: number;
  speed: number;
  volume: number;
  opacity: number;
  fadeInFrames: number;
  fadeOutFrames: number;
  fadeInInterpolation: Interpolation;
  fadeOutInterpolation: Interpolation;
  /** Owning track index at copy time (relative offset is computed at paste). */
  trackIndex: number;
  /** Keyframe tracks captured verbatim (clip-relative frames; restored 1:1). */
  volumeTrack?: KeyframeTrackView | null;
  opacityTrack?: KeyframeTrackView | null;
  positionTrack?: KeyframeTrackView | null;
  scaleTrack?: KeyframeTrackView | null;
  cropTrack?: KeyframeTrackView | null;
}

/** The whole clipboard payload (≥1 clip + the anchor track for relative tracks). */
export interface ClipboardPayload {
  clips: ClipboardClip[];
  /** The minimum track index across the copied clips (anchor for relative placement). */
  anchorTrackIndex: number;
}

// ── Module-level store (session-scoped; not persisted) ──────────────────────────
let board: ClipboardPayload | null = null;

/** True when there is something to paste. */
export function clipboardHasContent(): boolean {
  return board !== null && board.clips.length > 0;
}

/** Read the current clipboard payload (null when empty). */
export function readClipboard(): ClipboardPayload | null {
  return board;
}

/** Clear the clipboard (exposed for tests / explicit reset). */
export function clearClipboard(): void {
  board = null;
}

/**
 * Capture the given clips into the clipboard. `trackIndexOf` resolves a clip id to
 * its owning track index in the live timeline. Clips are normalised to the minimum
 * start frame so paste can re-anchor at the playhead. No-op (clears) if `clips` is
 * empty.
 */
export function writeClipboard(
  clips: ClipView[],
  trackIndexOf: (clipId: string) => number,
): void {
  if (clips.length === 0) {
    board = null;
    return;
  }
  const anchorStart = Math.min(...clips.map((c) => c.startFrame));
  const anchorTrackIndex = Math.min(...clips.map((c) => trackIndexOf(c.id)));
  board = {
    anchorTrackIndex,
    clips: clips.map((c) => ({
      mediaRef: c.mediaRef,
      mediaType: c.mediaType,
      startOffsetFrames: c.startFrame - anchorStart,
      durationFrames: c.durationFrames,
      trimStartFrame: c.trimStartFrame,
      trimEndFrame: c.trimEndFrame,
      speed: c.speed,
      volume: c.volume,
      opacity: c.opacity,
      fadeInFrames: c.fadeInFrames,
      fadeOutFrames: c.fadeOutFrames,
      fadeInInterpolation: c.fadeInInterpolation,
      fadeOutInterpolation: c.fadeOutInterpolation,
      trackIndex: trackIndexOf(c.id),
      volumeTrack: c.volumeTrack ?? null,
      opacityTrack: c.opacityTrack ?? null,
      positionTrack: c.positionTrack ?? null,
      scaleTrack: c.scaleTrack ?? null,
      cropTrack: c.cropTrack ?? null,
    })),
  };
}

/**
 * Properties the paste path CANNOT fully restore through the existing tools, and why.
 * Surfaced here (and referenced in the menu wiring) so the gap is documented, not silent:
 *
 *  - `linkGroupId`: `add_clips` re-mints link groups itself (a video-with-audio asset
 *    auto-links a fresh audio clip). A pasted clip therefore gets a NEW link group, not
 *    the original's — there is no tool to set an explicit link-group id.
 *  - `name` (custom display name): no tool field; the clip name re-resolves from media.
 *  - Static `transform` (centerX/centerY/width/height/flip): `set_clip_properties`
 *    accepts a `transform` patch, BUT `ClipView` (the timeline read model the UI sees)
 *    does not carry the static transform — only the ANIMATED position/scale/crop tracks
 *    are visible here. So a static (un-keyframed) transform offset is not captured and
 *    thus not restored. Keyframed position/scale/crop ARE restored via `set_keyframes`.
 *  - Static `rotation`: same as transform — not present on `ClipView`; only the animated
 *    `rotation` keyframe track (if any) round-trips. (ClipView has no rotationTrack field
 *    either, so rotation does not round-trip at all today — noted for a follow-up.)
 *  - Text styling (font/color/alignment/content): `ClipView` does not expose text style,
 *    so a pasted text clip keeps the add_clips default content; styling is not restored.
 */
export const PASTE_LIMITATIONS = [
  "linkGroupId (add_clips re-mints link groups; no set-link tool)",
  "name (no tool field; re-resolves from media)",
  "static transform / rotation (not present on the timeline read model ClipView)",
  "text style/content (not present on ClipView)",
] as const;

// ── Paste ───────────────────────────────────────────────────────────────────

/** Map a `KeyframeTrackView` to the `set_keyframes` scalar row form `[frame, value, interp]`. */
function scalarRows(track: KeyframeTrackView | null | undefined): unknown[] {
  if (!track || track.keyframes.length === 0) return [];
  return track.keyframes.map((k: KeyframeView) => [k.frame, k.value, k.interpolationOut]);
}

/**
 * Paste the clipboard at `playheadFrame`. Recreates the clips via `add_clips` (one
 * undoable backend step), then restores each clip's expressible properties and any
 * animated keyframe tracks. Resolves to the number of clips pasted (0 = nothing to
 * paste / dispatch failed). Returns the list of property fields that could not be
 * restored (see {@link PASTE_LIMITATIONS}) for the caller to log.
 *
 * New-clip ids are resolved by DIFFING the timeline before/after `add_clips` (the
 * tool returns only a prose summary, not structured ids), matching new clips to the
 * paste entries by track + start frame.
 */
export async function pasteClipboard(
  before: TimelineView,
  playheadFrame: number,
): Promise<{ pasted: number; unrestored: readonly string[] }> {
  const payload = board;
  if (!payload || payload.clips.length === 0) {
    return { pasted: 0, unrestored: [] };
  }

  // Build add_clips entries anchored at the playhead. We always omit trackIndex so the
  // tool auto-creates shared tracks (the original track may not exist after edits, and
  // mixing explicit/omitted trackIndex is rejected by the tool). Relative track layout
  // is therefore not preserved on paste — clips land on one shared video/audio track.
  const entries = payload.clips.map((c) => ({
    mediaRef: c.mediaRef,
    startFrame: Math.max(0, playheadFrame + c.startOffsetFrames),
    durationFrames: c.durationFrames,
  }));

  const res = await editorEdit("add_clips", { entries });
  if (!res.ok) {
    return { pasted: 0, unrestored: [] };
  }

  // Refetch the authoritative timeline and diff to find the newly-created clip ids.
  const wire = await getTimeline();
  if (wire === undefined) {
    // Outside Tauri (no backend) — the add was a no-op success; nothing to restore.
    return { pasted: payload.clips.length, unrestored: PASTE_LIMITATIONS };
  }
  const after = adaptTimeline(wire);
  const beforeIds = new Set<string>();
  for (const t of before.tracks) for (const c of t.clips) beforeIds.add(c.id);

  // Index new clips by (mediaRef, startFrame) so we can match each paste entry to the
  // clip(s) the tool produced (a video-with-audio asset yields a video + linked audio
  // clip — both share mediaRef/start; we restore properties onto the matching media-type).
  type NewClip = { id: string; mediaType: ClipType; mediaRef: string; startFrame: number };
  const newClips: NewClip[] = [];
  for (const t of after.tracks) {
    for (const c of t.clips) {
      if (!beforeIds.has(c.id)) {
        newClips.push({ id: c.id, mediaType: c.mediaType, mediaRef: c.mediaRef, startFrame: c.startFrame });
      }
    }
  }

  const used = new Set<string>();
  const unrestored = new Set<string>();

  for (let i = 0; i < payload.clips.length; i++) {
    const spec = payload.clips[i];
    const targetStart = Math.max(0, playheadFrame + spec.startOffsetFrames);
    // Match the new clip with the same mediaRef + start whose media type matches the
    // spec (so the linked-audio twin doesn't steal the video clip's restore, and vice
    // versa). Falls back to any same-ref/start clip if the type doesn't match exactly.
    let match = newClips.find(
      (n) =>
        !used.has(n.id) &&
        n.mediaRef === spec.mediaRef &&
        n.startFrame === targetStart &&
        n.mediaType === spec.mediaType,
    );
    if (!match) {
      match = newClips.find(
        (n) => !used.has(n.id) && n.mediaRef === spec.mediaRef && n.startFrame === targetStart,
      );
    }
    if (!match) {
      // Could not resolve the created clip — the clip exists (add_clips succeeded) but
      // its properties can't be restored. Record everything as unrestored for this clip.
      for (const l of PASTE_LIMITATIONS) unrestored.add(l);
      continue;
    }
    used.add(match.id);

    // Restore scalar + fade properties expressible via set_clip_properties.
    await editorEdit("set_clip_properties", {
      clipIds: [match.id],
      trimStartFrame: spec.trimStartFrame,
      trimEndFrame: spec.trimEndFrame,
      speed: spec.speed,
      volume: spec.volume,
      opacity: spec.opacity,
      fadeInFrames: spec.fadeInFrames,
      fadeOutFrames: spec.fadeOutFrames,
      fadeInInterpolation: spec.fadeInInterpolation,
      fadeOutInterpolation: spec.fadeOutInterpolation,
    });

    // Restore animated keyframe tracks (clip-relative frames round-trip 1:1). Setting a
    // scalar above clears volume/opacity tracks, so apply keyframes AFTER the scalars.
    const volRows = scalarRows(spec.volumeTrack);
    if (volRows.length > 0) {
      await editorEdit("set_keyframes", { clipId: match.id, property: "volume", keyframes: volRows });
    }
    const opRows = scalarRows(spec.opacityTrack);
    if (opRows.length > 0) {
      await editorEdit("set_keyframes", { clipId: match.id, property: "opacity", keyframes: opRows });
    }
    if (spec.positionTrack && spec.positionTrack.keyframes.length > 0) {
      const rows = spec.positionTrack.keyframes.map((k) => {
        const v = k.value as unknown as { a?: number; b?: number } | number;
        const pair = typeof v === "number" ? [v, v] : [v.a ?? 0, v.b ?? 0];
        return [k.frame, pair[0], pair[1], k.interpolationOut];
      });
      await editorEdit("set_keyframes", { clipId: match.id, property: "position", keyframes: rows });
    }
    if (spec.scaleTrack && spec.scaleTrack.keyframes.length > 0) {
      const rows = spec.scaleTrack.keyframes.map((k) => {
        const v = k.value as unknown as { a?: number; b?: number } | number;
        const pair = typeof v === "number" ? [v, v] : [v.a ?? 0, v.b ?? 0];
        return [k.frame, pair[0], pair[1], k.interpolationOut];
      });
      await editorEdit("set_keyframes", { clipId: match.id, property: "scale", keyframes: rows });
    }
    if (spec.cropTrack && spec.cropTrack.keyframes.length > 0) {
      const rows = spec.cropTrack.keyframes.map((k) => {
        const v = k.value as unknown as { top?: number; right?: number; bottom?: number; left?: number };
        return [k.frame, v.top ?? 0, v.right ?? 0, v.bottom ?? 0, v.left ?? 0, k.interpolationOut];
      });
      await editorEdit("set_keyframes", { clipId: match.id, property: "crop", keyframes: rows });
    }
  }

  // The expressible-but-missing fields are always partially unrestorable (link group,
  // static transform/rotation, text style) — report them so the gap is visible.
  for (const l of PASTE_LIMITATIONS) unrestored.add(l);
  return { pasted: payload.clips.length, unrestored: [...unrestored] };
}
