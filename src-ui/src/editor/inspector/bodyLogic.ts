// Inspector tab-BODY logic (E12-S3..S8) — PURE functions, no React, no Tauri.
//
// The interactive field components (ScrubbableNumberField, InspectorPositionFields,
// the Video/Audio/Text tabs) are thin views over these. Everything here is a pure
// function of its inputs so it is exercised by `tsc --noEmit` + the framework-free
// `body.checks.ts` (same convention as `logic.ts` / `parity.checks.ts`).
//
// Behaviour ported from the reference `Inspector/Components/ScrubbableNumberField
// .swift`, `InspectorPositionFields.swift`, and `InspectorView.swift`
// (sharedClipValue / scrub-field ranges). Govern: `docs/reference/inspector.md`
// §ScrubbableNumberField / §Scrub-field-ranges / §Audio-tab.

import type { ClipView, Interpolation, KeyframeTrackView } from "../types";
import { VolumeScale } from "../theme";

// ── Scrub math (reference ScrubbableNumberField) ─────────────────────────────

/** The drag-modifier multiplier applied to base sensitivity. */
export type ScrubModifier = "none" | "coarse" | "fine";

/** Window-space px the pointer must travel before a scrub begins (reference 3 px). */
export const SCRUB_DRAG_THRESHOLD = 3;

/** Resolve the sensitivity multiplier for a modifier (Shift coarse ×10, Ctrl fine ×0.1). */
export function scrubModifierMultiplier(mod: ScrubModifier): number {
  switch (mod) {
    case "coarse":
      return 10;
    case "fine":
      return 0.1;
    default:
      return 1;
  }
}

/** A scrub field's numeric contract (reference field config). */
export interface ScrubRange {
  /** Inclusive lower bound. */
  min: number;
  /** Inclusive upper bound (use Infinity for open-ended, e.g. Scale). */
  max: number;
  /** Base per-pixel sensitivity (before modifier). */
  sensitivity: number;
  /**
   * Display multiplier — the VALUE is multiplied by this for display and divided
   * back on parse (e.g. Scale stores 0..1 but shows 0..100 %). Treated as 1 if 0.
   */
  displayMultiplier: number;
  /** Decimal places shown. */
  decimals: number;
  /** Trailing unit suffix (e.g. " %", " °", " s", " dB", " x", ""). */
  suffix: string;
}

/** clamp(v, min, max). */
export function clamp(v: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, v));
}

/** displayMultiplier treated as 1 when 0 (reference `mult == 0 ? 1 : mult`). */
export function effectiveMultiplier(displayMultiplier: number): number {
  return displayMultiplier === 0 ? 1 : displayMultiplier;
}

/**
 * `next = clamp(dragStartValue + dx * sens / mult)` (reference). `dx` is window-space
 * px since drag start; `mult` is the display multiplier (so a screen px maps to a
 * value step regardless of the display scale).
 */
export function scrubNext(
  dragStartValue: number,
  dx: number,
  range: ScrubRange,
  mod: ScrubModifier,
): number {
  const sens = range.sensitivity * scrubModifierMultiplier(mod);
  const mult = effectiveMultiplier(range.displayMultiplier);
  const raw = dragStartValue + (dx * sens) / mult;
  return clamp(raw, range.min, range.max);
}

/** Render a stored value for display: `value * displayMultiplier` to `decimals`, + suffix. */
export function formatScrub(value: number, range: ScrubRange): string {
  const shown = value * effectiveMultiplier(range.displayMultiplier);
  return `${shown.toFixed(range.decimals)}${range.suffix}`;
}

/**
 * Parse typed text back to a stored value (reference): strip suffix, trim, replace
 * "," with ".", parse float, DIVIDE by displayMultiplier, clamp to range. Returns
 * null when the text is not a finite number (caller keeps the prior value).
 */
export function parseScrub(text: string, range: ScrubRange): number | null {
  let s = text;
  if (range.suffix && s.endsWith(range.suffix)) s = s.slice(0, -range.suffix.length);
  s = s.trim().replace(",", ".");
  // Allow a bare suffix token left behind (e.g. user kept "%").
  s = s.replace(/[^0-9eE+\-.]/g, "");
  if (s === "" || s === "+" || s === "-" || s === ".") return null;
  const parsed = Number.parseFloat(s);
  if (!Number.isFinite(parsed)) return null;
  const stored = parsed / effectiveMultiplier(range.displayMultiplier);
  return clamp(stored, range.min, range.max);
}

// ── Shared-clip value (reference sharedClipValue) ────────────────────────────

