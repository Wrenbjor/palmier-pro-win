// AgentPanelController — the single command seam for the agent panel's side effects
// (E8-S8). Mirrors the media-panel's `MediaPanelController` boundary convention.
//
// Today every command runs against the local store + a `MockAgentStream` so the panel
// animates without a backend. When the real Tauri agent commands land they REPLACE the
// local apply:
//   - `send()`        → `await invoke('agent_send', { text, mentions })` then subscribe
//                        to the `agent://event` stream (text_delta / tool_use_complete /
//                        message_stop). The run loop + tool dispatch is palmier-agent
//                        E8-S4; the Tauri bridge is later.
//   - `cancel()`      → `await invoke('agent_cancel')`.
//   - tool dispatch   → handled backend-side by `palmier_tools::execute` (NOT here —
//                        the frontend never touches HTTP / keyring / filesystem; PRD
//                        cross-cutting "Strict layering", FOUNDATION §4).
//   - model/backend   → `await invoke('agent_status')` seeds `BackendStatus`; the
//                        selected model persists via `await invoke('set_pref', ...)`.
// Each such seam is marked `// TODO(integration): Tauri agent commands`.
//
// The frontend models the loop's three events exactly (agent-panel.md lines 33-38), so
// swapping the MockAgentStream for the real Tauri event subscription is a transport
// change, not a re-model.

import { appendTextDelta, effectiveModel, isEmptyAssistantTurn } from "./logic";
import {
  MOCK_TOOL_RESULT_TEXT,
  mockClosingTurn,
  mockToolTurn,
  runMockTurn,
  type AgentStreamEvent,
  type MockStreamHandle,
} from "./mock-stream";
import type { AgentPanelStore } from "./store";
import { MODEL_PREF_KEY, type AgentMention, type AgentMessage } from "./types";

/** Generate a local UUID-ish id (replaced by backend ids when commands land). */
export function localId(prefix = "id"): string {
  const rnd = Math.random().toString(36).slice(2, 10);
  return `${prefix}-${rnd}-${Date.now().toString(36)}`;
}

export class AgentPanelController {
  private stream: MockStreamHandle | null = null;
  /** The assistant message id currently being streamed into (for delta routing). */
  private streamingAssistantId: string | null = null;

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
   * Persist the selected model.
   * TODO(integration): Tauri agent commands — `await invoke('set_pref', { key:
   * "agentModel", value })`; today it persists to localStorage (best-effort).
   */
  setModel(model: import("./types").AgentModelId): void {
    this.store.setPreferredModel(model);
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
   * TODO(integration): Tauri agent commands — replace the MockAgentStream with
   *   await invoke('agent_send', { text, mentions: referencedMentions(...) });
   *   const unlisten = await listen('agent://event', (e) => this.onStreamEvent(e.payload));
   * The orphan-tool repair, 2-cache-breakpoint body, and tool dispatch all happen
   * backend-side in palmier-agent (E8-S4) — the frontend only renders the event stream.
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
    this.store.setDraft("");
    this.store.setMentions([]);
    this.store.setStreamError(null);
    this.kickOffStream(text);
  }

  /** Send a starter prompt directly (empty-state click). */
  sendPrompt(prompt: string): void {
    this.store.setDraft(prompt);
    this.send();
  }

  /**
   * `cancel()` (agent-panel.md lines 109-110): cancel the in-flight stream and drop the
   * empty assistant turn (no half-written tool_use committed).
   * TODO(integration): Tauri agent commands — `await invoke('agent_cancel')`.
   */
  cancel(): void {
    this.stream?.cancel();
    this.stream = null;
    this.dropEmptyAssistantTurn();
    this.streamingAssistantId = null;
    this.store.setStreaming(false);
    this.store.syncMessagesIntoCurrentSession();
  }

  // --- Streaming loop (MockAgentStream stand-in for palmier-agent E8-S4) ------

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
   * Handle one stream event — the exact three the real loop consumes (agent-panel.md
   * lines 33-38). On `message_stop(tool_use)` it dispatches the pending tool (mock) and
   * resumes; on any other stop it ends the turn.
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
   * `runPendingToolUses` (agent-panel.md lines 111-114): collect the assistant's
   * `.toolUse` blocks, dispatch each (here: a canned mock result; real:
   * `palmier_tools::execute` backend-side), append ONE user message of results, then
   * resume the stream.
   * TODO(integration): Tauri agent commands — tool dispatch is backend-side; the
   * frontend just receives the follow-on event stream.
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
    this.stream = null;
    this.streamingAssistantId = null;
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

  newChat(): void {
    this.cancelIfStreaming();
    this.store.newChat();
  }

  selectSession(id: string): void {
    this.cancelIfStreaming();
    this.store.selectSession(id);
  }

  closeTab(id: string): void {
    if (id === this.store.getState().currentSessionId) this.cancelIfStreaming();
    this.store.closeTab(id);
  }

  deleteSession(id: string): void {
    if (id === this.store.getState().currentSessionId) this.cancelIfStreaming();
    this.store.deleteSession(id);
  }

  private cancelIfStreaming(): void {
    if (this.store.getState().isStreaming) this.cancel();
  }

  // --- Backend status (E8-S6 seam) -------------------------------------------

  /**
   * Seed the backend status (key present / signed-in / plan / catalog).
   * TODO(integration): Tauri agent commands — `this.store.setBackend(await
   * invoke('agent_status'))`; a `.anthropicAPIKeyChanged` event re-seeds it
   * (agent-panel.md line 56).
   */
  setBackend(status: import("./types").BackendStatus): void {
    this.store.setBackend(status);
  }
}
