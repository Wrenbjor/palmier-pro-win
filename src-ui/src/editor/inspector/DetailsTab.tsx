// Inspector DETAILS (Source) + AI EDIT tab bodies (E12-S7).
//
// DetailsTab — read-only metadata for a selected media asset: identity header
// (name + "AI" badge if generated), a File section (Type, Dimensions, Duration,
// Size, Path middle-truncated), and (for generated assets) References / Generated /
// Prompt sections.
//
// AIEditTab — the AI-Edit controls, gated on account (`isMisconfigured`) and on the
// selection being a single AI-eligible visual clip OR a visual media asset.
//
// CAVEAT (documented): `MediaAssetView` in this view-model carries only
// `{ id, isVisual }` — it does NOT yet carry name / type / dimensions / duration /
// size / path / generation metadata. So DetailsTab renders the fields it CAN derive
// and shows placeholders for the rest; full population awaits the media view-model
// (Epic 7/8 adapter work). AI-Edit actions route through `palmier-gen` / the
// generate/upscale tools (Epic 9) which are not reachable from this story's
// frontend-only scope — the controls render with their availability/disabled state.

import type { JSX } from "react";
import { middleTruncate } from "./logic";
import { formatBytes } from "./bodyLogic";
import { Section, TextButton, ToggleRow } from "./sections";
import { FontSize, Spacing, Theme } from "./theme";

/** The richer asset detail the Details tab WANTS — optional until the view-model carries it. */
export interface AssetDetail {
  id: string;
  isVisual: boolean;
  name?: string;
  /** "video" | "image" | "audio" | … */
  type?: string;
  width?: number;
  height?: number;
  durationSeconds?: number;
  sizeBytes?: number;
  path?: string;
  isGenerated?: boolean;
  generatedModel?: string;
  generatedAspect?: string;
  generatedResolution?: string;
  prompt?: string;
}

export interface DetailsTabProps {
  asset: AssetDetail;
  /** Copy text to the clipboard (defaults to the navigator clipboard). */
  onCopyPrompt?: (text: string) => void;
}

export function DetailsTab(props: DetailsTabProps): JSX.Element {
  const { asset } = props;
  const isAudio = asset.type === "audio";
  const isImage = asset.type === "image";

  function copyPrompt(): void {
    if (!asset.prompt) return;
    if (props.onCopyPrompt) props.onCopyPrompt(asset.prompt);
    else void navigator.clipboard?.writeText(asset.prompt);
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: Spacing.xl }}>
      <div style={identityStyle}>
        <span style={identityNameStyle}>{asset.name ?? "Unnamed asset"}</span>
        {asset.isGenerated && <span style={aiBadgeStyle}>AI</span>}
      </div>

      <Section title="File">
        <DetailRow label="Type" value={asset.type ?? "—"} />
        {!isAudio && (
          <DetailRow
            label="Dimensions"
            value={asset.width && asset.height ? `${asset.width} × ${asset.height}` : "—"}
          />
        )}
        {asset.durationSeconds != null && asset.durationSeconds > 0 && !isImage && (
          <DetailRow label="Duration" value={`${asset.durationSeconds.toFixed(2)} s`} />
        )}
        <DetailRow
          label="Size"
          value={asset.sizeBytes != null ? formatBytes(asset.sizeBytes) : "—"}
        />
        <DetailRow
          label="Path"
          value={asset.path ? middleTruncate(asset.path, 44) : "—"}
          title={asset.path}
        />
      </Section>

      {asset.isGenerated && (
        <Section title="Generated">
          <DetailRow label="Model" value={asset.generatedModel ?? "—"} />
          <DetailRow label="Aspect Ratio" value={asset.generatedAspect ?? "—"} />
          <DetailRow label="Resolution" value={asset.generatedResolution ?? "—"} />
          {asset.prompt && (
            <div style={{ display: "flex", flexDirection: "column", gap: Spacing.xs }}>
              <span style={detailLabelStyle}>Prompt</span>
              <div style={promptStyle}>{asset.prompt}</div>
              <TextButton label="Copy prompt" onClick={copyPrompt} />
            </div>
          )}
        </Section>
      )}
    </div>
  );
}

