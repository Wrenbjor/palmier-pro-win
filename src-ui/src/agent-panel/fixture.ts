// Agent-panel fixtures — a fake project state that drives the @mention autocomplete
// and a couple of seed sessions, so the panel is fully explorable in `vite dev`
// outside a Tauri webview. Swapped for the real project snapshot (media assets,
// timeline clips/ranges) + `load_sessions` when the Tauri agent commands land
// (TODO(integration)).

import { makeDisplayName } from "./logic";
import type {
  AgentMention,
  AgentMessage,
  BackendStatus,
  ChatSession,
} from "./types";

/** A mention from a (kind, id, label) plus optional frame range. */
function mention(
  kind: AgentMention["kind"],
  id: string,
  label: string,
  range?: { startFrame: number; endFrame: number },
): AgentMention {
  return { id, kind, label, displayName: makeDisplayName(label), range };
}

/**
 * The mention candidates the autocomplete offers — drawn from a fixture "project":
 * three media assets, two timeline clips, and one timeline range (half-open frames).
 */
export function makeFixtureMentions(): AgentMention[] {
  return [
    mention("mediaAsset", "asset-beach", "Beach Sunset"),
    mention("mediaAsset", "asset-city", "City Drone"),
    mention("mediaAsset", "asset-logo", "Logo Sting"),
    mention("timelineClip", "clip-intro", "Intro Clip"),
    mention("timelineClip", "clip-outro", "Outro Clip"),
    mention("timelineRange", "range-0090", "00:03-00:07", {
      startFrame: 90,
      endFrame: 210,
    }),
  ];
}

/** Default backend status for the fixture: BYOK (key present) → all models, can stream. */
export function makeFixtureBackend(): BackendStatus {
  return { hasApiKey: true, isSignedIn: false, isPaid: false, hasCredits: false };
}

/** A short prior conversation, used to seed one history tab. */
function seedMessages(): AgentMessage[] {
  return [
    {
      id: "m-u1",
      role: "user",
      blocks: [{ kind: "text", text: "How long is my timeline right now?" }],
    },
    {
      id: "m-a1",
      role: "assistant",
      blocks: [
        { kind: "text", text: "Let me check the current timeline." },
        {
          kind: "toolUse",
          id: "tu-1",
          name: "get_timeline",
          inputJson: '{"include_captions":false}',
        },
      ],
    },
    {
      id: "m-u2",
      role: "user",
      blocks: [
        {
          kind: "toolResult",
          toolUseId: "tu-1",
          isError: false,
          content: [
            {
              kind: "text",
              text: "Timeline duration: 00:42 across 8 clips on 3 tracks.",
            },
          ],
        },
      ],
    },
    {
      id: "m-a2",
      role: "assistant",
      blocks: [
        {
          kind: "text",
          text: "Your timeline is 42 seconds long with 8 clips across 3 tracks.",
        },
      ],
    },
  ];
}

/**
 * Seed sessions: one fresh empty *open* current session (prepended, as `load_sessions`
 * does — agent-panel.md line 152) plus one closed prior session for the history view.
 */
export function makeFixtureSessions(): ChatSession[] {
  const now = Date.now();
  return [
    {
      id: "session-current",
      title: "New chat",
      updatedAt: new Date(now).toISOString(),
      messages: [],
      isOpen: true,
    },
    {
      id: "session-timeline",
      title: "How long is my timeline right now?",
      updatedAt: new Date(now - 1000 * 60 * 30).toISOString(),
      messages: seedMessages(),
      isOpen: false,
    },
  ];
}
