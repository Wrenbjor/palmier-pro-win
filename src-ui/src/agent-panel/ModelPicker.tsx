// ModelPicker — the model selector (agent-panel.md "model picker {BYOK shows all,
// signed-in tier-allowed}"). A compact dropdown over the *available* models only; the
// selection persists to `"agentModel"` via the controller. Available-model derivation
// (BYOK = all three, signed-paid = catalog, signed-free = Haiku) lives in logic.ts.

import { useState } from "react";
import type { CSSProperties } from "react";
import { Spacing, Theme } from "./theme";
import { AGENT_MODELS, type AgentModelId } from "./types";

export interface ModelPickerProps {
  model: AgentModelId;
  available: AgentModelId[];
  onChange: (model: AgentModelId) => void;
}

export function ModelPicker({ model, available, onChange }: ModelPickerProps) {
  const [open, setOpen] = useState(false);
  const current = AGENT_MODELS.find((m) => m.id === model) ?? AGENT_MODELS[0];
  const options = AGENT_MODELS.filter((m) => available.includes(m.id));

  return (
    <div style={{ position: "relative" }}>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        disabled={options.length === 0}
        title="Model"
        aria-haspopup="listbox"
        aria-expanded={open}
        style={buttonStyle}
      >
        <span style={{ fontSize: 11 }}>{current.short}</span>
        <span style={{ opacity: 0.6, fontSize: 9 }}>▾</span>
      </button>

      {open && options.length > 0 && (
        <div style={menuStyle} role="listbox">
          {options.map((m) => (
            <button
              key={m.id}
              type="button"
              role="option"
              aria-selected={m.id === model}
              onClick={() => {
                onChange(m.id);
                setOpen(false);
              }}
              style={{
                ...itemStyle,
                color: m.id === model ? Theme.accent : Theme.text.secondary,
              }}
            >
              {m.id === model ? "● " : "○ "}
              {m.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

const buttonStyle: CSSProperties = {
  display: "inline-flex",
  alignItems: "center",
  gap: 4,
  border: `1px solid ${Theme.border.subtle}`,
  background: Theme.background.raised,
  color: Theme.text.secondary,
  borderRadius: 6,
  padding: "3px 8px",
  cursor: "pointer",
};

const menuStyle: CSSProperties = {
  position: "absolute",
  bottom: 30,
  left: 0,
  minWidth: 130,
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.primary}`,
  borderRadius: 8,
  boxShadow: "0 6px 20px rgba(0,0,0,0.5)",
  zIndex: 30,
  padding: Spacing.xs,
};

const itemStyle: CSSProperties = {
  display: "block",
  width: "100%",
  textAlign: "left",
  background: "transparent",
  border: "none",
  cursor: "pointer",
  fontSize: 12,
  padding: `${Spacing.xs}px ${Spacing.sm}px`,
  borderRadius: 6,
};
