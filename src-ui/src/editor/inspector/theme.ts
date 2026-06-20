// Inspector drawing constants — mirrors the editor/media-panel `theme.ts`
// conventions (self-contained, no shared-package dependency).
//
// Ported from `AppTheme.swift`. Per reconciliation ruling #21, accent-primary is
// #F5EFE4 and accent-timecode is #F29933 (NOT FOUNDATION §9's values). These live
// INSIDE the inspector module so the panel is self-contained (scope guard for the
// concurrent E12-S3..S9 siblings).

export function rgba(r: number, g: number, b: number, a = 1): string {
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

/** AppTheme palette subset used by the Inspector shell (matches editor Theme). */
export const Theme = {
  background: {
    base: rgba(10, 10, 10),
    surface: rgba(22, 22, 22),
    raised: rgba(30, 30, 30),
    headerBar: rgba(16, 16, 16),
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
  // accent-primary #F5EFE4 (ruling #21) — primary text / active tab.
  accent: rgba(0xf5, 0xef, 0xe4),
  // accent-timecode #F29933 (ruling #21) — timecode / numeric emphasis.
  accentTimecode: rgba(0xf2, 0x99, 0x33),
} as const;

/** AppTheme.Spacing scale (Constants.swift). */
export const Spacing = {
  xs: 4,
  sm: 6,
  smMd: 7,
  md: 8,
  lg: 12,
  xl: 16,
} as const;

/** AppTheme.FontSize scale. */
export const FontSize = {
  xxs: 9,
  xs: 11,
  sm: 12,
  md: 13,
} as const;

/** Wide letter-tracking for uppercased section headers (AppTheme.Tracking.wide). */
export const Tracking = {
  wide: 0.6,
} as const;
