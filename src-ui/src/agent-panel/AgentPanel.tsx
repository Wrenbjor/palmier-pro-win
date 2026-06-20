// AgentPanel — the collapsible right-side agent chat panel (docs/reference/
// agent-panel.md "collapsible right-side panel"). Composes the floating tab bar, the
// scrolling message list (auto-scroll + jump-to-bottom), and the bottom composer
// (multi-line editor, @mention picker, send/cancel, model picker, API-key indicator).
// The app shell mounts `<AgentPanel />`.
//
// Self-contained: if no `store`/`controller` prop is passed it creates its own from the
// fixtures, so the panel is mountable in isolation (and in `vite dev` outside a Tauri
// webview). A `MockAgentStream` (via the controller) drives a fake assistant turn so it
// animates without a backend. The real stream/dispatch arrive via Tauri agent
// commands/events (TODO(integration) — the run loop is palmier-agent E8-S4).

import { useEffect, useMemo, useRef } from "react";
import type { CSSProperties } from "react";
import { AgentPanelController } from "./controller";
import {
  availableModels as deriveAvailable,
  canStream as deriveCanStream,
  effectiveModel,
} from "./logic";
import { InputArea } from "./InputArea";
import { MessageList } from "./MessageList";
import { TabBar } from "./TabBar";
import { Spacing, Theme } from "./theme";
import {
  makeFixtureBackend,
  makeFixtureMentions,
  makeFixtureSessions,
} from "./fixture";
import {
  createAgentPanelStore,
  historySessions,
  openSessions,
  useAgentStore,
  type AgentPanelStore,
} from "./store";
import type { AgentMention } from "./types";

export interface AgentPanelProps {
  /** Inject a store (e.g. shared with the editor); otherwise one is created. */
  store?: AgentPanelStore;
  /** Inject a controller; otherwise one is created bound to the store. */
  controller?: AgentPanelController;
  /**
   * Seed from the fixtures (backend = BYOK, sample sessions + mention candidates) when
   * self-creating a store (default true). Set false to start empty (the real
   * `agent_status` / `load_sessions` commands populate it at integration).
   */
  seedFixture?: boolean;
  /** Mention candidates (fixture project state today; real project snapshot later). */
  mentionCandidates?: AgentMention[];
  /** Panel width when expanded (px). */
  width?: number;
}

