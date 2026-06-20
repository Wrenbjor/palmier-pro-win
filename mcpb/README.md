# `palmier-pro.mcpb` — Claude Desktop bundle (E7-S13, FR-28 / SM-8)

This directory is the **unpacked source** of the `palmier-pro.mcpb` bundle that lets
**Claude Desktop** connect to the local Palmier Pro MCP server. It is ported from the
macOS reference `../palmier-pro/mcpb/`; only the identity strings change (the wire
protocol and the loopback URL are identical, so existing reference clients connect
**with only the server URL changed** — and here it is the *same* URL: `127.0.0.1:19789`).

## Contents

- `manifest.json` — MCPB manifest, `manifest_version 0.4`. `name: palmier-pro`,
  `display_name: "Palmier Pro Windows"`, `version: 1.0.0`. The `server` block runs
  the Node entry point over stdio.
- `server/index.js` — the **stdio→HTTP shim**. Claude Desktop speaks MCP over stdio to
  this Node process; the process runs `mcp-remote` against
  `http://127.0.0.1:19789/mcp` (the loopback HTTP server from `palmier-mcp`),
  bridging stdio ⇄ HTTP. `--allow-http` is required because the target is plain HTTP
  on loopback; `--transport http-only` pins the streamable-HTTP transport.
- `server/package.json` / `server/package-lock.json` — pin `mcp-remote@0.1.38` (and its
  full locked dependency tree, copied verbatim from the reference). Run `npm ci` in
  `server/` to populate `node_modules/` before packing.
- `icon.png` — bundle icon (copied from the reference).

## Why a Node shim (not a native transport)

Claude Desktop's MCPB extensions launch a local **command** that speaks MCP over
**stdio**. Our server is **HTTP** on loopback. `mcp-remote` is the standard bridge:
it presents an stdio MCP server to Claude Desktop and proxies every request to the
HTTP endpoint. This is exactly how the macOS reference bundle works — identical
`index.js`, identical `mcp-remote` pin.

## Building the `.mcpb`

The `.mcpb` is a zip of this directory with `node_modules/` populated:

```sh
cd mcpb/server && npm ci && cd ..
# then zip manifest.json + icon.png + server/ into palmier-pro.mcpb
#   (e.g. `npx @anthropic-ai/mcpb pack .` or a plain zip per the MCPB spec)
```

`node_modules/` is intentionally **not** committed — it is restored from the lockfile
at pack time.

## Claude Desktop install flow (user-facing)

1. Palmier Pro is running with a project open (the MCP server is listening on
   `127.0.0.1:19789`).
2. Double-click `palmier-pro.mcpb` (or drag it onto Claude Desktop → Settings →
   Extensions). Claude Desktop extracts it under its extensions directory
   (`%APPDATA%\Claude\Extensions\` on Windows; platform equivalent elsewhere) and
   registers the stdio command.
3. Claude Desktop launches `node server/index.js`, which connects to the loopback
   server. The 30 tools + 2 resources + the verbatim `instructions` are then live.

The other clients connect **directly** to the HTTP URL (no bundle needed):

- **Claude Code:** `claude mcp add --transport http palmier-pro http://127.0.0.1:19789/mcp`
- **Codex:** `codex mcp add palmier-pro --url http://127.0.0.1:19789/mcp`
- **Cursor:** deeplink `cursor://anysphere.cursor-deeplink/mcp/install?name=palmier-pro&config=<base64>`
  or manual JSON pointing at `http://127.0.0.1:19789/mcp`.

## DEFERRED — in-app "Install for Claude Desktop" Help UX (NOT in this story)

E7-S13's scope is the **bundle artifacts only**. The in-app **Help → MCP
Instructions** tab — copy-URL button, the per-client install snippets above, and an
**"Install for Claude Desktop"** action that extracts this `.mcpb` into the Claude
Desktop extensions directory — is a small `palmier-tauri` Help addition shared with
Epic 1 / Epic 12. It was deliberately **left untouched** here (the worker was scoped
out of `palmier-tauri`). When that lands, it should bundle this directory as an app
resource and surface the strings listed above.
