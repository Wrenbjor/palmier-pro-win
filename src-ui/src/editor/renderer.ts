// Immediate-mode 2D draw loop for the timeline canvas (E3-S9).
//
// Ports the macOS reference draw math 1:1 onto Canvas 2D:
//   `Timeline/TimelineView.swift:201` drawContent order,
//   `Timeline/ClipRenderer.swift`, `Timeline/TimelineRuler.swift`,
//   `Timeline/PlayheadOverlay.swift`.
// Only the drawing primitives change backend (CGContext → CanvasRenderingContext2D);
// all geometry/sampling is `geometry.ts`. Canvas 2D is already top-left, Y-down — the
// reference's flipped NSView math (`body.minY + frac*h`, high dB → smaller y) maps
// directly with no axis flip.
//
// Draw order (timeline-model.md line 39):
//   backgrounds → range fill → clips → gaps → generating overlays → drag ghosts →
//   marquee → insertion line → razor preview → ruler.
// This story renders backgrounds → range fill → clips → ruler → playhead. The
// interaction overlays (ghosts/marquee/insertion/razor) are E3-S7/S10 and slot into
// the same ordered pipeline later.

import type { ClipView, TimelineView, TimelineViewport } from "./types";
import {
  ClipRender,
  Layout,
  Snap,
  Theme,
  Trim,
  VolumeScale,
  blend,
  isSmooth,
  rgba,
  smoothstep,
  trackColor,
  trackRgb,
} from "./theme";
import {
  type Rect,
  type TimelineLayout,
  clipRect,
  endFrame,
  formatTimecode,
  makeLayout,
  minorSubdivisions,
  sourceDurationFrames,
  sourceFramesConsumed,
  tickInterval,
  trackHeightAt,
  trackY,
  volumeAt,
  xForFrame,
} from "./geometry";
import { envelopeBodyRect, yForDb, yForOpacity } from "./envelope";

export interface RenderArgs {
  timeline: TimelineView;
  viewport: TimelineViewport;
  /** Device width/height in CSS px (logical, pre-DPR). */
  width: number;
  height: number;
}

/** Top-level entry: clears + draws the whole timeline in reference order. */
export function renderTimeline(
  ctx: CanvasRenderingContext2D,
  args: RenderArgs,
): void {
  const { timeline, viewport, width, height } = args;
  const layout = makeLayout(
    viewport.pixelsPerFrame,
    timeline.tracks.map((t) => t.displayHeight),
  );
  const scrollX = viewport.scrollX;

  // The canvas is translated left by scrollX so frame math stays in content space.
  ctx.save();
  ctx.translate(-scrollX, 0);

  drawBackground(ctx, layout, timeline, scrollX, width, height);
  drawRangeFill(ctx, layout, viewport, height);
  drawClips(ctx, layout, timeline, viewport);

  ctx.restore();

  // Ruler + playhead draw in screen space (their own scroll handling).
  drawRuler(ctx, layout, timeline.fps, viewport.pixelsPerFrame, scrollX, width);
  drawPlayhead(ctx, layout, viewport, scrollX, height);
}

// --- Backgrounds ---

function drawBackground(
  ctx: CanvasRenderingContext2D,
  layout: TimelineLayout,
  timeline: TimelineView,
  scrollX: number,
  width: number,
  height: number,
): void {
  // Base fill across the whole content viewport.
  ctx.fillStyle = Theme.background.base;
  ctx.fillRect(scrollX, 0, width, height);

  // Track lane bands (alternating raised/surface for legibility).
  for (let i = 0; i < timeline.tracks.length; i++) {
    const ty = trackY(layout, i);
    const th = trackHeightAt(layout, i);
    ctx.fillStyle = i % 2 === 0 ? Theme.background.surface : Theme.background.raised;
    ctx.fillRect(scrollX, ty, width, th);
    // Lane separator.
    ctx.strokeStyle = Theme.border.subtle;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(scrollX, ty + th - 0.5);
    ctx.lineTo(scrollX + width, ty + th - 0.5);
    ctx.stroke();
  }
}

