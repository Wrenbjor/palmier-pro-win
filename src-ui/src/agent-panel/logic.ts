// Pure agent-panel logic — model availability, can-stream gating, mention tokens,
// session-title derivation, message-block helpers, and auto-scroll math. No React,
// no Tauri, no side effects: this is the parity-checked core (see parity.checks.ts),
// mirrored from the reference `AgentService` (agent-panel.md §"Backend selection",
// §"Mentions & context hints", §"Sessions & persistence").

import {
  AGENT_MODELS,
  DEFAULT_MODEL,
  NEW_CHAT_TITLE,
  type AgentMention,
  type AgentMessage,
  type AgentModelId,
  type BackendStatus,
} from "./types";

// --- Model availability (agent-panel.md lines 53-54; ruling #20) --------------

/**
 * Which models the picker may show, given the backend status.
 * - BYOK (`hasApiKey`) → all three (agent-panel.md line 54).
 * - Signed-in PAID → catalog-driven (ruling #20): the Convex catalog, else just the
 *   default Sonnet 4.6. (Reference hard-coded `[sonnet46]`; ruling #20 keeps it flexible.)
 * - Signed-in FREE → Haiku 4.5 only.
 * - No backend → empty.
 */
export function availableModels(status: BackendStatus): AgentModelId[] {
  if (status.hasApiKey) return AGENT_MODELS.map((m) => m.id);
  if (status.isSignedIn) {
    if (status.isPaid) {
      const catalog = status.paidCatalog?.filter((id) =>
        AGENT_MODELS.some((m) => m.id === id),
      );
      return catalog && catalog.length > 0 ? catalog : [DEFAULT_MODEL];
    }
    return ["claude-haiku-4-5-20251001"];
  }
  return [];
}

/**
 * The model actually used: the persisted preference if still allowed, else the first
 * available, else the default Sonnet 4.6 (agent-panel.md line 54).
 */
export function effectiveModel(
  status: BackendStatus,
  preferred: AgentModelId | null,
): AgentModelId {
  const avail = availableModels(status);
  if (preferred && avail.includes(preferred)) return preferred;
  if (avail.length > 0) return avail[0];
  return DEFAULT_MODEL;
}

/**
 * `canStream` (agent-panel.md lines 51-52): a key is present, OR a signed-in account
 * has credits. `send()` is gated on this; when false the panel shows the inline
 * "sign in or add a key" hint.
 */
export function canStream(status: BackendStatus): boolean {
  return status.hasApiKey || (status.isSignedIn && status.hasCredits);
}

/** The inline gating message shown when `canStream` is false (agent-panel.md line 52). */
export const SIGN_IN_HINT =
  "Sign in to a paid plan or add an Anthropic API key to start.";

/**
 * Which backend a send would use (agent-panel.md lines 47-50):
 *   key → "anthropic" (BYOK direct), else signed-in → "palmier" (Convex proxy),
 *   else "none".
 */
export function selectedBackend(
  status: BackendStatus,
): "anthropic" | "palmier" | "none" {
  if (status.hasApiKey) return "anthropic";
  if (status.isSignedIn) return "palmier";
  return "none";
}

// --- Send gating (agent-panel.md line 232) ------------------------------------

/** Send is enabled iff not streaming, can stream, and the draft is non-empty. */
export function canSend(
  status: BackendStatus,
  isStreaming: boolean,
  draft: string,
): boolean {
  return !isStreaming && canStream(status) && draft.trim().length > 0;
}

// --- Mention tokens (agent-panel.md lines 133-135) ----------------------------

/**
 * `makeDisplayName` — collapse runs of whitespace and `-` into a single `-` so the
 * mention is a single `@token` word (agent-panel.md line 133).
 */
