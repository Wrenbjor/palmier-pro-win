// Music tab — a video/text → MUSIC GENERATION form (ruling #14), NOT a `/v1/music`
// sample library. The form is real and validating; Generate is WIRED to the real
// `generate_audio` tool through the editor bridge (controller.generate). When the
// backend is unconfigured / signed-out the tool returns the reference "Sign in to
// Palmier… AI generation is not available" string, which is surfaced inline as a
// gated reason (never a silent no-op). There is deliberately NO browse/audition/drag
// library UI anywhere (FOUNDATION §6.2 void).
//
// Form parity (docs/reference/media-panel.md §"Music tab", MusicTab.swift):
//   Mode    — video-to-music (scores the selected timeline span / whole timeline)
//             OR text-to-music (duration 1–600s at marked-range start or playhead).
//   Models  — `AudioModelConfig` where category == music AND inputs contain video
//             (video-to-music) / text (text-to-music). Filtered list below.
//   Cost    — a stubbed `CostEstimator` (real estimate + credit gate in Epic 9).
//   Gate    — requires sign-in + sufficient credits (surfaced; enforced in Epic 9).

import { useMemo, useState } from "react";
import { Spacing, Theme } from "./theme";
import type { MediaPanelController } from "./controller";
import { inTauri } from "./media-actions";

type Mode = "video" | "text";

/** Mirrors the render-relevant subset of the Rust `AudioModelConfig` (Epic 9). */
interface AudioModelConfig {
  id: string;
  label: string;
  category: "music" | "sfx" | "voice";
  /** Accepted generation inputs. */
  inputs: ("video" | "text" | "audio")[];
  /** Approx credits per second of output (drives the stub cost estimate). */
  creditsPerSecond: number;
}

// A representative model catalog. Epic 9 replaces this with the real `/v1/models`
// `AudioModelConfig` list; the panel only ever shows music models (ruling #14).
const MODELS: AudioModelConfig[] = [
  { id: "score-v1", label: "Score v1 (video→music)", category: "music", inputs: ["video"], creditsPerSecond: 4 },
  { id: "score-v2", label: "Score v2 (video+text)", category: "music", inputs: ["video", "text"], creditsPerSecond: 6 },
  { id: "compose-v1", label: "Compose v1 (text→music)", category: "music", inputs: ["text"], creditsPerSecond: 3 },
  // Non-music / non-video models are filtered OUT below (never shown).
  { id: "sfx-v1", label: "SFX v1", category: "sfx", inputs: ["text"], creditsPerSecond: 2 },
];

/** Stubbed CostEstimator (Epic 9 replaces with the real estimator). */
function estimateCredits(model: AudioModelConfig | undefined, seconds: number): number {
  if (!model) return 0;
  return Math.ceil(model.creditsPerSecond * Math.max(0, seconds));
}

export interface MusicTabProps {
  /**
   * The media-panel controller (Generate dispatches `generate_audio` through it).
   * When absent (standalone preview) the form renders but Generate stays a "not
   * connected" gated state.
   */
  controller?: MediaPanelController;
}

/** The Generate button's live state — drives label + styling + disabled. */
type GenerateState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "submitted" }
  | { kind: "gated"; reason: string };

