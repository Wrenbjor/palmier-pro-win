// Inspector field components (E12-S3 + E12-S4).
//
// S3: ScrubbableNumberField (drag-to-scrub + type-to-edit, coarse/fine modifiers,
//     mixed-value "—") and InspectorPositionFields (X/Y pair, one undo group).
// S4: ColorField (live swatch picker), FontPickerField (Featured + All groups,
//     hover-preview), TextContentField (plain-text <textarea> that never stomps
//     the caret).
//
// All scrub/parse/format math lives in `bodyLogic.ts` (pure, parity-checked); these
// are thin views that translate JS pointer/keyboard events into apply/commit calls.
//
// apply* (live preview) and commit* (final, pushes undo) are separate callbacks —
// the caller wires apply* to a transient edit and commit* to the committed
// `set_clip_properties` so multi-clip edits land in ONE named undo group
// (reference: apply* creates NO undo entry; only commit* does).

import { useEffect, useRef, useState } from "react";
import type { CSSProperties, JSX, ReactNode } from "react";
import {
  formatScrub,
  hexToRgba,
  parseScrub,
  rgbaToHex,
  scrubModifierMultiplier,
  scrubNext,
  SCRUB_DRAG_THRESHOLD,
  type ScrubModifier,
  type ScrubRange,
} from "./bodyLogic";
import { FontSize, Spacing, Theme } from "./theme";

// ── ScrubbableNumberField (E12-S3) ───────────────────────────────────────────

export interface ScrubbableNumberFieldProps {
  /** The current stored value, or null for a MIXED (multi-select) value → "—". */
  value: number | null;
  range: ScrubRange;
  /** Fired live during drag / on each parsed keystroke commit (preview, no undo). */
  onChange: (next: number) => void;
  /** Fired on pointer-up / blur after edit (the committed value, pushes undo). */
  onCommit: (next: number) => void;
  /** Optional trailing label (e.g. "X" / "Y" for position fields). */
  trailingLabel?: string;
  /** Disable interaction entirely (e.g. crop on multi-select). */
  disabled?: boolean;
  /** Fixed field width in px (reference fieldWidth; default flexes). */
  width?: number;
}

type DragState = {
  startX: number;
  startValue: number;
  moved: boolean;
};

function modifierFrom(e: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }): ScrubModifier {
  // Shift ⇒ coarse ×10; Ctrl (or Command→Ctrl) ⇒ fine ×0.1.
  if (e.shiftKey) return "coarse";
  if (e.ctrlKey || e.metaKey) return "fine";
  return "none";
}

export function ScrubbableNumberField(
  props: ScrubbableNumberFieldProps,
): JSX.Element {
  const { value, range, onChange, onCommit, trailingLabel, disabled, width } = props;
  const mixed = value === null;

  const [editing, setEditing] = useState(false);
  const [text, setText] = useState("");
  const drag = useRef<DragState | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  // Track the last committed value so pointer-up can decide commit-vs-noop.
  const liveValue = useRef<number | null>(value);
  liveValue.current = value;

  // ── Drag-to-scrub (pointer events on the value div) ──────────────────────
  function onPointerDown(e: React.PointerEvent<HTMLDivElement>): void {
    if (disabled || mixed || editing) return; // mixed blocks scrub
    (e.target as HTMLElement).setPointerCapture?.(e.pointerId);
    drag.current = { startX: e.clientX, startValue: value ?? 0, moved: false };
  }

  function onPointerMove(e: React.PointerEvent<HTMLDivElement>): void {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.startX;
    if (!d.moved && Math.abs(dx) < SCRUB_DRAG_THRESHOLD) return; // 3 px threshold
    d.moved = true;
    const next = scrubNext(d.startValue, dx, range, modifierFrom(e));
    liveValue.current = next;
    onChange(next); // live preview during drag
  }

  function endDrag(): void {
    const d = drag.current;
    drag.current = null;
    if (!d) return;
    if (d.moved && liveValue.current !== null) {
      onCommit(liveValue.current); // commit on pointer-up
    } else if (!d.moved) {
      // A click without drag → enter edit mode.
      beginEdit();
    }
  }

  function beginEdit(): void {
    if (disabled || mixed) return;
    setText(formatScrub(value ?? 0, range));
    setEditing(true);
    requestAnimationFrame(() => inputRef.current?.select());
  }

  function commitEdit(): void {
    const parsed = parseScrub(text, range);
    setEditing(false);
    if (parsed !== null && parsed !== value) {
      onChange(parsed);
      onCommit(parsed);
    }
  }

  const display = mixed ? "—" : formatScrub(value ?? 0, range);

  return (
    <div style={{ ...fieldShellStyle, width }}>
      {editing ? (
        <input
          ref={inputRef}
          style={fieldInputStyle}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={commitEdit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitEdit();
            else if (e.key === "Escape") setEditing(false);
          }}
        />
      ) : (
        <div
          role="spinbutton"
          aria-valuenow={value ?? undefined}
          aria-disabled={disabled || mixed}
          style={{
            ...fieldValueStyle,
            cursor: disabled || mixed ? "default" : "ew-resize",
            color: mixed ? Theme.text.muted : Theme.text.primary,
          }}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={() => {
            drag.current = null;
          }}
          onDoubleClick={beginEdit}
        >
          {display}
        </div>
      )}
      {trailingLabel && <span style={trailingLabelStyle}>{trailingLabel}</span>}
    </div>
  );
}

