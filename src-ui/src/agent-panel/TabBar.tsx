// TabBar — the top floating tab bar (agent-panel.md "top floating tab bar [open
// sessions, "+" new chat, clock=history]"). Open sessions render as tabs (each with a
// close ✕); "+" starts a new chat; the clock opens a history dropdown of all non-empty
// sessions (newest first) to reopen or delete.

import type { CSSProperties } from "react";
import { Spacing, Theme } from "./theme";
import type { ChatSession } from "./types";

export interface TabBarProps {
  openTabs: ChatSession[];
  history: ChatSession[];
  currentSessionId: string;
  historyOpen: boolean;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNewChat: () => void;
  onToggleHistory: () => void;
  onDelete: (id: string) => void;
}

export function TabBar({
  openTabs,
  history,
  currentSessionId,
  historyOpen,
  onSelect,
  onClose,
  onNewChat,
  onToggleHistory,
  onDelete,
}: TabBarProps) {
  return (
    <div style={barStyle}>
      <div style={tabsStyle}>
        {openTabs.map((s) => {
          const active = s.id === currentSessionId;
          return (
            <div
              key={s.id}
              style={{
                ...tabStyle,
                background: active ? Theme.background.prominent : "transparent",
                borderColor: active ? Theme.border.primary : "transparent",
              }}
            >
              <button
                type="button"
                onClick={() => onSelect(s.id)}
                title={s.title}
                style={tabLabelStyle}
                aria-pressed={active}
              >
                {s.title}
              </button>
              <button
                type="button"
                onClick={() => onClose(s.id)}
                title="Close tab"
                aria-label={`Close ${s.title}`}
                style={tabCloseStyle}
              >
                ✕
              </button>
            </div>
          );
        })}
      </div>

      <div style={{ display: "flex", gap: 2, position: "relative" }}>
        <button
          type="button"
          onClick={onNewChat}
          title="New chat"
          aria-label="New chat"
          style={iconButtonStyle}
        >
          +
        </button>
        <button
          type="button"
          onClick={onToggleHistory}
          title="History"
          aria-label="History"
          aria-expanded={historyOpen}
          style={{
            ...iconButtonStyle,
            color: historyOpen ? Theme.accent : Theme.text.secondary,
          }}
        >
          🕘
        </button>

        {historyOpen && (
          <div style={historyMenuStyle} role="menu">
            <div style={historyHeadStyle}>History</div>
            {history.length === 0 && (
              <div style={historyEmptyStyle}>No past chats yet.</div>
            )}
            {history.map((s) => (
              <div key={s.id} style={historyRowStyle}>
                <button
                  type="button"
                  onClick={() => onSelect(s.id)}
                  style={historyItemStyle}
                  title={s.title}
                >
                  <span style={{ flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {s.title}
                  </span>
                  <span style={{ color: Theme.text.muted, fontSize: 10 }}>
                    {relativeTime(s.updatedAt)}
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => onDelete(s.id)}
                  title="Delete chat"
                  aria-label={`Delete ${s.title}`}
                  style={historyDeleteStyle}
                >
                  🗑
                </button>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function relativeTime(iso: string): string {
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";
  const mins = Math.round((Date.now() - then) / 60000);
  if (mins < 1) return "now";
  if (mins < 60) return `${mins}m`;
  const hrs = Math.round(mins / 60);
  if (hrs < 24) return `${hrs}h`;
  return `${Math.round(hrs / 24)}d`;
}

const barStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: Spacing.sm,
  padding: Spacing.xs,
  background: Theme.background.surface,
  borderBottom: `1px solid ${Theme.border.subtle}`,
};

const tabsStyle: CSSProperties = {
  display: "flex",
  gap: Spacing.xs,
  overflowX: "auto",
  flex: 1,
  minWidth: 0,
};

const tabStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 2,
  borderRadius: 8,
  border: "1px solid transparent",
  padding: "2px 2px 2px 8px",
  maxWidth: 160,
};

const tabLabelStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  color: Theme.text.secondary,
  cursor: "pointer",
  fontSize: 12,
  maxWidth: 130,
  overflow: "hidden",
  textOverflow: "ellipsis",
  whiteSpace: "nowrap",
  padding: "2px 0",
};

const tabCloseStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  color: Theme.text.muted,
  cursor: "pointer",
  fontSize: 10,
  padding: "2px 4px",
  borderRadius: 4,
};

const iconButtonStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  color: Theme.text.secondary,
  cursor: "pointer",
  fontSize: 14,
  width: 28,
  height: 28,
  borderRadius: 6,
};

const historyMenuStyle: CSSProperties = {
  position: "absolute",
  top: 32,
  right: 0,
  width: 260,
  maxHeight: 320,
  overflowY: "auto",
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.primary}`,
  borderRadius: 10,
  boxShadow: "0 6px 20px rgba(0,0,0,0.5)",
  zIndex: 20,
  padding: Spacing.xs,
};

const historyHeadStyle: CSSProperties = {
  fontSize: 10,
  textTransform: "uppercase",
  letterSpacing: 0.5,
  color: Theme.text.muted,
  padding: `${Spacing.xs}px ${Spacing.sm}px`,
};

const historyEmptyStyle: CSSProperties = {
  fontSize: 12,
  color: Theme.text.muted,
  padding: Spacing.sm,
};

const historyRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: 2,
};

const historyItemStyle: CSSProperties = {
  flex: 1,
  minWidth: 0,
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
  background: "transparent",
  border: "none",
  color: Theme.text.secondary,
  cursor: "pointer",
  fontSize: 12,
  padding: `${Spacing.xs}px ${Spacing.sm}px`,
  borderRadius: 6,
  textAlign: "left",
};

const historyDeleteStyle: CSSProperties = {
  background: "transparent",
  border: "none",
  color: Theme.text.muted,
  cursor: "pointer",
  fontSize: 11,
  padding: "2px 4px",
};