export function makeDisplayName(raw: string): string {
  return raw
    .trim()
    .replace(/[\s-]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

/**
 * Attach a mention's token to a draft: append `@displayName ` (de-duped — if the same
 * token is already present we don't add it again). Returns the new draft. Collision
 * disambiguation (`#<first6 of id>`) is applied by `disambiguateMentions` over the set.
 */
export function attachMentionToken(draft: string, displayName: string): string {
  const token = `@${displayName}`;
  if (draftContainsToken(draft, displayName)) return draft;
  const sep = draft.length === 0 || draft.endsWith(" ") ? "" : " ";
  return `${draft}${sep}${token} `;
}

/** Whether `@displayName` literally appears as a token in the draft. */
export function draftContainsToken(draft: string, displayName: string): boolean {
  // Word-ish boundary: the token must not be a prefix of a longer token.
  const re = new RegExp(`@${escapeRegExp(displayName)}(?![\\w-])`);
  return re.test(draft);
}

/**
 * `disambiguateMentions` — when two attached mentions collapse to the same
 * `displayName`, append `#<first6 of id>` to make each unique (agent-panel.md line 135).
 * Returns a new list with adjusted `displayName`s (originals untouched).
 */
export function disambiguateMentions(mentions: AgentMention[]): AgentMention[] {
  const counts = new Map<string, number>();
  for (const m of mentions) counts.set(m.displayName, (counts.get(m.displayName) ?? 0) + 1);
  return mentions.map((m) =>
    (counts.get(m.displayName) ?? 0) > 1
      ? { ...m, displayName: `${m.displayName}#${m.id.slice(0, 6)}` }
      : m,
  );
}

/**
 * `referencedMentions` — the mentions whose `@displayName` literally appears in the
 * sent text (agent-panel.md line 98: `text.contains("@\(displayName)")`). Only these
 * carry a context hint.
 */
export function referencedMentions(
  text: string,
  mentions: AgentMention[],
): AgentMention[] {
  return mentions.filter((m) => draftContainsToken(text, m.displayName));
}

/**
 * `pruneDetachedMentions` — drop mentions whose `@token` was deleted from the draft
 * (agent-panel.md line 135).
 */
export function pruneDetachedMentions(
  draft: string,
  mentions: AgentMention[],
): AgentMention[] {
  return mentions.filter((m) => draftContainsToken(draft, m.displayName));
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// --- @mention autocomplete query (token under caret) --------------------------

/**
 * If the caret sits at the end of a partial `@query` token, return the query (without
 * the `@`); else null. Drives the autocomplete popup. The token starts at an `@`
 * preceded by start-of-string or whitespace and runs to the caret with no whitespace.
 */
export function mentionQueryAt(text: string, caret: number): string | null {
  const upto = text.slice(0, caret);
  const m = /(^|\s)@([^\s@]*)$/.exec(upto);
  return m ? m[2] : null;
}

/** Filter candidate mentions by a (case-insensitive) autocomplete query. */
export function filterMentionCandidates(
  candidates: AgentMention[],
  query: string,
): AgentMention[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return candidates.slice(0, 8);
  return candidates
    .filter(
      (c) =>
        c.displayName.toLowerCase().includes(q) ||
        c.label.toLowerCase().includes(q),
    )
    .slice(0, 8);
}

/**
 * Replace the partial `@query` under the caret with `@displayName ` and return the new
 * text + caret position. Used when the user picks an autocomplete row.
 */
export function applyMentionPick(
  text: string,
  caret: number,
  displayName: string,
): { text: string; caret: number } {
  const upto = text.slice(0, caret);
  const after = text.slice(caret);
  const replaced = upto.replace(/(^|\s)@([^\s@]*)$/, (_full, pre) => `${pre}@${displayName} `);
  return { text: replaced + after, caret: replaced.length };
}

// --- Session title (agent-panel.md line 156) ----------------------------------

/**
 * Derive a session title from the first user text (first 40 chars). Only fires while
 * the title is still the default "New chat"; otherwise the existing title is kept.
 */
export function deriveTitle(
  currentTitle: string,
  messages: AgentMessage[],
): string {
  if (currentTitle !== NEW_CHAT_TITLE) return currentTitle;
  const firstUserText = firstUserMessageText(messages);
  if (!firstUserText) return currentTitle;
  const trimmed = firstUserText.trim().replace(/\s+/g, " ");
  if (trimmed.length === 0) return currentTitle;
  return trimmed.slice(0, 40);
}

/** The concatenated text blocks of the first user message, or null. */
export function firstUserMessageText(messages: AgentMessage[]): string | null {
  const first = messages.find((m) => m.role === "user");
  if (!first) return null;
  const text = first.blocks
    .filter((b): b is { kind: "text"; text: string } => b.kind === "text")
    .map((b) => b.text)
    .join("")
    .trim();
  return text.length > 0 ? text : null;
}

/** A session is non-empty if it has at least one message (drop empties on save/load). */
export function isNonEmptySession(messages: AgentMessage[]): boolean {
  return messages.length > 0;
}

// --- Message-block helpers ----------------------------------------------------

/**
 * Append/extend the last `.text` block of an assistant message in place — the loop's
 * `text_delta` behavior (agent-panel.md line 106). If the last block is text, extend
 * it; otherwise push a new text block. Returns a NEW message (immutable update).
 */
export function appendTextDelta(msg: AgentMessage, delta: string): AgentMessage {
  const blocks = msg.blocks.slice();
  const last = blocks[blocks.length - 1];
  if (last && last.kind === "text") {
    blocks[blocks.length - 1] = { kind: "text", text: last.text + delta };
  } else {
    blocks.push({ kind: "text", text: delta });
  }
  return { ...msg, blocks };
}

/** True if the message has no content blocks (the empty turn dropped on cancel). */
export function isEmptyAssistantTurn(msg: AgentMessage): boolean {
  return (
    msg.role === "assistant" &&
    msg.blocks.every((b) => b.kind === "text" && b.text.length === 0)
  );
}

/**
 * Pretty-print a tool_use input JSON for the collapsed view (agent-panel.md line 41:
 * raw string forwarded verbatim — we only *display* it formatted, never re-store it).
 * Falls back to the raw string if it isn't valid JSON.
 */
export function formatToolInput(inputJson: string): string {
  try {
    return JSON.stringify(JSON.parse(inputJson), null, 2);
  } catch {
    return inputJson;
  }
}

// --- Auto-scroll (agent-panel.md line 232) ------------------------------------

/**
 * Whether the message list is "pinned" to the bottom (within the threshold). Used to
 * decide whether new content should auto-scroll and whether to show jump-to-bottom.
 */
export function isPinnedToBottom(
  scrollTop: number,
  scrollHeight: number,
  clientHeight: number,
  thresholdPx: number,
): boolean {
  return scrollHeight - (scrollTop + clientHeight) <= thresholdPx;
}