function drawRangeFill(
  ctx: CanvasRenderingContext2D,
  layout: TimelineLayout,
  viewport: TimelineViewport,
  height: number,
): void {
  const range = viewport.rangeSelection;
  if (!range) return;
  const start = Math.min(range.startFrame, range.endFrame);
  const end = Math.max(range.startFrame, range.endFrame);
  if (end <= start) return;
  const x0 = xForFrame(layout, start);
  const x1 = xForFrame(layout, end);
  ctx.fillStyle = rgba(255, 255, 255, 0.1);
  ctx.fillRect(x0, layout.rulerHeight, x1 - x0, height - layout.rulerHeight);
  ctx.strokeStyle = rgba(255, 255, 255, 0.5);
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(x0 + 0.5, layout.rulerHeight);
  ctx.lineTo(x0 + 0.5, height);
  ctx.moveTo(x1 - 0.5, layout.rulerHeight);
  ctx.lineTo(x1 - 0.5, height);
  ctx.stroke();
}

// --- Clips ---

function drawClips(
  ctx: CanvasRenderingContext2D,
  layout: TimelineLayout,
  timeline: TimelineView,
  viewport: TimelineViewport,
): void {
  for (let ti = 0; ti < timeline.tracks.length; ti++) {
    const track = timeline.tracks[ti];
    for (const clip of track.clips) {
      const rect = clipRect(layout, clip, ti);
      const selected = viewport.selectedClipIds.has(clip.id);
      drawClip(ctx, clip, rect, selected, timeline.fps);
    }
  }
}

function roundedRectPath(
  ctx: CanvasRenderingContext2D,
  r: Rect,
  radius: number,
): void {
  const rr = Math.min(radius, r.w / 2, r.h / 2);
  ctx.beginPath();
  ctx.moveTo(r.x + rr, r.y);
  ctx.arcTo(r.x + r.w, r.y, r.x + r.w, r.y + r.h, rr);
  ctx.arcTo(r.x + r.w, r.y + r.h, r.x, r.y + r.h, rr);
  ctx.arcTo(r.x, r.y + r.h, r.x, r.y, rr);
  ctx.arcTo(r.x, r.y, r.x + r.w, r.y, rr);
  ctx.closePath();
}

function drawClip(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  rect: Rect,
  selected: boolean,
  fps: number,
): void {
  if (rect.w <= 0 || rect.h <= 0) return;
  const cornerRadius = Trim.clipCornerRadius;
  const colorType = clip.sourceClipType;
  const isMissing = !!clip.isMissing && !clip.isGenerating;

  // 1) Card fill (theme color α 0.45 selected / 0.3).
  ctx.fillStyle = trackColor(colorType, selected ? 0.45 : 0.3);
  roundedRectPath(ctx, rect, cornerRadius);
  ctx.fill();

  // Layout zones.
  const stripWidth = ClipRender.stripWidth;
  const handleW = Trim.handleWidth;
  const contentX = rect.x + stripWidth + 1;
  const contentWidth = rect.w - stripWidth - 1 - handleW;
  const contentY = rect.y + ClipRender.labelBarHeight;
  const mainHeight = rect.y + rect.h - contentY;

  // 2) Visual content placeholder by media type.
  if (clip.mediaType === "audio") {
    drawWaveform(ctx, clip, { x: contentX, y: contentY, w: contentWidth, h: mainHeight });
  } else if (mainHeight > 4 && contentWidth > 4) {
    drawContentPlaceholder(ctx, clip, rect, cornerRadius, {
      x: contentX,
      y: contentY,
      w: contentWidth,
      h: mainHeight,
    });
  }

  // 3) Rubber band (audio) / fade wedges (non-audio).
  if (clip.mediaType === "audio") {
    drawVolumeRubberBand(ctx, clip, rect, selected);
  } else {
    drawOpacityFades(ctx, clip, rect, selected);
  }

  // 4) Color strip on the left edge (sourceClipType color).
  ctx.fillStyle = trackColor(colorType, 1);
  roundedRectPath(
    ctx,
    { x: rect.x, y: rect.y, w: stripWidth, h: rect.h },
    cornerRadius,
  );
  ctx.fill();

  // 5) Border (selected white α0.9 w1.5 / primary 0.5).
  roundedRectPath(ctx, rect, cornerRadius);
  if (selected) {
    ctx.strokeStyle = Theme.selectionStroke;
    ctx.lineWidth = 1.5;
  } else {
    ctx.strokeStyle = Theme.border.primary;
    ctx.lineWidth = 0.5;
  }
  ctx.stroke();

  // 5b) Linked partners — "1 px subtle outline" (FOUNDATION §479) so the user can see
  // linked A/V clips. Subtle (low-opacity white) and only when NOT selected, so it
  // never fights the 1.5 px selection stroke drawn above.
  if (clip.linkGroupId && !selected) {
    roundedRectPath(ctx, rect, cornerRadius);
    ctx.strokeStyle = rgba(255, 255, 255, 0.28);
    ctx.lineWidth = 1;
    ctx.stroke();
  }

  // 6) Missing-media red wash + red border.
  if (isMissing) {
    const [r, g, b] = Theme.status.error;
    roundedRectPath(ctx, rect, cornerRadius);
    ctx.fillStyle = rgba(r, g, b, Theme.opacity.moderate);
    ctx.fill();
    roundedRectPath(ctx, rect, cornerRadius);
    ctx.strokeStyle = rgba(r, g, b, Theme.opacity.prominent);
    ctx.lineWidth = 1.5;
    ctx.stroke();
  }

  // 7) Label bar.
  drawLabelBar(ctx, clip, rect, contentX, contentWidth, fps);

  // 8) Non-volume keyframe diamonds near the clip bottom.
  drawKeyframeMarkers(ctx, clip, rect);

  // 9) Trim handles (muted fills on both edges).
  ctx.fillStyle = Theme.text.muted;
  ctx.fillRect(rect.x, rect.y, handleW, rect.h);
  ctx.fillRect(rect.x + rect.w - handleW, rect.y, handleW, rect.h);
}

