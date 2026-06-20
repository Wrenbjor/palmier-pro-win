// Timeline drawing constants + track-type colors (E3-S9).
//
// Ported 1:1 from the macOS reference `AppTheme.swift` / `Utilities/Constants.swift`.
// Per reconciliation ruling #21, `AppTheme.swift` is the ground truth for visuals
// and `track-text == track-image` (#B72DD2) is intentional (flagged as a possible
// upstream bug, kept for parity).
//
// These live INSIDE the editor module so the timeline canvas is self-contained
// and does not depend on the (currently empty) shared `src-ui/src/design/tokens.json`.
// `loadTrackColors()` will prefer tokens.json values when that file later defines
// `track.{video,audio,image,text,lottie}`, falling back to these constants.

import tokens from "../design/tokens.json";
import type { ClipType, Interpolation } from "./types";

/** rgba string helper. */
export function rgba(r: number, g: number, b: number, a = 1): string {
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

// --- Track-type colors (AppTheme.TrackColor) ---
const DEFAULT_TRACK_COLORS: Record<ClipType, [number, number, number]> = {
  video: [0x00, 0x91, 0xc2],
  audio: [0x58, 0xa8, 0x22],
  image: [0xb7, 0x2d, 0xd2],
  text: [0xb7, 0x2d, 0xd2], // == image (ruling #21, kept for parity)
  lottie: [0xe0, 0xa8, 0x00],
};

type TokenShape = { track?: Partial<Record<ClipType, string>> };

function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

/**
 * Resolve the [r,g,b] for a track type, preferring tokens.json when present.
 * tokens.json is currently `{}` so this returns the AppTheme defaults today.
 */
export function trackRgb(type: ClipType): [number, number, number] {
  const t = tokens as TokenShape;
  const fromToken = t.track?.[type];
  if (fromToken) {
    const parsed = parseHex(fromToken);
    if (parsed) return parsed;
  }
  return DEFAULT_TRACK_COLORS[type];
}

export function trackColor(type: ClipType, alpha = 1): string {
  const [r, g, b] = trackRgb(type);
  return rgba(r, g, b, alpha);
}

/** Linear RGBA lerp (replaces NSColor.blended(withFraction:) for the waveform tint). */
export function blend(
  base: [number, number, number],
  other: [number, number, number],
  fraction: number,
): [number, number, number] {
  return [
    Math.round(base[0] + (other[0] - base[0]) * fraction),
    Math.round(base[1] + (other[1] - base[1]) * fraction),
    Math.round(base[2] + (other[2] - base[2]) * fraction),
  ];
}

// --- AppTheme palette (subset used by the timeline) ---
export const Theme = {
  background: {
    base: rgba(10, 10, 10),
    surface: rgba(22, 22, 22),
    raised: rgba(30, 30, 30),
    prominent: rgba(44, 44, 44),
  },
  border: {
    primary: rgba(255, 255, 255, 0.16),
    subtle: rgba(255, 255, 255, 0.12),
  },
  text: {
    primary: rgba(255, 255, 255, 1),
    secondary: rgba(255, 255, 255, 0.8),
    tertiary: rgba(255, 255, 255, 0.62),
    muted: rgba(255, 255, 255, 0.34),
  },
  status: {
    // AppTheme.Status.error = #E54F4F
    error: [0xe5, 0x4f, 0x4f] as [number, number, number],
  },
  // AppTheme.Opacity
  opacity: { moderate: 0.25, prominent: 0.8 },
  playhead: rgba(255, 59, 48), // systemRed
  selectionStroke: rgba(255, 255, 255, 0.9),
  keyframeFill: rgba(255, 204, 0, 0.95), // systemYellow
} as const;

// --- Layout / Trim / Snap / Defaults (Constants.swift) ---
export const Layout = {
  trackHeight: 50,
  rulerHeight: 24,
  trackHeaderWidth: 100,
  dropZoneHeight: 60,
  insertThreshold: 10,
  dragThreshold: 3,
} as const;

export const Trim = {
  handleWidth: 4,
  clipCornerRadius: 3,
} as const;

export const Snap = {
  thresholdPixels: 8,
  stickyMultiplier: 1.5, // ruling #10 — NOT 2.5
  playheadMultiplier: 1.5,
} as const;

export const Defaults = {
  pixelsPerFrame: 4.0,
} as const;

// Clip card sub-geometry (ClipRenderer.swift)
export const ClipRender = {
  labelBarHeight: 16,
  stripWidth: 3,
  volumeKeyframeSize: 7,
  volumeFadeHandleEdgeInset: 6,
  fadeKneeTopInset: 4,
  /** Rubber-band DRAW axis — distinct from VolumeScale editing range (ruling #9). */
  volumeRubberBandTopDb: 6,
  volumeRubberBandBottomDb: -60,
} as const;

// VolumeScale — editing range / dB<->linear. Floor -60, ceiling +15 (ruling #9).
export const VolumeScale = {
  floorDb: -60,
  ceilingDb: 15,
  dbFromLinear(linear: number): number {
    if (linear <= 0) return VolumeScale.floorDb;
    return Math.min(
      VolumeScale.ceilingDb,
      Math.max(VolumeScale.floorDb, 20 * Math.log10(linear)),
    );
  },
  linearFromDb(db: number): number {
    if (db <= VolumeScale.floorDb) return 0;
    return Math.pow(10, Math.min(db, VolumeScale.ceilingDb) / 20);
  },
} as const;

/** smoothstep(t) = t*t*(3 - 2t) — reference easing for smooth interpolation. */
export function smoothstep(t: number): number {
  return t * t * (3 - 2 * t);
}

/** Resolve the fade curve interpolation — only `smooth` bends (reference parity). */
export function isSmooth(i: Interpolation): boolean {
  return i === "smooth";
}
