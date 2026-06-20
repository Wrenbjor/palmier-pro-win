// MessageList — the middle scrolling message list (agent-panel.md line 232):
// renders each AgentMessage's blocks, auto-scrolls on new content while pinned to the
// bottom, and shows a jump-to-bottom button once the user scrolls up.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties } from "react";
import { ContentBlockView } from "./MessageBlocks";
import { EmptyState } from "./EmptyState";
import { isPinnedToBottom } from "./logic";
import { Interaction, Spacing, Theme } from "./theme";
import type { AgentMessage, AgentStreamError } from "./types";

export interface MessageListProps {
  messages: AgentMessage[];
  isStreaming: boolean;
  streamError: AgentStreamError | null;
  /** Clicking a starter prompt in the empty state. */
  onStarter: (prompt: string) => void;
  /** Whether the composer is usable (drives the empty-state hint). */
  canStream: boolean;
}

export function MessageList({
  messages,
  isStreaming,
  streamError,
  onStarter,
  canStream,
}: MessageListProps) {
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const [pinned, setPinned] = useState(true);

  // Track whether the user is pinned to the bottom (drives auto-scroll + the button).
  const onScroll = () => {
    const el = scrollerRef.current;
    if (!el) return;
    setPinned(
      isPinnedToBottom(
        el.scrollTop,
        el.scrollHeight,
        el.clientHeight,
        Interaction.autoScrollThresholdPx,
      ),
    );
  };

  // Auto-scroll on new content ONLY while pinned (so we don't yank the user back down
  // when they've scrolled up to read history).
  useLayoutEffect(() => {
    if (!pinned) return;
    const el = scrollerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  });

  // On first mount, jump to the bottom.
  useEffect(() => {
    const el = scrollerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const jumpToBottom = () => {
    const el = scrollerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    setPinned(true);
  };

  const empty = messages.length === 0;

  return (
    <div style={wrapStyle}>
      <div ref={scrollerRef} onScroll={onScroll} style={scrollerStyle}>
        {empty ? (
          <EmptyState onStarter={onStarter} canStream={canStream} />
        ) : (
          <div style={listStyle}>
            {messages.map((m) => (
              <MessageRow key={m.id} message={m} />
            ))}
            {isStreaming && <StreamingIndicator />}
            {streamError && <StreamErrorBanner error={streamError} />}
          </div>
        )}
      </div>

      {!pinned && !empty && (
        <button
          type="button"
          onClick={jumpToBottom}
          title="Jump to latest"
          aria-label="Jump to latest"
          style={jumpButtonStyle}
        >
          ↓
        </button>
      )}
    </div>
  );
}

function MessageRow({ message }: { message: AgentMessage }) {
  const isUser = message.role === "user";
  // A user message that is ONLY tool results is rendered as a result row, not a bubble.
  const onlyToolResults =
    isUser && message.blocks.every((b) => b.kind === "toolResult");

  return (
    <div
      style={{
        display: "flex",
        justifyContent: isUser && !onlyToolResults ? "flex-end" : "flex-start",
      }}
    >
      <div
        style={{
          ...bubbleStyle,
          maxWidth: onlyToolResults ? "100%" : "86%",
          background: onlyToolResults
            ? "transparent"
            : isUser
              ? Theme.userBubble
              : Theme.assistantBubble,
          padding: onlyToolResults ? 0 : `${Spacing.sm}px ${Spacing.md}px`,
          border: onlyToolResults ? "none" : `1px solid ${Theme.border.subtle}`,
        }}
      >
        {!onlyToolResults && (
          <div style={roleLabelStyle}>{isUser ? "You" : "Palmier"}</div>
        )}
        <div style={{ display: "flex", flexDirection: "column", gap: Spacing.sm }}>
          {message.blocks.map((b, i) => (
            <ContentBlockView key={i} block={b} />
          ))}
          {message.mentions && message.mentions.length > 0 && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: Spacing.xs }}>
              {message.mentions.map((mn) => (
                <span key={mn.id} title={mn.label} style={mentionChipStyle}>
                  @{mn.displayName}
                </span>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function StreamingIndicator() {
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        gap: Spacing.sm,
        color: Theme.text.muted,
        fontSize: 12,
        padding: `0 ${Spacing.sm}px`,
      }}
    >
      <span style={spinnerStyle} />
      <span>Palmier is working…</span>
    </div>
  );
}

function StreamErrorBanner({ error }: { error: AgentStreamError }) {
  const text =
    error.kind === "upstream"
      ? error.message
      : error.kind === "unauthenticated"
        ? "Not authenticated. Sign in or add an Anthropic API key."
        : "Out of credits. Add an API key or top up your plan.";
  return (
    <div
      role="alert"
      style={{
        border: `1px solid ${Theme.status.error}`,
        background: Theme.status.errorBg,
        color: Theme.status.error,
        borderRadius: 8,
        padding: `${Spacing.sm}px ${Spacing.md}px`,
        fontSize: 12,
      }}
    >
      {text}
    </div>
  );
}

const wrapStyle: CSSProperties = {
  position: "relative",
  flex: 1,
  minHeight: 0,
  display: "flex",
};

const scrollerStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  overflowY: "auto",
  padding: Spacing.md,
};

const listStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: Spacing.md,
};

const bubbleStyle: CSSProperties = {
  borderRadius: 12,
  fontSize: 13,
  color: Theme.text.primary,
  display: "flex",
  flexDirection: "column",
  gap: Spacing.xs,
};

const roleLabelStyle: CSSProperties = {
  fontSize: 10,
  textTransform: "uppercase",
  letterSpacing: 0.5,
  color: Theme.text.muted,
};

const mentionChipStyle: CSSProperties = {
  fontSize: 11,
  padding: "1px 6px",
  borderRadius: 999,
  background: Theme.accentTimecode,
  color: "#000",
};

const jumpButtonStyle: CSSProperties = {
  position: "absolute",
  right: Spacing.md,
  bottom: Spacing.md,
  width: 32,
  height: 32,
  borderRadius: 999,
  border: `1px solid ${Theme.border.primary}`,
  background: Theme.background.prominent,
  color: Theme.text.primary,
  cursor: "pointer",
  fontSize: 16,
  boxShadow: "0 2px 8px rgba(0,0,0,0.4)",
};

const spinnerStyle: CSSProperties = {
  width: 10,
  height: 10,
  borderRadius: 999,
  border: `2px solid ${Theme.accentTimecode}`,
  borderTopColor: "transparent",
  display: "inline-block",
  animation: "agentspin 0.7s linear infinite",
};