// `clipBodyRect` + `yForDb` now live in `envelope.ts` (shared with hit-testing so the
// drawn line and the grabbable line can't drift) — imported above.
const clipBodyRect = envelopeBodyRect;

/** fade knee X clamped into the fixed fade lane (ClipRenderer.fadeHandleRenderX). */
function fadeHandleRenderX(
  rect: Rect,
  kfOffset: number,
  isLeft: boolean,
  pxPerFrame: number,
): number {
  const actual = rect.x + kfOffset * pxPerFrame;
  const inset = ClipRender.volumeFadeHandleEdgeInset;
  return isLeft
    ? Math.max(rect.x + inset, actual)
    : Math.min(rect.x + rect.w - inset, actual);
}

// --- Content placeholders (video thumbnail strip / image tile) ---
// Thumbnails/ImageBitmaps come from `palmier-media` later; until then we draw a
// tiled placeholder so geometry (aspect tiling) is visible and faithful.

function drawContentPlaceholder(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  clipRectArg: Rect,
  cornerRadius: number,
  draw: Rect,
): void {
  ctx.save();
  roundedRectPath(ctx, clipRectArg, cornerRadius);
  ctx.clip();
  ctx.beginPath();
  ctx.rect(draw.x, draw.y, draw.w, draw.h);
  ctx.clip();

  // Subtle vertical hatch standing in for a thumbnail/image filmstrip.
  const tileW = Math.max(12, draw.h * (16 / 9));
  ctx.fillStyle = rgba(255, 255, 255, 0.04);
  ctx.fillRect(draw.x, draw.y, draw.w, draw.h);
  ctx.strokeStyle = rgba(255, 255, 255, 0.08);
  ctx.lineWidth = 1;
  for (let x = draw.x; x < draw.x + draw.w; x += tileW) {
    ctx.beginPath();
    ctx.moveTo(x + 0.5, draw.y);
    ctx.lineTo(x + 0.5, draw.y + draw.h);
    ctx.stroke();
  }

  // Type glyph hint (text/lottie/image clips with no media yet).
  if (clip.mediaType === "text") {
    ctx.fillStyle = Theme.text.tertiary;
    ctx.font = "12px ui-sans-serif, system-ui, sans-serif";
    ctx.textBaseline = "middle";
    ctx.fillText("T", draw.x + 4, draw.y + draw.h / 2);
  }
  ctx.restore();
}

// --- Waveform (ClipRenderer.drawWaveform) ---