/**
 * The shared value across `clips` for a numeric accessor, or null when they differ
 * ("mixed" — renders "—" and blocks scrub). Mirrors reference `sharedClipValue`:
 * equal within an epsilon to fold floating-point noise.
 */
export function sharedClipValue(
  clips: readonly ClipView[],
  get: (c: ClipView) => number,
  epsilon = 1e-6,
): number | null {
  if (clips.length === 0) return null;
  const first = get(clips[0]);
  for (let i = 1; i < clips.length; i++) {
    if (Math.abs(get(clips[i]) - first) > epsilon) return null;
  }
  return first;
}

/** The shared boolean across clips, or null when mixed. */
export function sharedClipBool(
  clips: readonly ClipView[],
  get: (c: ClipView) => boolean,
): boolean | null {
  if (clips.length === 0) return null;
  const first = get(clips[0]);
  for (let i = 1; i < clips.length; i++) {
    if (get(clips[i]) !== first) return null;
  }
  return first;
}

/** The shared string across clips, or null when mixed. */
export function sharedClipString(
  clips: readonly ClipView[],
  get: (c: ClipView) => string,
): string | null {
  if (clips.length === 0) return null;
  const first = get(clips[0]);
  for (let i = 1; i < clips.length; i++) {
    if (get(clips[i]) !== first) return null;
  }
  return first;
}

// ── Keyframe sampling for FIELD SEEDING (display only) ───────────────────────
//
// The scrub fields display the value AT the active frame so they reflect keyframe
// state (reference "Values sample at activeFrame"). For seeding the field this only
// needs the value the user would see — a step/last-keyframe lookup is sufficient
// and avoids importing the full interpolation engine into the inspector. The
// authoritative interpolation lives in `palmier-model`; this is a display helper.

/** CLIP-RELATIVE frame for an absolute timeline frame (reference keyframeOffset). */
export function keyframeOffset(clip: ClipView, frame: number): number {
  return frame - clip.startFrame;
}

/**
 * The value of a scalar track at a clip-relative frame, falling back to `fallback`
 * when the track is empty. Uses the nearest keyframe at-or-before the frame (step
 * lookup) — adequate for SEEDING a field; the renderer owns true interpolation.
 */
export function sampleScalarTrack(
  track: KeyframeTrackView | null | undefined,
  relFrame: number,
  fallback: number,
): number {
  const kfs = track?.keyframes;
  if (!kfs || kfs.length === 0) return fallback;
  let value = kfs[0].value;
  for (const kf of kfs) {
    if (kf.frame <= relFrame) value = kf.value;
    else break;
  }
  return value;
}

/** True iff `clip` has a keyframe at exactly `relFrame` on the given track. */
export function hasKeyframeAt(
  track: KeyframeTrackView | null | undefined,
  relFrame: number,
): boolean {
  return !!track?.keyframes.some((k) => k.frame === relFrame);
}

/** The nearest keyframe frame strictly before `relFrame`, or null. */
export function previousKeyframeFrame(
  track: KeyframeTrackView | null | undefined,
  relFrame: number,
): number | null {
  const kfs = track?.keyframes;
  if (!kfs) return null;
  let prev: number | null = null;
  for (const kf of kfs) {
    if (kf.frame < relFrame) prev = kf.frame;
    else break;
  }
  return prev;
}

/** The nearest keyframe frame strictly after `relFrame`, or null. */
export function nextKeyframeFrame(
  track: KeyframeTrackView | null | undefined,
  relFrame: number,
): number | null {
  const kfs = track?.keyframes;
  if (!kfs) return null;
  for (const kf of kfs) {
    if (kf.frame > relFrame) return kf.frame;
  }
  return null;
}

// ── Keyframe-lane geometry + edit-row builders (E12-S8 follow-up) ─────────────
//
// The KEYFRAMES panel renders a per-property lane: a diamond per keyframe placed
// at `frameToLaneX(relFrame)`, draggable horizontally to MOVE the keyframe's frame
// (with a 4 px snap to the playhead + sibling keyframes), and right-clickable to
// pick its interpolation. All of these are pure functions so they are tsc-gated by
// `body.checks.ts` and the draw + hit-test share ONE frame↔x mapping (so a dragged
// diamond lands where it is shown).

/** The interpolation kinds the model / `set_keyframes` accept (reference wire form). */
export const INTERPOLATION_KINDS: readonly Interpolation[] = ["linear", "hold", "smooth"];

/** Display labels for the interpolation context menu. */
export const INTERPOLATION_LABEL: Record<Interpolation, string> = {
  linear: "Linear",
  hold: "Hold",
  smooth: "Smooth",
};

