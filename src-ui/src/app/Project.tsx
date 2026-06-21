// Project window — the functional NLE editor (the Project surface).
//
// Composes the four panels into the editor layout and wires them to the backend over
// ONE shared `EditorState` (the `Arc<ToolExecutor>` the MCP server + in-app agent also
// drive — crates/palmier-tauri/src/agent.rs):
//
//   ┌──────────┬──────────────────────────┬───────────┬────────┐
//   │  Media   │        Preview           │ Inspector │ Agent  │
//   │  panel   │      (top-center)        │  (right)  │  dock  │
//   │  (left)  ├──────────────────────────┤           │ (far   │
//   │          │       Timeline           │           │ right) │
//   │          │       (bottom)           │           │        │
//   └──────────┴──────────────────────────┴───────────┴────────┘
//
// Data flow (no polling — FOUNDATION §4 strict layering):
//   - mount: load `editor_get_timeline` + `editor_get_media` → adapt → stores.
//   - mutate: panel controllers dispatch tools via `editor_edit` (bridge.ts); the
//     backend emits `timeline://changed`; this surface refetches both reads and
//     re-seeds the stores. AGENT edits emit the same event, so they update the UI too.
//   - selection + playhead are shared store state, synced timeline ↔ inspector ↔ preview.
//   - media→timeline drag = `add_clips` at the playhead (the drop zone below the canvas).

