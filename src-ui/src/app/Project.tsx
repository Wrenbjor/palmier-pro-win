// Project window shell (E1-S4 + M2 boot integration): the per-project editor window
// chrome (1600×1000 / min 960×600). The timeline/editor canvas itself is owned by
// another worker under `src-ui/src/editor/` — this shell provides the window frame, the
// update badge, the editor mount point, AND the right-side agent dock.
//
// M2 boot integration mounts `<AgentPanel />` into the editor shell as the agent dock,
// wired to the real Tauri agent command/event surface (agent_send/agent_cancel +
// agent://event + agent_status). The panel self-creates its store/controller; here we
// seed its backend status from `agent_status` on mount and re-seed on the
// `anthropic-api-key-changed` event so the send-gate + model picker reflect the live
// keyring/account state.
import { useEffect, useMemo } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import UpdateBadge from "./UpdateBadge";
import { AgentPanel } from "../agent-panel";
import {
  AgentPanelController,
  createAgentPanelStore,
} from "../agent-panel";
import { TimelineEditor, createTimelineStore } from "../editor";
import { adaptTimeline, type WireTimeline } from "../editor/adapt";

export default function Project({ projectId }: { projectId: string }) {
  // Own the store + controller so we can seed the live backend status on mount. The
  // panel renders against these (no fixture seed — real `agent_status` populates it).
  const { store, controller } = useMemo(() => {
    const s = createAgentPanelStore();
    return { store: s, controller: new AgentPanelController(s) };
  }, []);

  // The timeline store — fed from the live backend via `editor_get_timeline`. Both the
  // agent (MCP) and the UI drive the same executor, so this reflects whatever the agent
  // does. Slice 1 is read-only display; it polls so edits from ANY source appear (a
  // `timeline://changed` event replaces the poll in a later slice).
  const timelineStore = useMemo(() => createTimelineStore(), []);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      try {
        const wire = await invoke<WireTimeline>("editor_get_timeline");
        if (!cancelled) timelineStore.setTimeline(adaptTimeline(wire));
      } catch (err) {
        // Outside a Tauri webview (plain `vite dev`) invoke is unavailable — ignore.
        console.debug("[editor] get_timeline skipped:", err);
      }
    };
    void load();
    const id = window.setInterval(load, 750);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [timelineStore]);

  useEffect(() => {
    // Seed the live backend status (BYOK key present / tier / models). No-op outside a
    // Tauri webview (plain `vite dev`), where the seeded/empty status stands in.
    void controller.refreshBackend();

    // Re-seed when the Anthropic key changes (agent-panel.md line 56) so the BYOK
    // send-gate + model picker update without a reload.
    let unlisten: UnlistenFn | undefined;
    listen("anthropic-api-key-changed", () => {
      void controller.refreshBackend();
    })
      .then((un) => {
        unlisten = un;
      })
      .catch((err) => {
        // Outside a Tauri webview the event API is unavailable — ignore.
        console.debug("[agent] key-changed listener skipped:", err);
      });
    return () => unlisten?.();
  }, [controller]);

  return (
    <div className="flex h-screen flex-col bg-[#0a0a0a] text-white">
      <header className="flex items-center justify-between border-b border-white/10 px-4 py-2">
        <span className="text-sm text-white/60">Project</span>
        <UpdateBadge />
      </header>
      <div className="flex flex-1 min-h-0">
        {/* Editor mount point — the live timeline canvas (slice 1: read-only display,
            fed from `editor_get_timeline`). Preview/media/inspector panels + click-edit
            land in later slices. */}
        <main className="flex flex-1 min-h-0 min-w-0" data-project-id={projectId}>
          <TimelineEditor store={timelineStore} style={{ flex: 1, minWidth: 0 }} />
        </main>
        {/* The agent dock — right-side AI chat panel, wired to the live agent backend. */}
        <AgentPanel store={store} controller={controller} seedFixture={false} />
      </div>
    </div>
  );
}
