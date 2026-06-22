// Preview panel — the viewport that composes the whole preview surface AND shows the
// actual video frame.
//
// ## What changed (the robust preview path)
// The original panel mounted a transparent viewport expecting a native wgpu swapchain
// to show THROUGH it ("plan A1"). That path was fragile (tao clip_children flicker)
// and never actually presented a frame. This panel instead paints the frame the
// backend composites OFFSCREEN: it calls `preview_render_frame(frame, maxWidth)` (a
// Tauri command that composites the ACTIVE timeline through the shared engine
// compositor and reads it back as a base64 JPEG), decodes it via `createImageBitmap`,
// and draws it onto a `<canvas>` letterboxed to the composition aspect.
//
// ## Single-flight coalescing (why the app no longer freezes)
// The earlier panel issued one `preview_render_frame` per rAF tick and put each
// base64-RGBA result onto the canvas. Combined with a synchronous backend command,
// that piled renders up into a 30–40 s backlog and froze the UI. This panel NEVER
// queues: it keeps AT MOST ONE render in flight (`renderRef`), tracks the latest
// DESIRED frame, and when the in-flight render resolves it renders again only if the
// desired frame moved — otherwise it stops. The backend command is now async + JPEG,
// so each render is cheap and off the UI thread.
//
// Playback is driven off render completion + wall-clock time: a rAF loop computes the
// TARGET frame from elapsed time and requests it, SKIPPING intermediate frames to keep
// up rather than rendering every frame. Scrub/seek/step request the latest position and
// drop stale ones, so scrubbing always reflects the newest pointer position. The
// playhead is kept in sync with the editor timeline via `onPlayheadChange`.
//
// Tabs / overlays / zoom / keyboard transport / settings are unchanged.

import { useCallback, useEffect, useMemo, useRef } from "react";