function drawWaveform(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  draw: Rect,
): void {
  const drawWidth = draw.w;
  const drawHeight = draw.h;
  if (drawWidth <= 2 || drawHeight <= 2) return;

  const samples = clip.waveform;
  const barCount = Math.floor(drawWidth);
  if (barCount <= 0) return;

  const totalSource = sourceDurationFrames(clip);
  const dbRange = 50;
  const needsPerBarVolume =
    (clip.volumeTrack?.keyframes.length ?? 0) > 0 ||
    clip.fadeInFrames > 0 ||
    clip.fadeOutFrames > 0;
  const staticShift = VolumeScale.dbFromLinear(clip.volume) / dbRange;

  // Tint: theme color blended 30% toward white, α 0.85.
  const [tr, tg, tb] = blend(trackRgb(clip.sourceClipType), [255, 255, 255], 0.3);
  ctx.fillStyle = rgba(tr, tg, tb, 0.85);

  if (samples && samples.length > 0 && totalSource > 0) {
    const startFrac = clip.trimStartFrame / totalSource;
    const endFrac = (clip.trimStartFrame + sourceFramesConsumed(clip)) / totalSource;
    const sampleStart = Math.max(
      0,
      Math.min(samples.length, Math.floor(startFrac * samples.length)),
    );
    const sampleEnd = Math.max(
      sampleStart,
      Math.min(samples.length, Math.floor(endFrac * samples.length)),
    );
    const visCount = sampleEnd - sampleStart;
    const dur = Math.max(1, clip.durationFrames);
    const frameStep = dur / barCount;

    for (let i = 0; i < barCount; i++) {
      const sStart = sampleStart + Math.floor((i * visCount) / barCount);
      const sEnd = Math.max(
        sStart + 1,
        sampleStart + Math.floor(((i + 1) * visCount) / barCount),
      );
      let loudest = 1;
      for (let j = sStart; j < Math.min(sEnd, sampleEnd); j++) {
        if (samples[j] < loudest) loudest = samples[j];
      }
      const dbShift = needsPerBarVolume
        ? VolumeScale.dbFromLinear(
            volumeAt(clip, clip.startFrame + Math.floor(i * frameStep)),
          ) / dbRange
        : staticShift;
      const amplitude = Math.min(1, Math.max(0, 1 - loudest + dbShift));
      const barHeight = Math.max(1, amplitude * (drawHeight - 2));
      ctx.fillRect(draw.x + i, draw.y + drawHeight - barHeight - 1, 1, barHeight);
    }
  } else {
    // Placeholder: flat low bars so the audio lane reads as audio before peaks load.
    const h = Math.max(1, (drawHeight - 2) * 0.15);
    for (let i = 0; i < barCount; i += 2) {
      ctx.fillRect(draw.x + i, draw.y + drawHeight - h - 1, 1, h);
    }
  }
}

// --- Volume rubber band (audio) — ClipRenderer.drawVolumeRubberBand ---

