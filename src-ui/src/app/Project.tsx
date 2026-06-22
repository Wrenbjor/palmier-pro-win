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
  useExport,
  writeClipboard,
  clipboardHasContent,
  pasteClipboard,
  PASTE_LIMITATIONS,
  type ToolMode,
} from "../editor";
import { adaptTimeline } from "../editor/adapt";
import { endFrame } from "../editor/geometry";
import type { ClipView, TimelineView } from "../editor/types";
import { getMedia, getTimeline, onTimelineChanged, editorEdit, importMedia } from "../editor/bridge";

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

/**
 * Per-dock visibility + a global "maximize" flag (View menu). `maximized` hides every
 * side panel regardless of its individual flag, giving the editor the full width; the
 * individual flags are preserved so un-maximizing restores the prior layout.
 */
interface PanelVisibility {
  media: boolean;
  inspector: boolean;
  agent: boolean;
  maximized: boolean;
}

/** Default layout — all docks visible (the original Project layout). */
const DEFAULT_PANELS: PanelVisibility = {
  media: true,
  inspector: true,
  agent: true,
  maximized: false,
};

/**
 * The three layout presets (View → Layout …). Each sets every dock at once so the
 * presets are distinct and predictable:
 *   - default:  all four docks (Media + Preview/Timeline + Inspector + Agent).
 *   - media:    editing-focused — Media + Inspector on, Agent hidden (more canvas).
 *   - vertical: distraction-free single-column — only the center editor column
 *               (all side docks hidden), suited to a portrait / vertical edit.
 */
