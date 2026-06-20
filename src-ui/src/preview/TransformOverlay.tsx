// Transform overlay (E5-S10) — direct-manipulation clip geometry in the viewport.
//
// Port of the macOS reference `TransformOverlayView.swift`: a bordered rect over the
// selected clip with 4 corner handles, a center move region, pink center-to-center
// snap guides, and rotation handling (the box rotates with the clip; the move-gesture
// hit target is the rotated bounding box). Geometry operates on the **center-based**
// Transform (ruling #7). All math lives in `geometry.ts` (ported verbatim); this
// component is the pointer-gesture + SVG shell.
//
// Edits flow out via `onApply` (live, during drag) and `onCommit` (drag end) so the
// panel can route them through Tauri commands into the edit engine (strict layering).

import { useRef, useState } from "react";

import {
  clipFrame,
  movedTransform,
  rectMidX,
  rectMidY,
  resizedTransform,
  rotatedHitFrame,
  videoContentRect,
  type Rect,
} from "./geometry";
import { ALL_CORNERS, type Corner, type Transform } from "./types";

const HANDLE_SIZE = 10;
const BORDER_COLOR = "rgba(255,255,255,0.8)";
const CENTER_GUIDE_COLOR = "rgba(255,51,153,0.8)"; // reference pink (1.0, 0.2, 0.6)

export interface TransformOverlayProps {
  /** The viewport size in CSS px (the transparent region over the wgpu surface). */
  viewSize: { width: number; height: number };
  /** The composition aspect ratio (timeline width/height). */
  videoAspect: number;
  /** The selected clip's current transform, or null when nothing is selected. */
  transform: Transform | null;
  /** Aspect lock for resize (media canvas aspect), or null for free resize. */
  mediaCanvasAspect?: number | null;
  /** Live edit during drag (applies to the timeline, not yet committed). */
  onApply: (t: Transform) => void;
  /** Commit on drag end (pushes to history). */
  onCommit: (t: Transform, actionName: string) => void;
}

type Drag =
  | { kind: "move"; start: Transform }
  | { kind: "resize"; corner: Corner; start: Transform };

export function TransformOverlay({
  viewSize,
  videoAspect,
  transform,
  mediaCanvasAspect = null,
  onApply,
  onCommit,
}: TransformOverlayProps) {
  const drag = useRef<Drag | null>(null);
  const origin = useRef<{ x: number; y: number }>({ x: 0, y: 0 });
  const [guideX, setGuideX] = useState(false);
  const [guideY, setGuideY] = useState(false);

  const videoRect = videoContentRect(viewSize, videoAspect);

  if (!transform) {
    return <svg width={viewSize.width} height={viewSize.height} style={overlaySvgStyle} />;
  }

  const rect = clipFrame(transform, videoRect);
  const rotation = transform.rotation;

  const beginMove = (e: React.PointerEvent) => {
    e.stopPropagation();
    (e.target as Element).setPointerCapture(e.pointerId);
    drag.current = { kind: "move", start: transform };
    origin.current = { x: e.clientX, y: e.clientY };
  };

  const beginResize = (corner: Corner) => (e: React.PointerEvent) => {
    e.stopPropagation();
    (e.target as Element).setPointerCapture(e.pointerId);
    drag.current = { kind: "resize", corner, start: transform };
    origin.current = { x: e.clientX, y: e.clientY };
  };

  const onMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const translation = {
      width: e.clientX - origin.current.x,
      height: e.clientY - origin.current.y,
    };
    if (d.kind === "move") {
      const rotated = d.start.rotation !== 0;
      const { transform: moved, snap } = movedTransform(d.start, translation, videoRect, rotated);
      if (guideX !== snap.x) setGuideX(snap.x);
      if (guideY !== snap.y) setGuideY(snap.y);
      onApply(moved);
    } else {
      const resized = resizedTransform(
        d.start,
        d.corner,
        translation,
        videoRect,
        mediaCanvasAspect,
        d.start.rotation !== 0,
      );
      onApply(resized);
    }
  };

  const onUp = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    const translation = {
      width: e.clientX - origin.current.x,
      height: e.clientY - origin.current.y,
    };
    drag.current = null;
    setGuideX(false);
    setGuideY(false);
    if (d.kind === "move") {
      const rotated = d.start.rotation !== 0;
      const { transform: moved } = movedTransform(d.start, translation, videoRect, rotated);
      onCommit(moved, "Change Position");
    } else {
      const resized = resizedTransform(
        d.start,
        d.corner,
        translation,
        videoRect,
        mediaCanvasAspect,
        d.start.rotation !== 0,
      );
      onCommit(resized, "Change Scale");
    }
  };

  const hit = rotatedHitFrame({ width: rect.width, height: rect.height }, rotation);

  return (
    <svg
      width={viewSize.width}
      height={viewSize.height}
      style={overlaySvgStyle}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
    >
      {/* Move hit region — the rotated bounding box, transparent. */}
      <rect
        x={rectMidX(rect) - hit.width / 2}
        y={rectMidY(rect) - hit.height / 2}
        width={hit.width}
        height={hit.height}
        fill="rgba(255,255,255,0.001)"
        style={{ cursor: "move" }}
        onPointerDown={beginMove}
      />

      {/* The clip box + corner handles, rotated about the clip center. */}
      <g transform={`rotate(${rotation} ${rectMidX(rect)} ${rectMidY(rect)})`}>
        <rect
          x={rect.x}
          y={rect.y}
          width={rect.width}
          height={rect.height}
          fill="none"
          stroke={BORDER_COLOR}
          strokeWidth={1}
        />
        {ALL_CORNERS.map((corner) => {
          const p = cornerPoint(corner, rect);
          return (
            <rect
              key={corner}
              x={p.x - HANDLE_SIZE / 2}
              y={p.y - HANDLE_SIZE / 2}
              width={HANDLE_SIZE}
              height={HANDLE_SIZE}
              fill={BORDER_COLOR}
              style={{ cursor: resizeCursor(corner) }}
              onPointerDown={beginResize(corner)}
            />
          );
        })}
      </g>

      {/* Center-to-center snap guides (pink). */}
      {guideX && (
        <line
          x1={rectMidX(videoRect)}
          y1={videoRect.y}
          x2={rectMidX(videoRect)}
          y2={videoRect.y + videoRect.height}
          stroke={CENTER_GUIDE_COLOR}
          strokeWidth={1}
        />
      )}
      {guideY && (
        <line
          x1={videoRect.x}
          y1={rectMidY(videoRect)}
          x2={videoRect.x + videoRect.width}
          y2={rectMidY(videoRect)}
          stroke={CENTER_GUIDE_COLOR}
          strokeWidth={1}
        />
      )}
    </svg>
  );
}

const overlaySvgStyle: React.CSSProperties = {
  position: "absolute",
  inset: 0,
  pointerEvents: "auto",
};

function cornerPoint(corner: Corner, rect: Rect): { x: number; y: number } {
  switch (corner) {
    case "topLeft":
      return { x: rect.x, y: rect.y };
    case "topRight":
      return { x: rect.x + rect.width, y: rect.y };
    case "bottomLeft":
      return { x: rect.x, y: rect.y + rect.height };
    case "bottomRight":
      return { x: rect.x + rect.width, y: rect.y + rect.height };
  }
}

function resizeCursor(corner: Corner): string {
  return corner === "topLeft" || corner === "bottomRight" ? "nwse-resize" : "nesw-resize";
}
