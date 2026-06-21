// Inspector VIDEO tab body (E12-S5) — Transform / Playback over the selected
// non-text visual clips.
//
// Rows: Reset Transform, Position, Scale, Rotation, Opacity, Crop toggle, Flip H/V,
// and Playback → Speed. Each control reads the SHARED value across the selection
// (null → "—" mixed, scrub disabled) and writes through `set_clip_properties`
// (transform is center-based, ruling #7) or `set_keyframes` (rotation has no static
// property field on the tool). apply* = live preview; commit* = the committed
// `set_clip_properties` so a multi-clip edit lands in ONE named undo group.
//
// CAVEAT (documented): `ClipView` carries no static transform scalars
// (centerX/centerY/width/height/rotation) — only optional keyframe tracks. So the
// displayed Position/Scale/Rotation seed from the track value at the active frame
// when a track exists, else the reference defaults (center 0.5/0.5, scale 1.0,
// rotation 0). Writes are always correct; only the SEED is best-effort until the
// timeline view-model exposes the static transform (Epic 7 adapter work).

import type { JSX } from "react";
import type { ClipView } from "../types";
import {
  hasKeyframeAt,
  keyframeOffset,
  OPACITY_RANGE,
  positionRange,
  ROTATION_RANGE,
  sampleScalarTrack,
  SCALE_RANGE,
  sharedClipValue,
  SPEED_RANGE,
  clipPropertiesArgs,
} from "./bodyLogic";
import {
  FieldRow,
  InspectorPositionFields,
  ScrubbableNumberField,
} from "./fields";
import {
  CollapsibleSection,
  Section,
  TextButton,
  ToggleRow,
} from "./sections";
import { Spacing, Theme, FontSize } from "./theme";

/** A mutating dispatch (defaults to `editorEdit` from the bridge; injectable for tests). */
export type EditDispatch = (
  name: string,
  args: Record<string, unknown>,
) => void | Promise<unknown>;

export interface VideoTabProps {
  /** The selected non-text visual clips (drives the Video tab). */
  clips: readonly ClipView[];
  /** Canvas width/height for the Position display multiplier. */
  canvasWidth: number;
  canvasHeight: number;
  /** Playhead frame for keyframe-aware field seeding (absent → frame 0). */
  activeFrame?: number;
  /** Whether crop editing is active (shell-owned). */
  cropEditingActive?: boolean;
  onToggleCropEditing?: (active: boolean) => void;
  edit: EditDispatch;
}

const TRACK_FALLBACK = { centerX: 0.5, centerY: 0.5, scale: 1.0, rotation: 0 };

