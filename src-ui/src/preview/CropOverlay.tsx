// Crop overlay (E5-S10) — direct-manipulation crop with rule-of-thirds.
//
// Port of the macOS reference `CropOverlayView.swift`: the dimmed-outside crop rect
// with rule-of-thirds guides, a pan-inside region, 4 resize corners, an aspect-lock
// toggle, all **counter-rotated to clip-local axes** (drag deltas are rotated by
// -rotation via `clipLocalDelta` so crop math stays correct when the clip is tilted).
// The whole overlay rotates with the clip about its center; the gesture math undoes
// that rotation. All math lives in `geometry.ts` (ported verbatim).

import { useRef } from "react";

import {
  clipFrame,
  clipLocalDelta,
  cropFrame,
  pannedCrop,
  resizedCrop,
  videoContentRect,
  type Rect,
} from "./geometry";
import { ALL_CORNERS, type Corner, type Crop, type Transform } from "./types";

const HANDLE_SIZE = 10;
const BORDER_COLOR = "rgba(110,231,255,0.95)"; // reference timecode accent (cyan)
const GUIDE_COLOR = "rgba(110,231,255,0.45)";
const DIM_COLOR = "rgba(0,0,0,0.8)";

export interface CropOverlayProps {
  viewSize: { width: number; height: number };
  videoAspect: number;
  /** The selected clip's current transform (for clip rect + rotation). */
  transform: Transform | null;
  /** The selected clip's current crop. */
  crop: Crop | null;
  /** Aspect lock (normalized target/source aspect), or null for free resize. */
  aspectNormalized?: number | null;
  onApply: (c: Crop) => void;
  onCommit: (c: Crop) => void;
}

type Drag = { kind: "pan"; start: Crop } | { kind: "resize"; corner: Corner; start: Crop };

export function CropOverlay({
  viewSize,
  videoAspect,
  transform,
  crop,
  aspectNormalized = null,
  onApply,
  onCommit,
}: CropOverlayProps) {
  const drag = useRef<Drag | null>(null);
  const origin = useRef<{ x: number; y: number }>({ x: 0, y: 0 });

  const videoRect = videoContentRect(viewSize, videoAspect);

  if (!transform || !crop) {
    return <svg width={viewSize.width} height={viewSize.height} style={overlaySvgStyle} />;
  }

  const rect = clipFrame(transform, videoRect);
  const cropRect = cropFrame(crop, rect);
  const rotation = transform.rotation;
  const cx = rect.x + rect.width / 2;
  const cy = rect.y + rect.height / 2;

  const begin = (next: Drag) => (e: React.PointerEvent) => {
    e.stopPropagation();
    (e.target as Element).setPointerCapture(e.pointerId);
    drag.current = next;
    origin.current = { x: e.clientX, y: e.clientY };
  };

  const compute = (e: React.PointerEvent): Crop | null => {
    const d = drag.current;
    if (!d) return null;
    const screen = { width: e.clientX - origin.current.x, height: e.clientY - origin.current.y };
    const local = clipLocalDelta(screen, rotation);
    if (d.kind === "pan") return pannedCrop(d.start, local, rect);
    return resizedCrop(d.start, d.corner, local, rect, aspectNormalized);
  };

  const onMove = (e: React.PointerEvent) => {
    const updated = compute(e);
    if (updated) onApply(updated);
  };
  const onUp = (e: React.PointerEvent) => {
    const updated = compute(e);
    drag.current = null;
    if (updated) onCommit(updated);
  };

  // Dimmed regions (4 bands around the crop rect, in clip-local space pre-rotation).
  const bands = dimBands(rect, cropRect);
  const thirds = ruleOfThirdsLines(cropRect);

  return (
    <svg
      width={viewSize.width}
      height={viewSize.height}
      style={overlaySvgStyle}
      onPointerMove={onMove}
      onPointerUp={onUp}
      onPointerCancel={onUp}
    >
      <g transform={`rotate(${rotation} ${cx} ${cy})`}>
        {/* Dim outside the crop. */}
        {bands.map((b, i) => (
          <rect key={i} x={b.x} y={b.y} width={b.width} height={b.height} fill={DIM_COLOR} pointerEvents="none" />
        ))}
        {/* Rule-of-thirds guides. */}
        {thirds.map((l, i) => (
          <line key={i} x1={l.x1} y1={l.y1} x2={l.x2} y2={l.y2} stroke={GUIDE_COLOR} strokeWidth={1} pointerEvents="none" />
        ))}
        {/* Crop border. */}
        <rect
          x={cropRect.x}
          y={cropRect.y}
          width={cropRect.width}
          height={cropRect.height}
          fill="none"
          stroke={BORDER_COLOR}
          strokeWidth={2}
          pointerEvents="none"
        />
        {/* Pan region. */}
        <rect
          x={cropRect.x}
          y={cropRect.y}
          width={cropRect.width}
          height={cropRect.height}
          fill="rgba(255,255,255,0.001)"
          style={{ cursor: "grab" }}
          onPointerDown={begin({ kind: "pan", start: crop })}
        />
        {/* Resize corners. */}
        {ALL_CORNERS.map((corner) => {
          const p = cornerPoint(corner, cropRect);
          return (
            <rect
              key={corner}
              x={p.x - HANDLE_SIZE / 2}
              y={p.y - HANDLE_SIZE / 2}
              width={HANDLE_SIZE}
              height={HANDLE_SIZE}
              fill={BORDER_COLOR}
              style={{ cursor: resizeCursor(corner) }}
              onPointerDown={begin({ kind: "resize", corner, start: crop })}
            />
          );
        })}
      </g>
    </svg>
  );
}

const overlaySvgStyle: React.CSSProperties = {
  position: "absolute",
  inset: 0,
  pointerEvents: "auto",
};

/** The 4 dim bands around a crop rect inside the clip rect (reference Canvas fills). */
function dimBands(clipRect: Rect, cropRect: Rect): Rect[] {
  return [
    // top
    { x: clipRect.x, y: clipRect.y, width: clipRect.width, height: cropRect.y - clipRect.y },
    // bottom
    {
      x: clipRect.x,
      y: cropRect.y + cropRect.height,
      width: clipRect.width,
      height: clipRect.y + clipRect.height - (cropRect.y + cropRect.height),
    },
    // left
    { x: clipRect.x, y: cropRect.y, width: cropRect.x - clipRect.x, height: cropRect.height },
    // right
    {
      x: cropRect.x + cropRect.width,
      y: cropRect.y,
      width: clipRect.x + clipRect.width - (cropRect.x + cropRect.width),
      height: cropRect.height,
    },
  ];
}

/** Rule-of-thirds lines inside a crop rect (reference `thirds` path). */
function ruleOfThirdsLines(cropRect: Rect): { x1: number; y1: number; x2: number; y2: number }[] {
  const lines: { x1: number; y1: number; x2: number; y2: number }[] = [];
  for (let i = 1; i <= 2; i++) {
    const f = i / 3;
    lines.push({
      x1: cropRect.x + cropRect.width * f,
      y1: cropRect.y,
      x2: cropRect.x + cropRect.width * f,
      y2: cropRect.y + cropRect.height,
    });
    lines.push({
      x1: cropRect.x,
      y1: cropRect.y + cropRect.height * f,
      x2: cropRect.x + cropRect.width,
      y2: cropRect.y + cropRect.height * f,
    });
  }
  return lines;
}

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
