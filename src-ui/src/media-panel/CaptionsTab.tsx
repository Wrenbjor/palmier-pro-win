// Captions tab — the full caption-generation form (E4-S14). The form is real and
// validating; Generate is WIRED to the real `add_captions` tool through the editor
// bridge (controller.generateCaptions) — on-device transcription then styled caption
// clips on a new track. When transcription isn't possible (no speech / model missing
// / unsupported language) the tool returns a reason, surfaced inline (no silent
// no-op). Outside Tauri (design preview) Generate is inert with a "not connected"
// hint.
//
// Form parity (docs/reference/media-panel.md §"Captions tab", CaptionTab.swift):
//   Source     — auto (selected clips else all captionable audio) | pick a track
//   Language   — Auto-detect + a locale list (Whisper-equivalent of
//                Transcription.supportedLocales(); the real list is sourced in Epic 10)
//   Style      — font / size / color / background / case / profanity-censor.
//                CASE = auto/upper/lower ONLY (ruling #18 — no title-case).
//   Placement  — center X / center Y (0..1) with center-snap guides + threshold.
// Agent-mode hands a prompt draft to the agent panel (no compute) — surfaced as a
// hint here; the agent-panel handoff is wired with the agent surface.

import { useMemo, useState } from "react";
import { Spacing, Theme } from "./theme";
import type { MediaPanelController } from "./controller";
import { inTauri } from "./media-actions";

/** A locale option for the language picker. */
interface Locale {
  code: string;
  label: string;
}

// A representative subset of the Whisper language set. Epic 10 replaces this with the
// real `Transcription.supportedLocales()` equivalent sourced from palmier-transcribe.
const LOCALES: Locale[] = [
  { code: "en", label: "English" },
  { code: "es", label: "Spanish" },
  { code: "fr", label: "French" },
  { code: "de", label: "German" },
  { code: "it", label: "Italian" },
  { code: "pt", label: "Portuguese" },
  { code: "ja", label: "Japanese" },
  { code: "ko", label: "Korean" },
  { code: "zh", label: "Chinese" },
];

const FONTS = ["Inter", "Arial", "Helvetica", "Georgia", "Courier New"];

// AppTheme.Caption constants (parity with crates/palmier-text caption_theme +
// docs/reference/transcription.md §Constants). Carried verbatim so the form's
// placement/style bounds match the engine.
const CAPTION = {
  defaultFontSize: 48,
  minFontSize: 12,
  maxFontSize: 300,
  minPosition: 0,
  maxPosition: 1,
  centerSnapValue: 0.5,
  centerSnapThreshold: 0.02,
  defaultCenter: { x: 0.5, y: 0.9 },
} as const;

/** Center-snap: placement snaps to `centerSnapValue` within `centerSnapThreshold`. */
function snapToCenter(v: number): number {
  return Math.abs(v - CAPTION.centerSnapValue) <= CAPTION.centerSnapThreshold
    ? CAPTION.centerSnapValue
    : v;
}

/** Mirrors the Rust `CaptionRequest` the Epic-10 `generate_captions` command takes. */
export interface CaptionRequest {
  source: "auto" | "track";
  language: string; // "auto" or a locale code
  style: {
    font: string;
    size: number;
    color: string;
    background: string;
    case: "auto" | "upper" | "lower";
    censorProfanity: boolean;
  };
  placement: { centerX: number; centerY: number };
}

// Map the form's CaptionRequest → the `add_captions` tool inputSchema args
// (crates/palmier-tools schema_add_captions). `source:"auto"` omits clipIds so the
// tool auto-picks the primary spoken track; `language:"auto"` omits language so it
// uses the system default. The track-pick + multi-clip selection threads in when
// the tab shares the editor's clip selection — today auto-detect is the path.
// Exported as a pure function so the parity checks can assert the exact wire shape
// against the Rust `add_captions` arg names without rendering the component.
export function toAddCaptionsArgs(r: CaptionRequest): Record<string, unknown> {
  const args: Record<string, unknown> = {
    fontName: r.style.font,
    fontSize: r.style.size,
    color: r.style.color,
    centerX: r.placement.centerX,
    centerY: r.placement.centerY,
    textCase: r.style.case,
    censorProfanity: r.style.censorProfanity,
  };
  if (r.language !== "auto") args.language = r.language;
  return args;
}

