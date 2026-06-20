// Agent-panel view-model types (E8-S8).
//
// FRONTEND view types for the in-app AI chat panel — a TS mirror of the Rust
// `palmier-agent` value types (`AgentMessage` / `AgentContentBlock` / `ChatSession`
// in `docs/reference/agent-panel.md` §"Core value types"). They carry only what the
// panel needs to RENDER and DRIVE the chat: message blocks, sessions/tabs, the model
// picker, mentions, and the streaming/can-stream gating.
//
// The real stream + dispatch land via Tauri commands/events from `palmier-agent`
// (the run loop is E8-S4; the Tauri bridge is later). Until then a `MockAgentStream`
// drives a fake assistant turn through the `AgentPanelController` command seam.
//
// Naming follows `docs/reference/agent-panel.md`. IDs are UUID-ish strings.

// --- Models (FR-32 / agent-panel.md lines 33-34, 53-54) -----------------------

/**
 * The three Anthropic models. `id` is the wire id sent to the API; `label` is the
 * picker label. BYOK shows all three; signed-in is tier-gated (see availableModels).
 */
export type AgentModelId =
  | "claude-sonnet-4-6"
  | "claude-opus-4-8"
  | "claude-haiku-4-5-20251001";

export interface AgentModel {
  id: AgentModelId;
  label: string;
  /** Short label for the compact picker button. */
  short: string;
}

/** The model catalogue (agent-panel.md lines 33-34). Order = picker order. */
export const AGENT_MODELS: readonly AgentModel[] = [
  { id: "claude-sonnet-4-6", label: "Sonnet 4.6", short: "Sonnet" },
  { id: "claude-opus-4-8", label: "Opus 4.8", short: "Opus" },
  { id: "claude-haiku-4-5-20251001", label: "Haiku 4.5", short: "Haiku" },
];

export const DEFAULT_MODEL: AgentModelId = "claude-sonnet-4-6";

/** Config key the selection persists to (agent-panel.md line 54: `"agentModel"`). */
export const MODEL_PREF_KEY = "agentModel";

/**
 * Backend selection state (agent-panel.md lines 47-54). The frontend never reads the
 * keyring / Clerk session directly — this snapshot arrives via a Tauri command at
 * integration time (TODO: agent commands). `availableModels`/`canStream` are derived
 * from it (see logic.ts), exactly as the reference's `AgentService`.
 */
export interface BackendStatus {
  /** A non-empty Anthropic key is in the OS keyring (BYOK path). */
  hasApiKey: boolean;
  /** Clerk session is active (Convex-proxied path). */
  isSignedIn: boolean;
  /** Signed-in account is a paid plan (free = Haiku only). */
  isPaid: boolean;
  /** Signed-in account has credits left (gates `canStream` on the proxied path). */
  hasCredits: boolean;
  /**
   * Catalog-allowed models for a signed-in PAID plan (ruling #20: paid is
   * catalog-driven, default Sonnet 4.6; the Convex catalog MAY enable Opus). Empty =
   * fall back to the default. Ignored for BYOK (which always shows all three).
   */
  paidCatalog?: AgentModelId[];
}

// --- Content blocks (agent-panel.md lines 39-43) ------------------------------

export type AgentRole = "user" | "assistant";

/** A block inside a tool result: text or an inlined image. */
export type ToolResultBlock =
  | { kind: "text"; text: string }
  | { kind: "image"; base64: string; mediaType: string };

/**
 * One content block of an `AgentMessage`. Mirrors the Rust `AgentContentBlock` enum
 * with its `kind` discriminator (text / toolUse / toolResult). **`inputJson` is a raw
 * JSON string forwarded verbatim** (agent-panel.md lines 41-42) — the panel only
 * displays it (collapsed), it never normalizes it.
 */
