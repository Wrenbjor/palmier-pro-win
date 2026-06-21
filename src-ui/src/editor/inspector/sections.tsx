// Shared section primitives for the Inspector tab bodies (E12-S5..S7).
//
// Collapsible + plain sections, a toggle button, and a segmented control — the
// reference Inspector grouping chrome, themed with the inspector tokens.

import { useState } from "react";
import type { CSSProperties, JSX, ReactNode } from "react";
import { FontSize, Spacing, Theme, Tracking } from "./theme";

export function Section(props: {
  title?: string;
  /** Trailing control rendered on the header row (e.g. a Reset button). */
  trailing?: ReactNode;
  children: ReactNode;
}): JSX.Element {
  return (
    <div style={sectionStyle}>
      {(props.title || props.trailing) && (
        <div style={sectionHeaderStyle}>
          {props.title && <div style={sectionTitleStyle}>{props.title.toUpperCase()}</div>}
          {props.trailing}
        </div>
      )}
      <div style={sectionBodyStyle}>{props.children}</div>
    </div>
  );
}

export function CollapsibleSection(props: {
  title: string;
  defaultExpanded?: boolean;
  trailing?: ReactNode;
  children: ReactNode;
}): JSX.Element {
  const [expanded, setExpanded] = useState(props.defaultExpanded ?? true);
  return (
    <div style={sectionStyle}>
      <div style={sectionHeaderStyle}>
        <button
          type="button"
          style={disclosureStyle}
          aria-expanded={expanded}
          onClick={() => setExpanded((e) => !e)}
        >
          <span style={{ color: Theme.text.muted }}>{expanded ? "▾" : "▸"}</span>
          <span style={sectionTitleStyle}>{props.title.toUpperCase()}</span>
        </button>
        {props.trailing}
      </div>
      {expanded && <div style={sectionBodyStyle}>{props.children}</div>}
    </div>
  );
}

export function TextButton(props: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
}): JSX.Element {
  return (
    <button
      type="button"
      style={{
        ...textButtonStyle,
        opacity: props.disabled ? 0.4 : 1,
        cursor: props.disabled ? "default" : "pointer",
      }}
      disabled={props.disabled}
      onClick={props.onClick}
    >
      {props.label}
    </button>
  );
}

export function ToggleRow(props: {
  label: string;
  /** null → mixed (indeterminate). */
  value: boolean | null;
  onToggle: (next: boolean) => void;
  disabled?: boolean;
}): JSX.Element {
  const on = props.value === true;
  return (
    <div style={toggleRowStyle}>
      <span style={{ fontSize: FontSize.xs, color: Theme.text.tertiary }}>
        {props.label}
      </span>
      <button
        type="button"
        role="switch"
        aria-checked={props.value === null ? "mixed" : on}
        disabled={props.disabled}
        onClick={() => props.onToggle(!on)}
        style={{
          ...switchStyle,
          background: on ? Theme.accentTimecode : Theme.background.base,
          opacity: props.disabled ? 0.4 : 1,
          cursor: props.disabled ? "default" : "pointer",
        }}
      >
        <span
          style={{
            ...switchKnobStyle,
            transform: on ? "translateX(14px)" : "translateX(0)",
          }}
        />
      </button>
    </div>
  );
}

export function SegmentedControl<T extends string>(props: {
  options: { value: T; label: string }[];
  /** null → no segment highlighted (mixed). */
  value: T | null;
  onChange: (value: T) => void;
}): JSX.Element {
  return (
    <div style={segmentedStyle} role="radiogroup">
      {props.options.map((opt) => {
        const active = opt.value === props.value;
        return (
          <button
            key={opt.value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => props.onChange(opt.value)}
            style={{
              ...segmentStyle,
              background: active ? Theme.background.raised : "transparent",
              color: active ? Theme.text.primary : Theme.text.tertiary,
            }}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}

// ── Styles ────────────────────────────────────────────────────────────────────

const sectionStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: Spacing.smMd,
};

const sectionHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: Spacing.sm,
};

const sectionBodyStyle: CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: Spacing.sm,
};

const sectionTitleStyle: CSSProperties = {
  fontSize: FontSize.xxs,
  fontWeight: 600,
  letterSpacing: Tracking.wide,
  color: Theme.text.muted,
};

const disclosureStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.xs,
  background: "transparent",
  border: "none",
  padding: 0,
  cursor: "pointer",
};

const textButtonStyle: CSSProperties = {
  background: "transparent",
  border: `1px solid ${Theme.border.subtle}`,
  borderRadius: 4,
  color: Theme.text.secondary,
  fontSize: FontSize.xxs,
  padding: `2px ${Spacing.sm}px`,
};

const toggleRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: Spacing.sm,
};

const switchStyle: CSSProperties = {
  position: "relative",
  width: 32,
  height: 18,
  borderRadius: 9,
  border: `1px solid ${Theme.border.subtle}`,
  padding: 1,
  flexShrink: 0,
};

const switchKnobStyle: CSSProperties = {
  display: "block",
  width: 14,
  height: 14,
  borderRadius: "50%",
  background: Theme.text.primary,
  transition: "transform 120ms ease",
};

const segmentedStyle: CSSProperties = {
  display: "flex",
  background: Theme.background.base,
  border: `1px solid ${Theme.border.subtle}`,
  borderRadius: 4,
  padding: 1,
  gap: 1,
};

const segmentStyle: CSSProperties = {
  flex: 1,
  border: "none",
  borderRadius: 3,
  fontSize: FontSize.xs,
  padding: `2px ${Spacing.sm}px`,
  cursor: "pointer",
};
