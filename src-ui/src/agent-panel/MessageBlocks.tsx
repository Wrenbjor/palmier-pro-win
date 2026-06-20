// Message-block rendering (agent-panel.md "message rendering = text/tool_use/
// tool_result blocks"). Each `AgentContentBlock` renders to its own element:
//   - text        → plain prose
//   - toolUse     → a "tool: <name>" header with a collapsible JSON input (collapsed
//                   by default), the raw `inputJson` pretty-printed for display only
//   - toolResult  → text and/or inlined images; error results get error styling

import { useState } from "react";
import type { CSSProperties } from "react";
import { formatToolInput } from "./logic";
import { Spacing, Theme } from "./theme";
import type { AgentContentBlock, ToolResultBlock } from "./types";

export function ContentBlockView({ block }: { block: AgentContentBlock }) {
  switch (block.kind) {
    case "text":
      return <TextBlock text={block.text} />;
    case "toolUse":
      return <ToolUseBlock name={block.name} inputJson={block.inputJson} />;
    case "toolResult":
      return (
        <ToolResultView content={block.content} isError={block.isError} />
      );
  }
}

function TextBlock({ text }: { text: string }) {
  if (text.length === 0) return null;
  return <div style={{ whiteSpace: "pre-wrap", lineHeight: 1.5 }}>{text}</div>;
}

function ToolUseBlock({ name, inputJson }: { name: string; inputJson: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div style={toolBoxStyle}>
      <button
        type="button"
        onClick={() => setOpen((o) => !o)}
        style={toolHeaderStyle}
        aria-expanded={open}
        title={open ? "Collapse input" : "Expand input"}
      >
        <span style={{ opacity: 0.7 }}>{open ? "▾" : "▸"}</span>
        <span style={{ opacity: 0.7 }}>🛠</span>
        <span style={{ fontWeight: 600 }}>{name}</span>
        <span style={{ color: Theme.text.muted, fontSize: 11 }}>tool call</span>
      </button>
      {open && (
        <pre style={toolJsonStyle}>
          <code>{formatToolInput(inputJson)}</code>
        </pre>
      )}
    </div>
  );
}

function ToolResultView({
  content,
  isError,
}: {
  content: ToolResultBlock[];
  isError: boolean;
}) {
  return (
    <div
      style={{
        ...toolBoxStyle,
        borderColor: isError ? Theme.status.error : Theme.border.subtle,
        background: isError ? Theme.status.errorBg : Theme.toolBlock,
      }}
    >
      <div
        style={{
          ...toolHeaderStyle,
          cursor: "default",
          color: isError ? Theme.status.error : Theme.text.tertiary,
        }}
      >
        <span>{isError ? "⚠" : "↩"}</span>
        <span style={{ fontWeight: 600 }}>
          {isError ? "tool error" : "tool result"}
        </span>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: Spacing.sm }}>
        {content.map((b, i) =>
          b.kind === "text" ? (
            <div
              key={i}
              style={{
                whiteSpace: "pre-wrap",
                fontSize: 12,
                color: isError ? Theme.status.error : Theme.text.secondary,
              }}
            >
              {b.text}
            </div>
          ) : (
            <img
              key={i}
              src={`data:${b.mediaType};base64,${b.base64}`}
              alt="tool result"
              style={{ maxWidth: "100%", borderRadius: 6 }}
            />
          ),
        )}
      </div>
    </div>
  );
}

const toolBoxStyle: CSSProperties = {
  border: `1px solid ${Theme.border.subtle}`,
  background: Theme.toolBlock,
  borderRadius: 8,
  padding: Spacing.sm,
  display: "flex",
  flexDirection: "column",
  gap: Spacing.xs,
};

const toolHeaderStyle: CSSProperties = {
  display: "flex",
  alignItems: "center",
  gap: Spacing.sm,
  background: "transparent",
  border: "none",
  color: Theme.text.tertiary,
  cursor: "pointer",
  padding: 0,
  fontSize: 12,
  textAlign: "left",
};

const toolJsonStyle: CSSProperties = {
  margin: 0,
  padding: Spacing.sm,
  background: Theme.background.base,
  borderRadius: 6,
  fontSize: 11,
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
  color: Theme.text.secondary,
  overflowX: "auto",
};