function drawVolumeRubberBand(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  rect: Rect,
  selected: boolean,
): void {
  if (clip.durationFrames <= 0) return;
  const pxPerFrame = rect.w / clip.durationFrames;
  if (pxPerFrame <= 0) return;

  const body = clipBodyRect(rect);
  const alpha = selected ? 0.95 : 0.75;
  const lineColor = rgba(255, 255, 255, alpha);
  const fadeColor = rgba(255, 255, 255, alpha * 0.7);

  // 1) Volume line through kfs, or flat at static volume.
  ctx.strokeStyle = lineColor;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  const kfs = (clip.volumeTrack?.keyframes ?? []).filter(
    (k) => k.frame >= 0 && k.frame <= clip.durationFrames,
  );
  if (kfs.length > 0) {
    const firstX = rect.x + kfs[0].frame * pxPerFrame;
    const firstY = yForDb(kfs[0].value, body);
    ctx.moveTo(rect.x, firstY);
    ctx.lineTo(firstX, firstY);
    for (let i = 0; i < kfs.length - 1; i++) {
      const a = kfs[i];
      const b = kfs[i + 1];
      const aX = rect.x + a.frame * pxPerFrame;
      const bX = rect.x + b.frame * pxPerFrame;
      const aY = yForDb(a.value, body);
      const bY = yForDb(b.value, body);
      if (a.interpolationOut === "linear") {
        ctx.lineTo(bX, bY);
      } else if (a.interpolationOut === "hold") {
        ctx.lineTo(bX, aY);
        ctx.lineTo(bX, bY);
      } else {
        const steps = 12;
        for (let s = 1; s <= steps; s++) {
          const t = s / steps;
          const x = aX + (bX - aX) * t;
          const dB = a.value + (b.value - a.value) * smoothstep(t);
          ctx.lineTo(x, yForDb(dB, body));
        }
      }
    }
    ctx.lineTo(rect.x + rect.w, yForDb(kfs[kfs.length - 1].value, body));
  } else {
    const volY = yForDb(VolumeScale.dbFromLinear(clip.volume), body);
    ctx.moveTo(rect.x, volY);
    ctx.lineTo(rect.x + rect.w, volY);
  }
  ctx.stroke();

  drawFades(ctx, clip, rect, body, pxPerFrame, fadeColor, false);

  if (!selected) return;
  drawFadeKneeSquares(ctx, clip, rect, body, pxPerFrame, lineColor);
  // Volume keyframe diamonds on the band.
  ctx.fillStyle = lineColor;
  ctx.strokeStyle = rgba(0, 0, 0, 0.5);
  ctx.lineWidth = 0.5;
  const half = ClipRender.volumeKeyframeSize / 2;
  for (const kf of kfs) {
    const cx = rect.x + kf.frame * pxPerFrame;
    const cy = yForDb(kf.value, body);
    diamond(ctx, cx, cy, half);
  }
}

// --- Opacity fades (non-audio) — ClipRenderer.drawOpacityFades ---

function drawOpacityFades(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  rect: Rect,
  selected: boolean,
): void {
  if (clip.durationFrames <= 0) return;
  const pxPerFrame = rect.w / clip.durationFrames;
  if (pxPerFrame <= 0) return;
  const body = clipBodyRect(rect);
  const alpha = selected ? 0.95 : 0.75;
  const lineColor = rgba(255, 255, 255, alpha);
  const fadeColor = rgba(255, 255, 255, alpha * 0.7);

  // Opacity envelope line across the clip (FOUNDATION §479): the level the user grabs
  // to drag-to-set / Alt-drag a keyframe. Flat at `clip.opacity` with no keyframes,
  // else a polyline through the opacity keyframe values (mirrors the volume band).
  drawOpacityEnvelope(ctx, clip, rect, body, pxPerFrame, lineColor, selected);

  drawFades(ctx, clip, rect, body, pxPerFrame, fadeColor, true);
  if (!selected) return;
  drawFadeKneeSquares(ctx, clip, rect, body, pxPerFrame, lineColor);
}

/** Opacity envelope line — same line structure as the volume rubber band (0..1 axis). */
function drawOpacityEnvelope(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  rect: Rect,
  body: Rect,
  pxPerFrame: number,
  lineColor: string,
  selected: boolean,
): void {
  ctx.strokeStyle = lineColor;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  const kfs = (clip.opacityTrack?.keyframes ?? []).filter(
    (k) => k.frame >= 0 && k.frame <= clip.durationFrames,
  );
  if (kfs.length > 0) {
    const firstX = rect.x + kfs[0].frame * pxPerFrame;
    const firstY = yForOpacity(kfs[0].value, body);
    ctx.moveTo(rect.x, firstY);
    ctx.lineTo(firstX, firstY);
    for (let i = 0; i < kfs.length - 1; i++) {
      const a = kfs[i];
      const b = kfs[i + 1];
      const bX = rect.x + b.frame * pxPerFrame;
      const aY = yForOpacity(a.value, body);
      const bY = yForOpacity(b.value, body);
      if (a.interpolationOut === "linear") {
        ctx.lineTo(bX, bY);
      } else if (a.interpolationOut === "hold") {
        ctx.lineTo(bX, aY);
        ctx.lineTo(bX, bY);
      } else {
        const aX = rect.x + a.frame * pxPerFrame;
        const steps = 12;
        for (let s = 1; s <= steps; s++) {
          const t = s / steps;
          const x = aX + (bX - aX) * t;
          const v = a.value + (b.value - a.value) * smoothstep(t);
          ctx.lineTo(x, yForOpacity(v, body));
        }
      }
    }
    ctx.lineTo(rect.x + rect.w, yForOpacity(kfs[kfs.length - 1].value, body));
  } else {
    const oy = yForOpacity(clip.opacity, body);
    ctx.moveTo(rect.x, oy);
    ctx.lineTo(rect.x + rect.w, oy);
  }
  ctx.stroke();

  if (!selected) return;
  // Opacity keyframe diamonds on the line.
  ctx.fillStyle = lineColor;
  ctx.strokeStyle = rgba(0, 0, 0, 0.5);
  ctx.lineWidth = 0.5;
  const half = ClipRender.volumeKeyframeSize / 2;
  for (const kf of kfs) {
    const cx = rect.x + kf.frame * pxPerFrame;
    const cy = yForOpacity(kf.value, body);
    diamond(ctx, cx, cy, half);
  }
}

