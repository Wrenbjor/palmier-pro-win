// Media-panel drawing constants — mirrors the editor's `theme.ts` conventions.
//
// Ported from the macOS reference `AppTheme.swift` / `Constants.swift`. Per
// reconciliation ruling #21, AppTheme is the ground truth for visuals and
// `track-text == track-image` (#B72DD2) is intentional (kept for parity). These
// live INSIDE the media-panel module so the panel is self-contained and does not
// depend on the shared package.json/lockfile (scope guard). `tokens.json` values
// are preferred when that file later defines `track.*` (it is `{}` today).

import tokens from "../design/tokens.json";
import type { MediaType } from "./types";

export function rgba(r: number, g: number, b: number, a = 1): string {
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

// --- Media/track-type colors (AppTheme.TrackColor) — same hexes as editor ------
const DEFAULT_TYPE_COLORS: Record<MediaType, [number, number, number]> = {
  video: [0x00, 0x91, 0xc2],
  audio: [0x58, 0xa8, 0x22],
  image: [0xb7, 0x2d, 0xd2],
  text: [0xb7, 0x2d, 0xd2], // == image (ruling #21, kept for parity)
  lottie: [0xe0, 0xa8, 0x00],
};

type TokenShape = { track?: Partial<Record<MediaType, string>> };

function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

export function typeRgb(type: MediaType): [number, number, number] {
  const t = tokens as TokenShape;
  const fromToken = t.track?.[type];
  if (fromToken) {
    const parsed = parseHex(fromToken);
    if (parsed) return parsed;
  }
  return DEFAULT_TYPE_COLORS[type];
}

export function typeColor(type: MediaType, alpha = 1): string {
  const [r, g, b] = typeRgb(type);
  return rgba(r, g, b, alpha);
}

// --- AppTheme palette (subset used by the panel; matches editor's Theme) -------
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
  // AppTheme accent (computed value, ruling #21): accent-primary #F5EFE4
  accent: rgba(0xf5, 0xef, 0xe4),
  // accent-timecode #F29933 (ruling #21)
  accentTimecode: rgba(0xf2, 0x99, 0x33),
  selectionFill: rgba(0xf2, 0x99, 0x33, 0.22),
  selectionStroke: rgba(0xf2, 0x99, 0x33, 0.9),
  marqueeFill: rgba(0xf2, 0x99, 0x33, 0.12),
  marqueeStroke: rgba(0xf2, 0x99, 0x33, 0.7),
  status: {
    error: rgba(0xe5, 0x4f, 0x4f),
    errorRgb: [0xe5, 0x4f, 0x4f] as [number, number, number],
  },
} as const;

// --- Spacing scale (Constants.swift `Spacing`) — load-bearing for grid math ----
// gridDimensions uses Spacing.xl for inter-tile spacing and Spacing.md*2 for the
// outer padding (media-panel.md §"Sorting / filtering / view modes").
export const Spacing = {
  xs: 4,
  sm: 6,
  md: 8,
  lg: 12,
  xl: 16,
} as const;

// --- Selection / double-click constants ---------------------------------------
export const Interaction = {
  /** Marquee starts after a 3px drag (reference `DragGesture(minDistance: 3)`). */
  marqueeMinDistance: 3,
  /** Double-click window in ms (NSEvent.doubleClickInterval default ~500ms). */
  doubleClickIntervalMs: 500,
} as const;