/**
 * The horizontal pixel snap radius for the keyframe lane (spec / SnapEngine 4 px).
 * Smaller than the timeline's 8 px base — the lane is denser and the reference
 * keyframe drag uses a tight 4 px catch.
 */
export const KEYFRAME_SNAP_PX = 4;

/** Map a clip-relative frame to a lane x (px). `pxPerFrame` is the lane's scale. */
export function frameToLaneX(relFrame: number, pxPerFrame: number): number {
  return relFrame * pxPerFrame;
}

/** Inverse of `frameToLaneX`: a lane x (px) → the nearest clip-relative frame. */
export function laneXToFrame(x: number, pxPerFrame: number): number {
  if (pxPerFrame <= 0) return 0;
  return Math.round(x / pxPerFrame);
}

/**
 * Snap a proposed clip-relative frame to the nearest snap target within
 * `KEYFRAME_SNAP_PX`. Targets are the playhead (clip-relative) and the OTHER
 * keyframes' frames on the same track (the dragged one excluded). Returns the
 * snapped frame, or `proposed` when nothing is in range. Mirrors the SnapEngine's
 * "closest target within a pixel threshold" rule, scoped to the lane.
 */
export function snapKeyframeFrame(
  proposed: number,
  pxPerFrame: number,
  playheadRel: number | null,
  otherFrames: readonly number[],
): number {
  if (pxPerFrame <= 0) return proposed;
  const thresholdFrames = KEYFRAME_SNAP_PX / pxPerFrame;
  const targets: number[] = [...otherFrames];
  if (playheadRel !== null) targets.push(playheadRel);
  let best = proposed;
  let bestDist = Number.POSITIVE_INFINITY;
  for (const t of targets) {
    const d = Math.abs(proposed - t);
    if (d <= thresholdFrames && d < bestDist) {
      bestDist = d;
      best = t;
    }
  }
  return best;
}

/**
 * Coerce a stored keyframe value (number | AnimPair `{a,b}`/`{x,y}` | crop) into an
 * `arity`-length number list — the leading values of a `set_keyframes` row.
 */
export function keyframeValues(value: unknown, arity: number): number[] {
  if (typeof value === "number") return arity === 1 ? [value] : new Array(arity).fill(value);
  if (value && typeof value === "object") {
    const rec = value as Record<string, number>;
    if (arity === 2) return [rec.a ?? rec.x ?? 0, rec.b ?? rec.y ?? 0];
    if (arity === 4) return [rec.top ?? 0, rec.right ?? 0, rec.bottom ?? 0, rec.left ?? 0];
  }
  return new Array(arity).fill(0);
}

/**
 * Build ONE `set_keyframes` row `[frame, …values, interp]` for a keyframe. The
 * trailing `interp` is always emitted so a moved/edited keyframe keeps its easing
 * (the tool accepts the bare-values form too, but emitting interp is lossless).
 */
export function keyframeRow(
  frame: number,
  value: unknown,
  arity: number,
  interp: Interpolation,
): (number | string)[] {
  return [frame, ...keyframeValues(value, arity), interp];
}

/** Build the full, frame-sorted `set_keyframes` row list for a track. */
export function keyframeRows(
  track: KeyframeTrackView | null | undefined,
  arity: number,
): (number | string)[][] {
  const kfs = track?.keyframes ?? [];
  return [...kfs]
    .sort((a, b) => a.frame - b.frame)
    .map((k) => keyframeRow(k.frame, k.value, arity, k.interpolationOut));
}

/**
 * Build the `set_keyframes` rows after MOVING the keyframe currently at `fromFrame`
 * to `toFrame` (values + interp preserved), dropping any keyframe that already sits
 * at the destination (a move onto a sibling collapses, mirroring upsert-by-frame),
 * and re-sorting by frame. Returns rows ready for `set_keyframes`.
 */
export function moveKeyframeRows(
  track: KeyframeTrackView | null | undefined,
  arity: number,
  fromFrame: number,
  toFrame: number,
): (number | string)[][] {
  const kfs = track?.keyframes ?? [];
  const moved = kfs.find((k) => k.frame === fromFrame);
  if (!moved) return keyframeRows(track, arity);
  const kept = kfs.filter((k) => k.frame !== fromFrame && k.frame !== toFrame);
  const next = [...kept, { ...moved, frame: toFrame }];
  next.sort((a, b) => a.frame - b.frame);
  return next.map((k) => keyframeRow(k.frame, k.value, arity, k.interpolationOut));
}

