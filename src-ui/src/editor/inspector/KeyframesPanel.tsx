// Inspector KEYFRAMES side panel (E12-S8) — per-property stamp/navigate + a
// drag-to-move lane with snap and a per-keyframe interpolation menu, for a SINGLE
// selected clip.
//
// Per animatable row: [prev-kf chevron][diamond stamp][next-kf chevron][lane]. The
// stamp is a FILLED diamond when a keyframe exists at the active frame (click
// removes) and HOLLOW otherwise (click adds). Controls are disabled (40%) when the
// playhead is outside the clip; chevrons navigate previous/next keyframe and disable
// when none. The lane draws one diamond per keyframe at `frameToLaneX(relFrame)`;
// dragging a diamond MOVES its frame (live feedback, 4 px snap to the playhead +
// sibling keyframes) and right-clicking it opens an interpolation menu. Rows: video
// clip → Position, Scale, Rotation, Opacity, Crop; audio → Volume only.
//
// Ops map to `set_keyframes` (REPLACE the property track): add/remove/move/interp all
// rewrite the full frame-sorted row list for that property as `[frame, …values,
// interp]`. Frames are CLIP-RELATIVE (the tool's contract) — converted via
// `keyframeOffset`. Interpolation kinds are EXACTLY the model's set (linear / hold /
// smooth); the menu never offers a kind the backend rejects.

import { useEffect, useRef, useState, type JSX } from "react";
import type { ClipView, Interpolation, KeyframeTrackView } from "../types";
import {
  frameToLaneX,
  hasKeyframeAt,
  INTERPOLATION_KINDS,
  INTERPOLATION_LABEL,
  keyframeOffset,
  keyframeRow,
  keyframeRows,
  laneXToFrame,
  moveKeyframeRows,
  nextKeyframeFrame,
  previousKeyframeFrame,
  sampleScalarTrack,
  setKeyframeInterpRows,
  snapKeyframeFrame,
} from "./bodyLogic";
import { FontSize, Spacing, Theme } from "./theme";
import type { EditDispatch } from "./VideoTab";

/** The animatable properties (reference `AnimatableProperty`). */
export type AnimatableProperty =
  | "position"
  | "scale"
  | "rotation"
  | "opacity"
  | "crop"
  | "volume";

const VIDEO_PROPERTIES: AnimatableProperty[] = [
  "position",
  "scale",
  "rotation",
  "opacity",
  "crop",
];
const AUDIO_PROPERTIES: AnimatableProperty[] = ["volume"];

/** Default lane scale (px per clip-relative frame) when the host supplies none. */
const DEFAULT_LANE_PX_PER_FRAME = 1;
/** The lane track width in px (the diamonds are positioned within this). */
const LANE_WIDTH = 96;

export interface KeyframesPanelProps {
  clip: ClipView;
  /** Absolute playhead frame. */
  activeFrame: number;
  /** Seek the playhead (chevron navigation). */
  onSeek?: (absoluteFrame: number) => void;
  edit: EditDispatch;
  /**
   * Lane scale: px per clip-relative frame for the drag/draw mapping. Optional so
   * the existing mount type-checks; defaults to {@link DEFAULT_LANE_PX_PER_FRAME}.
   * Drag distance ÷ this = frame delta, so the snap radius (4 px) lands correctly.
   */
  lanePxPerFrame?: number;
}

/** The KeyframeTrackView for a property on the clip. */
function trackFor(
  clip: ClipView,
  property: AnimatableProperty,
): KeyframeTrackView | null | undefined {
  switch (property) {
    case "opacity":
      return clip.opacityTrack;
    case "volume":
      return clip.volumeTrack;
    case "position":
      return clip.positionTrack;
    case "scale":
      return clip.scaleTrack;
    case "rotation":
      return clip.rotationTrack;
    case "crop":
      return clip.cropTrack;
  }
}

/** The number of value columns a `set_keyframes` row needs for a property. */
function arityFor(property: AnimatableProperty): number {
  switch (property) {
    case "position":
    case "scale":
      return 2;
    case "crop":
      return 4;
    default:
      return 1;
  }
}

