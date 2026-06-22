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

import { useEffect, useMemo, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

import { registerMenuHandlers } from "./menu-events";
import UpdateBadge from "./UpdateBadge";
import { AgentPanel, AgentPanelController, createAgentPanelStore } from "../agent-panel";

import {
  EditController,
  TimelineEditor,
  Toolbar,
  createTimelineStore,
  useTimelineStore,
  type ToolMode,
} from "../editor";
import { adaptTimeline } from "../editor/adapt";
import { endFrame } from "../editor/geometry";
import type { ClipView, TimelineView } from "../editor/types";
import { getMedia, getTimeline, onTimelineChanged, editorEdit } from "../editor/bridge";

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

/** Find a clip by id across all tracks (matches the Toolbar's `findClip` helper). */
function findClipById(timeline: TimelineView, id: string): ClipView | null {
  for (const track of timeline.tracks) {
    const c = track.clips.find((cc) => cc.id === id);
    if (c) return c;
  }
  return null;
}

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

  // ── Main-menu (`menu://<id>`) → editor wiring (E1-S3 consolidation) ─────────
  //
  // The native menu bar (menu.rs) registers Ctrl-accelerators that emit `menu://<id>`
  // events. On Windows the native accelerator can consume the keystroke before the
  // timeline canvas's own `onKeyDown` sees it, so the Edit-menu commands must reach the
  // `EditController` HERE — where the controller + live selection/playhead state live —
  // not in App.tsx (which has no controller). This is the SOLE project-surface
  // `registerMenuHandlers` call; App.tsx no longer registers for this surface, so the
  // `menu://` family is subscribed exactly once.
  //
  // Each handler reads live state via `editor.store.getState()` (not a captured slice)
  // so the menu acts on the CURRENT selection/playhead even though the effect runs once.
  // Semantics mirror the Toolbar / TimelineEditor 1:1 so menu + toolbar + keyboard agree:
  //   - split: split each SELECTED clip the playhead bisects (Toolbar `splitAtPlayhead`).
  //   - trim-start/end: trim the dragged edge to the playhead on each bisected selected
  //     clip (Toolbar `trimStartToPlayhead` / `trimEndToPlayhead`, propagateToLinked).
  //   - delete: non-ripple `deleteClips` of the selection (TimelineEditor Delete key).
  //   - select-all: select every clip id across all tracks.
  //   - undo/redo: route to the controller (same path the Toolbar buttons use).
  //   - save: persist via the backend (flushes the shared executor state to the bundle).
  // cut/copy/paste stay logged no-ops: the editor has no clip-clipboard feature yet, so
  // there is nothing to wire (a clipboard is a separate story, not invented here).
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    const bisectedSelected = (): {
      timeline: TimelineView;
      at: number;
      clipIds: string[];
    } | null => {
      const st = editor.store.getState();
      const tl = st.timeline;
      if (!tl) return null;
      const at = st.viewport.playheadFrame;
      const ids: string[] = [];
      for (const id of st.viewport.selectedClipIds) {
        for (const track of tl.tracks) {
          const clip = track.clips.find((c) => c.id === id);
          if (clip && at > clip.startFrame && at < endFrame(clip)) ids.push(id);
        }
      }
      return { timeline: tl, at, clipIds: ids };
    };

    registerMenuHandlers({
      undo: () => {
        editor.controller.undo("user");
      },
      redo: () => {
        editor.controller.redo("user");
      },
      split: () => {
        const b = bisectedSelected();
        if (!b) return;
        for (const id of b.clipIds) {
          editor.controller.dispatch({ kind: "split", clipId: id, atFrame: b.at });
        }
      },
      "trim-start": () => {
        const b = bisectedSelected();
        if (!b) return;
        for (const id of b.clipIds) {
          const clip = findClipById(b.timeline, id);
          if (!clip) continue;
          editor.controller.dispatch({
            kind: "trim",
            clipId: id,
            edge: "left",
            deltaFrames: b.at - clip.startFrame,
            propagateToLinked: true,
          });
        }
      },
      "trim-end": () => {
        const b = bisectedSelected();
        if (!b) return;
        for (const id of b.clipIds) {
          const clip = findClipById(b.timeline, id);
          if (!clip) continue;
          editor.controller.dispatch({
            kind: "trim",
            clipId: id,
            edge: "right",
            deltaFrames: b.at - endFrame(clip),
            propagateToLinked: true,
          });
        }
      },
      delete: () => {
        const st = editor.store.getState();
        const sel = [...st.viewport.selectedClipIds];
        if (sel.length === 0) return;
        editor.controller.dispatch({ kind: "deleteClips", clipIds: sel, ripple: false });
        editor.store.setSelection([]);
      },
      "select-all": () => {
        const tl = editor.store.getState().timeline;
        if (!tl) return;
        const all: string[] = [];
        for (const track of tl.tracks) for (const c of track.clips) all.push(c.id);
        editor.store.setSelection(all);
      },
      // File → Save (Ctrl+S). Save As is a follow-up (needs a path dialog).
      save: () => {
        void invoke("save_project").catch((err) =>
          console.debug("[menu] save_project failed:", err),
        );
      },
    })
      .then((un) => {
        unlisten = un;
      })
      .catch((err) => {
        // Outside a Tauri webview (plain `vite dev`) the event API is unavailable.
        console.debug("[menu] handler registration skipped:", err);
      });

    return () => unlisten?.();
  }, [editor.store, editor.controller]);

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

  // Total timeline length in frames (max clip end over all tracks) — drives the preview
  // scrub bar range + the playback stop-at-end. Mirrors `Timeline::total_frames`.
  const totalFrames = useMemo(() => {
    if (!timeline) return 0;
    let max = 0;
    for (const track of timeline.tracks) {
      for (const clip of track.clips) {
        max = Math.max(max, clip.startFrame + clip.durationFrames);
      }
    }
    return max;
  }, [timeline]);

  // A revision counter bumped whenever the timeline content changes, so the preview
  // recomposites the CURRENT frame even when the playhead didn't move (e.g. a clip was
  // added/trimmed at the playhead). The serialized timeline is a cheap structural key.
  const revision = useMemo(
    () => (timeline ? JSON.stringify(timeline).length + timeline.tracks.length : 0),
    [timeline],
  );

  // Keep the preview store's active-tab duration in sync (the scrub bar reads it).
  useEffect(() => {
    preview.setDuration(preview.getState().activeTabId, totalFrames);
  }, [preview, totalFrames]);

  // Mirror the timeline playhead into the preview store so the scrub bar follows
  // timeline seeks (timeline → preview sync). The preview drives back via
  // `onPlayheadChange` (preview → timeline) during playback/seek.
  useEffect(() => {
    preview.setActivePlayhead(playheadFrame);
  }, [preview, playheadFrame]);

  // ── Tool mode — owned here so the Toolbar (E12-S9) and the TimelineEditor's V/C
  //    keyboard shortcuts stay in sync (controlled `tool` prop). ────────────────
  const [tool, setTool] = useState<ToolMode>("pointer");

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
              durationFrames={totalFrames}
              revision={revision}
              onPlayheadChange={(frame) => editor.store.setPlayhead(frame)}
              store={preview}
            />
          </div>
          <div className="h-[358px] flex-shrink-0 border-t border-white/10 flex flex-col min-h-0">
            {/* Editor toolbar (E12-S9) — above the timeline, drives tool/zoom/edits. */}
            <Toolbar
              store={editor.store}
              controller={editor.controller}
              tool={tool}
              onToolChange={setTool}
            />
            <div
              className="flex-1 min-h-0 relative"
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
                tool={tool}
                onToolChange={setTool}
                onSeek={(frame) => editor.store.setPlayhead(frame)}
              />
            </div>
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
