// Preview settings presets (E5-S10) — aspect / quality / fps / zoom.
//
// Ported 1:1 from the macOS reference `PreviewContainerView.swift` private enums
// `AspectPreset` / `QualityPreset` / `ZoomPreset` and the fps list. The viewport's
// aspect/quality/zoom menu (docs/reference/preview-engine.md) drives these; selecting
// an aspect/quality/fps applies new timeline settings (width/height/fps) and zoom sets
// the canvas zoom. Values are verbatim so projects round-trip identically to macOS.

/** Aspect-ratio presets (reference `AspectPreset`). */
export interface AspectPreset {
  label: string;
  width: number;
  height: number;
}

export const ASPECT_PRESETS: AspectPreset[] = [
  { label: "16:9", width: 1920, height: 1080 },
  { label: "9:14", width: 1080, height: 1680 },
  { label: "9:16", width: 1080, height: 1920 },
  { label: "1:1", width: 1080, height: 1080 },
  { label: "4:3", width: 1440, height: 1080 },
  { label: "2.4:1", width: 2560, height: 1080 },
];

/** Frame-rate presets (reference fps list). */
export const FPS_PRESETS: number[] = [24, 25, 30, 50, 60];

/** Quality / resolution presets (reference `QualityPreset`), keyed by short edge. */
export interface QualityPreset {
  label: string;
  shortEdge: number;
}

export const QUALITY_PRESETS: QualityPreset[] = [
  { label: "720p", shortEdge: 720 },
  { label: "1080p", shortEdge: 1080 },
  { label: "2K", shortEdge: 1440 },
  { label: "4K", shortEdge: 2160 },
];

/**
 * Scale a resolution to a quality preset's short edge, preserving aspect ratio
 * (reference `QualityPreset.resolution(currentWidth:currentHeight:)`).
 */
export function qualityResolution(
  preset: QualityPreset,
  currentWidth: number,
  currentHeight: number,
): { width: number; height: number } {
  const target = preset.shortEdge;
  if (currentWidth <= currentHeight) {
    return { width: target, height: Math.trunc((target * currentHeight) / currentWidth) };
  }
  return { width: Math.trunc((target * currentWidth) / currentHeight), height: target };
}

/** Whether a quality preset matches the current resolution (reference `matches`). */
export function qualityMatches(preset: QualityPreset, width: number, height: number): boolean {
  return Math.min(width, height) === preset.shortEdge;
}

/** Quality badge label for a resolution (reference `qualityBadgeLabel`). */
export function qualityBadgeLabel(width: number, height: number): string {
  const h = Math.min(width, height);
  if (h <= 720) return "HD";
  if (h <= 1080) return "FHD";
  if (h <= 1440) return "2K";
  return "4K";
}

/** Greatest common divisor (for the aspect badge). */
function gcd(a: number, b: number): number {
  a = Math.abs(a);
  b = Math.abs(b);
  while (b) {
    [a, b] = [b, a % b];
  }
  return a || 1;
}

/** Aspect badge label `w:h` reduced (reference `aspectBadgeLabel`). */
export function aspectBadgeLabel(width: number, height: number): string {
  const g = gcd(width, height);
  return `${width / g}:${height / g}`;
}

/** Zoom presets (reference `ZoomPreset`). `fit` is value 1.0 ("Fit"). */
export interface ZoomPreset {
  label: string;
  value: number;
}

export const ZOOM_PRESETS: ZoomPreset[] = [
  { label: "25%", value: 0.25 },
  { label: "50%", value: 0.5 },
  { label: "75%", value: 0.75 },
  { label: "Fit", value: 1.0 },
  { label: "125%", value: 1.25 },
  { label: "150%", value: 1.5 },
  { label: "200%", value: 2.0 },
];

/** Min/max canvas zoom (reference `PreviewNSView` cmd-scroll clamp 0.1..8.0). */
export const ZOOM_MIN = 0.1;
export const ZOOM_MAX = 8.0;

/** The zoom badge label (reference `zoomBadgeLabel`): "Fit" at 1.0, else a percent. */
export function zoomBadgeLabel(zoom: number): string {
  if (Math.abs(zoom - 1.0) < 0.01) return "Fit";
  return `${Math.round(zoom * 100)}%`;
}

/** Whether a zoom preset is the active zoom (reference `isZoomPresetActive`). */
export function isZoomPresetActive(preset: ZoomPreset, zoom: number): boolean {
  return Math.abs(zoom - preset.value) < 0.01;
}