// ── InspectorPositionFields (E12-S3) ─────────────────────────────────────────

export interface InspectorPositionFieldsProps {
  /** Shared top-left X (normalised 0..1 of canvas), or null for mixed. */
  x: number | null;
  /** Shared top-left Y, or null for mixed. */
  y: number | null;
  rangeX: ScrubRange;
  rangeY: ScrubRange;
  /** Live preview of one axis (other axis untouched). */
  onApply: (axis: "x" | "y", value: number) => void;
  /** Commit both-axes in ONE named undo group ("Change Position"). */
  onCommit: (axis: "x" | "y", value: number) => void;
}

/** X then Y, fieldWidth 36, trailing labels "X"/"Y" (reference InspectorPositionFields). */
export function InspectorPositionFields(
  props: InspectorPositionFieldsProps,
): JSX.Element {
  const { x, y, rangeX, rangeY, onApply, onCommit } = props;
  return (
    <div style={{ display: "flex", gap: Spacing.sm }}>
      <ScrubbableNumberField
        value={x}
        range={rangeX}
        width={36}
        trailingLabel="X"
        onChange={(v) => onApply("x", v)}
        onCommit={(v) => onCommit("x", v)}
      />
      <ScrubbableNumberField
        value={y}
        range={rangeY}
        width={36}
        trailingLabel="Y"
        onChange={(v) => onApply("y", v)}
        onCommit={(v) => onCommit("y", v)}
      />
    </div>
  );
}

// ── ColorField (E12-S4) ──────────────────────────────────────────────────────

export interface ColorFieldProps {
  /** Current color as `#RRGGBBAA` hex, or null for mixed. */
  hex: string | null;
  /** Live during the native picker drag (no undo) — pass to a debounced commit. */
  onChange: (hex: string) => void;
  /** Final value on close (pushes undo). */
  onCommit: (hex: string) => void;
  /** Whether the alpha channel is editable (`supportsOpacity`). */
  supportsOpacity?: boolean;
}

/**
 * A swatch that opens the browser color picker. The native `<input type=color>`
 * fires `input` live during drag (→ onChange) and `change` on close (→ onCommit),
 * matching the reference NSColorPanel `colorDidChangeNotification` semantics. The
 * first seed notification is suppressed (we only emit on user interaction). Alpha
 * (`supportsOpacity`) is edited via a separate range since `<input type=color>`
 * has no alpha channel.
 */