function drawFades(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  rect: Rect,
  body: Rect,
  pxPerFrame: number,
  fadeColor: string,
  fillFromTop: boolean,
): void {
  const kneeY = body.y + ClipRender.fadeKneeTopInset;
  const silenceY = body.y + body.h;
  const fillTopY = fillFromTop ? body.y : kneeY;
  const fillAlpha = fillFromTop ? 0.6 : 0.35;

  if (clip.fadeInFrames > 0) {
    const leftOffset = Math.min(clip.fadeInFrames, clip.durationFrames);
    const kneeX = fadeHandleRenderX(rect, leftOffset, true, pxPerFrame);
    drawFadeWedge(
      ctx,
      { x: rect.x, y: silenceY },
      { x: kneeX, y: kneeY },
      clip.fadeInInterpolation,
      fadeColor,
      fillTopY,
      fillAlpha,
    );
  }
  if (clip.fadeOutFrames > 0) {
    const rightOffset = Math.max(0, clip.durationFrames - clip.fadeOutFrames);
    const kneeX = fadeHandleRenderX(rect, rightOffset, false, pxPerFrame);
    drawFadeWedge(
      ctx,
      { x: rect.x + rect.w, y: silenceY },
      { x: kneeX, y: kneeY },
      clip.fadeOutInterpolation,
      fadeColor,
      fillTopY,
      fillAlpha,
    );
  }
}

function fadeCurvePoints(
  start: { x: number; y: number },
  end: { x: number; y: number },
  interp: "linear" | "hold" | "smooth",
): { x: number; y: number }[] {
  if (!isSmooth(interp)) return [end];
  const steps = 12;
  const out: { x: number; y: number }[] = [];
  for (let s = 1; s <= steps; s++) {
    const t = s / steps;
    out.push({
      x: start.x + (end.x - start.x) * t,
      y: start.y + (end.y - start.y) * smoothstep(t),
    });
  }
  return out;
}

function drawFadeWedge(
  ctx: CanvasRenderingContext2D,
  silentCorner: { x: number; y: number },
  knee: { x: number; y: number },
  interp: "linear" | "hold" | "smooth",
  color: string,
  fillTopY: number,
  fillAlpha: number,
): void {
  const curve = fadeCurvePoints(silentCorner, knee, interp);
  const topY = fillTopY;

  // Fill the wedge above the curve.
  ctx.save();
  ctx.beginPath();
  ctx.moveTo(silentCorner.x, silentCorner.y);
  ctx.lineTo(silentCorner.x, topY);
  ctx.lineTo(knee.x, topY);
  if (topY !== knee.y) ctx.lineTo(knee.x, knee.y);
  for (let i = curve.length - 1; i >= 1; i--) ctx.lineTo(curve[i].x, curve[i].y);
  ctx.closePath();
  ctx.fillStyle = rgba(0, 0, 0, fillAlpha);
  ctx.fill();
  ctx.restore();

  // Stroke the curve.
  ctx.strokeStyle = color;
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  ctx.moveTo(silentCorner.x, silentCorner.y);
  for (const p of curve) ctx.lineTo(p.x, p.y);
  ctx.stroke();
}

