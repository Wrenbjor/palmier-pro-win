// adaptTimeline — map the `get_timeline` wire JSON → the canvas `TimelineView`.
//
// The `editor_get_timeline` Tauri command (crates/palmier-tauri/src/commands.rs)
// returns the reference-shaped timeline: a Codable `Timeline` with clip/track fields
// equal to their DEFAULTS omitted, caption clips collapsed into per-track
// `captionGroups`, and `totalFrames` / `canGenerate` injected (read.rs `get_timeline`).
// This adapter is the single place that turns that serde payload into the fully
// defaulted `TimelineView` the renderer needs — the seam the editor's `index.ts`/
// `types.ts` always anticipated ("the adapter that turns the serde payload into a
// TimelineView is the only thing that changes").
//
// Default reconstruction MUST match read.rs's stripping (and palmier-model's serde
// defaults): mediaType 'video', sourceClipType = mediaType, speed 1, volume 1,
// opacity 1, trims/fades 0, fade interp 'smooth'; track muted/hidden false,
// syncLocked true. Caption-group rows are expanded back into individual ClipViews so
// the canvas draws them like any other clip.

import type {
  ClipType,
  ClipView,
  Interpolation,
  TimelineView,
  TrackView,
} from "./types";

/** A loosely-typed wire object (the parsed `editor_get_timeline` JSON). */
type Json = Record<string, unknown>;

function asNumber(v: unknown, fallback: number): number {
  return typeof v === "number" && Number.isFinite(v) ? v : fallback;
}
function asString(v: unknown, fallback: string): string {
  return typeof v === "string" ? v : fallback;
}
function asBool(v: unknown, fallback: boolean): boolean {
  return typeof v === "boolean" ? v : fallback;
}

const CLIP_TYPES: readonly ClipType[] = [
  "video",
  "image",
  "text",
  "lottie",
  "audio",
];
function asClipType(v: unknown, fallback: ClipType): ClipType {
  return typeof v === "string" && (CLIP_TYPES as readonly string[]).includes(v)
    ? (v as ClipType)
    : fallback;
}

const INTERPS: readonly Interpolation[] = ["linear", "hold", "smooth"];
function asInterp(v: unknown, fallback: Interpolation): Interpolation {
  return typeof v === "string" && (INTERPS as readonly string[]).includes(v)
    ? (v as Interpolation)
    : fallback;
}

/** Default lane height — matches palmier-model `default_display_height` (50). */
const DEFAULT_DISPLAY_HEIGHT = 50;

/** Map one wire clip object (defaults stripped) → a fully-defaulted ClipView. */
function adaptClip(raw: Json): ClipView {
  const mediaType = asClipType(raw.mediaType, "video");
  return {
    id: asString(raw.id, ""),
    // The wire carries no display name (it lives on the media asset); fall back to
    // text content for text clips, else the mediaRef tail. The canvas shows this.
    name:
      asString(raw.textContent, "") ||
      asString(raw.name, "") ||
      asString(raw.mediaRef, ""),
    mediaRef: asString(raw.mediaRef, ""),
    mediaType,
    // sourceClipType defaults to mediaType when stripped (read.rs drops it on equal).
    sourceClipType: asClipType(raw.sourceClipType, mediaType),
    startFrame: asNumber(raw.startFrame, 0),
    durationFrames: asNumber(raw.durationFrames, 0),
    trimStartFrame: asNumber(raw.trimStartFrame, 0),
    trimEndFrame: asNumber(raw.trimEndFrame, 0),
    speed: asNumber(raw.speed, 1),
    volume: asNumber(raw.volume, 1),
    opacity: asNumber(raw.opacity, 1),
    fadeInFrames: asNumber(raw.fadeInFrames, 0),
    fadeOutFrames: asNumber(raw.fadeOutFrames, 0),
    fadeInInterpolation: asInterp(raw.fadeInInterpolation, "smooth"),
    fadeOutInterpolation: asInterp(raw.fadeOutInterpolation, "smooth"),
    linkGroupId:
      typeof raw.linkGroupId === "string" ? raw.linkGroupId : null,
    // Generation status / missing media are media-library concerns; the timeline
    // wire doesn't carry them, so leave the optional flags unset.
  };
}

/**
 * Expand a track's `captionGroups` back into individual ClipViews. read.rs collapses
 * caption clips sharing a `captionGroupId` into a group with `shared` props + per-clip
 * `[clipId, startFrame, durationFrames, text]` rows; we rebuild each as a text clip so
 * the canvas renders it. The `clipFormat` array names the row columns (defensive read).
 */
function expandCaptionGroups(track: Json): ClipView[] {
  const groups = track.captionGroups;
  if (!Array.isArray(groups)) return [];
  const out: ClipView[] = [];
  for (const g of groups) {
    if (typeof g !== "object" || g === null) continue;
    const group = g as Json;
    const shared = (typeof group.shared === "object" && group.shared !== null
      ? group.shared
      : {}) as Json;
    const rows = Array.isArray(group.clips) ? group.clips : [];
    const gid = asString(group.captionGroupId, "");
    for (const r of rows) {
      if (!Array.isArray(r)) continue;
      const [clipId, startFrame, durationFrames, text] = r as unknown[];
      out.push(
        adaptClip({
          ...shared,
          id: clipId,
          startFrame,
          durationFrames,
          textContent: text,
          captionGroupId: gid,
          // Caption clips are text overlays.
          mediaType: asClipType(shared.mediaType, "text"),
        }),
      );
    }
  }
  return out;
}

/** Map one wire track (defaults stripped) → a fully-defaulted TrackView. */
function adaptTrack(raw: Json): TrackView {
  const type = asClipType(raw.type, "video");
  const looseClips = Array.isArray(raw.clips)
    ? raw.clips
        .filter((c): c is Json => typeof c === "object" && c !== null)
        .map(adaptClip)
    : [];
  const captionClips = expandCaptionGroups(raw);
  const clips = [...looseClips, ...captionClips].sort(
    (a, b) => a.startFrame - b.startFrame,
  );
  return {
    id: asString(raw.id, ""),
    type,
    muted: asBool(raw.muted, false),
    hidden: asBool(raw.hidden, false),
    syncLocked: asBool(raw.syncLocked, true),
    displayHeight: DEFAULT_DISPLAY_HEIGHT,
    clips,
  };
}

/**
 * Map the `editor_get_timeline` wire JSON → a `TimelineView`. Tolerant of missing
 * fields (a `{}` payload decodes to an empty default timeline), so it never throws on
 * a partial / early-boot payload.
 */
export function adaptTimeline(wire: unknown): TimelineView {
  const raw = (typeof wire === "object" && wire !== null ? wire : {}) as Json;
  const tracks = Array.isArray(raw.tracks)
    ? raw.tracks
        .filter((t): t is Json => typeof t === "object" && t !== null)
        .map(adaptTrack)
    : [];
  return {
    fps: asNumber(raw.fps, 30),
    width: asNumber(raw.width, 1920),
    height: asNumber(raw.height, 1080),
    tracks,
  };
}
