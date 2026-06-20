// Help window surface (E1-S9): 2 tabs — Shortcuts + MCP instructions.
//
// - Shortcuts tab matches the §6.1 menu table (kept in sync with the Rust `MENU_TABLE`).
// - MCP tab shows endpoint `http://127.0.0.1:19789/mcp` + Cursor / Claude Desktop /
//   Claude Code / Codex install snippets verbatim (settings-account-app.md "Help / MCP
//   instructions content").
//
// The menu's Help items emit `help://select-tab` to pick the tab; we subscribe so
// "MCP Instructions" lands on the MCP tab.
import { useEffect, useState } from "react";
import { onHelpSelectTab } from "../app/api";

type HelpTab = "shortcuts" | "mcp";

const MCP_ENDPOINT = "http://127.0.0.1:19789/mcp";

// Mirrors `crates/palmier-tauri/src/menu.rs` MENU_TABLE (Ctrl for Cmd, F11 fullscreen).
const SHORTCUTS: { group: string; rows: [string, string][] }[] = [
  {
    group: "File",
    rows: [
      ["New", "Ctrl+N"],
      ["Open", "Ctrl+O"],
      ["Save", "Ctrl+S"],
      ["Save As", "Ctrl+Shift+S"],
      ["Import Media", "Ctrl+I"],
      ["Export", "Ctrl+E"],
    ],
  },
  {
    group: "Edit",
    rows: [
      ["Undo", "Ctrl+Z"],
      ["Redo", "Ctrl+Shift+Z"],
      ["Cut", "Ctrl+X"],
      ["Copy", "Ctrl+C"],
      ["Paste", "Ctrl+V"],
      ["Select All", "Ctrl+A"],
      ["Split at Playhead", "Ctrl+K"],
      ["Trim Start to Playhead", "Q"],
      ["Trim End to Playhead", "W"],
      ["Delete", "Backspace"],
    ],
  },
  {
    group: "View",
    rows: [
      ["Media Panel", "Ctrl+0"],
      ["Inspector", "Ctrl+Alt+0"],
      ["Agent Panel", "Ctrl+Alt+A"],
      ["Maximize Focused Panel", "`"],
      ["Layout: Default", "Ctrl+1"],
      ["Layout: Media", "Ctrl+2"],
      ["Layout: Vertical", "Ctrl+3"],
      ["Enter Full Screen", "F11"],
    ],
  },
  {
    group: "App",
    rows: [
      ["Settings", "Ctrl+,"],
      ["Keyboard Shortcuts", "?"],
      ["Quit", "Ctrl+Q"],
    ],
  },
];

const MCP_SNIPPETS: { label: string; code: string }[] = [
  {
    label: "Claude Code",
    code: `claude mcp add --transport http palmier-pro ${MCP_ENDPOINT}`,
  },
  {
    label: "Codex",
    code: `codex mcp add palmier-pro --url ${MCP_ENDPOINT}`,
  },
  {
    label: "Claude Desktop / Cursor (manual JSON)",
    code: `{
  "command": "npx",
  "args": ["-y", "mcp-remote", "${MCP_ENDPOINT}", "--allow-http", "--transport", "http-only"]
}`,
  },
];

export default function Help() {
  const [tab, setTab] = useState<HelpTab>("shortcuts");

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    onHelpSelectTab((t) => {
      if (t === "mcp" || t === "shortcuts") setTab(t);
    }).then((un) => {
      unlisten = un;
    });
    return () => unlisten?.();
  }, []);

  return (
    <div className="flex h-screen flex-col bg-[#161616] text-white">
      <nav className="flex gap-1 border-b border-white/10 px-4 pt-3">
        {(["shortcuts", "mcp"] as HelpTab[]).map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => setTab(t)}
            className={`rounded-t-md px-4 py-2 text-sm ${
              tab === t ? "bg-white/10 font-medium" : "text-white/60 hover:bg-white/5"
            }`}
          >
            {t === "shortcuts" ? "Keyboard Shortcuts" : "MCP Instructions"}
          </button>
        ))}
      </nav>

      <main className="flex-1 overflow-auto p-6">
        {tab === "shortcuts" ? (
          <div className="grid grid-cols-2 gap-x-10 gap-y-6">
            {SHORTCUTS.map((s) => (
              <section key={s.group}>
                <h2 className="mb-2 text-xs font-medium uppercase tracking-wide text-white/40">
                  {s.group}
                </h2>
                <table className="w-full text-sm">
                  <tbody>
                    {s.rows.map(([label, key]) => (
                      <tr key={label} className="border-b border-white/5">
                        <td className="py-1.5 text-white/80">{label}</td>
                        <td className="py-1.5 text-right font-mono text-white/60">{key}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </section>
            ))}
          </div>
        ) : (
          <div>
            <p className="mb-4 text-sm text-white/70">
              The Palmier Pro MCP server runs locally at{" "}
              <code className="rounded bg-black/40 px-1.5 py-0.5 font-mono text-[#F29933]">
                {MCP_ENDPOINT}
              </code>
              . Add it to your AI client:
            </p>
            {MCP_SNIPPETS.map((s) => (
              <section key={s.label} className="mb-5">
                <h3 className="mb-1.5 text-sm font-medium">{s.label}</h3>
                <pre className="overflow-x-auto rounded-md border border-white/10 bg-black/40 p-3 text-xs text-white/80">
                  <code>{s.code}</code>
                </pre>
              </section>
            ))}
          </div>
        )}
      </main>
    </div>
  );
}
