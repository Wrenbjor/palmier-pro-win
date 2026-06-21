// Adapter: `editor_get_timeline` wire JSON → `TimelineView` (the editor store shape).
//
// The Tauri `editor_get_timeline` command returns the same compact shape as the MCP
// `get_timeline` tool — positions + ids, not the full clip model. This fills the richer
// `ClipView` fields (trim/volume/opacity/fades/keyframes) with neutral defaults so the
// canvas can render clip blocks at the right positions. A full-fidelity serializer
// (real volume/trim/keyframes) is a later slice; clips still draw correctly without it.

import type { ClipType, ClipView, Interpolation, TimelineView, TrackView } from "./types";

interface WireClip {
  id: string;
  startFrame: number;
  durationFrames: number;
  mediaRef?: string;
  linkGroupId?: string | null;
  mediaType?: string;
  sourceClipType?: string;
}
interface WireTrack {
  id: string;
  label?: string;
  type: string;
  clips?: WireClip[];
}
export interface WireTimeline {
  fps: number;
  width: number;
  height: number;
  totalFrames?: number;
  tracks?: WireTrack[];
}

const CLIP_TYPES: readonly ClipType[] = ["video", "image", "text", "lottie", "audio"];

function asClipType(v: string | undefined, fallback: ClipType): ClipType {
  return v && (CLIP_TYPES as readonly string[]).includes(v) ? (v as ClipType) : fallback;
}

const LINEAR: Interpolation = "linear";

function adaptClip(c: WireClip, trackType: ClipType): ClipView {
  const mediaType = asClipType(c.mediaType, trackType);
  return {
    id: c.id,
    name: c.mediaRef ?? "Clip",
    mediaRef: c.mediaRef ?? "",
    mediaType,
    sourceClipType: asClipType(c.sourceClipType, mediaType),
    startFrame: c.startFrame,
    durationFrames: c.durationFrames,
    trimStartFrame: 0,
    trimEndFrame: 0,
    speed: 1,
    volume: 1,
    opacity: 1,
    fadeInFrames: 0,
    fadeOutFrames: 0,
    fadeInInterpolation: LINEAR,
    fadeOutInterpolation: LINEAR,
    linkGroupId: c.linkGroupId ?? null,
  };
}

function adaptTrack(t: WireTrack): TrackView {
  const trackType = asClipType(t.type, "video");
  return {
    id: t.id,
    type: trackType,
    muted: false,
    hidden: false,
    syncLocked: false,
    displayHeight: 50,
    clips: (t.clips ?? []).map((c) => adaptClip(c, trackType)),
  };
}

export function adaptTimeline(wire: WireTimeline): TimelineView {
  return {
    fps: wire.fps,
    width: wire.width,
    height: wire.height,
    tracks: (wire.tracks ?? []).map(adaptTrack),
  };
}
