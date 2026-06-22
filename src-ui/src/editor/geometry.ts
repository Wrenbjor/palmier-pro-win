// Pure timeline geometry + sampling math (E3-S9).
//
// This is the FRONTEND mirror of `palmier-edit::geometry` + the render-critical
// sampling on `palmier-model::Clip`. The Rust crate is the source of truth for the
// engine path (E3-S5); this TS copy exists so the canvas can draw before the Tauri
// bridge lands, and so the drawing math is unit-testable for parity (golden tick
// positions, clip_rect, waveform bar mapping — timeline-model.md §"Geometry").
//
// Ported 1:1 from `Timeline/TimelineGeometry.swift`, `Timeline/TimelineRuler.swift`,
// `Timeline/ClipRenderer.swift` (sampling helpers), `Models/Keyframe.swift` (sample),
// and `Models/Timeline.swift` (Clip derived props). Rounding is `Math.round` which is
// ties-away-from-zero for positive values — matching Rust `f64::round` for the
// non-negative frame counts used here (carry-forward rounding note).

import type { ClipView, Interpolation, KeyframeTrackView } from "./types";
import { Layout, smoothstep, VolumeScale } from "./theme";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** ties-away-from-zero round, matching Rust `f64::round`. */
export function roundTiesAway(v: number): number {
  return Math.sign(v) * Math.round(Math.abs(v));
}

// --- Clip derived frame math (Models/Timeline.swift:54-56) ---

export function endFrame(clip: ClipView): number {
  return clip.startFrame + clip.durationFrames;
}

/** round(durationFrames * speed) — source frames the clip consumes. */
export function sourceFramesConsumed(clip: ClipView): number {
  return roundTiesAway(clip.durationFrames * clip.speed);
}

/** sourceFramesConsumed + trimStart + trimEnd — full source span. */
export function sourceDurationFrames(clip: ClipView): number {
  return sourceFramesConsumed(clip) + clip.trimStartFrame + clip.trimEndFrame;
}

// --- Geometry (TimelineGeometry.swift) ---

export interface TimelineLayout {
  pixelsPerFrame: number;
  headerWidth: number;
  rulerHeight: number;
  trackHeights: number[];
  /** Precomputed cumulative Y per track (top edge). */
  cumulativeY: number[];
}

export function makeLayout(
  pixelsPerFrame: number,
  trackHeights: number[],
  headerWidth = 0,
): TimelineLayout {
  const cumulativeY: number[] = [];
  let y = Layout.rulerHeight + Layout.dropZoneHeight;
  for (const h of trackHeights) {
    cumulativeY.push(y);
    y += h;
  }
  return {
    pixelsPerFrame,
    headerWidth,
    rulerHeight: Layout.rulerHeight,
    trackHeights,
    cumulativeY,
  };
}

export function trackHeightAt(layout: TimelineLayout, index: number): number {
  return layout.trackHeights[index] ?? Layout.trackHeight;
}

/** Screen-space Y/height band of a track's header row (the reserved left gutter). */
export interface TrackHeaderBand {
  /** Top edge (screen px) — the track's lane top; headers don't scroll vertically. */
  y: number;
  /** Band height (screen px) = the track's displayHeight. */
  h: number;
}

/**
 * The header gutter band for track `index` — its top Y and height, matching the
 * canvas track lane drawn by `renderer.drawBackground`. The header is a fixed-width
 * (`Layout.trackHeaderWidth`) DOM column at the timeline's left edge; only the Y axis
 * is shared with the canvas. Pure so the header overlay and parity checks agree.
 */
export function trackHeaderBand(layout: TimelineLayout, index: number): TrackHeaderBand {
  return { y: trackY(layout, index), h: trackHeightAt(layout, index) };
}

export function trackY(layout: TimelineLayout, index: number): number {
  return layout.cumulativeY[index] ?? layout.rulerHeight;
}