// ── AI Edit tab ──────────────────────────────────────────────────────────────

export interface AIAction {
  id: string;
  label: string;
  /** Disabled reason; undefined → enabled. */
  disabledReason?: string;
  onRun?: () => void;
}

export interface AIEditTabProps {
  /** Account misconfigured → AI Edit hidden/disabled with the reason text. */
  isMisconfigured: boolean;
  /** Whether the context is a clip (vs a media asset) — drives the scope toggles. */
  hasClipContext: boolean;
  /** Whether "Use trimmed portion only" applies (trimStart>0 || trimEnd>0). */
  trimmed?: boolean;
  /** The available actions (Enhance / Edit / Rerun / Create Video / Audio …). */
  actions: AIAction[];
}

export function AIEditTab(props: AIEditTabProps): JSX.Element {
  if (props.isMisconfigured) {
    return (
      <div style={disabledNoticeStyle}>
        AI editing is unavailable — sign in to Palmier and configure your account.
      </div>
    );
  }
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: Spacing.xl }}>
      {props.hasClipContext && (
        <Section title="Scope">
          <ToggleRow label="Replace clip source" value={true} onToggle={() => {}} />
          {props.trimmed && (
            <ToggleRow label="Use trimmed portion only" value={false} onToggle={() => {}} />
          )}
        </Section>
      )}
      <Section title="AI">
        {props.actions.map((a) => (
          <div key={a.id} style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <TextButton
              label={a.label}
              disabled={!!a.disabledReason}
              onClick={() => a.onRun?.()}
            />
            {a.disabledReason && <span style={reasonStyle}>{a.disabledReason}</span>}
          </div>
        ))}
        {props.actions.length === 0 && (
          <div style={disabledNoticeStyle}>No AI actions available for this selection.</div>
        )}
      </Section>
    </div>
  );
}

function DetailRow(props: { label: string; value: string; title?: string }): JSX.Element {
  return (
    <div style={detailRowStyle}>
      <span style={detailLabelStyle}>{props.label}</span>
      <span style={detailValueStyle} title={props.title}>
        {props.value}
      </span>
    </div>
  );
}

// ── Styles ────────────────────────────────────────────────────────────────────

const identityStyle = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
} as const;

const identityNameStyle = {
  fontSize: FontSize.md,
  fontWeight: 600,
  color: Theme.text.primary,
} as const;

const aiBadgeStyle = {
  fontSize: FontSize.xxs,
  fontWeight: 700,
  color: Theme.background.base,
  background: Theme.accentTimecode,
  borderRadius: 3,
  padding: "1px 4px",
} as const;

const detailRowStyle = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
} as const;

const detailLabelStyle = {
  fontSize: FontSize.xs,
  color: Theme.text.tertiary,
  flexShrink: 0,
} as const;

const detailValueStyle = {
  fontSize: FontSize.xs,
  color: Theme.text.secondary,
  marginLeft: "auto",
  textAlign: "right" as const,
  whiteSpace: "nowrap" as const,
  overflow: "hidden",
  textOverflow: "ellipsis",
};

const promptStyle = {
  fontSize: FontSize.xs,
  color: Theme.text.secondary,
  background: Theme.background.raised,
  border: `1px solid ${Theme.border.subtle}`,
  borderRadius: 4,
  padding: Spacing.sm,
  whiteSpace: "pre-wrap" as const,
};

const disabledNoticeStyle = {
  fontSize: FontSize.xs,
  color: Theme.text.muted,
  padding: Spacing.sm,
} as const;

const reasonStyle = {
  fontSize: FontSize.xxs,
  color: Theme.text.muted,
} as const;
