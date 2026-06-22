// Envelope (volume rubber band / opacity line) mapping + hit-testing (E12-S?).
//
// FOUNDATION §479: "Volume rubber band = line across audio clips for volume_at(frame)
// (drag to set; Alt-drag inserts keyframe). Opacity envelope = line across video clips
// for opacity_at(frame)" — same interaction model.
//
// This is the ONE shared mapping the renderer draws with AND the input controller
// hit-tests against, so the line you grab is exactly the line drawn (no draw/hit-test
// drift). The value↔y math mirrors `renderer.ts:yForDb` (volume) and the opacity line
// mapping defined here; the body rect mirrors `renderer.ts:clipBodyRect`.
//
// Value semantics (matching the view-model + backend tools):
//   • Volume keyframe values are dB (renderer `yForDb`, geometry `volumeAt` →
//     `linearFromDb(sample)`); the STATIC `clip.volume` is linear and the band draws at
//     `yForDb(dbFromLinear(volume))`. Drag-to-set stores linear volume via
//     `set_clip_properties { volume }`; Alt-drag inserts a dB keyframe via
//     `set_keyframes { property: "volume", keyframes: [[frame, dB]] }`.
//   • Opacity values are 0..1 linear for both the static `clip.opacity` and the opacity
//     keyframe track. Drag-to-set stores `set_clip_properties { opacity }`; Alt-drag
//     inserts an opacity keyframe via `set_keyframes { property: "opacity", … }`.

import type { ClipView } from "./types";
import type { Rect } from "./geometry";
import { sampleTrack } from "./geometry";
import { ClipRender, VolumeScale } from "./theme";

/** Which envelope a clip carries — audio → volume band, else opacity line. */
export type EnvelopeKind = "volume" | "opacity";

export function envelopeKindFor(clip: ClipView): EnvelopeKind {
  return clip.mediaType === "audio" ? "volume" : "opacity";
}

/**
 * Body area below the label bar — IDENTICAL to `renderer.ts:clipBodyRect` so the line
 * drawn and the line hit-tested live in the same rect.
 */
export function envelopeBodyRect(rect: Rect): Rect {
  return {
    x: rect.x,
    y: rect.y + ClipRender.labelBarHeight,
    w: rect.w,
    h: Math.max(0, rect.h - ClipRender.labelBarHeight - 1),
  };
}

// ── Volume (dB) axis — mirrors renderer.ts:yForDb ────────────────────────────

/** y(forDb) — high dB → smaller y across the body (renderer.ts:yForDb, §272). */
export function yForDb(db: number, body: Rect): number {
  const top = ClipRender.volumeRubberBandTopDb;
  const bottom = ClipRender.volumeRubberBandBottomDb;
  const clamped = Math.min(top, Math.max(bottom, db));
  const frac = (top - clamped) / (top - bottom);
  return body.y + frac * body.h;
}

/** Inverse of `yForDb`: a body y → dB, clamped to the DRAW axis [bottom, top]. */
export function dbForY(y: number, body: Rect): number {
  const top = ClipRender.volumeRubberBandTopDb;
  const bottom = ClipRender.volumeRubberBandBottomDb;
  if (body.h <= 0) return top;
  const frac = Math.min(1, Math.max(0, (y - body.y) / body.h));
  return top - frac * (top - bottom);
}

// ── Opacity (0..1) axis ──────────────────────────────────────────────────────

/** y(forOpacity) — opacity 1 → top of body, 0 → bottom. */
export function yForOpacity(opacity: number, body: Rect): number {
  const clamped = Math.min(1, Math.max(0, opacity));
  const frac = 1 - clamped;
  return body.y + frac * body.h;
}

/** Inverse of `yForOpacity`: a body y → opacity 0..1. */
export function opacityForY(y: number, body: Rect): number {
  if (body.h <= 0) return 1;
  const frac = Math.min(1, Math.max(0, (y - body.y) / body.h));
  return 1 - frac;
}

// ── Sampling the drawn line value (clip-relative frame) ───────────────────────

/**
 * The value the envelope line is drawn at, in the line's NATIVE units (dB for volume,
 * 0..1 for opacity), at a clip-relative frame. Mirrors what the renderer draws:
 *   • volume: keyframe dB sampled, else flat `dbFromLinear(clip.volume)`.
 *   • opacity: keyframe value sampled, else flat `clip.opacity`.
 */