/** clipRect = (x = header + start*ppf, y = trackY + 2, w = dur*ppf, h = h - 4). */
export function clipRect(
  layout: TimelineLayout,
  clip: ClipView,
  trackIndex: number,
): Rect {
  const y = trackY(layout, trackIndex);
  const h = trackHeightAt(layout, trackIndex);
  return {
    x: layout.headerWidth + clip.startFrame * layout.pixelsPerFrame,
    y: y + 2,
    w: clip.durationFrames * layout.pixelsPerFrame,
    h: h - 4,
  };
}

/** frameAt(x) = max(0, floor((x - header) / ppf)). */
export function frameAt(layout: TimelineLayout, x: number): number {
  return Math.max(
    0,
    Math.floor((x - layout.headerWidth) / layout.pixelsPerFrame),
  );
}

export function xForFrame(layout: TimelineLayout, frame: number): number {
  return layout.headerWidth + frame * layout.pixelsPerFrame;
}

/** trackAt(y) — linear scan; returns last track when below. */
export function trackAt(layout: TimelineLayout, y: number): number {
  for (let i = 0; i < layout.cumulativeY.length; i++) {
    if (y < layout.cumulativeY[i] + layout.trackHeights[i]) return i;
  }
  return Math.max(0, layout.trackHeights.length - 1);
}

export type TrackDropTarget =
  | { kind: "existing"; index: number }
  | { kind: "newAt"; index: number };

/** dropTargetAt(y) — top zone / between-track / past-last → new track. */
export function dropTargetAt(layout: TimelineLayout, y: number): TrackDropTarget {
  const count = layout.trackHeights.length;
  if (count === 0) return { kind: "newAt", index: 0 };
  if (y < layout.cumulativeY[0]) return { kind: "newAt", index: 0 };

  const threshold = Layout.insertThreshold;
  for (let i = 0; i < count - 1; i++) {
    const bottomOfTrack = layout.cumulativeY[i] + layout.trackHeights[i];
    const topOfNext = layout.cumulativeY[i + 1];
    if (y >= bottomOfTrack - threshold && y <= topOfNext + threshold) {
      return { kind: "newAt", index: i + 1 };
    }
  }

  const lastBottom =
    layout.cumulativeY[count - 1] + layout.trackHeights[count - 1];
  if (y >= lastBottom) return { kind: "newAt", index: count };

  for (let i = 0; i < count; i++) {
    if (y < layout.cumulativeY[i] + layout.trackHeights[i]) {
      return { kind: "existing", index: i };
    }
  }
  return { kind: "existing", index: Math.max(0, count - 1) };
}

// --- Ruler tick math (TimelineRuler.swift) ---

/** Choose a tick interval keeping major ticks ~80px apart. */
export function tickInterval(pixelsPerFrame: number, fps: number): number {
  const targetPixels = 80;
  const rawFrames = targetPixels / pixelsPerFrame;
  const candidates = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1200, 1800, 3600].map(
    (s) => s * fps,
  );
  return candidates.find((c) => c >= rawFrames) ?? candidates[candidates.length - 1];
}

/** Minor subdivisions: first of [10,5,4,2] where each minor ≥ 12px. */
export function minorSubdivisions(
  framesPerMajor: number,
  pixelsPerFrame: number,
): number {
  const majorPixels = framesPerMajor * pixelsPerFrame;
  for (const divisions of [10, 5, 4, 2]) {
    if (majorPixels / divisions >= 12) return divisions;
  }
  return 0;
}

// --- Timecode (Utilities/TimeFormatting.swift) — HH:MM:SS:FF ---

function twoDigit(value: number): string {
  return value >= 0 && value < 10 ? `0${value}` : `${value}`;
}