const PROP_LABEL: Record<AnimatableProperty, string> = {
  position: "Position",
  scale: "Scale",
  rotation: "Rotation",
  opacity: "Opacity",
  crop: "Crop",
  volume: "Volume",
};

/** Live drag state for a single dragged diamond. */
interface DragState {
  property: AnimatableProperty;
  /** The keyframe's ORIGINAL clip-relative frame (the move source). */
  fromFrame: number;
  /** Pointer x at drag start (window space). */
  startX: number;
  /** The current (snapped) clip-relative frame the diamond is shown at. */
  currentFrame: number;
}

/** Open interpolation-menu state. */
interface MenuState {
  property: AnimatableProperty;
  frame: number;
  /** Screen position for the menu. */
  x: number;
  y: number;
}

export function KeyframesPanel(props: KeyframesPanelProps): JSX.Element {
  const { clip, activeFrame, onSeek, edit } = props;
  const lanePxPerFrame =
    props.lanePxPerFrame && props.lanePxPerFrame > 0
      ? props.lanePxPerFrame
      : DEFAULT_LANE_PX_PER_FRAME;
  const isAudio = clip.mediaType === "audio";
  const properties = isAudio ? AUDIO_PROPERTIES : VIDEO_PROPERTIES;

  const clipStart = clip.startFrame;
  const clipEnd = clip.startFrame + clip.durationFrames;
  const insideClip = activeFrame >= clipStart && activeFrame < clipEnd;
  const rel = keyframeOffset(clip, activeFrame);
  const playheadRel = insideClip ? rel : null;

  const [drag, setDrag] = useState<DragState | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);
  // Keep the latest drag in a ref so the window listeners (bound once) see it.
  const dragRef = useRef<DragState | null>(null);
  dragRef.current = drag;

  // Window-level move/up listeners while a drag is live: update live feedback on
  // move, commit a `set_keyframes` on release.
  useEffect(() => {
    if (!drag) return;
    const property = drag.property;
    const arity = arityFor(property);
    const track = trackFor(clip, property);
    const otherFrames =
      track?.keyframes.filter((k) => k.frame !== drag.fromFrame).map((k) => k.frame) ?? [];

    const clampRel = (f: number) => Math.max(0, Math.min(clip.durationFrames, f));

    const onMove = (e: PointerEvent) => {
      const cur = dragRef.current;
      if (!cur) return;
      const dxPx = e.clientX - cur.startX;
      const proposed = clampRel(cur.fromFrame + laneXToFrame(dxPx, lanePxPerFrame));
      const snapped = clampRel(
        snapKeyframeFrame(proposed, lanePxPerFrame, playheadRel, otherFrames),
      );
      if (snapped !== cur.currentFrame) {
        setDrag({ ...cur, currentFrame: snapped });
      }
    };
    const onUp = () => {
      const cur = dragRef.current;
      setDrag(null);
      if (!cur) return;
      if (cur.currentFrame !== cur.fromFrame) {
        const rows = moveKeyframeRows(track, arity, cur.fromFrame, cur.currentFrame);
        void edit("set_keyframes", { clipId: clip.id, property, keyframes: rows });
      }
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [drag?.property, drag?.fromFrame, clip.id, lanePxPerFrame, playheadRel]);

  // Dismiss the interpolation menu on any outside interaction.
  useEffect(() => {
    if (!menu) return;
    const dismiss = () => setMenu(null);
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("blur", dismiss);
    return () => {
      window.removeEventListener("pointerdown", dismiss);
      window.removeEventListener("blur", dismiss);
    };
  }, [menu]);

  const setInterp = (property: AnimatableProperty, frame: number, interp: Interpolation) => {
    const arity = arityFor(property);
    const track = trackFor(clip, property);
    const rows = setKeyframeInterpRows(track, arity, frame, interp);
    void edit("set_keyframes", { clipId: clip.id, property, keyframes: rows });
    setMenu(null);
  };

  return (
    <div style={panelStyle}>
      <div style={headerStyle}>KEYFRAMES</div>
      {properties.map((property) => {
        const track = trackFor(clip, property);
        const present = hasKeyframeAt(track, rel);
        const prev = previousKeyframeFrame(track, rel);
        const next = nextKeyframeFrame(track, rel);
        const arity = arityFor(property);
        const kfs = track?.keyframes ?? [];

        const toggleStamp = () => {
          if (!insideClip) return;
          let rows: (number | string)[][];
          if (present) {
            rows = keyframeRows(track, arity).filter((r) => r[0] !== rel);
          } else {
            const seed = sampleSeed(track, rel, arity);
            rows = [
              ...keyframeRows(track, arity),
              keyframeRow(rel, valuesToSeed(seed), arity, "smooth"),
            ];
          }
          void edit("set_keyframes", { clipId: clip.id, property, keyframes: rows });
        };

        return (
          <div key={property} style={rowStyle}>
            <span style={rowLabelStyle}>{PROP_LABEL[property]}</span>
            <div style={controlsStyle}>
              <button
                type="button"
                aria-label={`previous ${property} keyframe`}
                disabled={prev === null}
                onClick={() => prev !== null && onSeek?.(clipStart + prev)}
                style={{ ...chevronStyle, opacity: prev === null ? 0.4 : 1 }}
              >
                ‹
              </button>
              <button
                type="button"
                aria-label={`toggle ${property} keyframe`}
                aria-pressed={present}
                disabled={!insideClip}
                onClick={toggleStamp}
                style={{ ...stampStyle, opacity: insideClip ? 1 : 0.4 }}
              >
                <span style={{ color: present ? Theme.accentTimecode : "transparent" }}>◆</span>
                {!present && <span style={hollowDiamondStyle}>◇</span>}
              </button>
              <button
                type="button"
                aria-label={`next ${property} keyframe`}
                disabled={next === null}
                onClick={() => next !== null && onSeek?.(clipStart + next)}
                style={{ ...chevronStyle, opacity: next === null ? 0.4 : 1 }}
              >
                ›
              </button>
              {/* Drag-to-move lane: one diamond per keyframe at frameToLaneX. */}
              <div style={laneStyle} aria-label={`${property} keyframe lane`}>
                {kfs.map((kf) => {
                  const shownFrame =
                    drag && drag.property === property && drag.fromFrame === kf.frame
                      ? drag.currentFrame
                      : kf.frame;
                  const dragging =
                    !!drag && drag.property === property && drag.fromFrame === kf.frame;
                  return (
                    <span
                      key={kf.frame}
                      role="button"
                      aria-label={`${property} keyframe at frame ${kf.frame} (${INTERPOLATION_LABEL[kf.interpolationOut]})`}
                      title={INTERPOLATION_LABEL[kf.interpolationOut]}
                      onPointerDown={(e) => {
                        if (e.button !== 0) return;
                        e.preventDefault();
                        setMenu(null);
                        setDrag({
                          property,
                          fromFrame: kf.frame,
                          startX: e.clientX,
                          currentFrame: kf.frame,
                        });
                      }}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        setMenu({ property, frame: kf.frame, x: e.clientX, y: e.clientY });
                      }}
                      style={{
                        ...laneDiamondStyle,
                        left: frameToLaneX(shownFrame, lanePxPerFrame),
                        color: dragging ? Theme.accent : Theme.accentTimecode,
                        // Hold interpolation reads as a hard step → square the glyph.
                        opacity: dragging ? 0.9 : 1,
                      }}
                    >
                      {kf.interpolationOut === "hold" ? "◼" : "◆"}
                    </span>
                  );
                })}
              </div>
            </div>
          </div>
        );
      })}

      {menu && (
        <div
          role="menu"
          aria-label="keyframe interpolation"
          style={{ ...menuStyle, left: menu.x, top: menu.y }}
          onPointerDown={(e) => e.stopPropagation()}
        >
          {INTERPOLATION_KINDS.map((kind) => (
            <button
              key={kind}
              type="button"
              role="menuitem"
              onClick={() => setInterp(menu.property, menu.frame, kind)}
              style={menuItemStyle}
            >
              {INTERPOLATION_LABEL[kind]}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Wrap a flat seed array as the value shape `keyframeRow`/`keyframeValues` expect. */
function valuesToSeed(seed: number[]): unknown {
  if (seed.length === 1) return seed[0];
  if (seed.length === 2) return { a: seed[0], b: seed[1] };
  if (seed.length === 4) return { top: seed[0], right: seed[1], bottom: seed[2], left: seed[3] };
  return seed[0] ?? 0;
}

/** Seed values for a NEW keyframe: sample the existing track's nearest value, else zeros. */
function sampleSeed(
  track: KeyframeTrackView | null | undefined,
  rel: number,
  arity: number,
): number[] {
  const kfs = track?.keyframes;
  if (!kfs || kfs.length === 0) return new Array(arity).fill(0);
  // For scalar tracks reuse the shared sampler; for pair/crop fall back to the
  // nearest stored value coerced to the arity.
  if (arity === 1) return [sampleScalarTrack(track, rel, 0)];
  let nearest = kfs[0].value;
  for (const k of kfs) {
    if (k.frame <= rel) nearest = k.value;
    else break;
  }
  const rec = nearest as unknown as Record<string, number>;
  if (arity === 2) return [rec.a ?? rec.x ?? 0, rec.b ?? rec.y ?? 0];
  return [rec.top ?? 0, rec.right ?? 0, rec.bottom ?? 0, rec.left ?? 0];
}

// ── Styles ────────────────────────────────────────────────────────────────────

const panelStyle = {
  position: "relative" as const,
  display: "flex",
  flexDirection: "column" as const,
  gap: Spacing.sm,
  borderLeft: `1px solid ${Theme.border.subtle}`,
  paddingLeft: Spacing.md,
  minWidth: 120,
};

const headerStyle = {
  fontSize: FontSize.xxs,
  fontWeight: 600,
  color: Theme.text.muted,
  letterSpacing: 0.6,
};

const rowStyle = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  height: 22,
};

const rowLabelStyle = {
  fontSize: FontSize.xs,
  color: Theme.text.tertiary,
};

const controlsStyle = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.xs,
};

const chevronStyle = {
  background: "transparent",
  border: "none",
  color: Theme.text.tertiary,
  cursor: "pointer",
  fontSize: FontSize.sm,
  padding: 0,
  width: 12,
};

const stampStyle = {
  position: "relative" as const,
  background: "transparent",
  border: "none",
  cursor: "pointer",
  fontSize: FontSize.sm,
  width: 16,
  height: 16,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
};

const hollowDiamondStyle = {
  position: "absolute" as const,
  color: Theme.text.tertiary,
};

const laneStyle = {
  position: "relative" as const,
  height: 16,
  width: LANE_WIDTH,
  flex: "0 0 auto",
  borderLeft: `1px solid ${Theme.border.subtle}`,
  marginLeft: Spacing.xs,
};

const laneDiamondStyle = {
  position: "absolute" as const,
  top: 0,
  transform: "translateX(-50%)",
  fontSize: FontSize.sm,
  lineHeight: "16px",
  cursor: "grab",
  userSelect: "none" as const,
  touchAction: "none" as const,
};

const menuStyle = {
  position: "fixed" as const,
  zIndex: 1000,
  display: "flex",
  flexDirection: "column" as const,
  minWidth: 96,
  padding: Spacing.xs,
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.primary}`,
  borderRadius: 4,
  boxShadow: "0 4px 12px rgba(0, 0, 0, 0.5)",
};

const menuItemStyle = {
  background: "transparent",
  border: "none",
  color: Theme.text.secondary,
  cursor: "pointer",
  fontSize: FontSize.sm,
  textAlign: "left" as const,
  padding: `${Spacing.xs}px ${Spacing.sm}px`,
  borderRadius: 3,
};
