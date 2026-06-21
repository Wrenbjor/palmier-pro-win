// Preview panel — the viewport that composes the whole preview surface AND shows the
// actual video frame.
//
// ## What changed (the robust preview path)
// The original panel mounted a transparent viewport expecting a native wgpu swapchain
// to show THROUGH it ("plan A1"). That path was fragile (tao clip_children flicker)
// and never actually presented a frame. This panel instead paints the frame the
// backend composites OFFSCREEN: it calls `preview_render_frame(frame, maxWidth)` (a
// Tauri command that composites the ACTIVE timeline through the shared engine
// compositor and reads it back as base64 RGBA), decodes it, and draws it onto a
// `<canvas>` letterboxed to the composition aspect.
//
// Playback is a `requestAnimationFrame` loop throttled to `composition.fps`: Play
// advances the playhead frame-by-frame, renders each frame, and stops/loops at the
// end; Pause stops it. Seek/scrub and step (±1) render a single frame. The playhead
// is kept in sync with the editor timeline via `onPlayheadChange` (preview ↔ timeline).
//
// Tabs / overlays / zoom / keyboard transport / settings are unchanged.

import { useCallback, useEffect, useMemo, useRef } from "react";

import {
  inTauri,
  previewRenderFrame,
} from "./api";
import { zoomAboutPoint } from "./geometry";
import { ZOOM_MAX, ZOOM_MIN } from "./presets";
import { CropOverlay } from "./CropOverlay";
import { PreviewTabs } from "./PreviewTabs";
import { SettingsMenu } from "./SettingsMenu";
import { TransformOverlay } from "./TransformOverlay";
import { TransportControls } from "./TransportControls";
import { createPreviewStore, usePreviewStore, type PreviewStore } from "./store";
import { type Crop, type Transform } from "./types";

/** Default preview render width (matches the backend `DEFAULT_MAX_WIDTH`). */
const PREVIEW_MAX_WIDTH = 960;

export interface PreviewPanelProps {
  /** The Tauri window label whose native surface this panel presents into. */
  windowLabel?: string;
  /** Composition geometry — timeline width/height/fps (drives aspect + timecode). */
  composition: { width: number; height: number; fps: number };
  /** Duration of the active tab in frames (for the scrub bar + playback stop). */
  durationFrames?: number;
  /** The selected clip's transform/crop (the overlays manipulate these). */
  selected?: { clipId: string; transform: Transform; crop: Crop; mediaCanvasAspect?: number | null } | null;
  /** Whether the crop overlay is active (else the transform overlay). */
  cropEditing?: boolean;
  /** Apply new timeline settings (aspect/fps/quality menu). */
  onApplyTimelineSettings?: (settings: { fps: number; width: number; height: number }) => void;
  /**
   * Called whenever the preview moves the playhead (playback tick / seek / step), so
   * the host can mirror it into the timeline store (preview ↔ timeline sync). The
   * editor's playhead is the source of truth on external seeks; this closes the loop
   * for preview-driven motion.
   */
  onPlayheadChange?: (frame: number) => void;
  /**
   * Bump this to force a repaint of the current frame WITHOUT moving the playhead —
   * e.g. when the timeline content changed (`timeline://changed`) so the same frame
   * now composites differently (a clip was added/trimmed/recolored at the playhead).
   */
  revision?: number;
  /** Optional shared store (else the panel owns one). */
  store?: PreviewStore;
}

/**
 * Decode a base64 string to a `Uint8ClampedArray` over a fresh (non-shared)
 * `ArrayBuffer` — the exact shape `ImageData`'s constructor accepts.
 */
