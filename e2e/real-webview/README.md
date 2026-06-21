# Real-webview e2e (Layer B) — OPTIONAL, deferred from the gate

This is the *second*, optional layer of the self-validation harness. Layer A (the
mock-Tauri Playwright suite in `e2e/tests/` + the `scripts/mcp-smoke.ps1` backend
oracle) is the required, always-green deliverable and runs with zero extra tooling.

Layer B drives the **real** WebView2 webview the app actually ships, so it catches
things the mock cannot: the real Rust `invoke` bridge, real window labels/menus, real
data flowing from the live editor state into the UI. It is **deferred from being a
per-PR gate** because, on this Windows box, the toolchain is not yet provisioned and
the setup is materially flakier than Layer A (see "Status" below).

## Why tauri-driver (not tauri-plugin-playwright)

On Windows (June 2026), the reliable path is **`tauri-driver`** — Tauri's official
WebDriver proxy. It sits in front of `msedgedriver.exe` (Edge WebView2 is
Chromium/Edge-based on Windows) and speaks the W3C WebDriver protocol, so any
WebDriver client (WebdriverIO, or Selenium) can drive the native window.

`tauri-plugin-playwright` was considered and rejected for the first cut:
- It is a community plugin, not first-party, and requires compiling a plugin into the
  app (a `--features e2e`-style build) — i.e. you no longer test the real shipping
  binary, you test a special build.
- Playwright's own CDP attach does **not** work against WebView2's embedded protocol
  surface reliably on Windows; that's exactly why `tauri-driver` exists.

`tauri-driver` keeps the binary unmodified and is documented as the supported approach,
so it is the lower-flake choice here. Reference: <https://v2.tauri.app/develop/tests/webdriver/>.

## Status (this session): SCAFFOLDED, NOT RUN

Verified on this box:
- `tauri-driver`  — NOT installed (`cargo install tauri-driver --locked`)
- `msedgedriver`  — NOT installed (must match installed Edge **149.0.4022.80**)
- WebdriverIO     — NOT installed

Because none of the three are present, this layer was **not executed**. The files here
are a working skeleton: install the three deps below and `wdio.conf.ts` is ready to run.
Do not treat this layer as verified until you have run it and seen it pass.

## To enable (one-time setup)

```pwsh
# 1. The WebDriver proxy (Rust). --locked pins versions.
pwsh -File scripts/with-msvc.ps1 cargo install tauri-driver --locked

# 2. Edge WebDriver matching the installed Edge MAJOR version (149 here).
#    Download msedgedriver for 149.x and put it on PATH:
#    https://developer.microsoft.com/microsoft-edge/tools/webdriver/
#    (or: winget install --id Microsoft.Edge.WebDriver  — verify it matches 149)

# 3. WebdriverIO client.
cmd /c "cd e2e/real-webview && pnpm install"

# 4. Build a release-ish app binary tauri-driver can launch (the wdio config points at
#    target/debug/palmier-tauri.exe by default; override with PALMIER_APP_BIN).
pwsh -File scripts/with-msvc.ps1 cargo build -p palmier-tauri --no-default-features
```

## To run

```pwsh
cmd /c "cd e2e/real-webview && pnpm test"
```

`wdio.conf.ts` starts/stops `tauri-driver` automatically (it shells out to it as a
service), launches the app binary under WebDriver, and runs `specs/app.e2e.ts`, which
asserts the same three surfaces as Layer A but in the *real* webview. On Windows the
app opens the Home window first; the spec navigates via the app's own menu/links rather
than rewriting the hash, because window routing is owned by Rust.

## Known caveats / flakiness to expect

- **Driver/Edge version skew**: `msedgedriver` MUST match the Edge major version. WebView2
  auto-updates Edge, so this breaks silently over time — pin/refresh it in CI.
- **First-window timing**: the app does real boot work (settings, MCP bind, preview wgpu
  init). The spec uses generous waits; cold boot can still exceed them on a busy box.
- **Single session**: `tauri-driver` supports one app session at a time — these specs
  must run serially (`maxInstances: 1`), unlike Layer A's parallel browser contexts.
- **No wgpu in headless CI**: the preview viewport needs a GPU adapter; on a headless CI
  runner without one, the Project surface may degrade. Assert on DOM chrome, not pixels.