export type AgentContentBlock =
  | { kind: "text"; text: string }
  | { kind: "toolUse"; id: string; name: string; inputJson: string }
  | {
      kind: "toolResult";
      toolUseId: string;
      content: ToolResultBlock[];
      isError: boolean;
    };

/** A chat message — a role plus an ordered list of content blocks. */
export interface AgentMessage {
  id: string;
  role: AgentRole;
  blocks: AgentContentBlock[];
  /** Mentions referenced in this (user) message — rendered as chips. */
  mentions?: AgentMention[];
}

// --- Mentions (agent-panel.md lines 129-138) ----------------------------------

export type MentionKind = "mediaAsset" | "timelineClip" | "timelineRange";

/**
 * An `@`-mention the user attached to a draft. The panel inserts `@displayName ` into
 * the editor and renders a chip; the real context-hint JSON + image inlining is built
 * backend-side at send (E8-S5). `displayName` is the collapsed token (spaces and `-`
 * → a single `-`, agent-panel.md line 133); collisions disambiguate with `#<first6>`.
 */
export interface AgentMention {
  /** Stable id of the underlying asset/clip/range (used for `#<first6>` disambig). */
  id: string;
  kind: MentionKind;
  /** The single-word token (no leading `@`), e.g. `Beach-Sunset` or `00:03-00:07`. */
  displayName: string;
  /** Human label shown in the autocomplete row + chip tooltip. */
  label: string;
  /** For timelineRange: half-open frame range (start inclusive, end exclusive). */
  range?: { startFrame: number; endFrame: number };
}

// --- Sessions (agent-panel.md lines 149-156) ----------------------------------

/**
 * A chat session = one tab. `title` defaults to "New chat" and auto-derives from the
 * first 40 chars of the first user text once (agent-panel.md line 156). `isOpen`
 * controls whether it has a tab; history shows closed sessions too.
 */
export interface ChatSession {
  id: string;
  title: string;
  /** ISO-8601 string (chat sessions encode dates as iso8601, ruling carry-forward). */
  updatedAt: string;
  messages: AgentMessage[];
  isOpen: boolean;
}

export const NEW_CHAT_TITLE = "New chat";

// --- Stream errors (agent-panel.md lines 51-52, 92, 110) ----------------------

export type AgentStreamError =
  | { kind: "upstream"; message: string }
  | { kind: "unauthenticated" }
  | { kind: "insufficientCredits" };

// --- Starter prompts (agent-panel.md line 30 / empty state) -------------------

export interface StarterPrompt {
  /** Short id (stable for tests / telemetry). */
  id: string;
  title: string;
  prompt: string;
  icon: string;
}

/**
 * The 7 empty-state starter prompts (docs/reference/agent-panel.md "empty state =
 * 7 starter prompts"): generate B-roll, generate opening, captions, VO, music,
 * organize media, transcript-driven cut.
 */
export const STARTER_PROMPTS: readonly StarterPrompt[] = [
  {
    id: "broll",
    title: "Generate B-roll",
    prompt: "Generate B-roll to cover the gaps in my current edit.",
    icon: "🎞️",
  },
  {
    id: "opening",
    title: "Generate an opening",
    prompt: "Generate an opening sequence for this video.",
    icon: "🎬",
  },
  {
    id: "captions",
    title: "Add captions",
    prompt: "Generate captions for the clips on my timeline.",
    icon: "💬",
  },
  {
    id: "vo",
    title: "Add a voiceover",
    prompt: "Write and generate a voiceover for this edit.",
    icon: "🎙️",
  },
  {
    id: "music",
    title: "Add music",
    prompt: "Generate background music that fits the mood of this edit.",
    icon: "🎵",
  },
  {
    id: "organize",
    title: "Organize my media",
    prompt: "Organize my media library into folders by content.",
    icon: "🗂️",
  },
  {
    id: "cut",
    title: "Cut from transcript",
    prompt: "Make a transcript-driven cut, removing the filler and dead air.",
    icon: "✂️",
  },
];