export interface CaptionsTabProps {
  /**
   * The media-panel controller (Generate dispatches `add_captions` through it). When
   * absent (standalone preview) the form renders but Generate stays a "not connected"
   * gated state.
   */
  controller?: MediaPanelController;
}

/** The Generate button's live state — drives label + styling + disabled. */
type GenerateState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done" }
  | { kind: "gated"; reason: string };

export function CaptionsTab({ controller }: CaptionsTabProps = {}) {
  const [source, setSource] = useState<"auto" | "track">("auto");
  const [language, setLanguage] = useState("auto");
  const [font, setFont] = useState(FONTS[0]);
  const [size, setSize] = useState<number>(CAPTION.defaultFontSize);
  const [color, setColor] = useState("#ffffff");
  const [background, setBackground] = useState("#000000");
  // case = auto/upper/lower only (ruling #18 — no title-case).
  const [caseMode, setCaseMode] = useState<"auto" | "upper" | "lower">("auto");
  const [censor, setCensor] = useState(false);
  const [centerX, setCenterX] = useState<number>(CAPTION.defaultCenter.x);
  const [centerY, setCenterY] = useState<number>(CAPTION.defaultCenter.y);
  const [genState, setGenState] = useState<GenerateState>({ kind: "idle" });

  const request: CaptionRequest = useMemo(
    () => ({
      source,
      language,
      style: { font, size, color, background, case: caseMode, censorProfanity: censor },
      placement: { centerX, centerY },
    }),
    [source, language, font, size, color, background, caseMode, censor, centerX, centerY],
  );

  // Form is valid when size is within the AppTheme.Caption clamp and placement is
  // within the normalized frame `[minPosition, maxPosition]`.
  const valid =
    size >= CAPTION.minFontSize &&
    size <= CAPTION.maxFontSize &&
    centerX >= CAPTION.minPosition &&
    centerX <= CAPTION.maxPosition &&
    centerY >= CAPTION.minPosition &&
    centerY <= CAPTION.maxPosition;

  const connected = inTauri() && !!controller;
  const busy = genState.kind === "running";

  const onGenerate = async () => {
    if (!controller || busy) return;
    setGenState({ kind: "running" });
    const res = await controller.generateCaptions(toAddCaptionsArgs(request));
    if (!res.attempted) {
      // Outside Tauri — design preview. Honest "not connected" hint, not an error.
      setGenState({
        kind: "gated",
        reason: "Not connected to the editor — open a project to caption.",
      });
      return;
    }
    if (res.ok) {
      setGenState({ kind: "done" });
      return;
    }
    setGenState({
      kind: "gated",
      reason:
        res.error ??
        "Couldn't transcribe: no speech found, or the on-device model isn't available.",
    });
  };

  return (
    <div style={formStyle}>
      <h2 style={headingStyle}>Captions</h2>
      <p style={noteStyle}>
        Transcribe on-device and place styled caption clips on a new track.
      </p>

      <Field label="Source">
        <select
          value={source}
          onChange={(e) => setSource(e.target.value as "auto" | "track")}
          style={inputStyle}
        >
          <option value="auto">Auto (selected clips or all audio)</option>
          <option value="track">Pick a track…</option>
        </select>
      </Field>

      <Field label="Language">
        <select
          value={language}
          onChange={(e) => setLanguage(e.target.value)}
          style={inputStyle}
        >
          <option value="auto">Auto-detect</option>
          {LOCALES.map((l) => (
            <option key={l.code} value={l.code}>
              {l.label}
            </option>
          ))}
        </select>
      </Field>

      <SectionLabel>Style</SectionLabel>

      <Field label="Font">
        <select value={font} onChange={(e) => setFont(e.target.value)} style={inputStyle}>
          {FONTS.map((f) => (
            <option key={f} value={f}>
              {f}
            </option>
          ))}
        </select>
      </Field>

      <Field label={`Size (${size}px)`}>
        <input
          type="range"
          min={CAPTION.minFontSize}
          max={CAPTION.maxFontSize}
          value={size}
          onChange={(e) => setSize(Number(e.target.value))}
        />
      </Field>

      <div style={{ display: "flex", gap: Spacing.md }}>
        <Field label="Text color">
          <input
            type="color"
            value={color}
            onChange={(e) => setColor(e.target.value)}
            style={colorInputStyle}
          />
        </Field>
        <Field label="Background">
          <input
            type="color"
            value={background}
            onChange={(e) => setBackground(e.target.value)}
            style={colorInputStyle}
          />
        </Field>
      </div>

      <Field label="Case">
        <div style={{ display: "flex", gap: Spacing.sm }}>
          {(["auto", "upper", "lower"] as const).map((c) => (
            <button
              key={c}
              onClick={() => setCaseMode(c)}
              style={{
                ...inputStyle,
                cursor: "pointer",
                background: caseMode === c ? Theme.accent : Theme.background.base,
                color: caseMode === c ? "#000" : Theme.text.secondary,
              }}
            >
              {c}
            </button>
          ))}
        </div>
      </Field>

      <label style={checkboxRowStyle}>
        <input
          type="checkbox"
          checked={censor}
          onChange={(e) => setCensor(e.target.checked)}
        />
        <span>Censor profanity</span>
      </label>

      <SectionLabel>Placement</SectionLabel>

      <Field
        label={`Center X (${centerX.toFixed(2)}${centerX === CAPTION.centerSnapValue ? " · snapped" : ""})`}
      >
        <input
          type="range"
          min={CAPTION.minPosition}
          max={CAPTION.maxPosition}
          step={0.01}
          value={centerX}
          onChange={(e) => setCenterX(snapToCenter(Number(e.target.value)))}
        />
      </Field>
      <Field
        label={`Center Y (${centerY.toFixed(2)}${centerY === CAPTION.centerSnapValue ? " · snapped" : ""})`}
      >
        <input
          type="range"
          min={CAPTION.minPosition}
          max={CAPTION.maxPosition}
          step={0.01}
          value={centerY}
          onChange={(e) => setCenterY(snapToCenter(Number(e.target.value)))}
        />
      </Field>

      <button
        disabled={!valid || busy || !connected}
        onClick={() => void onGenerate()}
        title={
          !connected
            ? "Open a project to generate captions"
            : !valid
              ? "Adjust size / placement to a valid range"
              : "Transcribe and add caption clips"
        }
        style={{
          ...(connected && valid && !busy
            ? generateEnabledStyle
            : generateDisabledStyle),
          opacity: valid ? 1 : 0.6,
        }}
      >
        {busy
          ? "Generating captions…"
          : !connected
            ? "Generate (open a project)"
            : "Generate captions"}
      </button>

      {genState.kind === "done" && (
        <p style={{ ...statusNoteStyle, color: Theme.accent }}>
          Captions added to a new track.
        </p>
      )}
      {genState.kind === "gated" && (
        <p style={{ ...statusNoteStyle, color: Theme.status.error }}>
          {genState.reason}
        </p>
      )}

      <p style={agentHintStyle}>
        Tip: agent-mode can refine captions (remove fillers, fix names, translate) —
        it hands a prompt draft to the agent panel.
      </p>
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label style={{ display: "flex", flexDirection: "column", gap: 4 }}>
      <span style={{ fontSize: 11, color: Theme.text.muted }}>{label}</span>
      {children}
    </label>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <div
      style={{
        fontSize: 11,
        fontWeight: 600,
        color: Theme.text.secondary,
        marginTop: Spacing.sm,
        textTransform: "uppercase",
        letterSpacing: 0.4,
      }}
    >
      {children}
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
  marginTop: Spacing.xs,
} as const;
const inputStyle = {
  fontSize: 12,
  color: Theme.text.primary,
  background: Theme.background.base,
  border: `1px solid ${Theme.border.primary}`,
  borderRadius: 6,
  padding: "5px 8px",
} as const;
const colorInputStyle = {
  width: 48,
  height: 28,
  padding: 0,
  border: `1px solid ${Theme.border.primary}`,
  borderRadius: 6,
  background: "transparent",
  cursor: "pointer",
} as const;
const checkboxRowStyle = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
  fontSize: 12,
  color: Theme.text.secondary,
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
