// Preview panel (E5-S10) — the viewport that composes the whole preview surface.
//
// Port of the macOS reference `PreviewContainerView`: a tab bar on top, a transparent
// center viewport (the native wgpu surface from E5-S1 plan A1 shows THROUGH this region
// — the webview is transparent here so DWM composites the swapchain underneath), the
// transform/crop overlays floating over it, and the scrub + transport + settings bar
// at the bottom. Keyboard transport (J/K/L/Space/Home/End/←/→) and cmd/ctrl-scroll zoom
// are bound here.
//
// Strict layering (FOUNDATION §4): every transport/edit action calls a Tauri command
// into `palmier-engine`; the engine streams the reactive `current_frame` back as the
// `preview://current-frame` event, which this panel subscribes to and writes into the
// store. Outside Tauri (`vite dev`) the commands are no-ops and the playhead is driven
// locally so the panel still renders for design work.

import { useCallback, useEffect, useMemo, useRef } from "react";

import {
  inTauri,
  onCurrentFrame,
  onPlaybackState,
  previewApplyCrop,
  previewApplyTransform,
  previewInit,
  previewResize,
  previewSeek,
  previewSetTab,
  previewStep,
  previewTeardown,
  previewTogglePlayback,
  type SeekMode,
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

export interface PreviewPanelProps {
  /** The Tauri window label whose native surface this panel presents into. */
  windowLabel?: string;
  /** Composition geometry — timeline width/height/fps (drives aspect + timecode). */
  composition: { width: number; height: number; fps: number };
  /** Duration of the active tab in frames (for the scrub bar). */
  durationFrames?: number;
  /** The selected clip's transform/crop (the overlays manipulate these). */
  selected?: { clipId: string; transform: Transform; crop: Crop; mediaCanvasAspect?: number | null } | null;
  /** Whether the crop overlay is active (else the transform overlay). */
  cropEditing?: boolean;
  /** Apply new timeline settings (aspect/fps/quality menu). */
  onApplyTimelineSettings?: (settings: { fps: number; width: number; height: number }) => void;
  /** Optional shared store (else the panel owns one). */
  store?: PreviewStore;
}

export function PreviewPanel({
  windowLabel = "main",
  composition,
  durationFrames = 0,
  selected = null,
  cropEditing = false,
  onApplyTimelineSettings,
  store: providedStore,
}: PreviewPanelProps) {
  const store = useMemo(() => providedStore ?? createPreviewStore(), [providedStore]);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const sizeRef = useRef<{ width: number; height: number }>({ width: 0, height: 0 });

  const tabs = usePreviewStore(store, (s) => s.tabs);
  const activeTabId = usePreviewStore(store, (s) => s.activeTabId);
  const frame = usePreviewStore(store, (s) => s.playheads[s.activeTabId] ?? 0);
  const playing = usePreviewStore(store, (s) => s.playing);
  const zoom = usePreviewStore(store, (s) => s.canvasZoom);
  const offset = usePreviewStore(store, (s) => s.canvasOffset);

  const aspect = composition.height > 0 ? composition.width / composition.height : 16 / 9;

  // ── Surface lifecycle: init the wgpu present surface, track viewport resize. ──
  useEffect(() => {
    if (!inTauri()) return;
    void previewInit(windowLabel);
    return () => {
      void previewTeardown();
    };
  }, [windowLabel]);

  useEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0].contentRect;
      sizeRef.current = { width: r.width, height: r.height };
      void previewResize(Math.max(1, Math.round(r.width)), Math.max(1, Math.round(r.height)));
      // Force a re-render so overlays re-measure (store touch).
      store.setOffset({ ...store.getState().canvasOffset });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [store]);

  // ── Reactive playhead + playback-state from the engine. ──
  useEffect(() => {
    let unCurrent: (() => void) | undefined;
    let unPlay: (() => void) | undefined;
    onCurrentFrame((p) => {
      // Route to the timeline tab or the active asset tab.
      if (p.isTimeline) store.setPlayhead("__timeline__", p.frame);
      else store.setActivePlayhead(p.frame);
    }).then((un) => (unCurrent = un));
    onPlaybackState((pl) => store.setPlaying(pl)).then((un) => (unPlay = un));
    return () => {
      unCurrent?.();
      unPlay?.();
    };
  }, [store]);

  // ── Transport actions (Tauri commands; local fallback outside Tauri). ──
  const seek = useCallback(
    (target: number, mode: SeekMode) => {
      const f = Math.max(0, target);
      store.setActivePlayhead(f); // optimistic; engine echoes via event.
      void previewSeek(f, mode);
    },
    [store],
  );

  const togglePlay = useCallback(() => {
    const next = !store.getState().playing;
    store.setPlaying(next); // optimistic; engine echoes.
    void previewTogglePlayback();
  }, [store]);

  const step = useCallback(
    (delta: number) => {
      const f = Math.max(0, store.getState().playheads[store.getState().activeTabId] ?? 0) + delta;
      store.setActivePlayhead(Math.max(0, f));
      void previewStep(delta);
    },
    [store],
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
          seek(0, "exact");
          break;
        case "End":
          e.preventDefault();
          seek(dur, "exact");
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
      if (selected) void previewApplyTransform(selected.clipId, t);
    },
    [selected],
  );
  const commitTransform = useCallback(
    (t: Transform) => {
      if (selected) void previewApplyTransform(selected.clipId, t);
    },
    [selected],
  );
  const applyCrop = useCallback(
    (c: Crop) => {
      if (selected) void previewApplyCrop(selected.clipId, c);
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
        onSelect={(id) => {
          store.selectTab(id);
          void previewSetTab(id);
        }}
        onClose={(id) => store.closeTab(id)}
        onCloseAll={() => store.closeAllTabs()}
      />

      {/* Transparent viewport — the native wgpu surface shows through here. */}
      <div
        ref={viewportRef}
        className="relative flex-1 overflow-hidden"
        style={{ background: "transparent" }}
        onWheel={onWheel}
        data-preview-viewport
      >
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
              onCommit={commitTransform}
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
          onSeek={seek}
          onScrubBegin={() => store.setScrubbing(true)}
          onScrubEnd={(f) => {
            store.setScrubbing(false);
            seek(f, "exact");
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
