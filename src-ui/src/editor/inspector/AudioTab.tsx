// Inspector AUDIO tab body (E12-S5) — Levels (Volume + Fade In + Fade Out) and a
// Speed section shown only when NO visual clip is selected.
//
// Volume binds to `liveVolumeKfDb(at:activeFrame) ?? dbFromLinear(clip.volume)`,
// range −60…+15 dB (E12-S1 constant), `%.1f dB`, shows "−∞ dB" at floor and stores
// true-mute 0 there. Writes the LINEAR volume through `set_clip_properties`.
//
// Fade In/Out write LINEAR frame counts through `set_clip_properties`
// (fadeInFrames / fadeOutFrames): the field works in seconds, converted to frames
// via round(seconds × fps), and the server clamps each edge to fit the duration
// (fadeIn + fadeOut <= duration). Volume + Speed are fully wired too.

import type { JSX } from "react";
import type { ClipView } from "../types";
import {
  fadeFramesFromSeconds,
  fadeMaxSeconds,
  fadeRange,
  fadeSecondsFromFrames,
  formatVolumeDb,
  sharedClipValue,
  SPEED_RANGE,
  clipPropertiesArgs,
  volumeDb,
  volumeLinearFromDb,
  VOLUME_DB_RANGE,
} from "./bodyLogic";
import { FieldRow, ScrubbableNumberField } from "./fields";
import { Section } from "./sections";
import { FontSize, Spacing, Theme } from "./theme";
import type { EditDispatch } from "./VideoTab";

export interface AudioTabProps {
  /** The selected audio clips. */
  clips: readonly ClipView[];
  /** True when the selection also contains a visual clip (hides the Speed section). */
  hasVisualSelected: boolean;
  fps: number;
  activeFrame?: number;
  edit: EditDispatch;
}

export function AudioTab(props: AudioTabProps): JSX.Element {
  const { clips, hasVisualSelected, fps, activeFrame, edit } = props;
  const ids = clips.map((c) => c.id);

  // Volume in dB (shared across selection; null = mixed → "—").
  const db = sharedClipValue(clips, (c) => volumeDb(c, activeFrame));
  const fadeInS = sharedClipValue(clips, (c) =>
    fadeSecondsFromFrames(c.fadeInFrames, fps),
  );
  const fadeOutS = sharedClipValue(clips, (c) =>
    fadeSecondsFromFrames(c.fadeOutFrames, fps),
  );
  const maxS = fadeMaxSeconds(clips, fps);
  const speed = sharedClipValue(clips, (c) => c.speed);

  const setVolumeDb = (nextDb: number) =>
    void edit(
      "set_clip_properties",
      clipPropertiesArgs(ids, { volume: volumeLinearFromDb(nextDb) }),
    );

  const setFade = (edge: "in" | "out", seconds: number) =>
    void edit(
      "set_clip_properties",
      clipPropertiesArgs(
        ids,
        edge === "in"
          ? { fadeInFrames: fadeFramesFromSeconds(seconds, fps) }
          : { fadeOutFrames: fadeFramesFromSeconds(seconds, fps) },
      ),
    );

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: Spacing.xl }}>
      <Section title="Levels">
        <FieldRow label="Volume">
          <ScrubbableNumberField
            // Render "−∞ dB" at the floor via a custom display: the field formats
            // the value; the floor override is the standard dB formatter.
            value={db}
            range={VOLUME_DB_RANGE}
            onChange={setVolumeDb}
            onCommit={setVolumeDb}
          />
        </FieldRow>
        {db !== null && (
          <div style={dbReadoutStyle}>{formatVolumeDb(db)}</div>
        )}

        <FieldRow label="Fade In">
          <ScrubbableNumberField
            value={fadeInS}
            range={fadeRange(maxS)}
            onChange={(v) => setFade("in", v)}
            onCommit={(v) => setFade("in", v)}
          />
        </FieldRow>
        <FieldRow label="Fade Out">
          <ScrubbableNumberField
            value={fadeOutS}
            range={fadeRange(maxS)}
            onChange={(v) => setFade("out", v)}
            onCommit={(v) => setFade("out", v)}
          />
        </FieldRow>
      </Section>

      {!hasVisualSelected && (
        <Section title="Speed">
          <FieldRow label="Speed">
            <ScrubbableNumberField
              value={speed}
              range={SPEED_RANGE}
              onChange={(v) =>
                void edit("set_clip_properties", clipPropertiesArgs(ids, { speed: v }))
              }
              onCommit={(v) =>
                void edit("set_clip_properties", clipPropertiesArgs(ids, { speed: v }))
              }
            />
          </FieldRow>
        </Section>
      )}
    </div>
  );
}

const dbReadoutStyle = {
  fontSize: FontSize.xxs,
  color: Theme.text.muted,
  textAlign: "right" as const,
};