export function formatTimecode(frame: number, fps: number): string {
  if (fps <= 0) return "00:00:00:00";
  const absFrame = Math.abs(frame);
  const totalSeconds = Math.floor(absFrame / fps);
  const ff = absFrame % fps;
  const ss = totalSeconds % 60;
  const mm = Math.floor(totalSeconds / 60) % 60;
  const hh = Math.floor(totalSeconds / 3600);
  const sign = frame < 0 ? "-" : "";
  return `${sign}${twoDigit(hh)}:${twoDigit(mm)}:${twoDigit(ss)}:${twoDigit(ff)}`;
}

// --- Keyframe sampling (Models/Keyframe.swift `sample`) ---
// `frame` is CLIP-RELATIVE. Switches on the LEAVING keyframe's interpolationOut.

export function sampleTrack(
  track: KeyframeTrackView | null | undefined,
  frame: number,
  fallback: number,
): number {
  const kfs = track?.keyframes;
  if (!kfs || kfs.length === 0) return fallback;
  if (kfs.length === 1) return kfs[0].value;
  if (frame <= kfs[0].frame) return kfs[0].value;
  const last = kfs[kfs.length - 1];
  if (frame >= last.frame) return last.value;

  // first kf whose frame > frame → segment [a, b]
  let bIdx = kfs.findIndex((k) => k.frame > frame);
  if (bIdx <= 0) return last.value;
  const a = kfs[bIdx - 1];
  const b = kfs[bIdx];
  const raw = (frame - a.frame) / (b.frame - a.frame);
  return interpolate(a.value, b.value, raw, a.interpolationOut);
}

function interpolate(
  a: number,
  b: number,
  raw: number,
  interp: Interpolation,
): number {
  switch (interp) {
    case "hold":
      return a;
    case "linear":
      return a + (b - a) * raw;
    case "smooth":
      return a + (b - a) * smoothstep(raw);
  }
}

// --- Clip value sampling (Models/Timeline.swift) ---

/** fadeMultiplier(at:) — rel-frame fade ramp; only `smooth` bends. */
export function fadeMultiplier(clip: ClipView, frame: number): number {
  const rel = frame - clip.startFrame;
  if (rel < 0 || rel > clip.durationFrames) return 0;
  const inMul =
    clip.fadeInFrames > 0
      ? ramp(Math.min(1, rel / clip.fadeInFrames), clip.fadeInInterpolation)
      : 1;
  const outRel = clip.durationFrames - rel;
  const outMul =
    clip.fadeOutFrames > 0
      ? ramp(Math.min(1, outRel / clip.fadeOutFrames), clip.fadeOutInterpolation)
      : 1;
  return Math.min(inMul, outMul);
}

function ramp(t: number, interp: Interpolation): number {
  // linear AND hold both treated as a linear ramp for fades; only smooth bends.
  return interp === "smooth" ? smoothstep(t) : t;
}

/** rawOpacityAt — opacity track sample, fallback static opacity. */
export function rawOpacityAt(clip: ClipView, frame: number): number {
  const rel = frame - clip.startFrame;
  return sampleTrack(clip.opacityTrack, rel, clip.opacity);
}

/** opacityAt — raw * fade (fade only for non-audio with a fade present). */
export function opacityAt(clip: ClipView, frame: number): number {
  const raw = rawOpacityAt(clip, frame);
  const hasFade = clip.fadeInFrames > 0 || clip.fadeOutFrames > 0;
  if (clip.mediaType !== "audio" && hasFade) {
    return raw * fadeMultiplier(clip, frame);
  }
  return raw;
}

/** volumeAt — static linear * kfGain * fade. Volume kf values are dB. */
export function volumeAt(clip: ClipView, frame: number): number {
  const rel = frame - clip.startFrame;
  const active = (clip.volumeTrack?.keyframes.length ?? 0) > 0;
  const kfGain = active
    ? VolumeScale.linearFromDb(sampleTrack(clip.volumeTrack, rel, 0))
    : 1.0;
  const hasFade = clip.fadeInFrames > 0 || clip.fadeOutFrames > 0;
  const fade = hasFade ? fadeMultiplier(clip, frame) : 1.0;
  return clip.volume * kfGain * fade;
}
