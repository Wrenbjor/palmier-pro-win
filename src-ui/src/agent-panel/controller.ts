// AgentPanelController — the single command seam for the agent panel's side effects
// (E8-S8 + M2 boot integration). Mirrors the media-panel's `MediaPanelController`
// boundary convention.
//
// Two transports, picked at runtime:
//   - **Real (in a Tauri webview):** `send()` → `invoke('agent_send', …)` and the
//     `agent://event` subscription drives the message list; `cancel()` →
//     `invoke('agent_cancel', …)`; the model preference persists via
//     `invoke('agent_set_pref', …)`; backend status seeds from `invoke('agent_status')`.
//     Tool dispatch happens backend-side in `palmier_tools::execute` over the SHARED
//     `EditorState` (the same one the MCP server drives) — the frontend never touches
//     HTTP / keyring / filesystem (PRD "Strict layering", FOUNDATION §4).
//   - **Mock (plain `vite dev` / tests, outside a Tauri webview):** the original
//     `MockAgentStream` drives a fake assistant turn so the panel animates with no
//     backend.
//
// The frontend models the loop's events exactly (agent-panel.md lines 33-38), so the
// real `agent://event` payload maps 1:1 onto the same `onStreamEvent` handler the mock
// feeds — swapping the transport is not a re-model.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { appendTextDelta, effectiveModel, isEmptyAssistantTurn, referencedMentions } from "./logic";
import {
  MOCK_TOOL_RESULT_TEXT,
  mockClosingTurn,
  mockToolTurn,
  runMockTurn,
  type AgentStreamEvent,
  type MockStreamHandle,
} from "./mock-stream";
import type { AgentPanelStore } from "./store";
import {
  MODEL_PREF_KEY,
  NEW_CHAT_TITLE,
  type AgentContentBlock,
  type AgentMention,
  type AgentMessage,
  type BackendStatus,
  type ChatSession,
} from "./types";

/** Generate a local UUID-ish id (used for optimistic user/assistant messages). */
export function localId(prefix = "id"): string {
  const rnd = Math.random().toString(36).slice(2, 10);
  return `${prefix}-${rnd}-${Date.now().toString(36)}`;
}

/** Whether we're running inside a Tauri webview (real agent backend available). */
function isTauri(): boolean {
  return typeof globalThis !== "undefined" && "__TAURI_INTERNALS__" in globalThis;
}

/**
 * The wire shape of a tab/history row from `agent_list_sessions` (the Rust
 * `SessionSummary`, camelCase). The backend owns the tab list, current/open state,
 * and persistence; this carries everything the panel needs to render a tab/history
 * row (the per-session messages arrive via `agent_get_session` on switch).
 */
type SessionSummaryWire = {
  id: string;
  title: string;
  /** Unix seconds (history sorts newest-first by this). */
  updatedAt: number;
  isOpen: boolean;
  isCurrent: boolean;
  messageCount: number;
};

/** The wire shape of an `agent://event` payload from `palmier-tauri`. */
type AgentBackendEvent =
  | { type: "text_delta"; sessionId: string; text: string }
  | { type: "tool_use_complete"; sessionId: string; id: string; name: string; inputJson: string }
  | { type: "tool_result"; sessionId: string; toolUseId: string; isError: boolean; text: string }
  | { type: "done"; sessionId: string; stopReason: string }
  | { type: "error"; sessionId: string; message: string };

export class AgentPanelController {
  private stream: MockStreamHandle | null = null;
  /** The assistant message id currently being streamed into (for delta routing). */
  private streamingAssistantId: string | null = null;
  /** Unsubscribe for the live `agent://event` listener (real transport). */
  private unlisten: (() => void) | null = null;
  /** The session id whose turn is in flight (real transport). */
  private streamingSessionId: string | null = null;

  constructor(private store: AgentPanelStore) {}

  // --- Composer ---------------------------------------------------------------

  setDraft(draft: string): void {
    this.store.setDraft(draft);
  }

  setMentions(mentions: AgentMention[]): void {
    this.store.setMentions(mentions);
  }

  // --- Model picker -----------------------------------------------------------

  /**
   * Persist the selected model. In a Tauri webview this calls
   * `agent_set_pref` (config key `"agentModel"`); always mirrors to localStorage as a
   * best-effort fallback for plain `vite dev`.
   */
  setModel(model: import("./types").AgentModelId): void {
    this.store.setPreferredModel(model);
    if (isTauri()) {
      void invoke("agent_set_pref", { model }).catch((err) =>
        console.debug("[agent] agent_set_pref failed:", err),
      );
    }
    try {
      globalThis.localStorage?.setItem(MODEL_PREF_KEY, model);
    } catch {
      /* no localStorage (SSR / sandbox) — ignore */
    }
  }

