// Generation panel (E4-S11) — cards per active generation job below the media list.
//
// Each card shows thumbnail / prompt / model / status / progress / cancel. Failed
// jobs PERSIST until dismissed (the dismiss button only appears for terminal jobs).
// Jobs are fed by Epic 9 (`palmier-gen`) via Tauri events today they come from the
// fixture / controller. Cancel calls `controller.cancelJob` (TODO(E9): real command).

import { Spacing, Theme } from "./theme";
import { isTerminalJob, type GenJob } from "./types";

export interface GenerationPanelProps {
  jobs: GenJob[];
  onCancel: (id: string) => void;
  onDismiss: (id: string) => void;
}

export function GenerationPanel({ jobs, onCancel, onDismiss }: GenerationPanelProps) {
  if (jobs.length === 0) return null;
  // Newest first (createdAt desc).
  const ordered = [...jobs].sort((a, b) => b.createdAt - a.createdAt);

  return (
    <div
      style={{
        borderTop: `1px solid ${Theme.border.subtle}`,
        background: Theme.background.surface,
        padding: Spacing.md,
        display: "flex",
        flexDirection: "column",
        gap: Spacing.sm,
        maxHeight: 220,
        overflowY: "auto",
      }}
    >
      <div style={{ fontSize: 11, color: Theme.text.muted, fontWeight: 600 }}>
        Generations
      </div>
      {ordered.map((job) => (
        <JobCard
          key={job.id}
          job={job}
          onCancel={() => onCancel(job.id)}
          onDismiss={() => onDismiss(job.id)}
        />
      ))}
    </div>
  );
}

function statusLabel(job: GenJob): { text: string; tone: string } {
  switch (job.status.kind) {
    case "queued":
      return { text: "Queued", tone: Theme.text.tertiary };
    case "running":
      return {
        text: `Generating ${Math.round(job.status.progress * 100)}%`,
        tone: Theme.accentTimecode,
      };
    case "succeeded":
      return { text: "Done", tone: Theme.accent };
    case "failed":
      return { text: `Failed: ${job.status.message}`, tone: Theme.status.error };
    case "cancelled":
      return { text: "Cancelled", tone: Theme.text.muted };
  }
}

function JobCard({
  job,
  onCancel,
  onDismiss,
}: {
  job: GenJob;
  onCancel: () => void;
  onDismiss: () => void;
}) {
  const status = statusLabel(job);
  const terminal = isTerminalJob(job);
  const progress =
    job.status.kind === "running" ? job.status.progress : undefined;

  return (
    <div
      style={{
        display: "flex",
        gap: Spacing.sm,
        background: Theme.background.raised,
        border: `1px solid ${Theme.border.subtle}`,
        borderRadius: 6,
        padding: Spacing.sm,
      }}
    >
      <div
        style={{
          width: 48,
          height: 48,
          flexShrink: 0,
          borderRadius: 4,
          background: job.thumbnailUrl
            ? `center / cover no-repeat url(${job.thumbnailUrl})`
            : Theme.background.prominent,
        }}
      />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div
          title={job.prompt}
          style={{
            fontSize: 12,
            color: Theme.text.primary,
            whiteSpace: "nowrap",
            overflow: "hidden",
            textOverflow: "ellipsis",
          }}
        >
          {job.prompt}
        </div>
        <div style={{ fontSize: 10, color: Theme.text.muted }}>{job.model}</div>
        <div style={{ fontSize: 11, color: status.tone, marginTop: 2 }}>
          {status.text}
        </div>
        {progress != null && (
          <div
            style={{
              height: 3,
              borderRadius: 2,
              background: Theme.background.base,
              marginTop: 4,
              overflow: "hidden",
            }}
          >
            <div
              style={{
                width: `${Math.round(progress * 100)}%`,
                height: "100%",
                background: Theme.accentTimecode,
              }}
            />
          </div>
        )}
      </div>
      <div style={{ display: "flex", alignItems: "flex-start" }}>
        {terminal ? (
          <button onClick={onDismiss} style={cardButtonStyle} title="Dismiss">
            ✕
          </button>
        ) : (
          <button onClick={onCancel} style={cardButtonStyle} title="Cancel">
            Cancel
          </button>
        )}
      </div>
    </div>
  );
}

const cardButtonStyle = {
  fontSize: 11,
  padding: "2px 6px",
  borderRadius: 4,
  cursor: "pointer",
  color: Theme.text.secondary,
  background: "transparent",
  border: `1px solid ${Theme.border.primary}`,
} as const;