/**
 * Build the `set_keyframes` rows after changing the INTERPOLATION of the keyframe at
 * `frame` to `interp` (everything else preserved, frame-sorted). Mirrors the
 * reference per-keyframe interpolation menu.
 */
export function setKeyframeInterpRows(
  track: KeyframeTrackView | null | undefined,
  arity: number,
  frame: number,
  interp: Interpolation,
): (number | string)[][] {
  const kfs = track?.keyframes ?? [];
  return [...kfs]
    .sort((a, b) => a.frame - b.frame)
    .map((k) =>
      keyframeRow(k.frame, k.value, arity, k.frame === frame ? interp : k.interpolationOut),
    );
}

// ── Volume / dB bridge (E12-S1 VolumeScale; reference Audio-tab) ──────────────

/** The text shown at the dB floor (true-mute), reference "−∞ dB". */
export const NEG_INF_DB = "−∞ dB";

/**
 * The dB value to show for a clip's volume at `activeFrame` (reference
 * `liveVolumeKfDb(at:activeFrame) ?? VolumeScale.dbFromLinear(clip.volume)`): sample
 * the volume keyframe track when present, else the static scalar.
 */
export function volumeDb(clip: ClipView, activeFrame?: number): number {
  if (clip.volumeTrack && clip.volumeTrack.keyframes.length > 0 && activeFrame != null) {
    const lin = sampleScalarTrack(
      clip.volumeTrack,
      keyframeOffset(clip, activeFrame),
      clip.volume,
    );
    return VolumeScale.dbFromLinear(lin);
  }
  return VolumeScale.dbFromLinear(clip.volume);
}

/** Render a dB value: floor → "−∞ dB", else `%.1f dB`. */
export function formatVolumeDb(db: number): string {
  if (db <= VolumeScale.floorDb) return NEG_INF_DB;
  return `${db.toFixed(1)} dB`;
}

/**
 * The linear volume to STORE for a given dB. At the floor we store true-mute 0
 * (reference "stores true-mute 0"); otherwise `VolumeScale.linearFromDb`.
 */
export function volumeLinearFromDb(db: number): number {
  if (db <= VolumeScale.floorDb) return 0;
  return VolumeScale.linearFromDb(db);
}

// ── Fade seconds ↔ frames (reference Fade In/Out rows) ───────────────────────

/** `frames = round(seconds * fps)` (reference). */
export function fadeFramesFromSeconds(seconds: number, fps: number): number {
  return Math.round(seconds * fps);
}

/** `seconds = frames / fps` (guard fps 0). */
export function fadeSecondsFromFrames(frames: number, fps: number): number {
  return fps > 0 ? frames / fps : 0;
}

/**
 * `maxSeconds` for a fade scrub: single clip → its `durationFrames / fps`, else the
 * reference fallback 60.0.
 */
export function fadeMaxSeconds(clips: readonly ClipView[], fps: number): number {
  if (clips.length === 1 && fps > 0) return clips[0].durationFrames / fps;
  return 60.0;
}

// ── Field range presets (reference §Scrub-field-ranges; EXACT) ────────────────

/** Build the Position X/Y range (mult = canvas dimension; `%.0f`). */
export function positionRange(canvasDimension: number): ScrubRange {
  return {
    min: -10,
    max: 10,
    sensitivity: 0.01,
    displayMultiplier: canvasDimension,
    decimals: 0,
    suffix: "",
  };
}

/** Scale `0.01..∞`, mult 100, `%.0f %`. */
export const SCALE_RANGE: ScrubRange = {
  min: 0.01,
  max: Infinity,
  sensitivity: 0.01,
  displayMultiplier: 100,
  decimals: 0,
  suffix: " %",
};

/** Rotation `-3600..3600`, `%.0f °`. */
export const ROTATION_RANGE: ScrubRange = {
  min: -3600,
  max: 3600,
  sensitivity: 0.5,
  displayMultiplier: 1,
  decimals: 0,
  suffix: " °",
};

/** Opacity `0..1`, mult 100, `%.0f %`. */
export const OPACITY_RANGE: ScrubRange = {
  min: 0,
  max: 1,
  sensitivity: 0.01,
  displayMultiplier: 100,
  decimals: 0,
  suffix: " %",
};

/** Speed `0.25..4.0`, `%.2f x`, sens 0.01. */
export const SPEED_RANGE: ScrubRange = {
  min: 0.25,
  max: 4.0,
  sensitivity: 0.01,
  displayMultiplier: 1,
  decimals: 2,
  suffix: " x",
};

