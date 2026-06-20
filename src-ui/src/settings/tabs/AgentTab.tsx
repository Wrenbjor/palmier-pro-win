// Agent tab (E1-S9): Anthropic API key + MCP server status/toggle.
//
// - Key: SecureField with placeholder `sk-ant-...` (no key) or masked `•`×36 + last 4
//   (key present). Save when the draft is non-empty; trash when a key exists. Persists
//   via the keyring (`anthropic-api-key`, ruling #5).
// - MCP: green/grey dot + "Running on 127.0.0.1:19789" / "Stopped" reflecting **actual
//   server liveness** (a stub returning the pref + start-result until Epic 7); a toggle
//   to `set_mcp_enabled`; a "Setup instructions" link to the Help MCP tab.
import { useEffect, useState } from "react";
import {
  deleteAnthropicKey,
  getMcpStatus,
  hasAnthropicKey,
  openHelp,
  saveAnthropicKey,
  setMcpEnabled,
  type McpStatus,
} from "../../app/api";

export default function AgentTab() {
  const [keyPresent, setKeyPresent] = useState(false);
  const [draft, setDraft] = useState("");
  const [mcp, setMcp] = useState<McpStatus | null>(null);

  function refresh() {
    void hasAnthropicKey().then((p) => setKeyPresent(Boolean(p)));
    void getMcpStatus().then((m) => m && setMcp(m));
  }

  useEffect(refresh, []);

  const masked = keyPresent ? "•".repeat(36) : "";

  async function handleSave() {
    if (!draft.trim()) return;
    await saveAnthropicKey(draft.trim());
    setDraft("");
    refresh();
  }

  async function handleDelete() {
    await deleteAnthropicKey();
    setDraft("");
    refresh();
  }

  return (
    <div>
      <h1 className="mb-6 text-lg font-semibold">Agent</h1>

      <section className="mb-8">
        <h2 className="mb-2 text-xs font-medium uppercase tracking-wide text-white/40">
          Anthropic API key
        </h2>
        <p className="mb-3 text-xs text-white/50">
          Bring your own key to run the agent on your own Anthropic account.
        </p>
        <div className="flex items-center gap-2">
          <input
            type="password"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            placeholder={keyPresent ? masked : "sk-ant-..."}
            className="flex-1 rounded-md border border-white/15 bg-black/30 px-3 py-2 text-sm outline-none focus:border-[#F29933]"
          />
          {draft.trim() ? (
            <button
              type="button"
              onClick={handleSave}
              className="rounded-md bg-[#F29933] px-4 py-2 text-sm font-medium text-black hover:brightness-110"
            >
              Save
            </button>
          ) : keyPresent ? (
            <button
              type="button"
              onClick={handleDelete}
              title="Remove saved key"
              className="rounded-md border border-white/15 px-3 py-2 text-sm text-white/70 hover:bg-white/10"
            >
              Remove
            </button>
          ) : null}
        </div>
      </section>

      <section>
        <h2 className="mb-2 text-xs font-medium uppercase tracking-wide text-white/40">
          MCP server
        </h2>
        <div className="flex items-center justify-between border-b border-white/10 py-4">
          <div className="flex items-center gap-2">
            <span
              className={`h-2.5 w-2.5 rounded-full ${
                mcp?.running ? "bg-green-500" : "bg-white/30"
              }`}
            />
            <span className="text-sm">
              {mcp?.running ? `Running on ${mcp.bind}` : "Stopped"}
            </span>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={mcp?.enabled ?? false}
            onClick={() => {
              const next = !(mcp?.enabled ?? false);
              void setMcpEnabled(next).then(refresh);
            }}
            className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${
              mcp?.enabled ? "bg-[#F29933]" : "bg-white/20"
            }`}
          >
            <span
              className={`absolute top-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
                mcp?.enabled ? "translate-x-5" : "translate-x-0.5"
              }`}
            />
          </button>
        </div>
        <button
          type="button"
          onClick={() => void openHelp()}
          className="mt-3 text-sm text-[#F29933] hover:underline"
        >
          Setup instructions
        </button>
      </section>
    </div>
  );
}