export function envelopeValueAt(clip: ClipView, relFrame: number): number {
  if (envelopeKindFor(clip) === "volume") {
    const kfs = clip.volumeTrack?.keyframes ?? [];
    if (kfs.length > 0) return sampleTrack(clip.volumeTrack, relFrame, kfs[0].value);
    return VolumeScale.dbFromLinear(clip.volume);
  }
  const kfs = clip.opacityTrack?.keyframes ?? [];
  if (kfs.length > 0) return sampleTrack(clip.opacityTrack, relFrame, kfs[0].value);
  return clip.opacity;
}

/** Map a native envelope value (dB | opacity) to a body y for the given kind. */
export function yForEnvelopeValue(
  kind: EnvelopeKind,
  value: number,
  body: Rect,
): number {
  return kind === "volume" ? yForDb(value, body) : yForOpacity(value, body);
}

/** Map a body y back to a native envelope value (dB | opacity) for the given kind. */
export function envelopeValueForY(
  kind: EnvelopeKind,
  y: number,
  body: Rect,
): number {
  return kind === "volume" ? dbForY(y, body) : opacityForY(y, body);
}

// ── Hit-testing ───────────────────────────────────────────────────────────────

/** Px tolerance around the drawn line for a grab to count (reference ~handle feel). */
export const ENVELOPE_HIT_TOLERANCE_PX = 5;

export interface EnvelopeHit {
  kind: EnvelopeKind;
  /** The drawn line y at the cursor frame (body-space, content coords). */
  lineY: number;
  /** The clip body rect the line lives in. */
  body: Rect;
}

/**
 * Hit-test a content-space point against a clip's envelope line. Returns the hit (with
 * the line y under the cursor) when the point is within `ENVELOPE_HIT_TOLERANCE_PX` of
 * the drawn line AND inside the body x-span; else null.
 *
 * `relFrame` is the clip-relative frame under the cursor (the renderer samples the line
 * per-x, so we sample at the same frame the cursor sits on).
 */
export function hitTestEnvelope(
  clip: ClipView,
  rect: Rect,
  px: number,
  py: number,
  relFrame: number,
  tolerance = ENVELOPE_HIT_TOLERANCE_PX,
): EnvelopeHit | null {
  if (clip.durationFrames <= 0) return null;
  const body = envelopeBodyRect(rect);
  if (body.h <= 0 || body.w <= 0) return null;
  // Must be within the clip's body x-span (exclude the trim handles' feel a little).
  if (px < rect.x || px > rect.x + rect.w) return null;
  if (py < body.y || py > body.y + body.h) return null;

  const kind = envelopeKindFor(clip);
  const value = envelopeValueAt(clip, relFrame);
  const lineY = yForEnvelopeValue(kind, value, body);
  if (Math.abs(py - lineY) > tolerance) return null;
  return { kind, lineY, body };
}

// ── set_keyframes row helpers (merge/insert one scalar kf, sorted) ────────────

/** A scalar keyframe row as the `set_keyframes` tool expects: `[frame, value, interp?]`. */
export type ScalarKfRow = [number, number] | [number, number, string];

/**
 * Merge a single scalar keyframe (clip-relative `frame`, native-unit `value`) into the
 * existing rows for a clip's property track, REPLACING any keyframe at the same frame
 * and keeping the list sorted by frame. Existing per-keyframe interpolation is carried
 * through; the inserted/updated keyframe uses `interp` (default "linear" — a flat
 * drag-set reads cleaner than the smooth default, and matches the band the user drew).
 *
 * Returns the full replacement row list for `set_keyframes` (which REPLACES the track).
 */
export function mergeScalarKeyframe(
  existing: readonly { frame: number; value: number; interpolationOut: string }[],
  frame: number,
  value: number,
  interp = "linear",
): ScalarKfRow[] {
  const rows: ScalarKfRow[] = existing
    .filter((k) => k.frame !== frame)
    .map((k) => [k.frame, k.value, k.interpolationOut] as ScalarKfRow);
  rows.push([frame, value, interp]);
  rows.sort((a, b) => a[0] - b[0]);
  return rows;
}