/** Volume `-60..+15 dB` (E12-S1 constants), `%.1f dB`, sens 0.3. */
export const VOLUME_DB_RANGE: ScrubRange = {
  min: VolumeScale.floorDb,
  max: VolumeScale.ceilingDb,
  sensitivity: 0.3,
  displayMultiplier: 1,
  decimals: 1,
  suffix: " dB",
};

/** Text Size `12..300 pt`, `%.0f pt`. */
export const FONT_SIZE_RANGE: ScrubRange = {
  min: 12,
  max: 300,
  sensitivity: 0.5,
  displayMultiplier: 1,
  decimals: 0,
  suffix: " pt",
};

/** Build a Fade In/Out range `0..maxSeconds`, `%.2f s`, sens 0.02. */
export function fadeRange(maxSeconds: number): ScrubRange {
  return {
    min: 0,
    max: maxSeconds,
    sensitivity: 0.02,
    displayMultiplier: 1,
    decimals: 2,
    suffix: " s",
  };
}

// ── set_clip_properties arg builders (matches palmier-tools/properties.rs) ────

/** A `transform` patch (center-based, ruling #7) for `set_clip_properties`. */
export interface TransformPatchArg {
  centerX?: number;
  centerY?: number;
  width?: number;
  height?: number;
  flipHorizontal?: boolean;
  flipVertical?: boolean;
}

/** The full `set_clip_properties` argument object. */
export interface ClipPropertiesArgs {
  clipIds: string[];
  durationFrames?: number;
  trimStartFrame?: number;
  trimEndFrame?: number;
  speed?: number;
  volume?: number;
  opacity?: number;
  transform?: TransformPatchArg;
  /** Static rotation in degrees (clockwise). Clears the rotation keyframe track. */
  rotation?: number;
  /** Fade-in ramp length in frames (clamped to duration server-side). */
  fadeInFrames?: number;
  /** Fade-out ramp length in frames (clamped so fadeIn + fadeOut <= duration). */
  fadeOutFrames?: number;
  /** Fade-in easing: "linear" | "hold" | "smooth". */
  fadeInInterpolation?: string;
  /** Fade-out easing: "linear" | "hold" | "smooth". */
  fadeOutInterpolation?: string;
  content?: string;
  fontName?: string;
  fontSize?: number;
  /** Hex color string (validated server-side via parse_rgba). */
  color?: string;
  /** "left" | "center" | "right". */
  alignment?: string;
}

/** Build the args object for `editorEdit('set_clip_properties', …)`, dropping undefined. */
export function clipPropertiesArgs(
  clipIds: readonly string[],
  patch: Omit<ClipPropertiesArgs, "clipIds">,
): Record<string, unknown> {
  const out: Record<string, unknown> = { clipIds: [...clipIds] };
  for (const [k, v] of Object.entries(patch)) {
    if (v === undefined) continue;
    if (k === "transform" && v && typeof v === "object") {
      const t: Record<string, unknown> = {};
      for (const [tk, tv] of Object.entries(v as TransformPatchArg)) {
        if (tv !== undefined) t[tk] = tv;
      }
      if (Object.keys(t).length > 0) out.transform = t;
    } else {
      out[k] = v;
    }
  }
  return out;
}

// ── Color hex helpers (ColorField; matches server parse_rgba) ─────────────────

/** Clamp a 0..1 channel to a 0..255 int. */
function chan255(v: number): number {
  return clamp(Math.round(v * 255), 0, 255);
}

/** Convert sRGB RGBA (each 0..1) to a `#RRGGBBAA` hex string (server-accepted). */
export function rgbaToHex(r: number, g: number, b: number, a = 1): string {
  const h = (n: number) => chan255(n).toString(16).padStart(2, "0");
  return `#${h(r)}${h(g)}${h(b)}${h(a)}`;
}

/** Parse `#RGB`/`#RRGGBB`/`#RRGGBBAA` to RGBA 0..1, or null if malformed. */
export function hexToRgba(
  hex: string,
): { r: number; g: number; b: number; a: number } | null {
  let s = hex.trim();
  if (s.startsWith("#")) s = s.slice(1);
  if (s.length === 3) s = s.split("").map((c) => c + c).join("") + "ff";
  else if (s.length === 6) s = s + "ff";
  else if (s.length !== 8) return null;
  if (!/^[0-9a-fA-F]{8}$/.test(s)) return null;
  const n = (i: number) => parseInt(s.slice(i, i + 2), 16) / 255;
  return { r: n(0), g: n(2), b: n(4), a: n(6) };
}

// ── Byte / dimension formatters (Details tab) ────────────────────────────────

/** Human-readable byte size (reference byte formatter; binary-ish, 1 decimal). */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(1)} ${units[i]}`;
}
