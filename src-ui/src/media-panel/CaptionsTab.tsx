// Captions tab — minimal form shell so the rail (E4-S8) has a Captions body.
//
// The FULL captions form (Source / Language / Style / Placement + the
// `generate_captions` command) is E4-S14, wired to its real backend in Epic 10.
// This shell renders the form scaffold and a disabled Generate ("backend not
// available") so the rail switches to a real surface, not a blank panel.

import { useState } from "react";
import { Spacing, Theme } from "./theme";

export function CaptionsTab() {
  const [source, setSource] = useState("auto");
  const [language, setLanguage] = useState("auto");
  // case = auto/upper/lower only (ruling #18 — no title-case)
  const [caseMode, setCaseMode] = useState<"auto" | "upper" | "lower">("auto");

  return (
    <div style={formStyle}>
      <h2 style={headingStyle}>Captions</h2>
      <p style={noteStyle}>
        Transcribe and place captions. Generation lands in Epic 10 (E4-S14 wires
        the full form).
      </p>

      <Field label="Source">
        <select
          value={source}
          onChange={(e) => setSource(e.target.value)}
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
          {/* TODO(E10): Whisper-equivalent of Transcription.supportedLocales(). */}
          <option value="auto">Auto-detect</option>
          <option value="en">English</option>
          <option value="es">Spanish</option>
          <option value="fr">French</option>
        </select>
      </Field>

      <Field label="Case">
        <div style={{ display: "flex", gap: Spacing.sm }}>
          {(["auto", "upper", "lower"] as const).map((c) => (
            <button
              key={c}
              onClick={() => setCaseMode(c)}
              style={{
                ...inputStyle,
                cursor: "pointer",
                background:
                  caseMode === c ? Theme.accent : Theme.background.base,
                color: caseMode === c ? "#000" : Theme.text.secondary,
              }}
            >
              {c}
            </button>
          ))}
        </div>
      </Field>

      <button disabled title="Caption generation lands in Epic 10" style={generateDisabledStyle}>
        Generate (backend not available)
      </button>
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
  cursor: "not-allowed",
} as const;