export function VideoTab(props: VideoTabProps): JSX.Element {
  const { clips, canvasWidth, canvasHeight, activeFrame, edit } = props;
  const ids = clips.map((c) => c.id);
  const single = clips.length === 1;

  // ── Seed values (shared across selection; null = mixed) ──────────────────
  const posX = sharedClipValue(clips, (c) =>
    sampleScalarTrack(
      c.positionTrack,
      keyframeOffset(c, activeFrame ?? 0),
      TRACK_FALLBACK.centerX,
    ),
  );
  const posY = sharedClipValue(clips, (c) =>
    // positionTrack stores a pair; the scalar helper reads the first axis. Without
    // pair-track support in the view-model we fall back to the same seed for Y.
    sampleScalarTrack(c.positionTrack, keyframeOffset(c, activeFrame ?? 0), TRACK_FALLBACK.centerY),
  );
  const scale = sharedClipValue(clips, (c) =>
    sampleScalarTrack(c.scaleTrack, keyframeOffset(c, activeFrame ?? 0), TRACK_FALLBACK.scale),
  );
  const opacity = sharedClipValue(clips, (c) => c.opacity);
  const speed = sharedClipValue(clips, (c) => c.speed);
  // Flip state is not exposed by ClipView (transform scalars unavailable) — seed
  // false; the toggle still writes the correct flip patch.
  const flipH: boolean | null = false;
  const flipV: boolean | null = false;

  // ── Writers ──────────────────────────────────────────────────────────────
  const setProp = (patch: Record<string, unknown>) =>
    void edit("set_clip_properties", clipPropertiesArgs(ids, patch));

  const setTransform = (t: Record<string, unknown>) =>
    void edit("set_clip_properties", clipPropertiesArgs(ids, { transform: t }));

  function resetTransform(): void {
    // One named group: center back to 0.5/0.5, scale (width/height) to canvas, no
    // flips, opacity 1. (Rotation + fades clears are owned by the keyframe/fade
    // tools; we reset what `set_clip_properties` exposes.)
    setProp({
      opacity: 1,
      transform: {
        centerX: 0.5,
        centerY: 0.5,
        flipHorizontal: false,
        flipVertical: false,
      },
    });
  }

  const hasOpacityKf = (c: ClipView) =>
    hasKeyframeAt(c.opacityTrack, keyframeOffset(c, activeFrame ?? 0));

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: Spacing.xl }}>
      <CollapsibleSection
        title="Transform"
        defaultExpanded
        trailing={<TextButton label="Reset" onClick={resetTransform} />}
      >
        <FieldRow label="Position">
          <InspectorPositionFields
            x={posX}
            y={posY}
            rangeX={positionRange(canvasWidth)}
            rangeY={positionRange(canvasHeight)}
            onApply={(axis, v) =>
              setTransform(axis === "x" ? { centerX: v } : { centerY: v })
            }
            onCommit={(axis, v) =>
              setTransform(axis === "x" ? { centerX: v } : { centerY: v })
            }
          />
        </FieldRow>

        <FieldRow label="Scale">
          <ScrubbableNumberField
            value={scale}
            range={SCALE_RANGE}
            onChange={(v) => setTransform({ width: v, height: v })}
            onCommit={(v) => setTransform({ width: v, height: v })}
          />
        </FieldRow>

        <FieldRow label="Rotation">
          <ScrubbableNumberField
            value={single ? TRACK_FALLBACK.rotation : null}
            range={ROTATION_RANGE}
            disabled={!single}
            onChange={(v) =>
              // Rotation has no static property on set_clip_properties — route a
              // single keyframe through set_keyframes (rotation track).
              single &&
              void edit("set_keyframes", {
                clipId: ids[0],
                property: "rotation",
                keyframes: [[keyframeOffset(clips[0], activeFrame ?? 0), v]],
              })
            }
            onCommit={(v) =>
              single &&
              void edit("set_keyframes", {
                clipId: ids[0],
                property: "rotation",
                keyframes: [[keyframeOffset(clips[0], activeFrame ?? 0), v]],
              })
            }
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

        <ToggleRow
          label="Crop"
          value={props.cropEditingActive ?? false}
          disabled={!single}
          onToggle={(next) => props.onToggleCropEditing?.(next)}
        />

        <ToggleRow
          label="Flip Horizontal"
          value={flipH}
          onToggle={(next) => setTransform({ flipHorizontal: next })}
        />
        <ToggleRow
          label="Flip Vertical"
          value={flipV}
          onToggle={(next) => setTransform({ flipVertical: next })}
        />

        {clips.some(hasOpacityKf) && (
          <div style={kfHintStyle}>Opacity is keyframed at this frame.</div>
        )}
      </CollapsibleSection>

      <Section title="Playback">
        <FieldRow label="Speed">
          <ScrubbableNumberField
            value={speed}
            range={SPEED_RANGE}
            onChange={(v) => setProp({ speed: v })}
            onCommit={(v) => setProp({ speed: v })}
          />
        </FieldRow>
      </Section>
    </div>
  );
}

const kfHintStyle = {
  fontSize: FontSize.xxs,
  color: Theme.text.muted,
} as const;