  /** The model a send would actually use (preference clamped to availability). */
  currentModel(): import("./types").AgentModelId {
    const s = this.store.getState();
    return effectiveModel(s.backend, s.preferredModel);
  }

  // --- Send / cancel (E8-S4 loop seam) ---------------------------------------

  /**
   * `send(text, mentions)` (agent-panel.md lines 98-99): trim, append the user message,
   * and kick off the streaming loop. Guarded on `canSend` by the UI; this also no-ops
   * on an empty draft / mid-stream as a safety net.
   *
   * Real transport: `invoke('agent_send', { sessionId, userText, mentions, model })`
   * then render the `agent://event` stream. The orphan-tool repair, 2-cache-breakpoint
   * body, and tool dispatch all happen backend-side in palmier-agent / palmier-tools.
   */
  send(): void {
    const s = this.store.getState();
    const text = s.draft.trim();
    if (text.length === 0 || s.isStreaming) return;

    const userMsg: AgentMessage = {
      id: localId("msg-u"),
      role: "user",
      blocks: [{ kind: "text", text }],
      mentions: s.mentions.length > 0 ? s.mentions.slice() : undefined,
    };
    this.store.appendMessage(userMsg);
    const referenced = referencedMentions(text, s.mentions);
    this.store.setDraft("");
    this.store.setMentions([]);
    this.store.setStreamError(null);

    if (isTauri()) {
      void this.kickOffRealStream(text, referenced);
    } else {
      this.kickOffStream(text);
    }
  }

  /** Send a starter prompt directly (empty-state click). */
  sendPrompt(prompt: string): void {
    this.store.setDraft(prompt);
    this.send();
  }

  /**
   * `cancel()` (agent-panel.md lines 109-110): cancel the in-flight stream and drop the
   * empty assistant turn (no half-written tool_use committed). Real transport calls
   * `agent_cancel`; the backend drops the empty assistant turn server-side.
   */
  cancel(): void {
    if (isTauri() && this.streamingSessionId) {
      const sessionId = this.streamingSessionId;
      void invoke("agent_cancel", { sessionId }).catch((err) =>
        console.debug("[agent] agent_cancel failed:", err),
      );
    }
    this.stream?.cancel();
    this.stream = null;
    this.dropEmptyAssistantTurn();
    this.endStream();
  }

  // --- Real Tauri transport (M2 boot integration) ----------------------------

  /**
   * Kick off a live agent turn via `agent_send` and subscribe to `agent://event`.
   * The backend run loop streams text deltas, tool activity, and a terminal `done`.
   */
  private async kickOffRealStream(
    text: string,
    mentions: AgentMention[],
  ): Promise<void> {
    const sessionId = this.store.getState().currentSessionId;
    this.streamingSessionId = sessionId;
    this.store.setStreaming(true);

    const assistant: AgentMessage = { id: localId("msg-a"), role: "assistant", blocks: [] };
    this.store.appendMessage(assistant);
    this.streamingAssistantId = assistant.id;

    try {
      // Subscribe BEFORE invoking so no early event is missed.
      this.unlisten = await listen<AgentBackendEvent>("agent://event", (e) => {
        if (e.payload.sessionId !== sessionId) return;
        this.onBackendEvent(e.payload);
      });

      await invoke("agent_send", {
        sessionId,
        userText: text,
        mentions: mentions.map((m) => m.displayName),
        model: this.currentModel(),
      });
    } catch (err) {
      this.store.setStreamError({ kind: "upstream", message: String(err) });
      this.endStream();
    }
  }

  /** Map a backend `agent://event` onto the panel's message list. */
  private onBackendEvent(event: AgentBackendEvent): void {
    const assistantId = this.streamingAssistantId;
    switch (event.type) {
      case "text_delta":
        if (assistantId)
          this.store.updateMessage(assistantId, (m) => appendTextDelta(m, event.text));
        break;
      case "tool_use_complete":
        if (assistantId)
          this.store.updateMessage(assistantId, (m) => ({
            ...m,
            blocks: [
              ...m.blocks,
              { kind: "toolUse", id: event.id, name: event.name, inputJson: event.inputJson },
            ],
          }));
        break;
      case "tool_result":
        // Render the tool result as a user message block (matching the loop's shape).
        this.store.appendMessage({
          id: localId("msg-u"),
          role: "user",
          blocks: [
            {
              kind: "toolResult",
              toolUseId: event.toolUseId,
              isError: event.isError,
              content: [{ kind: "text", text: event.text }],
            },
          ],
        });
        break;
      case "error":
        this.store.setStreamError({ kind: "upstream", message: event.message });
        break;
      case "done":
        this.endStream();
        break;
    }
  }

  // --- Streaming loop (MockAgentStream stand-in, non-Tauri) -------------------

