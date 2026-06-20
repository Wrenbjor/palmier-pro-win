// Captions tab — the full caption-generation form (E4-S14). The form is real and
// validating; the GENERATION backend (`generate_captions` + `CaptionBuilder`) lands
// in Epic 10, so Generate stays disabled with "backend not available".
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

export function CaptionsTab() {
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

  const onGenerate = () => {
    // TODO(E10): await invoke('generate_captions', { request }); real CaptionBuilder.
    void request;
  };

  return (
    <div style={formStyle}>
      <h2 style={headingStyle}>Captions</h2>
      <p style={noteStyle}>
        Transcribe and place captions. Generation lands in Epic 10; the form below is
        live.
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
        disabled
        onClick={onGenerate}
        title="Caption generation lands in Epic 10"
        style={{ ...generateDisabledStyle, opacity: valid ? 1 : 0.6 }}
      >
        Generate (backend not available)
      </button>

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