export function ColorField(props: ColorFieldProps): JSX.Element {
  const { hex, onChange, onCommit, supportsOpacity } = props;
  const mixed = hex === null;
  const parsed = hex ? hexToRgba(hex) : null;
  const rgbHex = parsed ? rgbaToHex(parsed.r, parsed.g, parsed.b, 1).slice(0, 7) : "#000000";
  const alpha = parsed?.a ?? 1;

  function emit(rgb: string, a: number, commit: boolean): void {
    const p = hexToRgba(rgb) ?? { r: 0, g: 0, b: 0, a: 1 };
    const full = rgbaToHex(p.r, p.g, p.b, a);
    if (commit) onCommit(full);
    else onChange(full);
  }

  return (
    <div style={{ display: "flex", alignItems: "center", gap: Spacing.sm }}>
      <input
        type="color"
        aria-label="color"
        disabled={mixed}
        value={rgbHex}
        style={swatchStyle}
        onInput={(e) => emit((e.target as HTMLInputElement).value, alpha, false)}
        onChange={(e) => emit((e.target as HTMLInputElement).value, alpha, true)}
      />
      {supportsOpacity && (
        <input
          type="range"
          aria-label="alpha"
          min={0}
          max={1}
          step={0.01}
          disabled={mixed}
          value={alpha}
          style={{ flex: 1 }}
          onChange={(e) => emit(rgbHex, Number(e.target.value), false)}
          onPointerUp={(e) =>
            emit(rgbHex, Number((e.target as HTMLInputElement).value), true)
          }
        />
      )}
      {mixed && <span style={{ color: Theme.text.muted, fontSize: FontSize.xs }}>—</span>}
    </div>
  );
}

// ── FontPickerField (E12-S4) ─────────────────────────────────────────────────

export interface FontGroup {
  /** "Featured" (bundled) or "All fonts" (system). */
  label: string;
  families: string[];
}

export interface FontPickerFieldProps {
  /** Current family name, or null for mixed. */
  value: string | null;
  /** Featured (bundled) families then All (system) families. */
  groups: FontGroup[];
  /** Non-committing hover preview (reverts on cancel). */
  onPreview?: (family: string) => void;
  /** Final pick (pushes undo). */
  onChange: (family: string) => void;
  /** Closed without a pick → revert preview. */
  onCancel?: () => void;
}

/**
 * Two-group font picker. Hover fires a non-committing preview; selecting fires
 * `onChange`; closing without a pick fires `onCancel`. Each row renders in its own
 * font; the current font shows a checkmark. (The bundled/system enumeration is
 * supplied by the caller via `groups` — the `palmier-text` font-family command is
 * an additive backend touch out of this view's scope.)
 */
