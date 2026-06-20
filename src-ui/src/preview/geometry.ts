// Preview viewport + overlay geometry (E5-S10).
//
// Pure geometry ported 1:1 from the macOS reference preview surface:
//  - `PreviewContainerView.fitSize` / `videoContentRect` (TransformOverlayView,
//    CropOverlayView) — the letterboxed video rect inside the panel.
//  - `TransformOverlayView` move/resize gestures (center-based Transform, canvas-edge
//    + center snap, aspect lock, rotation-aware hit target).
//  - `CropOverlayView` pan/resize gestures (clip-local counter-rotation, rule-of-thirds
//    is drawn from `cropFrame`, aspect lock).
//  - `PreviewNSView.onCmdScroll` cmd/ctrl-scroll zoom-about-point.
//
// All of this is presentation-surface-agnostic: it computes rects/transforms in CSS
// px against a measured viewport size, so it works whether the wgpu frame is presented
// via the native child surface (E5-S1 plan A1) or any other surface — the overlays sit
// in the transparent webview region above it. Edits are returned as new center-based
// `Transform` / `Crop` values; the panel commits them via Tauri commands.

import type { Corner, Crop, Transform } from "./types";
import { topLeft, transformFromTopLeft, visibleHeightFraction, visibleWidthFraction } from "./types.ts";

/** A rectangle in CSS px (viewport-local). */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export function rectMidX(r: Rect): number {
  return r.x + r.width / 2;
}
export function rectMidY(r: Rect): number {
  return r.y + r.height / 2;
}

/** Canvas-edge snap threshold in px (reference `Snap.thresholdPixels`). */
export const SNAP_THRESHOLD_PX = 8;
/** Minimum normalized clip extent (reference `minSize`/`minVis` = 0.05). */
export const MIN_NORMALIZED = 0.05;

/**
 * The letterboxed video content rect inside a viewport of `viewSize`, for a video of
 * `videoAspect = width/height` (reference `videoContentRect` in both overlays). The
 * rect is centered; it fills the dimension that matches the smaller fit and centers
 * the other.
 */
export function videoContentRect(
  viewSize: { width: number; height: number },
  videoAspect: number,
): Rect {
  const viewAspect = viewSize.height > 0 ? viewSize.width / viewSize.height : 0;
  let w: number;
  let h: number;
  if (viewAspect > videoAspect) {
    h = viewSize.height;
    w = h * videoAspect;
  } else {
    w = viewSize.width;
    h = videoAspect > 0 ? w / videoAspect : 0;
  }
  return { x: (viewSize.width - w) / 2, y: (viewSize.height - h) / 2, width: w, height: h };
}

/**
 * The fit size (no zoom) of the canvas inside a container for a given aspect
 * (reference `PreviewContainerView.fitSize`). Distinct from `videoContentRect` only in
 * that it returns a size, not a centered rect; the panel multiplies it by `canvasZoom`.
 */
export function fitSize(
  container: { width: number; height: number },
  aspect: number,
): { width: number; height: number } {
  const widthFromHeight = container.height * aspect;
  if (widthFromHeight <= container.width) {
    return { width: widthFromHeight, height: container.height };
  }
  return { width: container.width, height: aspect > 0 ? container.width / aspect : 0 };
}

/** The clip's screen rect for a transform, inside a video rect (reference `clipFrame`). */
export function clipFrame(t: Transform, videoRect: Rect): Rect {
  const tl = topLeft(t);
  return {
    x: videoRect.x + tl.x * videoRect.width,
    y: videoRect.y + tl.y * videoRect.height,
    width: t.width * videoRect.width,
    height: t.height * videoRect.height,
  };
}

/** The crop's screen rect inside a clip rect (reference `cropFrame`). */
export function cropFrame(c: Crop, clipRect: Rect): Rect {
  return {
    x: clipRect.x + c.left * clipRect.width,
    y: clipRect.y + c.top * clipRect.height,
    width: visibleWidthFraction(c) * clipRect.width,
    height: visibleHeightFraction(c) * clipRect.height,
  };
}

// ── Transform move (reference TransformOverlayView.movedTransform) ────────────

/** Snap a normalized boundary value to 0 or 1 within a normalized threshold. */
function snapToBoundary(v: number, threshold: number): number {
  if (Math.abs(v) <= threshold) return 0;
  if (Math.abs(v - 1) <= threshold) return 1;
  return v;
}

/**
 * Move a transform by a screen-px translation, with canvas-edge + center snap when not
 * rotated (reference `movedTransform`). Returns the new transform and which center
 * guides are active. Snaps are skipped under rotation (their thresholds target the
 * axis-aligned bbox that no longer matches the visible edges).
 */
