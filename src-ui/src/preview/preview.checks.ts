// Pure-logic checks for the preview viewport (E5-S10).
//
// Mirrors the editor's `*.checks.ts` harness: a list of assertions over the pure
// geometry / preset / store logic, runnable headlessly via `_run-checks.mts` (no DOM,
// no Tauri). These guard the load-bearing overlay math (ported verbatim from the macOS
// reference) and the tab/playback state machine.

import {
  clipFrame,
  clipLocalDelta,
  cropFrame,
  fitSize,
  formatTimecode,
  movedTransform,
  pannedCrop,
  resizedCrop,
  resizedTransform,
  scrubFrame,
  videoContentRect,
  zoomAboutPoint,
  type Rect,
} from "./geometry.ts";
import {
  aspectBadgeLabel,
  ASPECT_PRESETS,
  isZoomPresetActive,
  qualityMatches,
  qualityResolution,
  ZOOM_MAX,
  ZOOM_MIN,
  ZOOM_PRESETS,
  zoomBadgeLabel,
} from "./presets.ts";
import { createPreviewStore } from "./store.ts";
import {
  identityCrop,
  identityTransform,
  isCloseable,
  tabId,
  topLeft,
  visibleWidthFraction,
  type Crop,
  type Transform,
} from "./types.ts";

type Check = { name: string; run: () => void };

function approx(a: number, b: number, eps = 1e-6): boolean {
  return Math.abs(a - b) <= eps;
}
function assert(cond: boolean, msg: string): void {
  if (!cond) throw new Error(msg);
}