import { useEffect, useMemo, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import UpdateBadge from "./UpdateBadge";
import { AgentPanel, AgentPanelController, createAgentPanelStore } from "../agent-panel";

import {
  EditController,
  TimelineEditor,
  createTimelineStore,
  useTimelineStore,
} from "../editor";
import { adaptTimeline } from "../editor/adapt";
import { getMedia, getTimeline, onTimelineChanged, editorEdit, inTauri } from "../editor/bridge";

import {
  MediaPanel,
  MediaPanelController,
  createMediaPanelStore,
  useMediaStore,
} from "../media-panel";
import { adaptMedia } from "../media-panel/adapt";
import { parseUri } from "../media-panel/drag";

import { PreviewPanel, createPreviewStore } from "../preview";

import {
  InspectorPanel,
  InspectorController,
  type InspectorInput,
  type MediaAssetView as InspectorAssetView,
} from "../editor/inspector";
import { makeTabBodies, makeAssetBody } from "../editor/inspector/tabBodies";

export default function Project({ projectId }: { projectId: string }) {
  // ── Shared stores + controllers (created once) ──────────────────────────────
  const editor = useMemo(() => {
    const store = createTimelineStore();
    return { store, controller: new EditController(store) };
  }, []);
  const media = useMemo(() => {
    const store = createMediaPanelStore();
    return { store, controller: new MediaPanelController(store) };
  }, []);
  const preview = useMemo(() => createPreviewStore(), []);
  const inspectorController = useMemo(() => new InspectorController(), []);
  const agent = useMemo(() => {
    const s = createAgentPanelStore();
    return { store: s, controller: new AgentPanelController(s) };
  }, []);

  // ── Initial load + `timeline://changed` refetch (replaces the 750ms poll) ────
  useEffect(() => {
    let disposed = false;

    const refetch = async () => {
      const [tl, md] = await Promise.all([getTimeline(), getMedia()]);
      if (disposed) return;
      if (tl !== undefined) editor.store.setTimeline(adaptTimeline(tl));
      if (md !== undefined) media.store.setSnapshot(adaptMedia(md));
    };

    void refetch();

    let unlisten: UnlistenFn | undefined;
    onTimelineChanged(() => {
      void refetch();
    })
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      })
      .catch((err) => {
        // Outside a Tauri webview the event API is unavailable — the fixture stands in.
        console.debug("[project] timeline://changed listener skipped:", err);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [editor.store, media.store]);

  // ── Agent backend status seed (preserved from the prior shell) ──────────────
  useEffect(() => {
    void agent.controller.refreshBackend();
    let unlisten: UnlistenFn | undefined;
    listen("anthropic-api-key-changed", () => {
      void agent.controller.refreshBackend();
    })
      .then((un) => {
        unlisten = un;
      })
      .catch((err) => {
        console.debug("[agent] key-changed listener skipped:", err);
      });
    return () => unlisten?.();
  }, [agent.controller]);

  // ── Inspector account seam (re-gates the AI Edit tab on sign-in/key change) ──
  useEffect(() => {
    void inspectorController.refresh();
    return () => inspectorController.dispose();
  }, [inspectorController]);

  // ── Reactive slices for selection / playhead sync + inspector input ─────────
  const timeline = useTimelineStore(editor.store, (s) => s.timeline);
  const selectedClipIds = useTimelineStore(
    editor.store,
    (s) => s.viewport.selectedClipIds,
  );
  const playheadFrame = useTimelineStore(
    editor.store,
    (s) => s.viewport.playheadFrame,
  );
  const isMarqueeSelecting = useTimelineStore(
    editor.store,
    (s) => !!s.viewport.rangeSelection,
  );
  const mediaSnapshot = useMediaStore(media.store, (s) => s.snapshot);
  const mediaSelection = useMediaStore(media.store, (s) => s.selection);

  // The inspector resolves over the live timeline + both selections + media library.
  const inspectorInput: Omit<InspectorInput, "account"> = useMemo(() => {
    const mediaAssets: InspectorAssetView[] = mediaSnapshot.assets.map((a) => ({
      id: a.id,
      isVisual: a.type === "video" || a.type === "image",
    }));
    return {
      timeline,
      selectedClipIds,
      selectedMediaAssetIds: new Set(mediaSelection),
      mediaAssets,
      isMarqueeSelecting,
      projectPath: null,
    };
  }, [
    timeline,
    selectedClipIds,
    mediaSelection,
    mediaSnapshot.assets,
    isMarqueeSelecting,
  ]);

  // ── Preview composition geometry tracks the live timeline ───────────────────
  const composition = useMemo(
    () => ({
      width: timeline?.width ?? 1920,
      height: timeline?.height ?? 1080,
      fps: timeline?.fps ?? 30,
    }),
    [timeline?.width, timeline?.height, timeline?.fps],
  );

  // Push the timeline to the preview engine when it changes (the engine rebuilds its
  // source resolver). No-op outside Tauri (preview/api.ts degrades gracefully).
  // The serialized view-model is what `preview_set_timeline` accepts today.
  const lastPushedRef = useRef<string>("");
  useEffect(() => {
    if (!timeline || !inTauri()) return;
    const serialized = JSON.stringify(timeline);
    if (serialized === lastPushedRef.current) return;
    lastPushedRef.current = serialized;
    void import("../preview/api").then((api) =>
      api.previewSetTimeline(timeline),
    );
  }, [timeline]);

  // Mirror the timeline playhead into the preview store so the scrub bar follows
  // timeline seeks (timeline ↔ preview sync).
  useEffect(() => {
    preview.setActivePlayhead(playheadFrame);
  }, [preview, playheadFrame]);

  // ── media → timeline drop = add_clips at the playhead ───────────────────────
  const [dropActive, setDropActive] = useState(false);
  const onTimelineDrop = async (e: React.DragEvent) => {
    e.preventDefault();
    setDropActive(false);
    const payload = e.dataTransfer.getData("text/plain");
    if (!payload) return;
    // First asset / moment line drives the placement (multi-drop appends sequentially
    // backend-side via overwrite semantics; here we place each at the playhead onward).
    const entries: { mediaRef: string; startFrame: number; durationFrames: number }[] =
      [];
    let cursor = editor.store.getState().viewport.playheadFrame;
    for (const line of payload.split("\n")) {
      const parsed = parseUri(line);
      if (!parsed) continue;
      if (parsed.kind === "folder") continue;
      const asset = mediaSnapshot.assets.find((a) => a.id === parsed.id);
      // Duration: the asset's seconds × fps, or a default 1s when unknown (stills/text).
      const fps = composition.fps;
      const durationFrames =
        asset && asset.durationSeconds != null
          ? Math.max(1, Math.round(asset.durationSeconds * fps))
          : fps;
      entries.push({ mediaRef: parsed.id, startFrame: cursor, durationFrames });
      cursor += durationFrames;
    }
    if (entries.length === 0) return;
    // Omit trackIndex on every entry → the tool auto-creates one shared track per zone.
    const res = await editorEdit("add_clips", { entries });
    if (!res.ok) {
      console.error("[project] add_clips failed:", res.error);
    }
    // The backend emits `timeline://changed` → the refetch updates the timeline.
  };

  return (
    <div className="flex h-screen flex-col bg-[#0a0a0a] text-white">
      <header className="flex items-center justify-between border-b border-white/10 px-4 py-2">
        <span className="text-sm text-white/60" data-project-id={projectId}>
          Project
        </span>
        <UpdateBadge />
      </header>

      <div className="flex flex-1 min-h-0">
        {/* Left dock — Media panel (shares the editor's fps for moment→frame math). */}
        <div className="w-[280px] flex-shrink-0 min-h-0">
          <MediaPanel
            store={media.store}
            controller={media.controller}
            seedFixture={false}
            fps={composition.fps}
          />
        </div>

        {/* Center column — Preview (top) over Timeline (bottom). */}
        <div className="flex flex-1 flex-col min-w-0 min-h-0">
          <div className="flex-1 min-h-0">
            <PreviewPanel
              composition={composition}
              durationFrames={0}
              store={preview}
            />
          </div>
          <div
            className="h-[320px] flex-shrink-0 border-t border-white/10 relative"
            style={{ outline: dropActive ? "2px solid #F29933" : "none" }}
            onDragOver={(e) => {
              e.preventDefault();
              if (!dropActive) setDropActive(true);
            }}
            onDragLeave={() => setDropActive(false)}
            onDrop={(e) => {
              void onTimelineDrop(e);
            }}
          >
            <TimelineEditor
              store={editor.store}
              controller={editor.controller}
              onSeek={(frame) => editor.store.setPlayhead(frame)}
            />
          </div>
        </div>

        {/* Right rail — Inspector. */}
        <div className="w-[300px] flex-shrink-0 min-h-0 border-l border-white/10">
          <InspectorPanel
            input={inspectorInput}
            controller={inspectorController}
            tabBodies={makeTabBodies({
              activeFrame: playheadFrame,
              onSeek: (frame) => editor.store.setPlayhead(frame),
            })}
            assetBody={makeAssetBody()}
          />
        </div>

        {/* Far right — the agent dock (wired to the live agent backend). */}
        <AgentPanel
          store={agent.store}
          controller={agent.controller}
          seedFixture={false}
        />
      </div>
    </div>
  );
}