  private kickOffStream(prompt: string): void {
    this.store.setStreaming(true);
    const assistant: AgentMessage = {
      id: localId("msg-a"),
      role: "assistant",
      blocks: [],
    };
    this.store.appendMessage(assistant);
    this.streamingAssistantId = assistant.id;

    this.stream = runMockTurn(
      mockToolTurn(prompt),
      (event) => this.onStreamEvent(event),
      (err) => {
        this.store.setStreamError(err);
        this.endStream();
      },
    );
  }

  /**
   * Handle one MOCK stream event (the exact three the real loop consumes,
   * agent-panel.md lines 33-38). On `message_stop(tool_use)` it dispatches the pending
   * tool (mock) and resumes; on any other stop it ends the turn.
   */
  private onStreamEvent(event: AgentStreamEvent): void {
    const assistantId = this.streamingAssistantId;
    if (!assistantId) return;

    switch (event.type) {
      case "text_delta":
        this.store.updateMessage(assistantId, (m) =>
          appendTextDelta(m, event.text),
        );
        break;
      case "tool_use_complete":
        this.store.updateMessage(assistantId, (m) => ({
          ...m,
          blocks: [
            ...m.blocks,
            {
              kind: "toolUse",
              id: event.id,
              name: event.name,
              inputJson: event.inputJson,
            },
          ],
        }));
        break;
      case "message_stop":
        if (event.stopReason === "tool_use") this.runPendingToolUses(assistantId);
        else this.endStream();
        break;
    }
  }

  /**
   * `runPendingToolUses` (agent-panel.md lines 111-114), MOCK transport only: collect
   * the assistant's `.toolUse` blocks, dispatch each (canned mock result), append ONE
   * user message of results, then resume the stream. (In the real transport this is
   * backend-side; the frontend just receives the follow-on event stream.)
   */
  private runPendingToolUses(assistantId: string): void {
    const s = this.store.getState();
    const assistant = s.messages.find((m) => m.id === assistantId);
    if (!assistant) {
      this.endStream();
      return;
    }
    const toolUses = assistant.blocks.filter((b) => b.kind === "toolUse");
    if (toolUses.length === 0) {
      this.endStream();
      return;
    }
    const resultMsg: AgentMessage = {
      id: localId("msg-u"),
      role: "user",
      blocks: toolUses.map((tu) => ({
        kind: "toolResult",
        toolUseId: tu.kind === "toolUse" ? tu.id : "",
        isError: false,
        content: [{ kind: "text", text: MOCK_TOOL_RESULT_TEXT }],
      })),
    };
    this.store.appendMessage(resultMsg);

    // Resume: stream a fresh assistant turn that ends the turn.
    const next: AgentMessage = {
      id: localId("msg-a"),
      role: "assistant",
      blocks: [],
    };
    this.store.appendMessage(next);
    this.streamingAssistantId = next.id;
    this.stream = runMockTurn(
      mockClosingTurn(),
      (event) => this.onStreamEvent(event),
      (err) => {
        this.store.setStreamError(err);
        this.endStream();
      },
    );
  }

  private endStream(): void {
    this.unlisten?.();
    this.unlisten = null;
    this.stream = null;
    this.streamingAssistantId = null;
    this.streamingSessionId = null;
    this.store.setStreaming(false);
    this.store.syncMessagesIntoCurrentSession();
  }

  /** Remove the in-flight assistant message if it has no content (cancellation path). */
  private dropEmptyAssistantTurn(): void {
    const id = this.streamingAssistantId;
    if (!id) return;
    const msg = this.store.getState().messages.find((m) => m.id === id);
    if (msg && isEmptyAssistantTurn(msg)) this.store.removeMessage(id);
  }

  // --- Sessions / tabs --------------------------------------------------------
  //
  // In a Tauri webview the BACKEND (`AgentState`) is the source of truth for the tab
  // list, the current/open state, and persistence to `<project>/chat/`. Each tab
  // operation invokes the matching Tauri command, then re-pulls `agent_list_sessions`
  // (+ `agent_get_session` for the now-current tab) to reconcile the store — so
  // opening/closing/switching/deleting round-trips to disk. Outside a webview (plain
  // `vite dev` / tests) it falls back to the self-contained local store.

  newChat(): void {
    this.cancelIfStreaming();
    if (isTauri()) {
      void invoke<string>("agent_new_session")
        .then(() => this.refreshSessions())
        .catch((err) => this.fallbackSessionOp("agent_new_session", err, () => this.store.newChat()));
    } else {
      this.store.newChat();
    }
  }

  selectSession(id: string): void {
    this.cancelIfStreaming();
    if (isTauri()) {
      void invoke("agent_open_session", { sessionId: id })
        .then(() => this.refreshSessions())
        .catch((err) =>
          this.fallbackSessionOp("agent_open_session", err, () => this.store.selectSession(id)),
        );
    } else {
      this.store.selectSession(id);
    }
  }