export function movedTransform(
  start: Transform,
  translation: { width: number; height: number },
  videoRect: Rect,
  rotated: boolean,
): { transform: Transform; snap: { x: boolean; y: boolean } } {
  if (videoRect.width <= 0 || videoRect.height <= 0) {
    return { transform: start, snap: { x: false, y: false } };
  }
  let t: Transform = {
    ...start,
    centerX: start.centerX + translation.width / videoRect.width,
    centerY: start.centerY + translation.height / videoRect.height,
  };
  if (rotated) return { transform: t, snap: { x: false, y: false } };

  // Canvas-edge snap: snap the clip's edges (top-left and bottom-right) to 0/1.
  const thX = SNAP_THRESHOLD_PX / videoRect.width;
  const thY = SNAP_THRESHOLD_PX / videoRect.height;
  t = snapTransformToCanvasEdges(t, thX, thY);

  // Center-to-center snap: snap clip center to canvas center (0.5).
  let snapX = false;
  let snapY = false;
  if (Math.abs(t.centerX - 0.5) <= thX) {
    t = { ...t, centerX: 0.5 };
    snapX = true;
  }
  if (Math.abs(t.centerY - 0.5) <= thY) {
    t = { ...t, centerY: 0.5 };
    snapY = true;
  }
  return { transform: t, snap: { x: snapX, y: snapY } };
}

/** Snap a transform's edges to the canvas edges (reference `snapToCanvasEdges`). */
function snapTransformToCanvasEdges(t: Transform, thX: number, thY: number): Transform {
  const tl = topLeft(t);
  let left = tl.x;
  let top = tl.y;
  const right = left + t.width;
  const bottom = top + t.height;
  // Snap whichever edge is near a boundary; keep size fixed (move only).
  const snappedLeft = snapToBoundary(left, thX);
  const snappedRight = snapToBoundary(right, thX);
  if (snappedLeft !== left) left = snappedLeft;
  else if (snappedRight !== right) left = snappedRight - t.width;
  const snappedTop = snapToBoundary(top, thY);
  const snappedBottom = snapToBoundary(bottom, thY);
  if (snappedTop !== top) top = snappedTop;
  else if (snappedBottom !== bottom) top = snappedBottom - t.height;
  return transformFromTopLeft({ x: left, y: top }, t.width, t.height, t);
}

// ── Transform resize (reference TransformOverlayView.resizedTransform) ────────

/**
 * Resize a transform by dragging `corner` by a screen-px translation, with optional
 * aspect lock (`mediaCanvasAspect`) and canvas-edge snap (reference
 * `resizedTransform`). The opposite edge pins so the rect can never invert.
 */
export function resizedTransform(
  start: Transform,
  corner: Corner,
  translation: { width: number; height: number },
  videoRect: Rect,
  mediaCanvasAspect: number | null,
  rotated: boolean,
): Transform {
  if (videoRect.width <= 0 || videoRect.height <= 0) return start;
  const minSize = MIN_NORMALIZED;
  const dx = translation.width / videoRect.width;
  const dy = translation.height / videoRect.height;
  const tl = topLeft(start);
  let left = tl.x;
  let top = tl.y;
  let right = left + start.width;
  let bottom = top + start.height;

  switch (corner) {
    case "topLeft":
      left += dx;
      top += dy;
      break;
    case "topRight":
      right += dx;
      top += dy;
      break;
    case "bottomLeft":
      left += dx;
      bottom += dy;
      break;
    case "bottomRight":
      right += dx;
      bottom += dy;
      break;
  }

  // Stop the dragged edge at the opposite edge so the rect can never invert.
  switch (corner) {
    case "topLeft":
      left = Math.min(left, right - minSize);
      top = Math.min(top, bottom - minSize);
      break;
    case "topRight":
      right = Math.max(right, left + minSize);
      top = Math.min(top, bottom - minSize);
      break;
    case "bottomLeft":
      left = Math.min(left, right - minSize);
      bottom = Math.max(bottom, top + minSize);
      break;
    case "bottomRight":
      right = Math.max(right, left + minSize);
      bottom = Math.max(bottom, top + minSize);
      break;
  }

  if (mediaCanvasAspect != null) {
    const w = right - left;
    const h = bottom - top;
    const widthFromHeight = h * mediaCanvasAspect;
    if (w >= widthFromHeight) {
      const adjustedH = w / mediaCanvasAspect;
      if (corner === "topLeft" || corner === "topRight") top = bottom - adjustedH;
      else bottom = top + adjustedH;
    } else {
      const adjustedW = h * mediaCanvasAspect;
      if (corner === "topLeft" || corner === "bottomLeft") left = right - adjustedW;
      else right = left + adjustedW;
    }
  }

  if (!rotated) {
    const snapH = SNAP_THRESHOLD_PX / videoRect.width;
    const snapV = SNAP_THRESHOLD_PX / videoRect.height;
    const movesLeft = corner === "topLeft" || corner === "bottomLeft";
    const movesTop = corner === "topLeft" || corner === "topRight";
    const hEdge = movesLeft ? left : right;
    const vEdge = movesTop ? top : bottom;
    const snappedH = snapToBoundary(hEdge, snapH);
    const snappedV = snapToBoundary(vEdge, snapV);

    if (snappedH !== hEdge) {
      if (movesLeft) left = snappedH;
      else right = snappedH;
      if (mediaCanvasAspect != null) {
        if (movesTop) top = bottom - (right - left) / mediaCanvasAspect;
        else bottom = top + (right - left) / mediaCanvasAspect;
      }
    } else if (snappedV !== vEdge) {
      if (movesTop) top = snappedV;
      else bottom = snappedV;
      if (mediaCanvasAspect != null) {
        if (movesLeft) left = right - (bottom - top) * mediaCanvasAspect;
        else right = left + (bottom - top) * mediaCanvasAspect;
      }
    }
  }

  return transformFromTopLeft(
    { x: left, y: top },
    Math.max(MIN_NORMALIZED, right - left),
    Math.max(MIN_NORMALIZED, bottom - top),
    { ...start },
  );
}