export function AgentPanel({
  store,
  controller,
  seedFixture = true,
  mentionCandidates,
  width = 380,
}: AgentPanelProps) {
  const owned = useRef<{
    store: AgentPanelStore;
    controller: AgentPanelController;
  } | null>(null);

  const { store: theStore, controller: theController } = useMemo(() => {
    if (store && controller) return { store, controller };
    if (store && !controller)
      return { store, controller: new AgentPanelController(store) };
    if (!owned.current) {
      const s = createAgentPanelStore(
        seedFixture
          ? {
              backend: makeFixtureBackend(),
              sessions: makeFixtureSessions(),
            }
          : undefined,
      );
      owned.current = { store: s, controller: new AgentPanelController(s) };
    }
    return owned.current;
  }, [store, controller, seedFixture]);

  const candidates = useMemo(
    () => mentionCandidates ?? (seedFixture ? makeFixtureMentions() : []),
    [mentionCandidates, seedFixture],
  );

  // In a Tauri webview, seed the backend status + session list (tab bar / history)
  // from the backend on mount — the backend is the source of truth for sessions and
  // their persistence. A no-op outside a webview (the fixtures / local store stand in).
  useEffect(() => {
    void theController.init();
  }, [theController]);

  const collapsed = useAgentStore(theStore, (s) => s.collapsed);
  const messages = useAgentStore(theStore, (s) => s.messages);
  const draft = useAgentStore(theStore, (s) => s.draft);
  const mentions = useAgentStore(theStore, (s) => s.mentions);
  const isStreaming = useAgentStore(theStore, (s) => s.isStreaming);
  const streamError = useAgentStore(theStore, (s) => s.streamError);
  const backend = useAgentStore(theStore, (s) => s.backend);
  const preferredModel = useAgentStore(theStore, (s) => s.preferredModel);
  const sessions = useAgentStore(theStore, (s) => s.sessions);
  const currentSessionId = useAgentStore(theStore, (s) => s.currentSessionId);
  const historyOpen = useAgentStore(theStore, (s) => s.historyOpen);

  const available = useMemo(() => deriveAvailable(backend), [backend]);
  const model = useMemo(
    () => effectiveModel(backend, preferredModel),
    [backend, preferredModel],
  );
  const canStream = deriveCanStream(backend);

  // `sessions` (subscribed above) drives the open-tab / history derivations and the
  // re-render when tabs change.
  const openTabs = useMemo(() => openSessions(sessions), [sessions]);
  const history = useMemo(() => historySessions(sessions), [sessions]);

  if (collapsed) {
    return (
      <div style={collapsedStyle}>
        <button
          type="button"
          onClick={() => theStore.toggleCollapsed()}
          title="Open agent"
          aria-label="Open agent chat"
          style={collapsedButtonStyle}
        >
          💬
        </button>
      </div>
    );
  }

  return (
    <div style={{ ...panelStyle, width }}>
      <style>{KEYFRAMES}</style>

      <div style={headerStyle}>
        <span style={{ fontSize: 12, fontWeight: 600, color: Theme.text.secondary }}>
          Agent
        </span>
        <button
          type="button"
          onClick={() => theStore.toggleCollapsed()}
          title="Collapse panel"
          aria-label="Collapse agent panel"
          style={collapseToggleStyle}
        >
          ⟩
        </button>
      </div>

      <TabBar
        openTabs={openTabs}
        history={history}
        currentSessionId={currentSessionId}
        historyOpen={historyOpen}
        onSelect={(id) => theController.selectSession(id)}
        onClose={(id) => theController.closeTab(id)}
        onNewChat={() => theController.newChat()}
        onToggleHistory={() => theStore.setHistoryOpen(!historyOpen)}
        onDelete={(id) => theController.deleteSession(id)}
      />

      <MessageList
        messages={messages}
        isStreaming={isStreaming}
        streamError={streamError}
        canStream={canStream}
        onStarter={(prompt) => theController.sendPrompt(prompt)}
      />

      <InputArea
        draft={draft}
        mentions={mentions}
        candidates={candidates}
        backend={backend}
        model={model}
        availableModels={available}
        isStreaming={isStreaming}
        onDraftChange={(d) => theController.setDraft(d)}
        onMentionsChange={(m) => theController.setMentions(m)}
        onModelChange={(m) => theController.setModel(m)}
        onSend={() => theController.send()}
        onCancel={() => theController.cancel()}
      />
    </div>
  );
}

const KEYFRAMES = `@keyframes agentspin { to { transform: rotate(360deg); } }`;

const panelStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  height: "100%",
  minHeight: 0,
  background: Theme.background.base,
  color: Theme.text.primary,
  fontFamily:
    "-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif",
  borderLeft: `1px solid ${Theme.border.subtle}`,
};

const headerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  padding: `${Spacing.xs}px ${Spacing.sm}px`,
  borderBottom: `1px solid ${Theme.border.subtle}`,
};

const collapseToggleStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  color: Theme.text.muted,
  cursor: "pointer",
  fontSize: 14,
  padding: "2px 6px",
};

const collapsedStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  padding: Spacing.xs,
  background: Theme.background.surface,
  borderLeft: `1px solid ${Theme.border.subtle}`,
  height: "100%",
};

const collapsedButtonStyle: CSSProperties = {
  width: 36,
  height: 36,
  borderRadius: 8,
  border: `1px solid ${Theme.border.subtle}`,
  background: Theme.background.raised,
  color: Theme.text.secondary,
  cursor: "pointer",
  fontSize: 16,
};
