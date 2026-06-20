// EmptyState — the empty-conversation view with the 7 starter prompts (docs/reference/
// agent-panel.md: "empty state = 7 starter prompts"). Clicking one sends it.

import type { CSSProperties } from "react";
import { SIGN_IN_HINT } from "./logic";
import { Spacing, Theme } from "./theme";
import { STARTER_PROMPTS } from "./types";

export interface EmptyStateProps {
  onStarter: (prompt: string) => void;
  canStream: boolean;
}

export function EmptyState({ onStarter, canStream }: EmptyStateProps) {
  return (
    <div style={wrapStyle}>
      <div style={{ textAlign: "center", marginBottom: Spacing.lg }}>
        <div style={{ fontSize: 28 }}>🌴</div>
        <div style={{ fontSize: 15, fontWeight: 600, marginTop: Spacing.xs }}>
          What should we make?
        </div>
        <div style={{ fontSize: 12, color: Theme.text.muted, marginTop: 2 }}>
          Ask Palmier to edit, generate, or organize — or pick a starting point.
        </div>
      </div>

      <div style={gridStyle}>
        {STARTER_PROMPTS.map((p) => (
          <button
            key={p.id}
            type="button"
            disabled={!canStream}
            onClick={() => onStarter(p.prompt)}
            title={p.prompt}
            style={{
              ...promptStyle,
              opacity: canStream ? 1 : 0.5,
              cursor: canStream ? "pointer" : "not-allowed",
            }}
          >
            <span style={{ fontSize: 18 }}>{p.icon}</span>
            <span style={{ fontSize: 12, fontWeight: 600 }}>{p.title}</span>
          </button>
        ))}
      </div>

      {!canStream && (
        <div style={hintStyle}>{SIGN_IN_HINT}</div>
      )}
    </div>
  );
}

const wrapStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  justifyContent: "center",
  height: "100%",
  padding: Spacing.lg,
};

const gridStyle: CSSProperties = {
  display: "grid",
  gridTemplateColumns: "1fr 1fr",
  gap: Spacing.sm,
  width: "100%",
  maxWidth: 360,
};

const promptStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  alignItems: "flex-start",
  gap: Spacing.xs,
  padding: Spacing.md,
  borderRadius: 10,
  border: `1px solid ${Theme.border.subtle}`,
  background: Theme.background.raised,
  color: Theme.text.primary,
  textAlign: "left",
};

const hintStyle: CSSProperties = {
  marginTop: Spacing.lg,
  fontSize: 12,
  color: Theme.text.muted,
  textAlign: "center",
  maxWidth: 320,
};
