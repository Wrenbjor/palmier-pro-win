// MockAgentStream — a fake assistant turn so the panel animates without a backend.
//
// It emits the SAME three event kinds the real loop consumes (agent-panel.md lines
// 33-38): `text_delta`, `tool_use_complete`, `message_stop`. The shape mirrors the
// reference `AnthropicStreamEvent` so swapping in the real Tauri event stream
// (TODO(integration)) is a transport change, not a re-model.
//
// One mock turn streams: a sentence of text (char chunks), then a `get_timeline`
// tool_use, then a `message_stop(tool_use)` — at which point the controller dispatches
// the (mock) tool, appends a tool_result, and resumes with a closing sentence ending
// in `message_stop(end_turn)`.

import type { AgentStreamError } from "./types";

/** The three events that reach the loop (agent-panel.md lines 33-38). */
export type AgentStreamEvent =
  | { type: "text_delta"; text: string }
  | { type: "tool_use_complete"; id: string; name: string; inputJson: string }
  | { type: "message_stop"; stopReason: AgentStopReason };

export type AgentStopReason =
  | "end_turn"
  | "tool_use"
  | "max_tokens"
  | "stop_sequence"
  | "refusal";

/** A single scripted turn: a sequence of events with per-event delays (ms). */
export interface MockTurn {
  events: { delayMs: number; event: AgentStreamEvent }[];
}

export interface MockStreamHandle {
  /** Cancel mid-stream — drops any pending events (the loop drops the empty turn). */
  cancel: () => void;
}

/** Split a sentence into small streaming chunks (a few chars each), like an SSE feed. */
function chunkText(text: string, size = 3): string[] {
  const out: string[] = [];
  for (let i = 0; i < text.length; i += size) out.push(text.slice(i, i + size));
  return out;
}

/**
 * The first half of the mock turn: the model "thinks", streams a sentence, then emits
 * a `get_timeline` tool_use and stops with `tool_use`.
 */
export function mockToolTurn(prompt: string): MockTurn {
  const opener =
    prompt.length > 0
      ? "On it. Let me look at your timeline first.\n\n"
      : "Let me take a look.\n\n";
  const events: MockTurn["events"] = chunkText(opener).map((text) => ({
    delayMs: 18,
    event: { type: "text_delta", text } as AgentStreamEvent,
  }));
  events.push({
    delayMs: 120,
    event: {
      type: "tool_use_complete",
      id: "mock-tu-1",
      name: "get_timeline",
      inputJson: '{"include_captions":false}',
    },
  });
  events.push({
    delayMs: 60,
    event: { type: "message_stop", stopReason: "tool_use" },
  });
  return { events };
}

/**
 * The second half of the mock turn, streamed AFTER the (mock) tool result is appended:
 * a closing sentence, then `message_stop(end_turn)`.
 */
export function mockClosingTurn(): MockTurn {
  const closing =
    "Your timeline is 42 seconds across 8 clips on 3 tracks. " +
    "Want me to tighten the pacing or add B-roll over the gaps?";
  const events: MockTurn["events"] = chunkText(closing).map((text) => ({
    delayMs: 16,
    event: { type: "text_delta", text } as AgentStreamEvent,
  }));
  events.push({
    delayMs: 40,
    event: { type: "message_stop", stopReason: "end_turn" },
  });
  return { events };
}

/** A canned tool_result for the mock `get_timeline` call (what `palmier-tools` returns). */
export const MOCK_TOOL_RESULT_TEXT =
  "Timeline duration: 00:42 across 8 clips on 3 tracks.";

/**
 * Drive a scripted `MockTurn`, invoking `onEvent` per event on its delay. Returns a
 * handle to cancel. This is the stand-in for `AgentClient::stream` — the real one is a
 * Tauri event subscription (TODO(integration)).
 */
export function runMockTurn(
  turn: MockTurn,
  onEvent: (event: AgentStreamEvent) => void,
  onError?: (err: AgentStreamError) => void,
): MockStreamHandle {
  let cancelled = false;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let i = 0;

  const step = () => {
    if (cancelled) return;
    if (i >= turn.events.length) return;
    const { delayMs, event } = turn.events[i++];
    timer = setTimeout(() => {
      if (cancelled) return;
      try {
        onEvent(event);
      } catch (e) {
        onError?.({ kind: "upstream", message: String(e) });
        return;
      }
      step();
    }, delayMs);
  };
  step();

  return {
    cancel: () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    },
  };
}
