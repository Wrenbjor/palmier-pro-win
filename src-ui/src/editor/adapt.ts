// adaptTimeline — map the `editor_get_timeline` wire JSON → the canvas `TimelineView`.
//
// The `editor_get_timeline` Tauri command (crates/palmier-tauri/src/commands.rs)
// returns the FULL-FIDELITY timeline (palmier-tools `read::full_timeline_json`),
// NOT the compact MCP `get_timeline` summary. That serializer emits every field the
// view types need with their real values — nothing is stripped to defaults, caption
// clips are ordinary clips in `clips` (no `captionGroups` collapse), and the six
// keyframe tracks are full `{ keyframes: [{ frame, value, interpolationOut }] }`
// objects (matching `KeyframeTrackView` / `KeyframeView`). Each track also carries
// the injected `displayHeight` (the model marks it non-serialized).
//
// This adapter is therefore a near-passthrough: it walks the wire shape and copies
// fields across, applying TOLERANT defaults only so a partial / early-boot payload
// (e.g. `{}`) still yields a valid, empty `TimelineView` instead of throwing. The
// defaults it falls back to still match palmier-model's serde defaults (speed 1,
// volume 1, opacity 1, fades 0, fade interp 'linear', track syncLocked true,
// displayHeight 50) so a missing field reads the same as the model would have.

import type {
  ClipType,
  ClipView,
  Interpolation,
  KeyframeTrackView,
  KeyframeView,
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

/**
 * Adapt one wire keyframe track (`{ keyframes: [{ frame, value, interpolationOut }] }`)
 * into a `KeyframeTrackView`. Returns `null` when absent or empty (a track is only
 * "active" when it holds keyframes — reference `!keyframes.isEmpty`). `value` is
 * passed through verbatim: scalar tracks (volume/opacity) carry a number; the
 * position/scale/crop tracks carry the model's `AnimPair`/`Crop` object, which the
 * view types accept as the generic `V`.
 */
function adaptKeyframeTrack(raw: unknown): KeyframeTrackView | null {
  if (typeof raw !== "object" || raw === null) return null;
  const rows = (raw as Json).keyframes;
  if (!Array.isArray(rows) || rows.length === 0) return null;
  const keyframes: KeyframeView[] = [];
  for (const r of rows) {
    if (typeof r !== "object" || r === null) continue;
    const kf = r as Json;
    keyframes.push({
      frame: asNumber(kf.frame, 0),
      // `value` is generic; pass through (number for scalar, object for pair/crop).
      value: kf.value as KeyframeView["value"],
      interpolationOut: asInterp(kf.interpolationOut, "smooth"),
    });
  }
  if (keyframes.length === 0) return null;
  return { keyframes };
}

/** Map one wire clip object → a fully-populated ClipView (near-passthrough). */
function adaptClip(raw: Json): ClipView {
  const mediaType = asClipType(raw.mediaType, "video");
  return {
    id: asString(raw.id, ""),
    // The wire carries no display name (it lives on the media asset); fall back to
    // text content for text clips, else the mediaRef. The canvas shows this.
    name:
      asString(raw.textContent, "") ||
      asString(raw.name, "") ||
      asString(raw.mediaRef, ""),
    mediaRef: asString(raw.mediaRef, ""),
    mediaType,
    // sourceClipType is always serialized by the full serializer; default to
    // mediaType only if somehow absent.
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
    fadeInInterpolation: asInterp(raw.fadeInInterpolation, "linear"),
    fadeOutInterpolation: asInterp(raw.fadeOutInterpolation, "linear"),
    linkGroupId: typeof raw.linkGroupId === "string" ? raw.linkGroupId : null,
    // Keyframe tracks — full passthrough; `null` when the track is absent/empty.
    volumeTrack: adaptKeyframeTrack(raw.volumeTrack),
    opacityTrack: adaptKeyframeTrack(raw.opacityTrack),
    positionTrack: adaptKeyframeTrack(raw.positionTrack),
    scaleTrack: adaptKeyframeTrack(raw.scaleTrack),
    rotationTrack: adaptKeyframeTrack(raw.rotationTrack),
    cropTrack: adaptKeyframeTrack(raw.cropTrack),
    // Per-source audio peaks (dB-normalised, 0 = loud … 1 = silent). The Tauri
    // `editor_get_timeline` command injects this onto audio clips from the cached
    // waveform pipeline; absent (e.g. peaks not yet computed) → placeholder bars.
    // The renderer slices this full-source array to the clip's trimmed window.
    waveform: adaptWaveform(raw.waveform),
    // Generation status / missing media are media-library concerns; the timeline
    // wire doesn't carry them, so leave the optional flags unset.
  };
}

/**
 * Adapt the wire `waveform` (a `number[]` of dB-normalised peaks) → `ClipView.waveform`.
 * Returns `null` when absent/empty/non-array so the renderer draws placeholder bars.
 * Filters to finite numbers (a malformed entry never throws the canvas).
 */
function adaptWaveform(raw: unknown): number[] | null {
  if (!Array.isArray(raw) || raw.length === 0) return null;
  const peaks = raw.filter(
    (v): v is number => typeof v === "number" && Number.isFinite(v),
  );
  return peaks.length > 0 ? peaks : null;
}

/** Map one wire track → a fully-populated TrackView. */
function adaptTrack(raw: Json): TrackView {
  const type = asClipType(raw.type, "video");
  const clips = Array.isArray(raw.clips)
    ? raw.clips
        .filter((c): c is Json => typeof c === "object" && c !== null)
        .map(adaptClip)
        .sort((a, b) => a.startFrame - b.startFrame)
    : [];
  return {
    id: asString(raw.id, ""),
    type,
    muted: asBool(raw.muted, false),
    hidden: asBool(raw.hidden, false),
    syncLocked: asBool(raw.syncLocked, true),
    // The full serializer injects displayHeight; fall back to the model default.
    displayHeight: asNumber(raw.displayHeight, DEFAULT_DISPLAY_HEIGHT),
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