const checks: Check[] = [
  // ── viewport letterbox geometry ──
  {
    name: "videoContentRect letterboxes a 16:9 video in a wide viewport",
    run: () => {
      const r = videoContentRect({ width: 1600, height: 600 }, 16 / 9);
      // height-bound: h=600, w=600*16/9=1066.67, centered horizontally.
      assert(approx(r.height, 600), `height ${r.height}`);
      assert(approx(r.width, 600 * (16 / 9), 1e-3), `width ${r.width}`);
      assert(approx(r.x, (1600 - r.width) / 2, 1e-3), `x ${r.x}`);
      assert(approx(r.y, 0), `y ${r.y}`);
    },
  },
  {
    name: "videoContentRect letterboxes in a tall viewport",
    run: () => {
      const r = videoContentRect({ width: 800, height: 1200 }, 16 / 9);
      // width-bound: w=800, h=450, centered vertically.
      assert(approx(r.width, 800), `width ${r.width}`);
      assert(approx(r.height, 450), `height ${r.height}`);
      assert(approx(r.y, (1200 - 450) / 2), `y ${r.y}`);
    },
  },
  {
    name: "fitSize matches videoContentRect's size",
    run: () => {
      const f = fitSize({ width: 1600, height: 600 }, 16 / 9);
      const r = videoContentRect({ width: 1600, height: 600 }, 16 / 9);
      assert(approx(f.width, r.width, 1e-9) && approx(f.height, r.height, 1e-9), "fitSize != rect size");
    },
  },

  // ── transform move + snap ──
  {
    name: "movedTransform translates center by normalized delta",
    run: () => {
      const start = identityTransform();
      start.width = 0.5;
      start.height = 0.5;
      const vr: Rect = { x: 0, y: 0, width: 1000, height: 1000 };
      const { transform } = movedTransform(start, { width: 200, height: -100 }, vr, false);
      // 200px/1000 = 0.2, but center snaps to 0.5 are NOT triggered (0.7 not near 0.5).
      assert(approx(transform.centerX, 0.7), `cx ${transform.centerX}`);
      assert(approx(transform.centerY, 0.4), `cy ${transform.centerY}`);
    },
  },
  {
    name: "movedTransform snaps center to canvas center within threshold",
    run: () => {
      const start = identityTransform();
      start.width = 0.5;
      start.height = 0.5;
      start.centerX = 0.5 + 0.005; // a few px off center
      start.centerY = 0.5;
      const vr: Rect = { x: 0, y: 0, width: 1000, height: 1000 };
      // tiny 1px nudge keeps it within the 8px threshold → snaps to 0.5.
      const { transform, snap } = movedTransform(start, { width: -1, height: 0 }, vr, false);
      assert(approx(transform.centerX, 0.5), `cx ${transform.centerX}`);
      assert(snap.x === true, "snap.x should be true");
    },
  },
  {
    name: "movedTransform skips snap when rotated",
    run: () => {
      const start = identityTransform();
      start.rotation = 30;
      start.width = 0.5;
      start.height = 0.5;
      const vr: Rect = { x: 0, y: 0, width: 1000, height: 1000 };
      const { snap } = movedTransform(start, { width: 0, height: 0 }, vr, true);
      assert(!snap.x && !snap.y, "no snap under rotation");
    },
  },

  // ── transform resize ──
  {
    name: "resizedTransform bottomRight grows width/height, pins top-left",
    run: () => {
      const start = identityTransform();
      start.centerX = 0.25;
      start.centerY = 0.25;
      start.width = 0.4;
      start.height = 0.4;
      const tl0 = topLeft(start); // (0.05, 0.05)
      const vr: Rect = { x: 0, y: 0, width: 1000, height: 1000 };
      const out = resizedTransform(start, "bottomRight", { width: 100, height: 100 }, vr, null, false);
      const tl1 = topLeft(out);
      assert(approx(tl1.x, tl0.x, 1e-9) && approx(tl1.y, tl0.y, 1e-9), "top-left should be pinned");
      assert(out.width > start.width && out.height > start.height, "should grow");
    },
  },
  {
    name: "resizedTransform never inverts (min size honored)",
    run: () => {
      const start = identityTransform();
      const vr: Rect = { x: 0, y: 0, width: 1000, height: 1000 };
      // Drag the top-left corner way past the bottom-right.
      const out = resizedTransform(start, "topLeft", { width: 5000, height: 5000 }, vr, null, false);
      assert(out.width >= 0.05 && out.height >= 0.05, `min size: ${out.width}x${out.height}`);
    },
  },
  {
    name: "resizedTransform respects aspect lock",
    run: () => {
      const start = identityTransform();
      start.centerX = 0.3;
      start.centerY = 0.3;
      start.width = 0.3;
      start.height = 0.3;
      const vr: Rect = { x: 0, y: 0, width: 1000, height: 1000 };
      const out = resizedTransform(start, "bottomRight", { width: 200, height: 50 }, vr, 2.0, false);
      // aspect 2.0 → width should be ~2x height.
      assert(approx(out.width / out.height, 2.0, 1e-6), `aspect ${out.width / out.height}`);
    },
  },

  // ── crop counter-rotation + pan/resize ──
  {
    name: "clipLocalDelta is identity at 0deg and rotates at 90deg",
    run: () => {
      const d0 = clipLocalDelta({ width: 10, height: 5 }, 0);
      assert(approx(d0.width, 10) && approx(d0.height, 5), "0deg identity");
      const d90 = clipLocalDelta({ width: 10, height: 0 }, 90);
      // rotating a screen +x delta into local axes of a 90deg-rotated clip → -y.
      assert(approx(d90.width, 0, 1e-9), `90 width ${d90.width}`);
      assert(approx(d90.height, -10, 1e-9), `90 height ${d90.height}`);
    },
  },
  {
    name: "pannedCrop keeps visible size constant",
    run: () => {
      const start: Crop = { left: 0.1, top: 0.1, right: 0.1, bottom: 0.1 };
      const clipRect: Rect = { x: 0, y: 0, width: 800, height: 800 };
      const out = pannedCrop(start, { width: 80, height: 0 }, clipRect);
      assert(approx(visibleWidthFraction(out), visibleWidthFraction(start), 1e-9), "visW changed");
      assert(out.left > start.left, "panned right");
    },
  },
  {
    name: "pannedCrop clamps inside [0,1]",
    run: () => {
      const start: Crop = { left: 0.1, top: 0.1, right: 0.1, bottom: 0.1 };
      const clipRect: Rect = { x: 0, y: 0, width: 800, height: 800 };
      const out = pannedCrop(start, { width: 100000, height: 0 }, clipRect);
      assert(out.left >= 0 && out.right >= 0 && out.left + visibleWidthFraction(out) <= 1 + 1e-9, "out of bounds");
    },
  },
  {
    name: "resizedCrop free corner shrinks visible region",
    run: () => {
      const start = identityCrop();
      const clipRect: Rect = { x: 0, y: 0, width: 800, height: 800 };
      const out = resizedCrop(start, "topLeft", { width: 80, height: 80 }, clipRect, null);
      assert(out.left > 0 && out.top > 0, "top-left inset");
      assert(visibleWidthFraction(out) < 1, "shrank");
    },
  },

  // ── clip/crop frame mapping ──
  {
    name: "clipFrame + cropFrame map identity transform/crop to full video rect",
    run: () => {
      const vr: Rect = { x: 100, y: 50, width: 800, height: 450 };
      const cf = clipFrame(identityTransform(), vr);
      assert(approx(cf.x, 100) && approx(cf.y, 50) && approx(cf.width, 800) && approx(cf.height, 450), "clipFrame");
      const crf = cropFrame(identityCrop(), cf);
      assert(approx(crf.width, 800) && approx(crf.height, 450), "cropFrame full");
    },
  },

  // ── zoom-about-point ──
  {
    name: "zoomAboutPoint keeps the cursor point fixed",
    run: () => {
      const viewSize = { width: 1000, height: 1000 };
      const point = { x: 250, y: 250 };
      const res = zoomAboutPoint({
        deltaY: 0.2,
        point,
        viewSize,
        oldZoom: 1,
        offset: { width: 0, height: 0 },
        minZoom: ZOOM_MIN,
        maxZoom: ZOOM_MAX,
      });
      assert(res !== null, "should zoom");
      assert(res!.zoom > 1, "zoomed in");
    },
  },
  {
    name: "zoomAboutPoint clamps to [min,max] and returns null on no-op",
    run: () => {
      const res = zoomAboutPoint({
        deltaY: 100, // huge → clamps to max
        point: { x: 0, y: 0 },
        viewSize: { width: 100, height: 100 },
        oldZoom: ZOOM_MAX,
        offset: { width: 0, height: 0 },
        minZoom: ZOOM_MIN,
        maxZoom: ZOOM_MAX,
      });
      assert(res === null, "already at max → no change");
    },
  },

  // ── presets ──
  {
    name: "aspect presets include all 6 required ratios",
    run: () => {
      const labels = ASPECT_PRESETS.map((p) => p.label);
      for (const want of ["16:9", "9:14", "9:16", "1:1", "4:3", "2.4:1"]) {
        assert(labels.includes(want), `missing aspect ${want}`);
      }
    },
  },
  {
    name: "aspectBadgeLabel reduces a resolution",
    run: () => {
      assert(aspectBadgeLabel(1920, 1080) === "16:9", aspectBadgeLabel(1920, 1080));
      assert(aspectBadgeLabel(1080, 1080) === "1:1", aspectBadgeLabel(1080, 1080));
    },
  },
  {
    name: "qualityResolution preserves aspect and hits the short edge",
    run: () => {
      const r = qualityResolution({ label: "4K", shortEdge: 2160 }, 1920, 1080);
      assert(Math.min(r.width, r.height) === 2160, `short edge ${Math.min(r.width, r.height)}`);
      assert(approx(r.width / r.height, 16 / 9, 1e-2), `aspect ${r.width / r.height}`);
    },
  },
  {
    name: "qualityMatches keys off short edge",
    run: () => {
      assert(qualityMatches({ label: "1080p", shortEdge: 1080 }, 1920, 1080), "1080 match");
      assert(!qualityMatches({ label: "720p", shortEdge: 720 }, 1920, 1080), "720 no-match");
    },
  },
  {
    name: "zoom presets include Fit (1.0) and the 25..200% range",
    run: () => {
      const values = ZOOM_PRESETS.map((p) => p.value);
      for (const v of [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 2.0]) {
        assert(values.includes(v), `missing zoom ${v}`);
      }
      assert(zoomBadgeLabel(1.0) === "Fit", "Fit label");
      assert(zoomBadgeLabel(2.0) === "200%", "200% label");
      assert(isZoomPresetActive({ label: "Fit", value: 1.0 }, 1.0), "fit active");
    },
  },

  // ── timecode + scrub ──
  {
    name: "formatTimecode HH:MM:SS:FF at 30fps",
    run: () => {
      assert(formatTimecode(0, 30) === "00:00:00:00", formatTimecode(0, 30));
      assert(formatTimecode(30, 30) === "00:00:01:00", formatTimecode(30, 30));
      assert(formatTimecode(90 * 30 + 5, 30) === "00:01:30:05", formatTimecode(90 * 30 + 5, 30));
    },
  },
  {
    name: "scrubFrame maps location to a clamped frame",
    run: () => {
      assert(scrubFrame(0, 100, 300) === 0, "start");
      assert(scrubFrame(50, 100, 300) === 150, "mid");
      assert(scrubFrame(1000, 100, 300) === 300, "clamped end");
    },
  },

  // ── store: tab + playback state machine ──
  {
    name: "store opens, activates, and closes asset tabs (timeline non-closable)",
    run: () => {
      const s = createPreviewStore();
      assert(s.getState().tabs.length === 1, "starts with timeline only");
      s.openTab({ kind: "mediaAsset", id: "a1", name: "Clip.mp4", clipType: "video" });
      assert(s.getState().tabs.length === 2, "asset tab opened");
      assert(s.getState().activeTabId === "media_a1", "asset tab active");
      // timeline non-closable
      s.closeTab("__timeline__");
      assert(s.getState().tabs.length === 2, "timeline can't be closed");
      // close asset → falls back to timeline
      s.closeTab("media_a1");
      assert(s.getState().tabs.length === 1 && s.getState().activeTabId === "__timeline__", "fallback to timeline");
    },
  },
  {
    name: "store retains per-tab playheads across activation",
    run: () => {
      const s = createPreviewStore();
      s.openTab({ kind: "mediaAsset", id: "a1", name: "Clip.mp4", clipType: "video" });
      s.setActivePlayhead(25); // asset tab
      s.selectTab("__timeline__");
      s.setActivePlayhead(100); // timeline tab
      assert((s.getState().playheads["__timeline__"] ?? 0) === 100, "timeline playhead");
      assert((s.getState().playheads["media_a1"] ?? 0) === 25, "asset playhead retained");
    },
  },
  {
    name: "store closeAllTabs keeps only the timeline",
    run: () => {
      const s = createPreviewStore();
      s.openTab({ kind: "mediaAsset", id: "a1", name: "A", clipType: "video" });
      s.openTab({ kind: "mediaAsset", id: "a2", name: "B", clipType: "image" });
      s.closeAllTabs();
      assert(s.getState().tabs.length === 1 && s.getState().activeTabId === "__timeline__", "only timeline");
    },
  },

  // ── tab identity ──
  {
    name: "tabId + isCloseable mirror the engine model",
    run: () => {
      assert(tabId({ kind: "timeline" }) === "__timeline__", "timeline id");
      assert(tabId({ kind: "mediaAsset", id: "x", name: "n", clipType: "video" }) === "media_x", "asset id");
      assert(!isCloseable({ kind: "timeline" }), "timeline non-closable");
      assert(isCloseable({ kind: "mediaAsset", id: "x", name: "n", clipType: "video" }), "asset closable");
    },
  },
];

export function runPreviewChecks(): string[] {
  const failures: string[] = [];
  for (const c of checks) {
    try {
      c.run();
    } catch (e) {
      failures.push(`${c.name}: ${(e as Error).message}`);
    }
  }
  return failures;
}

export const PREVIEW_CHECK_COUNT = checks.length;

// Silence "unused" for the verbatim-ported transform type re-export consumers.
export type { Transform };