function drawFadeKneeSquares(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  rect: Rect,
  body: Rect,
  pxPerFrame: number,
  lineColor: string,
): void {
  const kneeY = body.y + ClipRender.fadeKneeTopInset;
  const leftOffset = Math.min(clip.fadeInFrames, clip.durationFrames);
  const rightOffset = Math.max(0, clip.durationFrames - clip.fadeOutFrames);
  const leftX = fadeHandleRenderX(rect, leftOffset, true, pxPerFrame);
  const rightX = fadeHandleRenderX(rect, rightOffset, false, pxPerFrame);
  const size = ClipRender.volumeKeyframeSize;
  const half = size / 2;
  ctx.fillStyle = lineColor;
  ctx.strokeStyle = rgba(0, 0, 0, 0.5);
  ctx.lineWidth = 0.5;
  for (const x of [leftX, rightX]) {
    ctx.fillRect(x - half, kneeY - half, size, size);
    ctx.strokeRect(x - half, kneeY - half, size, size);
  }
}

function diamond(
  ctx: CanvasRenderingContext2D,
  cx: number,
  cy: number,
  half: number,
): void {
  ctx.beginPath();
  ctx.moveTo(cx, cy - half);
  ctx.lineTo(cx + half, cy);
  ctx.lineTo(cx, cy + half);
  ctx.lineTo(cx - half, cy);
  ctx.closePath();
  ctx.fill();
  ctx.stroke();
}

// --- Keyframe diamonds (non-volume) near clip bottom ---

function drawKeyframeMarkers(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  rect: Rect,
): void {
  if (clip.durationFrames <= 0) return;
  const frameSet = new Set<number>();
  for (const t of [clip.opacityTrack, clip.positionTrack, clip.scaleTrack, clip.cropTrack]) {
    for (const kf of t?.keyframes ?? []) frameSet.add(kf.frame + clip.startFrame);
  }
  if (frameSet.size === 0) return;
  const pxPerFrame = (rect.w - 2 * Trim.handleWidth) / clip.durationFrames;
  if (pxPerFrame <= 0) return;
  const baseX = rect.x + Trim.handleWidth;
  const y = rect.y + rect.h - 5;
  const half = 3;
  ctx.fillStyle = Theme.keyframeFill;
  ctx.strokeStyle = rgba(0, 0, 0, 0.5);
  ctx.lineWidth = 0.5;
  const lo = clip.startFrame;
  const hi = endFrame(clip);
  for (const f of [...frameSet].sort((a, b) => a - b)) {
    if (f < lo || f >= hi) continue;
    const x = baseX + (f - clip.startFrame) * pxPerFrame;
    diamond(ctx, x, y, half);
  }
}

// --- Label bar (ClipRenderer.drawLabelBar) ---

function drawLabelBar(
  ctx: CanvasRenderingContext2D,
  clip: ClipView,
  rect: Rect,
  contentX: number,
  contentWidth: number,
  fps: number,
): void {
  if (rect.w <= 20) return;
  const timecode = formatTimecode(clip.durationFrames, fps);
  const name = firstNonEmptyLine(clip.name || clip.mediaRef);
  const text = `${name}  ${timecode}`;
  const inset = 6;

  ctx.save();
  ctx.beginPath();
  ctx.rect(contentX + inset, rect.y, contentWidth - inset, ClipRender.labelBarHeight);
  ctx.clip();
  ctx.fillStyle = Theme.text.primary;
  ctx.font = "500 10px ui-sans-serif, system-ui, sans-serif";
  ctx.textBaseline = "middle";
  const tx = contentX + inset;
  const ty = rect.y + ClipRender.labelBarHeight / 2;
  ctx.fillText(text, tx, ty);

  // Underline the name when the clip is linked.
  if (clip.linkGroupId) {
    const nameW = ctx.measureText(name).width;
    ctx.strokeStyle = Theme.text.primary;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(tx, ty + 6);
    ctx.lineTo(tx + nameW, ty + 6);
    ctx.stroke();
  }
  ctx.restore();
}

function firstNonEmptyLine(s: string): string {
  for (const line of s.split(/\r?\n/)) {
    const t = line.trim();
    if (t) return t;
  }
  return s;
}

// --- Ruler (TimelineRuler.draw) ---