  closeTab(id: string): void {
    if (id === this.store.getState().currentSessionId) this.cancelIfStreaming();
    if (isTauri()) {
      void invoke("agent_close_session", { sessionId: id })
        .then(() => this.refreshSessions())
        .catch((err) =>
          this.fallbackSessionOp("agent_close_session", err, () => this.store.closeTab(id)),
        );
    } else {
      this.store.closeTab(id);
    }
  }

  deleteSession(id: string): void {
    if (id === this.store.getState().currentSessionId) this.cancelIfStreaming();
    if (isTauri()) {
      void invoke("agent_delete_session", { sessionId: id })
        .then(() => this.refreshSessions())
        .catch((err) =>
          this.fallbackSessionOp("agent_delete_session", err, () => this.store.deleteSession(id)),
        );
    } else {
      this.store.deleteSession(id);
    }
  }

  /**
   * Pull the authoritative session list from the backend (`agent_list_sessions`) and
   * reconcile it into the store: rebuild the `sessions` array from the summaries,
   * set `currentSessionId`, and load the now-current session's messages via
   * `agent_get_session`. No-op outside a Tauri webview. Call on mount (via `init`)
   * and after every tab operation.
   */
  async refreshSessions(): Promise<void> {
    if (!isTauri()) return;
    try {
      const summaries = await invoke<SessionSummaryWire[]>("agent_list_sessions");
      const currentId =
        summaries.find((s) => s.isCurrent)?.id ?? summaries[0]?.id ?? this.store.getState().currentSessionId;

      // Load the current session's messages so a switched-to / reopened tab renders
      // its conversation (the summary carries no messages).
      let currentMessages: AgentMessage[] = [];
      if (currentId) {
        try {
          const wire = await invoke<{ id: string; role: string; blocks: AgentContentBlock[] }[]>(
            "agent_get_session",
            { sessionId: currentId },
          );
          currentMessages = wire.map((m) => ({
            id: m.id,
            role: m.role === "assistant" ? "assistant" : "user",
            blocks: m.blocks,
          }));
        } catch (err) {
          console.debug("[agent] agent_get_session failed:", err);
        }
      }

      const sessions: ChatSession[] = summaries.map((s) => ({
        id: s.id,
        title: s.title || NEW_CHAT_TITLE,
        // Rust ships unix SECONDS; the store/history compares ISO strings.
        updatedAt: new Date(s.updatedAt * 1000).toISOString(),
        // Only the current session's messages are hydrated (history rows render from
        // their title/updatedAt; switching to one re-pulls its messages). A non-empty
        // messageCount keeps history sessions visible (history filters empty ones).
        messages:
          s.id === currentId
            ? currentMessages
            : s.messageCount > 0
              ? [{ id: `placeholder-${s.id}`, role: "user", blocks: [] }]
              : [],
        isOpen: s.isOpen || s.id === currentId,
      }));

      this.store.setState({
        sessions,
        currentSessionId: currentId,
        messages: currentMessages,
        draft: "",
        mentions: [],
        streamError: null,
        historyOpen: false,
      });
    } catch (err) {
      console.debug("[agent] agent_list_sessions failed:", err);
    }
  }

  /**
   * Initialize the panel against the backend (Tauri webview): seed the backend status
   * and the session list. Safe to call on mount; a no-op outside a webview.
   */
  async init(): Promise<void> {
    if (!isTauri()) return;
    await this.refreshBackend();
    await this.refreshSessions();
  }

  /** A backend session command failed — log it and fall back to the local store op. */
  private fallbackSessionOp(command: string, err: unknown, localOp: () => void): void {
    console.debug(`[agent] ${command} failed; using local store:`, err);
    localOp();
  }

  private cancelIfStreaming(): void {
    if (this.store.getState().isStreaming) this.cancel();
  }

  // --- Backend status (E8-S6 seam) -------------------------------------------

  /**
   * Seed the backend status (key present / signed-in / plan / catalog). Real transport
   * reads `agent_status`; otherwise sets the passed snapshot directly. Call
   * `refreshBackend()` on mount and on `anthropic-api-key-changed` to re-seed.
   */
  setBackend(status: BackendStatus): void {
    this.store.setBackend(status);
  }

  /**
   * Pull the live backend status from the Tauri command surface (M2 boot integration).
   * No-op outside a Tauri webview (the fixture/seeded status stands in).
   */
  async refreshBackend(): Promise<void> {
    if (!isTauri()) return;
    try {
      const status = await invoke<BackendStatus>("agent_status");
      this.store.setBackend(status);
    } catch (err) {
      console.debug("[agent] agent_status failed:", err);
    }
  }
}