function decodeBase64(b64: string): Uint8ClampedArray<ArrayBuffer> {
  const bin = atob(b64);
  const buf = new ArrayBuffer(bin.length);
  const out = new Uint8ClampedArray(buf);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export function PreviewPanel({
  composition,
  durationFrames = 0,
  selected = null,
  cropEditing = false,
  onApplyTimelineSettings,
  onPlayheadChange,
  revision = 0,
  store: providedStore,
}: PreviewPanelProps) {
  const store = useMemo(() => providedStore ?? createPreviewStore(), [providedStore]);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  // Offscreen canvas holding the backing-size frame, blitted scaled to the visible one.
  const backingRef = useRef<HTMLCanvasElement | null>(null);
  const sizeRef = useRef<{ width: number; height: number }>({ width: 0, height: 0 });

  const tabs = usePreviewStore(store, (s) => s.tabs);
  const activeTabId = usePreviewStore(store, (s) => s.activeTabId);
  const frame = usePreviewStore(store, (s) => s.playheads[s.activeTabId] ?? 0);
  const playing = usePreviewStore(store, (s) => s.playing);
  const zoom = usePreviewStore(store, (s) => s.canvasZoom);
  const offset = usePreviewStore(store, (s) => s.canvasOffset);

  const aspect = composition.height > 0 ? composition.width / composition.height : 16 / 9;

  // ── Frame painting: composite offscreen (backend) → draw letterboxed on canvas. ──
  // Tracks the latest in-flight request so a stale async render never paints over a
  // newer one (rapid scrub / play).
  const reqIdRef = useRef(0);
  const lastDrawnRef = useRef<number>(-1);

  // Paint a single frame index. Returns a promise that resolves once drawn (or skipped).
  const paintFrame = useCallback(
    async (frameIndex: number) => {
      const canvas = canvasRef.current;
      const viewport = viewportRef.current;
      if (!canvas || !viewport) return;

      // Size the visible canvas to the viewport (device-pixel aware), letterboxing the
      // composition aspect inside it.
      const vw = Math.max(1, Math.round(viewport.clientWidth));
      const vh = Math.max(1, Math.round(viewport.clientHeight));
      if (canvas.width !== vw || canvas.height !== vh) {
        canvas.width = vw;
        canvas.height = vh;
      }
      const ctx = canvas.getContext("2d");
      if (!ctx) return;

      const reqId = ++reqIdRef.current;
      const data = inTauri()
        ? await previewRenderFrame(frameIndex, PREVIEW_MAX_WIDTH)
        : undefined;
      // A newer request superseded this one — drop it (avoid out-of-order paints).
      if (reqId !== reqIdRef.current) return;

      // Clear to black (letterbox bars + the empty/no-Tauri case).
      ctx.fillStyle = "#000";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      if (!data) {
        lastDrawnRef.current = frameIndex;
        return;
      }

      // Put the backing-size RGBA into the offscreen canvas.
      let backing = backingRef.current;
      if (!backing) {
        backing = document.createElement("canvas");
        backingRef.current = backing;
      }
      if (backing.width !== data.width || backing.height !== data.height) {
        backing.width = data.width;
        backing.height = data.height;
      }
      const bctx = backing.getContext("2d");
      if (!bctx) return;
      const pixels = decodeBase64(data.rgbaBase64);
      if (pixels.length >= data.width * data.height * 4) {
        bctx.putImageData(new ImageData(pixels, data.width, data.height), 0, 0);
      }

      // Letterbox the composition aspect inside the canvas.
      const canvasAspect = canvas.width / canvas.height;
      let dw = canvas.width;
      let dh = canvas.height;
      if (canvasAspect > aspect) {
        dh = canvas.height;
        dw = dh * aspect;
      } else {
        dw = canvas.width;
        dh = dw / aspect;
      }
      const dx = (canvas.width - dw) / 2;
      const dy = (canvas.height - dh) / 2;
      ctx.imageSmoothingEnabled = true;
      ctx.drawImage(backing, 0, 0, data.width, data.height, dx, dy, dw, dh);
      lastDrawnRef.current = frameIndex;
    },
    [aspect],
  );

  // ── Resize tracking: re-measure + repaint the current frame on viewport resize. ──
  useEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0].contentRect;
      sizeRef.current = { width: r.width, height: r.height };
      void paintFrame(store.getState().playheads[store.getState().activeTabId] ?? 0);
      // Force overlays to re-measure (store touch).
      store.setOffset({ ...store.getState().canvasOffset });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [store, paintFrame]);

  // ── Initial paint + repaint when the external playhead / tab changes (seek). ──
  // The playback loop owns motion WHILE playing; this handles paused seeks/steps and
  // the timeline→preview mirror. Skips a redundant repaint of the already-drawn frame.
  useEffect(() => {
    if (playing) return; // the rAF loop is driving frames.
    if (lastDrawnRef.current === frame) return;
    void paintFrame(frame);
  }, [frame, activeTabId, playing, paintFrame]);

  // ── Content repaint: the timeline changed at the same frame → recomposite it. ──
  useEffect(() => {
    if (playing) return;
    void paintFrame(store.getState().playheads[store.getState().activeTabId] ?? 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [revision]);

  // ── Playback loop: rAF throttled to fps; advance the playhead, paint, stop at end. ──
  useEffect(() => {
    if (!playing) return;
    const fps = Math.max(1, composition.fps);
    const frameDurMs = 1000 / fps;
    let raf = 0;
    let cancelled = false;
    let lastTs = performance.now();
    // Render the current frame immediately on play.
    let current = store.getState().playheads[store.getState().activeTabId] ?? 0;
    const total = store.getState().durations[store.getState().activeTabId] ?? durationFrames;

    void paintFrame(current);

    const tick = (ts: number) => {
      if (cancelled) return;
      const elapsed = ts - lastTs;
      if (elapsed >= frameDurMs) {
        // Advance by however many frame-durations elapsed (catch up, don't accumulate lag).
        const steps = Math.max(1, Math.floor(elapsed / frameDurMs));
        lastTs = ts;
        current += steps;
        if (total > 0 && current >= total) {
          // Stop at the end (no loop — matches the reference's play-to-end-then-pause).
          current = total;
          store.setActivePlayhead(current);
          onPlayheadChange?.(current);
          store.setPlaying(false);
          void paintFrame(current);
          return;
        }
        store.setActivePlayhead(current);
        onPlayheadChange?.(current);
        void paintFrame(current);
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing, composition.fps, durationFrames, store, paintFrame, onPlayheadChange]);

  // ── Transport actions (local playhead + canvas render; no engine transport). ──
  const seek = useCallback(
    (target: number) => {
      const dur = store.getState().durations[store.getState().activeTabId] ?? durationFrames;
      const f = Math.max(0, dur > 0 ? Math.min(target, dur) : target);
      store.setActivePlayhead(f);
      onPlayheadChange?.(f);
      void paintFrame(f);
    },
    [store, durationFrames, paintFrame, onPlayheadChange],
  );

  const togglePlay = useCallback(() => {
    store.setPlaying(!store.getState().playing);
  }, [store]);

  const step = useCallback(
    (delta: number) => {
      const cur = store.getState().playheads[store.getState().activeTabId] ?? 0;
      seek(cur + delta);
    },
    [store, seek],
  );

  // ── Keyboard transport: Space/K play-pause, J/L step, Home/End, ←/→ frame. ──
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      const dur = store.getState().durations[store.getState().activeTabId] ?? durationFrames;
      switch (e.key) {
        case " ":
        case "k":
        case "K":
          e.preventDefault();
          togglePlay();
          break;
        case "j":
        case "J":
        case "ArrowLeft":
          e.preventDefault();
          step(-1);
          break;
        case "l":
        case "L":
        case "ArrowRight":
          e.preventDefault();
          step(1);
          break;
        case "Home":
          e.preventDefault();
          seek(0);
          break;
        case "End":
          e.preventDefault();
          seek(dur);
          break;
        default:
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [durationFrames, seek, step, togglePlay, store]);

  // ── cmd/ctrl-scroll zoom about the cursor (reference onCmdScroll). ──
  const onWheel = useCallback(
    (e: React.WheelEvent) => {
      if (!e.metaKey && !e.ctrlKey) return;
      e.preventDefault();
      const el = viewportRef.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      const point = { x: e.clientX - r.left, y: e.clientY - r.top };
      const sensitivity = 0.005;
      const deltaY = -e.deltaY * sensitivity;
      if (deltaY === 0) return;
      const result = zoomAboutPoint({
        deltaY,
        point,
        viewSize: { width: r.width, height: r.height },
        oldZoom: store.getState().canvasZoom,
        offset: store.getState().canvasOffset,
        minZoom: ZOOM_MIN,
        maxZoom: ZOOM_MAX,
      });
      if (result) {
        store.setZoom(result.zoom);
        store.setOffset(result.offset);
      }
    },
    [store],
  );

  // ── Overlay edit routing (Tauri commands into the edit engine). ──
  const applyTransform = useCallback(
    (t: Transform) => {
      if (selected) void import("./api").then((api) => api.previewApplyTransform(selected.clipId, t));
    },
    [selected],
  );
  const applyCrop = useCallback(
    (c: Crop) => {
      if (selected) void import("./api").then((api) => api.previewApplyCrop(selected.clipId, c));
    },
    [selected],
  );

  const viewSize = sizeRef.current;
  const transform = selected?.transform ?? null;
  const crop = selected?.crop ?? null;

  return (
    <div className="flex h-full flex-col bg-[#161616]">
      <PreviewTabs
        tabs={tabs}
        activeTabId={activeTabId}
        onSelect={(id) => store.selectTab(id)}
        onClose={(id) => store.closeTab(id)}
        onCloseAll={() => store.closeAllTabs()}
      />

      {/* Viewport — the composited frame is painted on the canvas below; overlays float over it. */}
      <div
        ref={viewportRef}
        className="relative flex-1 overflow-hidden"
        onWheel={onWheel}
        data-preview-viewport
      >
        {/* The composited preview frame (offscreen render → readback → here). */}
        <canvas
          ref={canvasRef}
          className="absolute inset-0 h-full w-full"
          data-preview-canvas
        />
        <div
          className="absolute"
          style={{
            left: "50%",
            top: "50%",
            transform: `translate(-50%, -50%) translate(${offset.width}px, ${offset.height}px) scale(${zoom})`,
            width: viewSize.width,
            height: viewSize.height,
          }}
        >
          {cropEditing ? (
            <CropOverlay
              viewSize={viewSize}
              videoAspect={aspect}
              transform={transform}
              crop={crop}
              aspectNormalized={null}
              onApply={applyCrop}
              onCommit={applyCrop}
            />
          ) : (
            <TransformOverlay
              viewSize={viewSize}
              videoAspect={aspect}
              transform={transform}
              mediaCanvasAspect={selected?.mediaCanvasAspect ?? null}
              onApply={applyTransform}
              onCommit={applyTransform}
            />
          )}
        </div>
      </div>

      <div className="border-t border-white/10">
        <TransportControls
          fps={composition.fps}
          frame={frame}
          durationFrames={store.getState().durations[activeTabId] ?? durationFrames}
          playing={playing}
          onTogglePlay={togglePlay}
          onSeek={(f) => seek(f)}
          onScrubBegin={() => store.setScrubbing(true)}
          onScrubEnd={(f) => {
            store.setScrubbing(false);
            seek(f);
          }}
        />
        <div className="flex items-center justify-end px-4 pb-2">
          <SettingsMenu
            width={composition.width}
            height={composition.height}
            fps={composition.fps}
            zoom={zoom}
            onApplyTimelineSettings={(s) => onApplyTimelineSettings?.(s)}
            onSetZoom={(z) => {
              store.resetView();
              store.setZoom(z);
            }}
          />
        </div>
      </div>
    </div>
  );
}
