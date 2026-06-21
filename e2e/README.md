# e2e — automated UI smoke for palmier-pro-win

Self-validation harness so a build agent can confirm the app renders with **no human
clicks**. Two layers:

| Layer | What it drives | Tooling | Status |
|-------|----------------|---------|--------|
| **A — UI smoke** (`tests/`) | Vite-served React UI in headless Chromium, with a **mocked** Tauri bridge | `@playwright/test` | **Required, green** |
| **B — real webview** (`real-webview/`) | The actual WebView2 window | `tauri-driver` + WebdriverIO | Optional, **scaffolded, not run** (see its README) |

The companion **backend oracle** is `scripts/mcp-smoke.ps1` — it launches the real app
and drives the live MCP server (`127.0.0.1:19789/mcp`) to assert editor state. Together,
Layer A + the MCP oracle give end-to-end coverage of both the UI and the agent-edit path.

## Layer A — run it

```pwsh
# one command; Vite auto-starts via the webServer block in playwright.config.ts
cmd /c "cd e2e && pnpm install && npx playwright install chromium && npx playwright test"
```

Or via the project test runner (matches the cargo sections' PASS/FAIL summary):

```pwsh
pwsh -File scripts/test.ps1 -Section ui-smoke
```

What it asserts:
- **Home** (`#/home`) — project browser: "Palmier Pro" title, "Recent Projects", a
  seeded recent tile, the New Project action.
- **Project** (`#/project/<id>`) — the editor mount (`[data-project-id]`), the timeline
  `<canvas>`, and the agent dock ("Agent" header + collapse control). NOT blank.
- **Settings** (`#/settings`) — the 5-tab nav (Account/General/Models/Storage…).

Screenshots of every surface are written to `e2e/artifacts/*.png` as build artifacts.
An HTML report lands in `e2e/playwright-report/` (`npx playwright show-report`).

## How the mock works

`tests/tauri-mock.ts` installs a complete `window.__TAURI_INTERNALS__` (the global the
real `@tauri-apps/api` `invoke`/`listen` dispatch to) **before** the app bundle loads,
via `page.addInitScript`. Its `invoke` router returns realistic camelCase payloads
matching the Rust command shapes (`get_settings`, `agent_status`, `get_timeline`,
`get_media`, …), and `metadata.currentWindow.label` selects the surface (route.ts
prefers the window label over the hash). This makes each surface render its real
content — not the empty/offline fallback you get in plain `vite dev`.

This is a **render/regression** smoke, not a behavioral test of the Rust backend — that
is what `scripts/mcp-smoke.ps1` covers against the live binary.