const LAYOUT_PRESETS: Record<"default" | "media" | "vertical", PanelVisibility> = {
  default: { media: true, inspector: true, agent: true, maximized: false },
  media: { media: true, inspector: true, agent: false, maximized: false },
  vertical: { media: false, inspector: false, agent: false, maximized: false },
};

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

  // ── Panel visibility / layout state (View menu) ─────────────────────────────
  // The four docks are individually toggleable; a "maximize" mode hides every side
  // panel to give the editor/timeline the full width; layout presets set all four at
  // once. The center column (preview + timeline + toolbar) is always shown.
  const [panels, setPanels] = useState<PanelVisibility>(DEFAULT_PANELS);
  const showMedia = panels.media && !panels.maximized;
  const showInspector = panels.inspector && !panels.maximized;
  const showAgent = panels.agent && !panels.maximized;

  // Shared export controller — drives BOTH the Toolbar Export button and File → Export.
  const exportController = useExport();

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
  //   - new/open: spawn/open a project via the backend (create_project / open_project_dialog).
  //   - export: run the SAME export flow as the Toolbar's Export button (shared useExport).
  //   - cut/copy/paste: an in-memory clip clipboard (clipboard.ts) — copy captures full
  //     clip specs; cut = copy + deleteClips; paste recreates them at the playhead via
  //     add_clips + property/keyframe restore (limits documented in PASTE_LIMITATIONS).
  //   - view toggles / maximize / layout presets: panel-visibility state (above).
  //   - tutorial/about: open the Help window (open_help command).
  useEffect(() => {
    let unlisten: (() => void) | undefined;

    // Resolve a clip id → its track index in the live timeline (clipboard capture).
    const trackIndexOf = (tl: TimelineView, clipId: string): number => {
      for (let ti = 0; ti < tl.tracks.length; ti++) {
        if (tl.tracks[ti].clips.some((c) => c.id === clipId)) return ti;
      }
      return 0;
    };

    // Capture the current selection's full clip specs into the in-memory clipboard.
    // Returns the captured clip ids (used by Cut to then delete them).
    const copySelectionToClipboard = (): string[] => {
      const st = editor.store.getState();
      const tl = st.timeline;
      if (!tl) return [];
      const ids = [...st.viewport.selectedClipIds];
      const clips: ClipView[] = [];
      for (const id of ids) {
        const clip = findClipById(tl, id);
        if (clip) clips.push(clip);
      }
      if (clips.length === 0) return [];
      writeClipboard(clips, (cid) => trackIndexOf(tl, cid));
      return clips.map((c) => c.id);
    };

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
      // File → Import Media (Ctrl+I). No paths ⇒ Rust opens a native multi-select
      // file dialog and imports the chosen files through the shared executor; the
      // backend emits `timeline://changed` so the Media panel refetches automatically.
      "import-media": () => {
        void importMedia().catch((err) =>
          console.debug("[menu] editor_import_media failed:", err),
        );
      },
      // File → Save (Ctrl+S). Flushes the shared executor state to the bundle.
      save: () => {
        void invoke("save_project").catch((err) =>
          console.debug("[menu] save_project failed:", err),
        );
      },
      // File → Save As (Ctrl+Shift+S). Prompts a native Save dialog for a new
      // `.palmier` path, writes the LIVE state there, registers it, and makes it the
      // active project (subsequent saves target the new file). Cancel is a no-op.
      "save-as": () => {
        void invoke("save_project_as").catch((err) =>
          console.debug("[menu] save_project_as failed:", err),
        );
      },
      // File → New (Ctrl+N). Backend opens a Save dialog for a new `.palmier`, then
      // opens its Project window. Cancel is a no-op (returns None).
      new: () => {
        void invoke("create_project").catch((err) =>
          console.debug("[menu] create_project failed:", err),
        );
      },
      // File → Open (Ctrl+O). Backend opens a native open dialog and opens the chosen
      // project's window. Cancel is a no-op.
      open: () => {
        void invoke("open_project_dialog").catch((err) =>
          console.debug("[menu] open_project_dialog failed:", err),
        );
      },
      // File → Export (Ctrl+E). Runs the SAME flow as the Toolbar Export button
      // (shared `useExport` controller): native Save dialog → export_video → progress.
      export: () => {
        void exportController.runExport();
      },
      // Edit → Copy (Ctrl+C). Capture the selected clips' full specs into the clipboard.
      copy: () => {
        copySelectionToClipboard();
      },
      // Edit → Cut (Ctrl+X). Copy then delete the selection (non-ripple), as one motion.
      cut: () => {
        const captured = copySelectionToClipboard();
        if (captured.length === 0) return;
        editor.controller.dispatch({
          kind: "deleteClips",
          clipIds: captured,
          ripple: false,
        });
        editor.store.setSelection([]);
      },
      // Edit → Paste (Ctrl+V). Recreate the clipboard clips at the playhead via the
      // backend (add_clips + set_clip_properties/set_keyframes). Best-effort property
      // restore; PASTE_LIMITATIONS documents the fields the tools can't express.
      paste: () => {
        if (!clipboardHasContent()) return;
        const st = editor.store.getState();
        const tl = st.timeline;
        if (!tl) return;
        void pasteClipboard(tl, st.viewport.playheadFrame)
          .then((r) => {
            if (r.pasted > 0 && r.unrestored.length > 0) {
              console.debug(
                "[menu] paste: clips recreated; fields not restored:",
                r.unrestored.join("; "),
                "(see PASTE_LIMITATIONS:",
                PASTE_LIMITATIONS.join("; ") + ")",
              );
            }
          })
          .catch((err) => console.debug("[menu] paste failed:", err));
      },
      // View → Toggle Media / Inspector / Agent — flip that dock's visibility. While
      // maximized, un-maximize first so the toggle is visible (clears the override).
      "toggle-media-panel": () => {
        setPanels((p) => ({ ...p, maximized: false, media: !p.media }));
      },
      "toggle-inspector": () => {
        setPanels((p) => ({ ...p, maximized: false, inspector: !p.inspector }));
      },
      "toggle-agent-panel": () => {
        setPanels((p) => ({ ...p, maximized: false, agent: !p.agent }));
      },
      // View → Maximize Panel — toggle full-width editor (hide every side dock); the
      // individual flags are preserved so toggling back restores the prior layout.
      "maximize-panel": () => {
        setPanels((p) => ({ ...p, maximized: !p.maximized }));
      },
      // View → Layout Default / Media / Vertical — apply a preset to all docks at once.
      "layout-default": () => setPanels(LAYOUT_PRESETS.default),
      "layout-media": () => setPanels(LAYOUT_PRESETS.media),
      "layout-vertical": () => setPanels(LAYOUT_PRESETS.vertical),
      // Help → Tutorial / About — open the Help window (real window, never a dead click).
      tutorial: () => {
        void invoke("open_help").catch((err) =>
          console.debug("[menu] open_help failed:", err),
        );
      },
      about: () => {
        void invoke("open_help").catch((err) =>
          console.debug("[menu] open_help (about) failed:", err),
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
    // `setPanels` is stable; `exportController.runExport` is a stable useCallback, so a
    // single registration captures the live handlers without re-subscribing per render.
  }, [editor.store, editor.controller, exportController.runExport]);

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
    // Thread the REAL per-asset fields from the enriched `editor_get_media` payload
    // (carried on the media-panel `MediaAssetView`) into the inspector view, so the
    // Details (Source) tab shows actual Type / Dimensions / Duration / Size / Path
    // (+ the Generated section for AI assets) rather than placeholders.
    const mediaAssets: InspectorAssetView[] = mediaSnapshot.assets.map((a) => ({
      id: a.id,
      isVisual: a.type === "video" || a.type === "image",
      name: a.name,
      type: a.type,
      width: a.width,
      height: a.height,
      durationSeconds: a.durationSeconds,
      sizeBytes: a.sizeBytes,
      path: a.path || undefined,
      isGenerated: a.isGenerated,
      generatedModel: a.generatedModel,
      generatedAspect: a.generatedAspect,
      generatedResolution: a.generatedResolution,
      prompt: a.prompt,
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
        {/* Left dock — Media panel (toggled via View menu / layout presets). */}
        {showMedia && (
          <div className="w-[280px] flex-shrink-0 min-h-0">
            <MediaPanel
              store={media.store}
              controller={media.controller}
              seedFixture={false}
              fps={composition.fps}
            />
          </div>
        )}

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
              exportController={exportController}
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

        {/* Right rail — Inspector (toggled via View menu / layout presets). */}
        {showInspector && (
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
        )}

        {/* Far right — the agent dock (toggled via View menu / layout presets). */}
        {showAgent && (
          <AgentPanel
            store={agent.store}
            controller={agent.controller}
            seedFixture={false}
          />
        )}
      </div>
    </div>
  );
}