import {
  inTauri,
  previewAudioPause,
  previewAudioPlay,
  previewAudioSeek,
  previewAudioStop,
  previewRenderFrame,
  type PreviewFrameData,
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

/**
 * Preview render width while PLAYING — kept small (matches the backend
 * `DEFAULT_MAX_WIDTH`) so each composite+readback+JPEG keeps up with the play loop.
 */
const PLAYBACK_MAX_WIDTH = 480;
/**
 * Preview render width for a PAUSED still / seek / step — crisper, since there is no
 * sustained request stream to keep up with (one render, then idle).
 */
const STILL_MAX_WIDTH = 960;

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
 * Decode a backend preview frame (a base64 JPEG/PNG) into an `ImageBitmap` ready to
 * blit onto a canvas. Uses `createImageBitmap` over a `Blob` so the decode runs off the
 * main thread where the browser supports it. Returns `null` on decode failure.
 */
async function decodePreviewFrame(data: PreviewFrameData): Promise<ImageBitmap | null> {
  try {
    const bin = atob(data.dataBase64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    const mime = data.format === "png" ? "image/png" : "image/jpeg";
    const blob = new Blob([bytes], { type: mime });
    return await createImageBitmap(blob);
  } catch {
    return null;
  }
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
  const sizeRef = useRef<{ width: number; height: number }>({ width: 0, height: 0 });

  const tabs = usePreviewStore(store, (s) => s.tabs);
  const activeTabId = usePreviewStore(store, (s) => s.activeTabId);
  const frame = usePreviewStore(store, (s) => s.playheads[s.activeTabId] ?? 0);
  const playing = usePreviewStore(store, (s) => s.playing);
  const zoom = usePreviewStore(store, (s) => s.canvasZoom);
  const offset = usePreviewStore(store, (s) => s.canvasOffset);

  const aspect = composition.height > 0 ? composition.width / composition.height : 16 / 9;

  // ── Frame painting: composite offscreen (backend) → draw letterboxed on canvas. ──
  // The frame most recently DRAWN to the canvas (for skip-redundant-repaint checks).
  const lastDrawnRef = useRef<number>(-1);
  // Keep `aspect` reachable from the imperative render engine without re-creating it.
  const aspectRef = useRef(aspect);
  aspectRef.current = aspect;

  // Blit a decoded preview frame onto the visible canvas, letterboxed to the
  // composition aspect. Sizing is device-pixel-aware (canvas tracks the viewport).
  const blit = useCallback((bitmap: ImageBitmap | null) => {
    const canvas = canvasRef.current;
    const viewport = viewportRef.current;
    if (!canvas || !viewport) return;
    const vw = Math.max(1, Math.round(viewport.clientWidth));
    const vh = Math.max(1, Math.round(viewport.clientHeight));
    if (canvas.width !== vw || canvas.height !== vh) {
      canvas.width = vw;
      canvas.height = vh;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    // Clear to black (letterbox bars + the empty/no-Tauri case).
    ctx.fillStyle = "#000";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    if (!bitmap) return;

    const a = aspectRef.current;
    const canvasAspect = canvas.width / canvas.height;
    let dw: number;
    let dh: number;
    if (canvasAspect > a) {
      dh = canvas.height;
      dw = dh * a;
    } else {
      dw = canvas.width;
      dh = dw / a;
    }
    const dx = (canvas.width - dw) / 2;
    const dy = (canvas.height - dh) / 2;
    ctx.imageSmoothingEnabled = true;
    ctx.drawImage(bitmap, 0, 0, bitmap.width, bitmap.height, dx, dy, dw, dh);
  }, []);

  // ── Single-flight render engine (NEVER queues). ──────────────────────────────
  // `desired` is the latest frame the UI wants shown; `inFlight` is true while one
  // `preview_render_frame` is outstanding. We render `desired`; when it resolves, if
  // `desired` has since moved we render again, else we stop. So at most ONE render is
  // ever in flight and intermediate frames are dropped (immediate scrub, no backlog).
  const engineRef = useRef<{
    desired: number;
    desiredWidth: number;
    inFlight: boolean;
  }>({ desired: 0, desiredWidth: STILL_MAX_WIDTH, inFlight: false });

  const pumpRef = useRef<() => void>(() => {});
  pumpRef.current = () => {
    const engine = engineRef.current;
    if (engine.inFlight) return; // a render is already running; it will re-pump.
    const target = engine.desired;
    const width = engine.desiredWidth;

    // No Tauri (plain vite dev): just clear to black for the empty viewport.
    if (!inTauri()) {
      blit(null);
      lastDrawnRef.current = target;
      return;
    }

    engine.inFlight = true;
    void (async () => {
      let bitmap: ImageBitmap | null = null;
      try {
        const data = await previewRenderFrame(target, width);
        if (data) bitmap = await decodePreviewFrame(data);
      } finally {
        engine.inFlight = false;
      }
      // Only paint if this is still the frame we want (drop stale results).
      if (engine.desired === target) {
        blit(bitmap);
        lastDrawnRef.current = target;
        bitmap?.close?.();
      } else {
        bitmap?.close?.();
        // Desired moved while we rendered — render the newest frame now.
        pumpRef.current();
      }
    })();
  };

  // Request frame `frameIndex` at `width`. Coalesces: updates the desired target and
  // kicks the pump (which renders it iff nothing is already in flight).
  const requestFrame = useCallback((frameIndex: number, width: number) => {
    const engine = engineRef.current;
    engine.desired = frameIndex;
    engine.desiredWidth = width;
    pumpRef.current();
  }, []);

  // Paint a single (paused) still at the crisper width. Used by seek/step/resize/init.
  const paintFrame = useCallback(
    (frameIndex: number) => {
      requestFrame(frameIndex, STILL_MAX_WIDTH);
    },
    [requestFrame],
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

  // ── Playback loop: wall-clock driven, frame-skipping, single-flight rendering. ──
  // Each rAF tick computes the TARGET frame from elapsed wall-clock time (not by
  // counting ticks), so playback runs at real-time speed regardless of render fps and
  // SKIPS frames the renderer can't keep up with. The render itself is coalesced by the
  // single-flight engine (`requestFrame`) — at most one in flight, intermediate frames
  // dropped — so a slow render slows the displayed frame rate but never the clock and
  // never backs up. The playhead/store advances per the computed target.
  useEffect(() => {
    if (!playing) return;
    const fps = Math.max(1, composition.fps);
    let raf = 0;
    let cancelled = false;
    const startTs = performance.now();
    const startFrame = store.getState().playheads[store.getState().activeTabId] ?? 0;
    const total = store.getState().durations[store.getState().activeTabId] ?? durationFrames;
    let lastTarget = -1;

    // Render the starting frame immediately on play (at the smaller playback width).
    requestFrame(startFrame, PLAYBACK_MAX_WIDTH);
    // Start AUDIO from the same frame — the cpal device clock is the smooth playback
    // clock the video loop stays roughly in sync with (both start at startFrame).
    void previewAudioPlay(startFrame);

    const tick = (ts: number) => {
      if (cancelled) return;
      // Target frame = start + elapsed-seconds * fps. Skips frames automatically.
      const elapsedSec = (ts - startTs) / 1000;
      let target = startFrame + Math.floor(elapsedSec * fps);

      if (total > 0 && target >= total) {
        // Stop at the end (no loop — matches the reference's play-to-end-then-pause).
        target = total;
        store.setActivePlayhead(target);
        onPlayheadChange?.(target);
        requestFrame(target, STILL_MAX_WIDTH);
        // Stop audio at the timeline end (releases the device until next play).
        void previewAudioStop();
        store.setPlaying(false);
        return;
      }
      if (target !== lastTarget) {
        lastTarget = target;
        store.setActivePlayhead(target);
        onPlayheadChange?.(target);
        // Coalesced render: drops intermediate frames if the renderer lags.
        requestFrame(target, PLAYBACK_MAX_WIDTH);
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      // Leaving the play state (pause toggle / unmount / dep change) → pause audio.
      // (The play-to-end path above already stopped; pause here is idempotent.)
      void previewAudioPause();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing, composition.fps, durationFrames, store, requestFrame, onPlayheadChange]);

  // ── Stop audio + release the device when the panel unmounts. ──────────────────
  useEffect(() => {
    return () => {
      void previewAudioStop();
    };
  }, []);

  // ── Transport actions (local playhead + canvas render; no engine transport). ──
  const seek = useCallback(
    (target: number) => {
      const dur = store.getState().durations[store.getState().activeTabId] ?? durationFrames;
      const f = Math.max(0, dur > 0 ? Math.min(target, dur) : target);
      store.setActivePlayhead(f);
      onPlayheadChange?.(f);
      void paintFrame(f);
      // Keep audio aligned to the new playhead (cheap cursor move; no re-decode). This
      // covers scrub-end, step (J/L, ←/→), and Home/End seeks.
      void previewAudioSeek(f);
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