export function FontPickerField(props: FontPickerFieldProps): JSX.Element {
  const { value, groups, onPreview, onChange, onCancel } = props;
  const [open, setOpen] = useState(false);
  const picked = useRef(false);

  function close(): void {
    setOpen(false);
    if (!picked.current) onCancel?.();
    picked.current = false;
  }

  return (
    <div style={{ position: "relative" }}>
      <button
        type="button"
        style={fontTriggerStyle}
        onClick={() => {
          picked.current = false;
          setOpen((o) => !o);
        }}
      >
        <span style={{ fontFamily: value ?? undefined }}>{value ?? "—"}</span>
        <span style={{ color: Theme.text.muted }}>▾</span>
      </button>
      {open && (
        <div style={fontMenuStyle} role="listbox" onMouseLeave={() => onPreview?.(value ?? "")}>
          {groups.map((group) => (
            <div key={group.label}>
              <div style={fontGroupLabelStyle}>{group.label}</div>
              {group.families.map((family) => (
                <button
                  key={family}
                  type="button"
                  role="option"
                  aria-selected={family === value}
                  style={{ ...fontRowStyle, fontFamily: family }}
                  onMouseEnter={() => onPreview?.(family)}
                  onClick={() => {
                    picked.current = true;
                    onChange(family);
                    close();
                  }}
                >
                  <span>{family}</span>
                  {family === value && <span aria-hidden>✓</span>}
                </button>
              ))}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── TextContentField (E12-S4) ─────────────────────────────────────────────────

export interface TextContentFieldProps {
  /** The current text content (null → mixed; shown empty). */
  value: string | null;
  /** Live apply on every keystroke (no undo). */
  onInput: (text: string) => void;
  /** Commit on blur (pushes undo). */
  onCommit: (text: string) => void;
  /** Minimum height in px (reference Text-tab Content min height 80). */
  minHeight?: number;
}

/**
 * Plain-text multi-line editor. App owns undo (`allowsUndo=false` equivalent — we
 * do not interfere with the platform textarea undo but commits go through the app
 * history). The KEY gotcha: never stomp the caret — only overwrite the textarea's
 * text from `value` when the editor is NOT focused and the strings differ (the
 * reference NSTextView-wrapper bug to avoid).
 */
export function TextContentField(props: TextContentFieldProps): JSX.Element {
  const { value, onInput, onCommit, minHeight = 80 } = props;
  const ref = useRef<HTMLTextAreaElement | null>(null);
  const [local, setLocal] = useState(value ?? "");

  // Only sync external value into the field when NOT focused AND it differs.
  useEffect(() => {
    const el = ref.current;
    const focused = el != null && document.activeElement === el;
    if (!focused && (value ?? "") !== local) {
      setLocal(value ?? "");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  return (
    <textarea
      ref={ref}
      style={{ ...textAreaStyle, minHeight }}
      spellCheck={false}
      autoCapitalize="off"
      autoCorrect="off"
      value={local}
      onChange={(e) => {
        setLocal(e.target.value);
        onInput(e.target.value);
      }}
      onBlur={() => onCommit(local)}
    />
  );
}

// ── Re-export the modifier helper (used by callers building events) ──────────
export { modifierFrom as scrubModifierFromEvent, scrubModifierMultiplier };

// ── Styles ────────────────────────────────────────────────────────────────────

const fieldShellStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.xs,
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.subtle}`,
  borderRadius: 4,
  padding: `2px ${Spacing.sm}px`,
  minWidth: 0,
};

const fieldValueStyle: CSSProperties = {
  flex: 1,
  fontSize: FontSize.xs,
  fontVariantNumeric: "tabular-nums",
  userSelect: "none",
  minWidth: 0,
  textAlign: "right",
};

const fieldInputStyle: CSSProperties = {
  flex: 1,
  background: "transparent",
  border: "none",
  outline: "none",
  color: Theme.text.primary,
  fontSize: FontSize.xs,
  textAlign: "right",
  minWidth: 0,
};

const trailingLabelStyle: CSSProperties = {
  fontSize: FontSize.xxs,
  color: Theme.text.muted,
  flexShrink: 0,
};

const swatchStyle: CSSProperties = {
  width: 24,
  height: 18,
  padding: 0,
  border: `1px solid ${Theme.border.subtle}`,
  borderRadius: 3,
  background: "transparent",
  cursor: "pointer",
};

const fontTriggerStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  gap: Spacing.sm,
  width: "100%",
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.subtle}`,
  borderRadius: 4,
  padding: `3px ${Spacing.sm}px`,
  color: Theme.text.primary,
  fontSize: FontSize.xs,
  cursor: "pointer",
};

const fontMenuStyle: CSSProperties = {
  position: "absolute",
  top: "100%",
  left: 0,
  right: 0,
  marginTop: 2,
  maxHeight: 240,
  overflowY: "auto",
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.primary}`,
  borderRadius: 4,
  zIndex: 10,
  boxShadow: "0 8px 24px rgba(0,0,0,0.5)",
};

const fontGroupLabelStyle: CSSProperties = {
  fontSize: FontSize.xxs,
  fontWeight: 600,
  color: Theme.text.muted,
  padding: `${Spacing.xs}px ${Spacing.sm}px 2px`,
  textTransform: "uppercase",
};

const fontRowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  width: "100%",
  background: "transparent",
  border: "none",
  color: Theme.text.secondary,
  fontSize: FontSize.sm,
  padding: `${Spacing.xs}px ${Spacing.sm}px`,
  cursor: "pointer",
  textAlign: "left",
};

const textAreaStyle: CSSProperties = {
  width: "100%",
  resize: "vertical",
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.subtle}`,
  borderRadius: 4,
  color: Theme.text.primary,
  fontSize: FontSize.sm,
  padding: Spacing.sm,
  outline: "none",
  fontFamily: "inherit",
};

// Small shared building blocks reused by the tab bodies.

export function FieldRow(props: { label: string; children: ReactNode }): JSX.Element {
  return (
    <div style={rowStyle}>
      <span style={rowLabelStyle}>{props.label}</span>
      <div style={{ flex: 1, minWidth: 0, display: "flex", justifyContent: "flex-end" }}>
        {props.children}
      </div>
    </div>
  );
}

const rowStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
};

const rowLabelStyle: CSSProperties = {
  fontSize: FontSize.xs,
  color: Theme.text.tertiary,
  flexShrink: 0,
  minWidth: 64,
};
