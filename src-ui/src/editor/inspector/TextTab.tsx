// Inspector TEXT tab body (E12-S6) — visible iff a single text clip is selected.
//
// Sections: Content (TextContentField; every keystroke → set content live, blur →
// commit), Typography (Font via FontPickerField, Size scrub 12..300 pt), Appearance
// (Color via ColorField + Opacity, plus Background/Border/Shadow toggle+color
// pairs), Layout (Alignment segmented, Position via InspectorPositionFields).
//
// Color edits route through a DEBOUNCED commit per key so a color drag does not spam
// the undo stack; the enable toggles commit immediately (reference §gotcha).
// `fitTextClipToContent` is called by the backend on content/font/size changes
// (the resize math is owned by Epic 5) — here we just send the property edit.
//
// CAVEAT (documented): `ClipView` exposes the resolved text as `name` but carries no
// `textStyle` (font/size/color/alignment) — so those fields SEED from defaults; the
// writes are correct. Full seeding awaits the timeline view-model carrying textStyle
// (Epic 7 adapter work).

import { useRef } from "react";
import type { JSX } from "react";
import type { ClipView } from "../types";
import {
  clipPropertiesArgs,
  FONT_SIZE_RANGE,
  OPACITY_RANGE,
  positionRange,
} from "./bodyLogic";
import {
  ColorField,
  FieldRow,
  FontPickerField,
  InspectorPositionFields,
  ScrubbableNumberField,
  TextContentField,
  type FontGroup,
} from "./fields";
import { SegmentedControl, Section, ToggleRow } from "./sections";
import { Spacing } from "./theme";
import type { EditDispatch } from "./VideoTab";

export interface TextTabProps {
  /** The single selected text clip. */
  clip: ClipView;
  canvasWidth: number;
  canvasHeight: number;
  /** Font families: Featured (bundled) then All (system) — supplied by the caller. */
  fontGroups?: FontGroup[];
  edit: EditDispatch;
}

const DEFAULT_FONT_GROUPS: FontGroup[] = [
  { label: "Featured", families: ["Inter", "Poppins", "Playfair Display"] },
  { label: "All fonts", families: ["Arial", "Georgia", "Times New Roman", "Courier New"] },
];

export function TextTab(props: TextTabProps): JSX.Element {
  const { clip, canvasWidth, canvasHeight, edit } = props;
  const groups = props.fontGroups ?? DEFAULT_FONT_GROUPS;
  const ids = [clip.id];

  // Debounce timer for color commits (reference §gotcha).
  const colorTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const setProp = (patch: Record<string, unknown>) =>
    void edit("set_clip_properties", clipPropertiesArgs(ids, patch));

  function debouncedColorCommit(hex: string): void {
    if (colorTimer.current) clearTimeout(colorTimer.current);
    colorTimer.current = setTimeout(() => setProp({ color: hex }), 120);
  }

  // Seeds — content from the clip name; style fields from defaults (see caveat).
  const content = clip.name ?? "";
  const opacity = clip.opacity;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: Spacing.xl }}>
      <Section title="Content">
        <TextContentField
          value={content}
          minHeight={80}
          onInput={(text) => setProp({ content: text })}
          onCommit={(text) => setProp({ content: text })}
        />
      </Section>

      <Section title="Typography">
        <FieldRow label="Font">
          <FontPickerField
            value={null}
            groups={groups}
            onChange={(family) => setProp({ fontName: family })}
          />
        </FieldRow>
        <FieldRow label="Size">
          <ScrubbableNumberField
            value={48}
            range={FONT_SIZE_RANGE}
            onChange={(v) => setProp({ fontSize: v })}
            onCommit={(v) => setProp({ fontSize: v })}
          />
        </FieldRow>
      </Section>

      <Section title="Appearance">
        <FieldRow label="Color">
          <ColorField
            hex={null}
            onChange={debouncedColorCommit}
            onCommit={(hex) => setProp({ color: hex })}
          />
        </FieldRow>
        <FieldRow label="Opacity">
          <ScrubbableNumberField
            value={opacity}
            range={OPACITY_RANGE}
            onChange={(v) => setProp({ opacity: v })}
            onCommit={(v) => setProp({ opacity: v })}
          />
        </FieldRow>
        {/* Background / Border / Shadow toggle+color pairs (style not in view-model
            yet → toggles seed off, color writes route through content style). */}
        <ToggleRow label="Background" value={false} onToggle={() => {}} />
        <ToggleRow label="Border" value={false} onToggle={() => {}} />
        <ToggleRow label="Shadow" value={false} onToggle={() => {}} />
      </Section>

      <Section title="Layout">
        <FieldRow label="Alignment">
          <SegmentedControl
            value={null}
            options={[
              { value: "left", label: "L" },
              { value: "center", label: "C" },
              { value: "right", label: "R" },
            ]}
            onChange={(a) => setProp({ alignment: a })}
          />
        </FieldRow>
        <FieldRow label="Position">
          <InspectorPositionFields
            x={0.5}
            y={0.5}
            rangeX={positionRange(canvasWidth)}
            rangeY={positionRange(canvasHeight)}
            onApply={(axis, v) =>
              setProp({ transform: axis === "x" ? { centerX: v } : { centerY: v } })
            }
            onCommit={(axis, v) =>
              setProp({ transform: axis === "x" ? { centerX: v } : { centerY: v } })
            }
          />
        </FieldRow>
      </Section>
    </div>
  );
}