/**
 * Rotation-aware hit target for a clip rect (reference `rotatedHitTarget`): the
 * axis-aligned bounding box of the rotated rect, used to size the move-gesture region.
 */
export function rotatedHitFrame(
  size: { width: number; height: number },
  degrees: number,
): { width: number; height: number } {
  const rad = (degrees * Math.PI) / 180;
  const absC = Math.abs(Math.cos(rad));
  const absS = Math.abs(Math.sin(rad));
  return {
    width: size.width * absC + size.height * absS,
    height: size.width * absS + size.height * absC,
  };
}

// ── Crop counter-rotation + pan/resize (reference CropOverlayView) ────────────

/**
 * Rotate a screen-space drag delta into the clip's local (unrotated) axes
 * (reference `clipLocal`), so crop math stays correct when the clip's rotation tilts
 * the overlay. This is the counter-rotation the spec calls out.
 */
export function clipLocalDelta(
  translation: { width: number; height: number },
  rotationDegrees: number,
): { width: number; height: number } {
  const r = (rotationDegrees * Math.PI) / 180;
  const c = Math.cos(r);
  const s = Math.sin(r);
  return {
    width: translation.width * c + translation.height * s,
    height: -translation.width * s + translation.height * c,
  };
}

/** Pan the crop by a clip-local translation (reference `pannedCrop`). */
export function pannedCrop(start: Crop, translation: { width: number; height: number }, clipRect: Rect): Crop {
  if (clipRect.width <= 0 || clipRect.height <= 0) return start;
  const dx = translation.width / clipRect.width;
  const dy = translation.height / clipRect.height;
  const visW = 1 - start.left - start.right;
  const visH = 1 - start.top - start.bottom;
  const L = Math.max(0, Math.min(start.left + dx, 1 - visW));
  const T = Math.max(0, Math.min(start.top + dy, 1 - visH));
  return { left: L, top: T, right: 1 - visW - L, bottom: 1 - visH - T };
}

/**
 * Resize the crop by dragging `corner` (reference `resizedCrop`, unlocked path).
 * Aspect-locked resize uses `resizedCropLocked`.
 */
export function resizedCrop(
  start: Crop,
  corner: Corner,
  translation: { width: number; height: number },
  clipRect: Rect,
  aspectNormalized: number | null,
): Crop {
  if (clipRect.width <= 0 || clipRect.height <= 0) return start;
  const minVis = MIN_NORMALIZED;
  if (aspectNormalized != null) {
    return resizedCropLocked(start, corner, translation, clipRect, aspectNormalized, minVis);
  }
  const dx = translation.width / clipRect.width;
  const dy = translation.height / clipRect.height;
  let L = start.left;
  let T = start.top;
  let R = start.right;
  let B = start.bottom;
  switch (corner) {
    case "topLeft":
      L += dx;
      T += dy;
      break;
    case "topRight":
      R -= dx;
      T += dy;
      break;
    case "bottomLeft":
      L += dx;
      B -= dy;
      break;
    case "bottomRight":
      R -= dx;
      B -= dy;
      break;
  }
  L = Math.max(0, Math.min(L, 1 - minVis - R));
  R = Math.max(0, Math.min(R, 1 - minVis - L));
  T = Math.max(0, Math.min(T, 1 - minVis - B));
  B = Math.max(0, Math.min(B, 1 - minVis - T));
  return { left: L, top: T, right: R, bottom: B };
}

