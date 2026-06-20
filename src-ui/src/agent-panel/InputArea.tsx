// InputArea — the bottom composer (agent-panel.md "bottom input [multi-line editor,
// @mention picker, send/cancel, model picker {BYOK shows all, signed-in tier-allowed},
// API-key indicator]"). A multi-line textarea with an inline @mention autocomplete,
// the model picker, an API-key/backend indicator, and the send/cancel button.

import { useMemo, useRef, useState } from "react";
import type { CSSProperties, KeyboardEvent } from "react";
import {
  applyMentionPick,
  attachMentionToken,
  canSend,
  disambiguateMentions,
  filterMentionCandidates,
  mentionQueryAt,
  pruneDetachedMentions,
} from "./logic";
import { ModelPicker } from "./ModelPicker";
import { Spacing, Theme } from "./theme";
import type {
  AgentMention,
  AgentModelId,
  BackendStatus,
} from "./types";

export interface InputAreaProps {
  draft: string;
  mentions: AgentMention[];
  candidates: AgentMention[];
  backend: BackendStatus;
  model: AgentModelId;
  availableModels: AgentModelId[];
  isStreaming: boolean;
  onDraftChange: (draft: string) => void;
  onMentionsChange: (mentions: AgentMention[]) => void;
  onModelChange: (model: AgentModelId) => void;
  onSend: () => void;
  onCancel: () => void;
}

export function InputArea(props: InputAreaProps) {
  const {
    draft,
    mentions,
    candidates,
    backend,
    model,
    availableModels,
    isStreaming,
    onDraftChange,
    onMentionsChange,
    onModelChange,
    onSend,
    onCancel,
  } = props;

  const taRef = useRef<HTMLTextAreaElement | null>(null);
  const [caret, setCaret] = useState(0);
  const [activeIdx, setActiveIdx] = useState(0);

  // The active @query under the caret (null = popup closed).
  const query = useMemo(() => mentionQueryAt(draft, caret), [draft, caret]);
  const filtered = useMemo(
    () => (query === null ? [] : filterMentionCandidates(candidates, query)),
    [query, candidates],
  );
  const popupOpen = query !== null && filtered.length > 0;

  const sendEnabled = canSend(backend, isStreaming, draft);

  const updateDraft = (text: string, nextCaret: number) => {
    onDraftChange(text);
    setCaret(nextCaret);
    // Prune mentions whose token was deleted.
    onMentionsChange(pruneDetachedMentions(text, mentions));
  };

  const pickMention = (m: AgentMention) => {
    // Insert the disambiguated token; if it collides with an existing mention name,
    // the disambiguation pass appends `#<first6>`.
    const merged = disambiguateMentions([...mentions, m]);
    const picked = merged[merged.length - 1];
    const next = applyMentionPick(draft, caret, picked.displayName);
    onMentionsChange(merged);
    onDraftChange(next.text);
    setCaret(next.caret);
    setActiveIdx(0);
    requestAnimationFrame(() => {
      const ta = taRef.current;
      if (ta) {
        ta.focus();
        ta.setSelectionRange(next.caret, next.caret);
      }
    });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (popupOpen) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIdx((i) => (i + 1) % filtered.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIdx((i) => (i - 1 + filtered.length) % filtered.length);
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        pickMention(filtered[Math.min(activeIdx, filtered.length - 1)]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setCaret(-1); // close popup without moving the real caret
        return;
      }
    }
    // Enter sends (Shift+Enter = newline).
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      if (sendEnabled) onSend();
    }
  };

  const insertAtToken = () => {
    // The toolbar "@" button: append a bare "@" to open the picker.
    const next = attachMentionToken(draft, "").replace(/ $/, "");
    const withAt = draft.endsWith("@") ? draft : `${next}@`;
    onDraftChange(withAt);
    setCaret(withAt.length);
    requestAnimationFrame(() => taRef.current?.focus());
  };

  return (
    <div style={wrapStyle}>
      {popupOpen && (
        <div style={popupStyle} role="listbox">
          {filtered.map((m, i) => (
            <button
              key={m.id}
              type="button"
              role="option"
              aria-selected={i === activeIdx}
              onMouseDown={(e) => {
                e.preventDefault();
                pickMention(m);
              }}
              onMouseEnter={() => setActiveIdx(i)}
              style={{
                ...popupRowStyle,
                background:
                  i === activeIdx ? Theme.background.prominent : "transparent",
              }}
            >
              <span style={kindBadgeStyle}>{kindIcon(m.kind)}</span>
              <span style={{ flex: 1, minWidth: 0 }}>
                <span style={{ fontWeight: 600 }}>@{m.displayName}</span>
                <span style={{ color: Theme.text.muted, marginLeft: 6 }}>
                  {m.label}
                </span>
              </span>
            </button>
          ))}
        </div>
      )}

      <textarea
        ref={taRef}
        value={draft}
        rows={2}
        placeholder="Message Palmier…  (@ to reference media, clips, or a range)"
        onChange={(e) => updateDraft(e.target.value, e.target.selectionStart ?? 0)}
        onKeyDown={onKeyDown}
        onKeyUp={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
        onClick={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
        style={textareaStyle}
      />

      <div style={controlsStyle}>
        <div style={{ display: "flex", alignItems: "center", gap: Spacing.sm }}>
          <button
            type="button"
            onClick={insertAtToken}
            title="Mention media / clip / range"
            aria-label="Insert mention"
            style={atButtonStyle}
          >
            @
          </button>
          <ModelPicker
            model={model}
            available={availableModels}
            onChange={onModelChange}
          />
          <ApiKeyIndicator backend={backend} />
        </div>

        {isStreaming ? (
          <button
            type="button"
            onClick={onCancel}
            title="Stop"
            style={{ ...sendButtonStyle, background: Theme.status.error }}
          >
            ■ Stop
          </button>
        ) : (
          <button
            type="button"
            onClick={onSend}
            disabled={!sendEnabled}
            title="Send (Enter)"
            style={{
              ...sendButtonStyle,
              background: sendEnabled ? Theme.accent : Theme.background.prominent,
              color: sendEnabled ? "#000" : Theme.text.muted,
              cursor: sendEnabled ? "pointer" : "not-allowed",
            }}
          >
            Send
          </button>
        )}
      </div>
    </div>
  );
}

/** API-key / backend indicator (agent-panel.md: "API-key indicator"). */
function ApiKeyIndicator({ backend }: { backend: BackendStatus }) {
  let label: string;
  let color: string;
  let title: string;
  if (backend.hasApiKey) {
    label = "API key";
    color = Theme.status.success;
    title = "Using your Anthropic API key (BYOK).";
  } else if (backend.isSignedIn) {
    label = backend.isPaid ? "Plan" : "Free";
    color = backend.hasCredits ? Theme.accent : Theme.status.error;
    title = backend.hasCredits
      ? "Streaming through your Palmier plan."
      : "No credits left on your plan.";
  } else {
    label = "No key";
    color = Theme.status.error;
    title = "Add an Anthropic API key or sign in to start.";
  }
  return (
    <span style={{ ...indicatorStyle, color }} title={title}>
      <span style={{ ...dotStyle, background: color }} />
      {label}
    </span>
  );
}

function kindIcon(kind: AgentMention["kind"]): string {
  if (kind === "mediaAsset") return "🎞";
  if (kind === "timelineClip") return "▭";
  return "↔";
}

const wrapStyle: CSSProperties = {
  position: "relative",
  borderTop: `1px solid ${Theme.border.subtle}`,
  background: Theme.background.surface,
  padding: Spacing.sm,
  display: "flex",
  flexDirection: "column",
  gap: Spacing.sm,
};

const textareaStyle: CSSProperties = {
  width: "100%",
  resize: "vertical",
  minHeight: 44,
  maxHeight: 160,
  background: Theme.background.base,
  border: `1px solid ${Theme.border.subtle}`,
  borderRadius: 8,
  color: Theme.text.primary,
  fontSize: 13,
  fontFamily: "inherit",
  padding: Spacing.sm,
  boxSizing: "border-box",
};

const controlsStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: Spacing.sm,
};