export function MusicTab({ controller }: MusicTabProps = {}) {
  const [mode, setMode] = useState<Mode>("video");
  const [prompt, setPrompt] = useState("");
  const [duration, setDuration] = useState(30);
  const [genState, setGenState] = useState<GenerateState>({ kind: "idle" });

  // Models filtered to category==music AND the input the current mode needs
  // (video-to-music needs video input; text-to-music needs text input).
  const models = useMemo(
    () =>
      MODELS.filter(
        (m) =>
          m.category === "music" &&
          m.inputs.includes(mode === "video" ? "video" : "text"),
      ),
    [mode],
  );
  const [modelId, setModelId] = useState(models[0]?.id ?? "");
  // Keep the selected model valid as the mode (and thus the list) changes.
  const selectedModel =
    models.find((m) => m.id === modelId) ?? models[0];

  // video-to-music scores the marked span (whole timeline when nothing marked);
  // text-to-music uses the explicit duration. The cost estimate uses that length.
  const estimatedSeconds = mode === "text" ? duration : 60; // span length unknown here → nominal
  const cost = estimateCredits(selectedModel, estimatedSeconds);

  const durationValid = duration >= 1 && duration <= 600;
  const textValid =
    mode === "text" ? prompt.trim().length > 0 && durationValid : true;
  const valid = !!selectedModel && textValid;

  const connected = inTauri() && !!controller;
  const busy = genState.kind === "running";

  // Map the form → the `generate_audio` tool inputSchema args (music category).
  // text-to-music passes the prompt + duration; video-to-music omits the prompt
  // (the model scores the timeline span — the backend resolves the span). `model`
  // is left to the backend's default music model (the stub catalog ids here aren't
  // the live `/v1/models` ids; the executor defaults to the first available model).
  const toGenerateAudioArgs = (): Record<string, unknown> => {
    const args: Record<string, unknown> = {};
    if (mode === "text") {
      args.prompt = prompt.trim();
      args.duration = duration;
    } else {
      // video-to-music: style the score from the prompt if provided; otherwise the
      // model scores the span. styleInstructions is the music-style channel.
      if (prompt.trim().length > 0) args.styleInstructions = prompt.trim();
    }
    return args;
  };

  const onGenerate = async () => {
    if (!controller || busy || !valid) return;
    setGenState({ kind: "running" });
    const res = await controller.generate("generate_audio", toGenerateAudioArgs());
    if (!res.attempted) {
      setGenState({
        kind: "gated",
        reason: "Not connected to the editor — open a project to generate.",
      });
      return;
    }
    if (res.ok) {
      setGenState({ kind: "submitted" });
      return;
    }
    setGenState({
      kind: "gated",
      reason: res.error ?? "AI generation is not available.",
    });
  };

  return (
    <div style={formStyle}>
      <h2 style={headingStyle}>Music</h2>
      <p style={noteStyle}>
        Generate a score from your video or a text prompt. Submit + credit gate land
        in Epic 9. This is a generation form, not a sample library.
      </p>

      <div style={{ display: "flex", gap: Spacing.sm }}>
        {(["video", "text"] as const).map((m) => (
          <button
            key={m}
            onClick={() => {
              setMode(m);
              // reset the model to a valid one for the new mode's filtered list
              const next = MODELS.find(
                (x) =>
                  x.category === "music" &&
                  x.inputs.includes(m === "video" ? "video" : "text"),
              );
              if (next) setModelId(next.id);
            }}
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

      <label style={fieldStyle}>
        <span style={labelStyle}>Model</span>
        <select
          value={selectedModel?.id ?? ""}
          onChange={(e) => setModelId(e.target.value)}
          style={inputStyle}
        >
          {models.map((m) => (
            <option key={m.id} value={m.id}>
              {m.label}
            </option>
          ))}
        </select>
      </label>

      {mode === "video" ? (
        <p style={noteStyle}>
          Scores the selected timeline span (or the whole timeline if nothing is
          marked). Models are filtered to music-capable audio models that accept
          video input.
        </p>
      ) : (
        <>
          <label style={fieldStyle}>
            <span style={labelStyle}>Prompt</span>
            <textarea
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              rows={3}
              placeholder="uplifting orchestral build…"
              style={{ ...inputStyle, resize: "vertical" }}
            />
          </label>
          <label style={fieldStyle}>
            <span style={labelStyle}>Duration (1–600s)</span>
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

      {/* Stubbed CostEstimator (Epic 9 replaces with the real estimate + gate). */}
      <div style={costRowStyle}>
        <span>Estimated cost</span>
        <span style={{ color: Theme.text.primary, fontWeight: 600 }}>
          ~{cost} credits
        </span>
      </div>
      <p style={agentHintStyle}>
        Requires sign-in to a plan with credits. If you are signed out, Generate
        explains how to enable it.
      </p>

      <button
        disabled={!valid || busy || !connected}
        onClick={() => void onGenerate()}
        title={
          !connected
            ? "Open a project to generate music"
            : !valid
              ? "Enter a prompt / valid duration"
              : "Generate a score"
        }
        style={{
          ...(connected && valid && !busy
            ? generateEnabledStyle
            : generateDisabledStyle),
          opacity: valid ? 1 : 0.6,
        }}
      >
        {busy
          ? "Submitting…"
          : !connected
            ? "Generate (open a project)"
            : "Generate music"}
      </button>

      {genState.kind === "submitted" && (
        <p style={{ ...statusNoteStyle, color: Theme.accent }}>
          Generating — the new score appears in your library when ready.
        </p>
      )}
      {genState.kind === "gated" && (
        <p style={{ ...statusNoteStyle, color: Theme.status.error }}>
          {genState.reason}
        </p>
      )}
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
const labelStyle = { fontSize: 11, color: Theme.text.muted } as const;
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
const agentHintStyle = {
  fontSize: 10,
  color: Theme.text.tertiary,
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
const costRowStyle = {
  display: "flex",
  justifyContent: "space-between",
  alignItems: "center",
  fontSize: 12,
  color: Theme.text.secondary,
  padding: `${Spacing.sm}px ${Spacing.md}px`,
  borderRadius: 6,
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.subtle}`,
} as const;
const generateDisabledStyle = {
  fontSize: 12,
  padding: "7px 10px",
  borderRadius: 6,
  marginTop: Spacing.sm,
  color: Theme.text.muted,
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.subtle}`,
  cursor: "not-allowed",
} as const;
const generateEnabledStyle = {
  fontSize: 12,
  padding: "7px 10px",
  borderRadius: 6,
  marginTop: Spacing.sm,
  fontWeight: 600,
  color: "#000",
  background: Theme.accent,
  border: "none",
  cursor: "pointer",
} as const;
const statusNoteStyle = {
  fontSize: 11,
  margin: 0,
} as const;