/** Aspect-locked crop resize (reference `resizedCropLocked`). */
function resizedCropLocked(
  start: Crop,
  corner: Corner,
  translation: { width: number; height: number },
  clipRect: Rect,
  aspectN: number,
  minVis: number,
): Crop {
  const dx = translation.width / clipRect.width;
  const dy = translation.height / clipRect.height;
  const L = start.left;
  const T = start.top;
  const R = start.right;
  const B = start.bottom;
  const startVisW = 1 - L - R;
  const startVisH = 1 - T - B;

  let widthDelta: number;
  let heightDelta: number;
  switch (corner) {
    case "topLeft":
      widthDelta = -dx;
      heightDelta = -dy;
      break;
    case "topRight":
      widthDelta = dx;
      heightDelta = -dy;
      break;
    case "bottomLeft":
      widthDelta = -dx;
      heightDelta = dy;
      break;
    case "bottomRight":
      widthDelta = dx;
      heightDelta = dy;
      break;
  }

  const sFromW = startVisW + widthDelta;
  const sFromH = aspectN * (startVisH + heightDelta);
  let s = Math.abs(widthDelta) > Math.abs(heightDelta * aspectN) ? sFromW : sFromH;

  const sMaxFromX = corner === "topRight" || corner === "bottomRight" ? 1 - L : 1 - R;
  const sMaxFromY = corner === "bottomLeft" || corner === "bottomRight" ? aspectN * (1 - T) : aspectN * (1 - B);
  const sMax = Math.min(sMaxFromX, sMaxFromY);
  const sMin = Math.max(minVis, minVis * aspectN);
  if (sMax < sMin) return start;
  s = Math.min(Math.max(s, sMin), sMax);

  const newVisW = s;
  const newVisH = s / aspectN;
  let newL = L;
  let newT = T;
  let newR = R;
  let newB = B;
  switch (corner) {
    case "topLeft":
      newL = 1 - R - newVisW;
      newT = 1 - B - newVisH;
      break;
    case "topRight":
      newR = 1 - L - newVisW;
      newT = 1 - B - newVisH;
      break;
    case "bottomLeft":
      newL = 1 - R - newVisW;
      newB = 1 - T - newVisH;
      break;
    case "bottomRight":
      newR = 1 - L - newVisW;
      newB = 1 - T - newVisH;
      break;
  }
  return { left: newL, top: newT, right: newR, bottom: newB };
}

// ── cmd/ctrl-scroll zoom about a point (reference PreviewNSView.onCmdScroll) ──

/**
 * Zoom about a point. Given the scroll delta (already scaled by sensitivity), the
 * pointer in top-down view coords, the view size, and the current zoom/offset, returns
 * the new zoom + offset so the point under the cursor stays put (reference
 * `onCmdScroll`). Zoom is clamped to `[ZOOM_MIN, ZOOM_MAX]`.
 */
export function zoomAboutPoint(args: {
  deltaY: number;
  point: { x: number; y: number };
  viewSize: { width: number; height: number };
  oldZoom: number;
  offset: { width: number; height: number };
  minZoom: number;
  maxZoom: number;
}): { zoom: number; offset: { width: number; height: number } } | null {
  const { deltaY, point, viewSize, oldZoom, offset, minZoom, maxZoom } = args;
  const factor = Math.exp(deltaY);
  const newZoom = Math.min(Math.max(oldZoom * factor, minZoom), maxZoom);
  if (Math.abs(newZoom - oldZoom) < 0.0001) return null;
  const fitW = viewSize.width / oldZoom;
  const fitH = viewSize.height / oldZoom;
  const ddx = (fitW * (newZoom - oldZoom)) / 2 + point.x * (1 - newZoom / oldZoom);
  const ddy = (fitH * (newZoom - oldZoom)) / 2 + point.y * (1 - newZoom / oldZoom);
  return {
    zoom: newZoom,
    offset: { width: offset.width + ddx, height: offset.height + ddy },
  };
}

/** Map a scrub-bar location to a frame (reference `scrubFrame`). */
export function scrubFrame(locationX: number, width: number, durationFrames: number): number {
  if (width <= 0) return 0;
  const fraction = Math.max(0, Math.min(1, locationX / width));
  return Math.trunc(fraction * Math.max(0, durationFrames));
}

/** Timecode `HH:MM:SS:FF` (reference `formatTimecode`). */
export function formatTimecode(frame: number, fps: number): string {
  const f = Math.max(0, Math.trunc(frame));
  const safeFps = Math.max(1, Math.trunc(fps));
  const totalSeconds = Math.trunc(f / safeFps);
  const frames = f % safeFps;
  const seconds = totalSeconds % 60;
  const minutes = Math.trunc(totalSeconds / 60) % 60;
  const hours = Math.trunc(totalSeconds / 3600);
  const p2 = (n: number) => String(n).padStart(2, "0");
  return `${p2(hours)}:${p2(minutes)}:${p2(seconds)}:${p2(frames)}`;
}