function drawRuler(
  ctx: CanvasRenderingContext2D,
  layout: TimelineLayout,
  fps: number,
  pixelsPerFrame: number,
  scrollX: number,
  width: number,
): void {
  const rulerH = layout.rulerHeight;
  // Background + bottom separator.
  ctx.fillStyle = Theme.background.surface;
  ctx.fillRect(0, 0, width, rulerH);
  ctx.strokeStyle = Theme.border.primary;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, rulerH - 0.5);
  ctx.lineTo(width, rulerH - 0.5);
  ctx.stroke();

  if (!(pixelsPerFrame > 0) || !Number.isFinite(pixelsPerFrame)) return;
  const framesPerMajor = tickInterval(pixelsPerFrame, fps);
  if (framesPerMajor <= 0) return;

  const startFrame = Math.max(
    0,
    Math.floor(scrollX / pixelsPerFrame) - framesPerMajor,
  );
  const endFrameV = Math.floor((scrollX + width) / pixelsPerFrame) + framesPerMajor;

  const minorCount = minorSubdivisions(framesPerMajor, pixelsPerFrame);
  const framesPerMinor = minorCount > 0 ? framesPerMajor / minorCount : 0;

  // Minor ticks first.
  if (framesPerMinor > 0) {
    ctx.strokeStyle = rgba(255, 255, 255, 0.34 * 0.4);
    ctx.lineWidth = 0.5;
    let minorFrame = Math.floor(startFrame / framesPerMinor) * framesPerMinor;
    while (minorFrame <= endFrameV) {
      if (minorFrame % framesPerMajor !== 0) {
        const localX = minorFrame * pixelsPerFrame - scrollX;
        if (localX >= 0 && localX <= width) {
          const isMidpoint =
            minorCount % 2 === 0 && minorFrame % (framesPerMajor / 2) === 0;
          const tickHeight = isMidpoint ? 6 : 4;
          ctx.beginPath();
          ctx.moveTo(localX, rulerH - tickHeight);
          ctx.lineTo(localX, rulerH);
          ctx.stroke();
        }
      }
      minorFrame += framesPerMinor;
    }
  }

  // Major ticks + labels.
  ctx.font =
    "10px ui-monospace, 'SF Mono', 'Cascadia Mono', Menlo, Consolas, monospace";
  ctx.textBaseline = "top";
  let frame = Math.floor(startFrame / framesPerMajor) * framesPerMajor;
  while (frame <= endFrameV) {
    const localX = frame * pixelsPerFrame - scrollX;
    if (localX >= 0 && localX <= width) {
      ctx.strokeStyle = Theme.text.muted;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.moveTo(localX, rulerH - 8);
      ctx.lineTo(localX, rulerH);
      ctx.stroke();
      ctx.fillStyle = Theme.text.tertiary;
      ctx.fillText(formatTimecode(frame, fps), localX + 3, 2);
    }
    frame += framesPerMajor;
  }
}

// --- Playhead (PlayheadOverlay) — red line + downward triangle ---

function drawPlayhead(
  ctx: CanvasRenderingContext2D,
  layout: TimelineLayout,
  viewport: TimelineViewport,
  scrollX: number,
  height: number,
): void {
  const x = viewport.playheadFrame * viewport.pixelsPerFrame - scrollX;
  if (x < 0 || x > ctx.canvas.width) {
    // still draw if just offscreen-left guard; simplest: clamp out-of-range.
  }
  const top = layout.rulerHeight;
  const triangleSize = 8;
  ctx.save();
  ctx.fillStyle = Theme.playhead;
  ctx.strokeStyle = Theme.playhead;
  ctx.lineWidth = 1;
  // Vertical line.
  ctx.beginPath();
  ctx.moveTo(x + 0.5, top);
  ctx.lineTo(x + 0.5, height);
  ctx.stroke();
  // Downward triangle at the top.
  const half = triangleSize / 2;
  ctx.beginPath();
  ctx.moveTo(x, top);
  ctx.lineTo(x - half, top - triangleSize);
  ctx.lineTo(x + half, top - triangleSize);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

// Re-export geometry helpers commonly needed alongside the renderer.
export { clipRect, makeLayout, formatTimecode };
export { Snap, Layout };