const atButtonStyle: CSSProperties = {
  width: 26,
  height: 26,
  borderRadius: 6,
  border: `1px solid ${Theme.border.subtle}`,
  background: Theme.background.raised,
  color: Theme.text.secondary,
  cursor: "pointer",
  fontSize: 13,
};

const sendButtonStyle: CSSProperties = {
  border: "none",
  borderRadius: 8,
  padding: "6px 14px",
  fontSize: 13,
  fontWeight: 600,
  color: "#fff",
  cursor: "pointer",
};

const indicatorStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 4,
  fontSize: 11,
};

const dotStyle: CSSProperties = {
  width: 6,
  height: 6,
  borderRadius: 999,
  display: "inline-block",
};

const popupStyle: CSSProperties = {
  position: "absolute",
  left: Spacing.sm,
  right: Spacing.sm,
  bottom: 96,
  maxHeight: 200,
  overflowY: "auto",
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.primary}`,
  borderRadius: 10,
  boxShadow: "0 6px 20px rgba(0,0,0,0.5)",
  zIndex: 30,
  padding: Spacing.xs,
};

const popupRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
  width: "100%",
  border: "none",
  background: "transparent",
  color: Theme.text.secondary,
  cursor: "pointer",
  fontSize: 12,
  padding: `${Spacing.xs}px ${Spacing.sm}px`,
  borderRadius: 6,
  textAlign: "left",
};

const kindBadgeStyle: CSSProperties = {
  width: 18,
  textAlign: "center",
  fontSize: 12,
  opacity: 0.8,
};
