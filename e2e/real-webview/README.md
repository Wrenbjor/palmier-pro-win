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

## Status: RUN, GREEN (3/3 passing)

Provisioned + executed on this box (June 2026):
- `tauri-driver`  — **v2.0.6** at `~/.cargo/bin/tauri-driver.exe` (`cargo install tauri-driver --locked`)
- `msedgedriver`  — **149.0.4022.80**, vendored at `.driver/msedgedriver.exe` (exact match to Edge/WebView2 149.0.4022.80)
- WebdriverIO     — **v9.29** (`@wdio/cli` + mocha; installed in this folder)
- app binary      — frozen `tauri build --debug --no-bundle` at `target/debug/palmier-tauri.exe`

`specs/app.e2e.ts` passes all three assertions against the **real** WebView2:
1. Home boots + renders the project browser (frozen frontend loads at `http://tauri.localhost/`).
2. The live MCP backend (the same `EditorState` the UI shares) answers on `127.0.0.1:19789`.
3. **The real invoke bridge**: a real Settings-button click fires `invoke("open_settings")`
   and Rust opens a new labelled WebView2 window (asserted via a new window handle + its
   Settings content). The app log shows `INFO app: opened window window=settings` on click.

## To run

```pwsh
pwsh -File scripts/e2e-webview.ps1            # dedicated runner (sources FFmpeg env, PASS/FAIL)
pwsh -File scripts/test.ps1 -Section webview   # same, via the project test runner
```

The runner kills any stray app/driver, sets `PALMIER_APP_BIN`, then `wdio.conf.ts`
spawns `tauri-driver` (passing `--native-driver` → the vendored msedgedriver), which
launches the app under WebDriver and runs `specs/app.e2e.ts`. Single session, serial.

## One-time setup (already done here; redo on a fresh box / after an Edge bump)

```pwsh
# 1. WebDriver proxy (Rust).
pwsh -File scripts/with-msvc.ps1 cargo install tauri-driver --locked

# 2. msedgedriver MATCHING the installed Edge/WebView2. This helper detects the version
#    and vendors the exact match into .driver/ (override the path via MSEDGEDRIVER_BIN):
pwsh -File e2e/real-webview/fetch-msedgedriver.ps1

# 3. WebdriverIO client.
cmd /c "cd e2e/real-webview && pnpm install"

# 4. Build the FROZEN testable binary (custom-protocol, frontend embedded — NOT a bare
#    `cargo build`, which points at the Vite devUrl and shows a blank webview). The
#    beforeBuildCommand is skipped because the dist is built separately:
cmd /c "cd src-ui && pnpm build"   # produce src-ui/dist
# then, from the repo root, run the tauri CLI (it lives in src-ui/node_modules/.bin):
pwsh -File scripts/with-msvc.ps1   # (sources ffmpeg/whisper env) ... then:
#   <repo>\src-ui\node_modules\.bin\tauri.CMD build --debug --no-bundle --config e2e/real-webview/tauri.e2e-build.json
```

## Hard-won gotchas (these are why it works now)

- **Force CLASSIC WebDriver.** WebdriverIO v9 negotiates WebDriver **BiDi** by default
  (`webSocketUrl: true`). With WebView2 + tauri-driver that binds the session to the
  WRONG `CoreWebView2` target — the transient `about:blank` host that never navigates —
  so every element query finds an empty document. `wdio:enforceWebDriverClassic: true`
  in the capabilities fixes it: the session lands on `http://tauri.localhost/`. This was
  THE blocker; without it the suite hangs on a blank page.
- **Build FROZEN, not bare.** A `cargo build --no-default-features` binary loads the UI
  from the Vite devUrl (localhost:5173); under WebDriver with no Vite that's a blank
  webview. Use `tauri build --debug --no-bundle` so the frontend is embedded
  (`custom-protocol`). The runner/wdio config assume this binary.
- **FFmpeg DLLs on PATH at runtime.** The frozen binary needs `C:\ffmpeg\bin` on PATH or
  boot fails partway and MCP never binds. The runner sources `scripts/ffmpeg-env.ps1`;
  tauri-driver spawns the app as a child and inherits that PATH.
- **Driver/Edge version skew.** `msedgedriver` MUST match the Edge major version. WebView2
  auto-updates Edge, so this breaks silently over time — re-vendor `.driver/msedgedriver.exe`
  after an Edge bump. `wdio.conf.ts` logs the driver version at startup so skew is visible.
- **Attach race.** Even with classic WebDriver the first attach can occasionally land on
  `about:blank`; the spec's `ensureAppDocument()` retries via `reloadSession()`. `onPrepare`
  also waits ~4s for tauri-driver's native-port handshake to settle before connecting.
- **Single session.** `tauri-driver` supports one app session at a time (`maxInstances: 1`),
  unlike Layer A's parallel browser contexts.
- **No native dialogs.** New Project / Open fire native Save/Open dialogs WebDriver can't
  drive, so the bridge assertion uses Settings (window-open, no dialog). Driving the
  project editor (timeline/agent panel) needs a project, which requires the dialog — out
  of scope for this smoke; the Layer-A mock suite + `mcp-smoke.ps1` cover those surfaces.
