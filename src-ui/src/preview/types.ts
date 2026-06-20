// Preview viewport view-model types (E5-S10).
//
// The FRONTEND view types for the preview panel: tabs, the center-based Transform /
// Crop the overlays manipulate, and the per-tab playback view. These mirror the
// render-relevant subset of the Rust `palmier-engine` preview model
// (`PreviewTab` / `palmier-model::Transform` / `Crop`) — only what the viewport +
// overlays need to draw and hit-test.
//
// Transform is **center-based** (ruling #7): `centerX/centerY/width/height/rotation`
// in normalized 0..1 canvas space, matching `palmier-model::Transform`. The overlays
// edit this and flow the result back through Tauri commands into `palmier-engine`
// (FOUNDATION §4 strict layering — the webview never touches the engine directly).

import type { ClipType } from "../editor/types";

export type { ClipType };

/** A preview tab — the always-present timeline tab, or a closable per-asset tab. */
export type PreviewTab =
  | { kind: "timeline" }
  | { kind: "mediaAsset"; id: string; name: string; clipType: ClipType };

/** The fixed id of the always-present timeline tab (mirrors Rust `TIMELINE_TAB_ID`). */
export const TIMELINE_TAB_ID = "__timeline__";

/** The stable tab id (mirrors Rust `PreviewTab::id`). */
export function tabId(tab: PreviewTab): string {
  return tab.kind === "timeline" ? TIMELINE_TAB_ID : `media_${tab.id}`;
}

/** Whether a tab can be closed — every tab except the timeline. */
export function isCloseable(tab: PreviewTab): boolean {
  return tab.kind !== "timeline";
}

/** Display label for a tab. */
export function tabDisplayName(tab: PreviewTab): string {
  return tab.kind === "timeline" ? "Timeline" : tab.name;
}

/**
 * A center-based transform in normalized 0..1 canvas space.
 * Mirrors `palmier-model::Transform` (ruling #7).
 */
export interface Transform {
  centerX: number;
  centerY: number;
  width: number;
  height: number;
  /** Rotation in degrees, clockwise about the clip center. */
  rotation: number;
  flipHorizontal: boolean;
  flipVertical: boolean;
}

/** The identity transform (full canvas, no rotation/flip). */
export function identityTransform(): Transform {
  return {
    centerX: 0.5,
    centerY: 0.5,
    width: 1,
    height: 1,
    rotation: 0,
    flipHorizontal: false,
    flipVertical: false,
  };
}

/** Computed top-left corner `(centerX - width/2, centerY - height/2)`. */
export function topLeft(t: Transform): { x: number; y: number } {
  return { x: t.centerX - t.width / 2, y: t.centerY - t.height / 2 };
}

/** Build a transform from a top-left corner + size (reference `init(topLeft:…)`). */
export function transformFromTopLeft(
  tl: { x: number; y: number },
  width: number,
  height: number,
  base: Transform,
): Transform {
  return {
    ...base,
    centerX: tl.x + width / 2,
    centerY: tl.y + height / 2,
    width,
    height,
  };
}

/**
 * A crop expressed as inset fractions on each edge (0..1), matching the reference
 * `Crop`. `visibleWidthFraction = 1 - left - right`.
 */
export interface Crop {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** The identity crop (no inset on any edge). */
export function identityCrop(): Crop {
  return { left: 0, top: 0, right: 0, bottom: 0 };
}

/** Visible width fraction `1 - left - right` (reference `visibleWidthFraction`). */
export function visibleWidthFraction(c: Crop): number {
  return 1 - c.left - c.right;
}

/** Visible height fraction `1 - top - bottom`. */
export function visibleHeightFraction(c: Crop): number {
  return 1 - c.top - c.bottom;
}

/** Which corner of a handle rect is being dragged. */
export type Corner = "topLeft" | "topRight" | "bottomLeft" | "bottomRight";

export const ALL_CORNERS: Corner[] = ["topLeft", "topRight", "bottomLeft", "bottomRight"];
