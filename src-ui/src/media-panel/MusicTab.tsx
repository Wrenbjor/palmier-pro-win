// Music tab — a video/text → MUSIC GENERATION form (ruling #14), NOT a `/v1/music`
// sample library. This is a minimal shell so the rail (E4-S8) has a Music body; the
// full form + credit gate + submit land in Epic 9 (E4-S14 wires it).
//
// Modes: video-to-music (scores the selected timeline span / whole timeline) OR
// text-to-music (duration 1–600s placed at marked-range start or playhead). There is
// deliberately NO browse/audition/drag library UI anywhere (FOUNDATION §6.2 line void).

import { useState } from "react";
import { Spacing, Theme } from "./theme";

type Mode = "video" | "text";

export function MusicTab() {
  const [mode, setMode] = useState<Mode>("video");
  const [prompt, setPrompt] = useState("");
  const [duration, setDuration] = useState(30);

  const durationValid = duration >= 1 && duration <= 600;
  const textValid = mode === "text" ? prompt.trim().length > 0 && durationValid : true;

  return (
    <div style={formStyle}>
      <h2 style={headingStyle}>Music</h2>
      <p style={noteStyle}>
        Generate a score from your video or a text prompt. Submit + credit gate land
        in Epic 9 (E4-S14 wires the full form). This is a generation form, not a
        sample library.
      </p>

      <div style={{ display: "flex", gap: Spacing.sm }}>
        {(["video", "text"] as const).map((m) => (
          <button
            key={m}
            onClick={() => setMode(m)}
            style={{
              ...inputStyle,
              cursor: "pointer",
              background: mode === m ? Theme.accent : Theme.background.base,
              color: mode === m ? "#000" : Theme.text.secondary,
            }}
          >
            {m === "video" ? "Video → Music" : "Text → Music"}
          </button>
        ))}
      </div>

      {mode === "video" ? (
        <p style={noteStyle}>
          Scores the selected timeline span (or the whole timeline if nothing is
          marked). Models filtered to music-capable audio models that accept video
          input.
        </p>
      ) : (
        <>
          <label style={fieldStyle}>
            <span style={{ fontSize: 11, color: Theme.text.muted }}>Prompt</span>
            <textarea
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              rows={3}
              placeholder="uplifting orchestral build…"
              style={{ ...inputStyle, resize: "vertical" }}
            />
          </label>
          <label style={fieldStyle}>
            <span style={{ fontSize: 11, color: Theme.text.muted }}>
              Duration (1–600s)
            </span>
            <input
              type="number"
              min={1}
              max={600}
              value={duration}
              onChange={(e) => setDuration(Number(e.target.value))}
              style={{
                ...inputStyle,
                borderColor: durationValid
                  ? Theme.border.primary
                  : Theme.status.error,
              }}
            />
          </label>
        </>
      )}

      {/* TODO(E9): real CostEstimator + credit/sign-in gate + submit command. */}
      <button
        disabled={!textValid}
        title="Music generation lands in Epic 9"
        style={{
          ...generateDisabledStyle,
          cursor: textValid ? "not-allowed" : "not-allowed",
        }}
      >
        Generate (backend not available)
      </button>
    </div>
  );
}

const formStyle = {
  display: "flex",
  flexDirection: "column",
  gap: Spacing.md,
  padding: Spacing.lg,
  overflowY: "auto",
} as const;
const fieldStyle = {
  display: "flex",
  flexDirection: "column",
  gap: 4,
} as const;
const headingStyle = {
  fontSize: 14,
  fontWeight: 600,
  color: Theme.text.primary,
  margin: 0,
} as const;
const noteStyle = {
  fontSize: 11,
  color: Theme.text.muted,
  margin: 0,
} as const;
const inputStyle = {
  fontSize: 12,
  color: Theme.text.primary,
  background: Theme.background.base,
  border: `1px solid ${Theme.border.primary}`,
  borderRadius: 6,
  padding: "5px 8px",
} as const;
const generateDisabledStyle = {
  fontSize: 12,
  padding: "7px 10px",
  borderRadius: 6,
  marginTop: Spacing.sm,
  color: Theme.text.muted,
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.subtle}`,
} as const;
