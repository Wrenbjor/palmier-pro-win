// Agent-panel drawing constants — mirrors the media-panel's `theme.ts` conventions.
//
// Ported from the macOS reference `AppTheme.swift`. Per reconciliation ruling #21,
// AppTheme is the ground truth for visuals; accent-primary = `#F5EFE4`,
// accent-timecode = `#F29933`. These live INSIDE the agent-panel module so the panel
// is self-contained and does not depend on the shared package.json/lockfile (scope
// guard). `tokens.json` is `{}` today; when it later defines `accent.*` those win.

import tokens from "../design/tokens.json";

export function rgba(r: number, g: number, b: number, a = 1): string {
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

type TokenShape = { accent?: { primary?: string; ai?: string } };

function parseHex(hex: string): [number, number, number] | null {
  const m = /^#?([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

function accentFromToken(fallback: string): string {
  const t = tokens as TokenShape;
  const hex = t.accent?.primary;
  if (hex) {
    const parsed = parseHex(hex);
    if (parsed) return rgba(parsed[0], parsed[1], parsed[2]);
  }
  return fallback;
}

// --- AppTheme palette (subset used by the panel; matches media-panel's Theme) ---
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
  // AppTheme accent-primary #F5EFE4 (ruling #21).
  accent: accentFromToken(rgba(0xf5, 0xef, 0xe4)),
  // accent-timecode #F29933 (ruling #21) — used for the AI / streaming spinner.
  accentTimecode: rgba(0xf2, 0x99, 0x33),
  // Speech bubbles.
  userBubble: rgba(0xf2, 0x99, 0x33, 0.16),
  assistantBubble: rgba(255, 255, 255, 0.05),
  toolBlock: rgba(255, 255, 255, 0.04),
  status: {
    error: rgba(0xe5, 0x4f, 0x4f),
    errorBg: rgba(0xe5, 0x4f, 0x4f, 0.12),
    success: rgba(0x58, 0xa8, 0x22),
  },
} as const;

// --- Spacing scale (Constants.swift `Spacing`) --------------------------------
export const Spacing = {
  xs: 4,
  sm: 6,
  md: 8,
  lg: 12,
  xl: 16,
} as const;

export const Interaction = {
  /**
   * Below this many px from the bottom we treat the list as "pinned" and keep
   * auto-scrolling on new content; past it the user has scrolled up so we stop and
   * show the jump-to-bottom button (agent-panel.md line 232).
   */
  autoScrollThresholdPx: 48,
} as const;
