// Inspector KEYFRAMES side panel (E12-S8) — per-property stamp/navigate controls
// for a SINGLE selected clip.
//
// Per animatable row: [prev-kf chevron][diamond stamp][next-kf chevron]. The stamp
// is a FILLED diamond when a keyframe exists at the active frame (click removes) and
// HOLLOW otherwise (click adds). Controls are disabled (40%) when the playhead is
// outside the clip; chevrons navigate previous/next keyframe and disable when none.
// Rows: video clip → Position, Scale, Rotation, Opacity, Crop; audio → Volume only.
//
// Ops map to `set_keyframes` (REPLACE the property track): adding/removing rewrites
// the full row list for that property. Frames are CLIP-RELATIVE (the tool's
// contract) — we convert via `keyframeOffset`.
//
// CAVEAT (documented): the reference also supports lane drag-to-move with snap
// (SnapEngine, 4 px) and a context menu for interpolation. The `ClipView`
// view-model exposes only volume/opacity/position/scale/crop tracks (no rotation
// track field) and no per-keyframe interpolation editing surface here — so this
// delivers the stamp + navigate controls (the core add/remove/seek behavior); the
// drag-to-move lane canvas + snap + interpolation menu remain for a follow-up once
// the view-model + a `seek`/snap seam are available.

import type { JSX } from "react";
import type { ClipView, KeyframeTrackView } from "../types";
import {
  hasKeyframeAt,
  keyframeOffset,
  nextKeyframeFrame,
  previousKeyframeFrame,
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

export interface KeyframesPanelProps {
  clip: ClipView;
  /** Absolute playhead frame. */
  activeFrame: number;
  /** Seek the playhead (chevron navigation). */
  onSeek?: (absoluteFrame: number) => void;
  edit: EditDispatch;
}

/** The KeyframeTrackView for a property on the clip (rotation/position/scale have view-model tracks). */
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
    case "crop":
      return clip.cropTrack;
    case "rotation":
      // No rotation track field in the view-model — treated as no keyframes here.
      return null;
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

export function KeyframesPanel(props: KeyframesPanelProps): JSX.Element {
  const { clip, activeFrame, onSeek, edit } = props;
  const isAudio = clip.mediaType === "audio";
  const properties = isAudio ? AUDIO_PROPERTIES : VIDEO_PROPERTIES;

  const clipStart = clip.startFrame;
  const clipEnd = clip.startFrame + clip.durationFrames;
  const insideClip = activeFrame >= clipStart && activeFrame < clipEnd;
  const rel = keyframeOffset(clip, activeFrame);

  return (
    <div style={panelStyle}>
      <div style={headerStyle}>KEYFRAMES</div>
      {properties.map((property) => {
        const track = trackFor(clip, property);
        const present = hasKeyframeAt(track, rel);
        const prev = previousKeyframeFrame(track, rel);
        const next = nextKeyframeFrame(track, rel);

        const toggleStamp = () => {
          if (!insideClip) return;
          const arity = arityFor(property);
          const existing = track?.keyframes ?? [];
          let rows: number[][];
          if (present) {
            // Remove the keyframe at rel.
            rows = existing
              .filter((k) => k.frame !== rel)
              .map((k) => [k.frame, ...toValues(k.value, arity)]);
          } else {
            // Add a keyframe at rel seeded with the sampled/zero value.
            const seed = sampleSeed(track, rel, arity);
            rows = [
              ...existing.map((k) => [k.frame, ...toValues(k.value, arity)]),
              [rel, ...seed],
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
            </div>
          </div>
        );
      })}
    </div>
  );
}

/** Coerce a stored keyframe value (number | pair | crop) into an arity-length number list. */
function toValues(value: unknown, arity: number): number[] {
  if (typeof value === "number") return arity === 1 ? [value] : new Array(arity).fill(value);
  if (value && typeof value === "object") {
    const rec = value as Record<string, number>;
    if (arity === 2) return [rec.a ?? rec.x ?? 0, rec.b ?? rec.y ?? 0];
    if (arity === 4) return [rec.top ?? 0, rec.right ?? 0, rec.bottom ?? 0, rec.left ?? 0];
  }
  return new Array(arity).fill(0);
}

/** Seed values for a NEW keyframe: sample the existing track's nearest value, else zeros. */
function sampleSeed(
  track: KeyframeTrackView | null | undefined,
  rel: number,
  arity: number,
): number[] {
  const kfs = track?.keyframes;
  if (!kfs || kfs.length === 0) return new Array(arity).fill(arity === 1 ? 0 : 0);
  let nearest = kfs[0].value;
  for (const k of kfs) {
    if (k.frame <= rel) nearest = k.value;
    else break;
  }
  return toValues(nearest, arity);
}

// ── Styles ────────────────────────────────────────────────────────────────────

const panelStyle = {
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
