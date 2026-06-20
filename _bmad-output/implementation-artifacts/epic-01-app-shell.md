---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
title: "Epic 1 — App Shell & Project Lifecycle (implementation stories)"
created: 2026-06-20
updated: 2026-06-20
governing_reference: [settings-account-app, project-io]
milestone: M1
---

# Epic 1 — App Shell & Project Lifecycle

## Epic goal

Boot to a working Home + Project window with the full reference main menu + keyboard shortcuts,
settings/registry persistence, and sample-project materialization, on the Tauri 2 substrate
(WebView2 / WebKitGTK). This is the foundation every other epic stands on: it owns the boot
sequence, the window/menu/lifecycle plumbing, the settings store, auth/account scaffolding,
telemetry/logging/crash handling, and the updater glue. Realizes **UJ-4** (hand-edit shell) and
the boot half of every other journey. Scope = **FR-1..FR-4** (PRD §4.1).

**Crates / surfaces (PRD §10):** `palmier-tauri` (binary), `palmier-auth`, `palmier-update`,
`palmier-telemetry`, `src-ui/app`, `src-ui/home`, `src-ui/settings`. (The actual `.palmier`
bundle read/write + serde model belong to Epic 2 / `palmier-project` + `palmier-model`; Epic 1
calls into that boundary but does not own it — see cross-epic dependency note below.)

**Governing reference docs:** `docs/reference/settings-account-app.md` (boot order, window config,
menu, settings tabs, auth state machine, keychain, updater, telemetry, Help/MCP-instructions
content) and `docs/reference/project-io.md` (registry, sample materialization, create/open
lifecycle, autosave-on-home). FOUNDATION §6.1 / §6.15 / §6.16.

## PRD §4.1 / §10 acceptance this epic must satisfy

- **FR-1 — Boot to editable Home.** App boots in the §6.1 sequence (crash handler → tracing →
  settings → Clerk/Convex config → model catalog → MCP server if enabled → Home window).
  **SM-1: cold start < 3 s to project window** on NVMe + RTX 4060-class HW. Settings read from
  `%APPDATA%\PalmierProWin\settings.json` (Win) / `~/.config/palmier-pro/settings.json` (Linux).
  Pref keys `io.palmier.pro.{notifications,telemetry,mcp}.enabled`; **absent ⇒ ON** (ruling #6).
  **The model-catalog load (`/v1/models`) is async + 24 h-cached and MUST NOT block reaching the
  Home/Project window** — offline / slow-Convex cold start still meets SM-1 (decouples SM-1 from
  R-4 / OQ-9; failed fetch degrades to cached/empty catalog, never a boot stall).
- **FR-2 — Project registry & windows.** One Project window per project; switching auto-saves the
  previous; registry in `project-registry.json` (ruling #3) sorted **newest-first by
  `lastOpenedDate` desc**; delete moves the bundle to Recycle Bin / Trash.
- **FR-3 — Main menu & shortcuts.** All §6.1 menu items + shortcuts present with identical
  bindings (Ctrl substituted for Cmd; **F11** fullscreen on Windows). Every shortcut is invokable
  and fires the named action.
- **FR-4 — Sample projects.** Fetch `/v1/samples`, resolve + materialize a `.palmier` bundle to
  `%APPDATA%\PalmierProWin\Samples\<slug>\` with download progress; a resolved sample opens and
  round-trips. Sample bundles use the **reference filenames** (`project.json`/`media.json`/
  `generation-log.json`/`chat/`/`thumbnail.jpg`) so import does not break (ruling #3).

**Milestone:** **M1 — Hand-Edit MVP** (PRD §12; Epics 1–6). Epic 1 is the first epic of M1.

## Spike / gating note

Epic 1 is **NOT spike-gated.** The two M1-relevant spikes gate other epics:
- **S-1 (wgpu→WebView)** gates **Epic 5** only — no preview/presentation mechanism is touched here.
  Do not write any Epic 1 story that assumes a rendered-preview path.
- **S-1b (Convex sample-payload Date encoding)** gates the **Epic 2 serde lock**, and therefore
  gates only the *round-trip fidelity* of FR-4 sample import (Epic 1 materializes the bundle;
  Epic 2 owns the serde shapes the bundle decodes into). Epic 1's sample stories depend on Epic 2's
  model/bundle types existing (cross-epic), not on S-1b directly — see E1-S8 dependencies.

One genuinely-blocked external dependency: **Convex access (OQ-9 / R-4).** All Convex-touching
Epic 1 surfaces (model catalog, samples list/resolve, Clerk/account, feedback) MUST degrade
gracefully when Convex is unreachable. The boot path is explicitly designed so a failed Convex
call never blocks reaching Home (FR-1). Account/sample/feedback features show their
unauthenticated/empty/offline state rather than erroring the app. **If Convex access is blocked at
build time, sample stories use a captured fixture `/v1/samples/resolve` payload** (mirrors the
S-1b fixture fallback).

---

## Stories

### E1-S1 — Tauri app skeleton + boot sequence

**Intent.** As a developer, I want the `palmier-tauri` binary to start up in the exact reference
boot order so that every later subsystem initializes against a stable, ordered lifecycle.

**Acceptance criteria.**
- **Given** a fresh launch, **When** `main()` runs, **Then** it executes the FOUNDATION §6.1 boot
  sequence in order: (1) install crash handler (Sentry + native panic hook), (2) init tracing
  subscriber, (3) read settings from `settings.json`, (4) configure Clerk + Convex clients from
  build-time config, (5) load `ModelCatalog` (`/v1/models`, **spawned async, non-blocking**),
  (6) start MCP server if `settings.mcp_enabled` (default true) — wired as a no-op/stub hook in
  this epic; the real server is Epic 7, (7) show the Home window. Steps 1–4 + 7 complete
  synchronously on the boot path; step 5 is `tokio::spawn`ed and never awaited before window show.
- **Given** Convex is unreachable, **When** the app boots, **Then** the Home window still appears
  (the async catalog fetch failure is logged + degrades to cached/empty catalog) and **SM-1**
  (< 3 s cold start to project window on §10 HW) is met offline — asserted by a boot-timing test
  that mocks Convex as unreachable.
- **Given** a fresh install with no `settings.json`, **When** settings are read, **Then** defaults
  are applied: `io.palmier.pro.{notifications,telemetry,mcp}.enabled` **absent ⇒ true** (ruling #6),
  `has_seen_welcome` absent ⇒ false. Settings persist to `%APPDATA%\PalmierProWin\settings.json`
  (Win) / `~/.config/palmier-pro/settings.json` (Linux) (FR-1), **not** a macOS preference domain
  (settings-account-app.md "Settings persistence is UserDefaults-backed booleans" gotcha).
- **Given** the app is already running and all windows are closed, **When** reopened, **Then** the
  Home window is shown (reference `Reopen with no windows → showHome()`).
- `applicationShouldOpenUntitledFile`-equivalent is **false** — boot does not auto-create a project.

**Implementation context.**
- Crate: `palmier-tauri` (`main.rs`, `setup`), with a thin `settings` module (serde `Settings`
  struct, absent-⇒-on boolean semantics).
- Reference: `Sources/PalmierPro/App/main.swift` (boot order), `App/AppDelegate.swift`
  (`applicationDidFinishLaunching`, reopen handling). Map per settings-account-app.md "Boot order"
  + "macOS/Apple APIs to replace" (`NSApplication`/`AppDelegate` → `tauri::Builder` setup).
- Key types: `Settings { mcp_enabled, notifications_enabled, telemetry_enabled, has_seen_welcome }`
  with `#[serde(default = "default_true")]` on the three `*_enabled` booleans.
- Section: settings-account-app.md "Boot order (`main.swift`, before `app.run()`)" + FOUNDATION §6.1
  boot sequence.

**Dependencies.** None (epic root).
**Parallel-safe?** No — it creates `main.rs` + the Tauri builder scaffold that S2–S9 extend; must
land first. Everything else branches off it.

---

### E1-S2 — Crash handler, tracing, and categorized logging (`palmier-telemetry`)

**Intent.** As a developer, I want async-signal-safe crash capture, daily-rotated logs, and the
reference categorized logger so that diagnostics survive on Windows/Linux exactly as on macOS.

**Acceptance criteria.**
- **Given** boot step 1, **When** the crash handler installs, **Then** a Rust panic hook + native
  signal/backtrace capture write a `crash.log` to the app data dir
  (`%APPDATA%\PalmierProWin\logs\` on Win / `~/.local/state/palmier-pro/` on Linux) — **NOT**
  `~/Library/Logs` (settings-account-app.md crash-log-path gotcha). The handler is
  async-signal-safe (write/backtrace/fsync only).
- **Given** boot step 2, **When** the tracing subscriber initializes, **Then** logs route through
  `tracing` + `tracing-subscriber` with the reference category set —
  `app/editor/export/preview/mcp/generation/project/transcription/search` — rotated **daily,
  retained 7 days** (FR-43, FOUNDATION §6.16), and mirrored to stderr.
- **Given** a `warning`, **When** Sentry is enabled, **Then** it becomes a Sentry breadcrumb;
  `error`/`fault` become Sentry capture-message (matches reference `Log` mapping).
- **Given** the telemetry pref, **When** the app launches, **Then** Sentry starts **only if**
  `io.palmier.pro.telemetry.enabled` (absent ⇒ ON, ruling #6) **AND** a non-empty DSN is present;
  options match the reference: `sendDefaultPii=false`, `tracesSampleRate=0.1`,
  `appHangTimeoutInterval=8.0`, `attachStacktrace=true`, env development(DEBUG)/production,
  releaseName `palmier-pro@<version>+<build>`. The enabled flag is **snapshotted at launch**
  (`enabledForCurrentLaunch`) — toggling requires restart.

**Implementation context.**
- Crate: `palmier-telemetry`.
- Reference: `Sources/PalmierPro/Telemetry/Telemetry.swift`, `Utilities/Log.swift`. Map per
  settings-account-app.md "Telemetry/logging" + "macOS/Apple APIs to replace" (`os.Logger` →
  `tracing`; Sentry Apple SDK → Sentry Rust SDK; `NSSetUncaughtExceptionHandler`/POSIX signal →
  Rust panic hook + `signal-hook`/`backtrace`, or Sentry native crash handler).
- Key types: `Telemetry::start(enabled, dsn)`, `Log` category enum, `enabled_for_current_launch`
  snapshot. DSN is build-injected (OQ-2; opt-out default).
- Section: settings-account-app.md "Telemetry/logging".

**Dependencies.** E1-S1 (boot path calls steps 1–2).
**Parallel-safe?** Yes — own crate `palmier-telemetry`; only the two boot-call lines in S1's
`main.rs` are shared (define `pub fn` here, S1 already reserves the call sites). Run in its own
worktree.

---

### E1-S3 — Main menu + keyboard shortcuts (`palmier-tauri` + `src-ui/app`)

**Intent.** As an editor (Priya, UJ-4), I want the full reference menu with identical shortcuts
(Ctrl for Cmd, F11 for fullscreen) so my muscle memory transfers from the Mac app.

**Acceptance criteria.**
- **Given** the app window, **When** the menu builds, **Then** every FOUNDATION §6.1 menu item is
  present with its exact binding, Cmd→Ctrl substituted, **F11** for Enter Full Screen on Windows
  (Linux follows desktop convention): Palmier Pro (About, Check for Updates, Settings `Ctrl+,`,
  Quit `Ctrl+Q`); File (New `Ctrl+N`, Open `Ctrl+O`, Save `Ctrl+S`, Save As `Ctrl+Shift+S`, Import
  Media `Ctrl+I`, Export `Ctrl+E`); Edit (Undo `Ctrl+Z`, Redo `Ctrl+Shift+Z`, Cut/Copy/Paste/
  Select All `Ctrl+X/C/V/A`, Split at Playhead `Ctrl+K`, Trim Start `Q`, Trim End `W`, Delete
  `Backspace/Del`); View (Media Panel `Ctrl+0`, Inspector `Ctrl+Alt+0`, Agent Panel `Ctrl+Alt+A`,
  Maximize Focused Panel `` ` ``, Layout Default/Media/Vertical `Ctrl+1/2/3`, Enter Full Screen
  `F11`); Help (Tutorial, Keyboard Shortcuts `?`, MCP Instructions, Send Feedback).
- **Given** any menu item, **When** invoked, **Then** it dispatches the named action as a **Tauri
  command/event** (not an `NSApp.sendAction` responder chain) — editor-action items (Undo, Split,
  Trim, Delete, etc.) emit events the editor frontend consumes; window/app items (Settings, About,
  Help tabs, Feedback, Check for Updates, Quit, fullscreen, panel toggles) invoke their handler
  directly. Items whose target subsystem is a later epic (Save/Export/clip edits) dispatch the
  event to a **registered no-op-or-stub handler** so the binding is provably invokable now.
- A menu-shortcut table test asserts each of the §6.1 rows resolves to a registered command id (FR-3).

**Implementation context.**
- Crate: `palmier-tauri` (Tauri `Menu`/`MenuItem` + accelerator strings, event emission),
  `src-ui/app` (event listeners for editor-action items, added as stubs where the owning epic is
  later).
- Reference: `Sources/PalmierPro/App/MainMenu.swift` (`MainMenuBuilder.buildMenu()`,
  `EditorActions` @objc protocol). Map per settings-account-app.md "macOS/Apple APIs to replace"
  (`NSMenu`/`#selector` → Tauri `Menu` + accelerators; editor actions → Tauri commands/events).
- Section: FOUNDATION §6.1 "Main menu items" table + note on Ctrl/F11.

**Dependencies.** E1-S1.
**Parallel-safe?** Partly — it adds a `menu` module + registers menu in S1's builder. Coordinate
the single builder-registration line with S1 (define menu in its own module, register via one
documented call). Treat as **sequential-after-S1** to avoid a builder merge conflict; UI event
stubs in `src-ui/app` are independent.

---

### E1-S4 — Home + Project + Settings/Help/Feedback windows (`palmier-tauri` + `src-ui/app`/`home`)

**Intent.** As a user, I want the Home, Project, Settings, Help, and Feedback windows sized and
behaving like the reference so the app feels identical on Windows/Linux.

**Acceptance criteria.**
- **Given** boot completes, **When** the Home window shows, **Then** it is **1200×1200 default,
  760×480 min** (FOUNDATION §6.1) and hosts the project browser + Recent + Sample carousel +
  Welcome overlay, the overlay dismissed by and persisting `has_seen_welcome` (FR-1).
- **Given** a project opens, **When** its window shows, **Then** it is **1600×1000 default,
  960×600 min**; **one Project window per project** (FR-2).
- **Given** Settings/Help/Feedback are opened, **When** each window shows, **Then** dimensions
  match the reference: Settings content **980×640** / min **760×480**, dark, transparent titlebar,
  full-size content; Help **900×560** / min **820×520**; Feedback **480×480** / min **480×420**,
  not-released-on-close (settings-account-app.md "Window config").
- **Given** a window is resized/moved, **When** reopened, **Then** size+position persist via
  `tauri-plugin-window-state` using distinct state keys (replacing the reference autosave names
  `PalmierProSettings-v2`/`PalmierProHelp-v1`; don't collide — settings-account-app.md gotcha).

**Implementation context.**
- Crate: `palmier-tauri` (`WebviewWindow` configs / `tauri.conf.json` window defs,
  `tauri-plugin-window-state`), `src-ui/app` shell + `src-ui/home` (Home content; the carousel
  consumes E1-S8's sample list, the browser consumes E1-S7's registry).
- Reference: window configs in settings-account-app.md "Window config" + FOUNDATION §6.1 "Windows".
  Map `NSWindow`/`NSWindowController`/autosave → Tauri `WebviewWindow` + `tauri-plugin-window-state`.
- Section: settings-account-app.md "Window config"; FOUNDATION §6.1 "Windows".

**Dependencies.** E1-S1.
**Parallel-safe?** Yes for the `src-ui/home` shell + window configs; the Home browser/carousel
bind to S7/S8 data via a defined props/event contract (stub the data source so this lands
independently). Own files (`src-ui/home/*`, window config block). Run in its own worktree.

---

### E1-S5 — Bundled fonts + app constants (`palmier-text` registration hook + `palmier-tauri`)

**Intent.** As a developer, I want the reference bundled fonts registered at boot and the layout
constants available so text rendering and UI sizing match the reference.

**Acceptance criteria.**
- **Given** boot step 3 (reference order: fonts register before window build), **When** fonts
  register, **Then** all `.ttf`/`.otf` under `Resources/Fonts/` are enumerated and made available
  via `fontdb` (system + bundled), replacing CoreText `CTFontManagerRegisterFontURLs`
  (settings-account-app.md "macOS/Apple APIs to replace").
- **Given** the reference `Constants.swift`, **When** ported, **Then** the layout/default constants
  used by the shell (default project name `"Untitled Project"`, `NSInitialToolTipDelay`-equivalent
  tooltip delay, storage directory defaults) are available as Rust constants.
- **GPL note (R-2):** bundled reference fonts are a GPL-tainted boundary (OQ-11); keep the font
  resources isolated so a future clean-room swap is possible. (No action beyond isolation; do not
  block on it.)

**Implementation context.**
- Crate: `palmier-tauri` (boot hook + constants) calling a `palmier-text` font-registration entry
  (the crate is owned by Epic 5/§6.6, but the registration hook is a thin, additive entry point —
  expose `pub fn register_bundled_fonts()` here and call it from boot).
- Reference: `Sources/PalmierPro/Utilities/BundledFonts.swift`, `Utilities/Constants.swift`. Map
  `CoreText` → `fontdb` (FOUNDATION §6.6).
- Section: settings-account-app.md "Boot order" step 3 + "macOS/Apple APIs to replace" (CoreText).

**Dependencies.** E1-S1.
**Parallel-safe?** Yes — additive boot hook + constants module + isolated font resources; the one
boot call line coordinates with S1. Run in its own worktree.

---

### E1-S6 — `palmier-auth`: Clerk + Convex client, account state machine, keyring

**Intent.** As a signed-in user (Sam, UJ-3), I want auth + account/credit state wired through a
Rust auth crate, and my Anthropic key stored in the OS keyring, so account-gated features and the
agent have a single source of truth.

**Acceptance criteria.**
- **Given** build-time config, **When** `configure()` runs, **Then** it reads the Clerk publishable
  key + Convex deployment/HTTP URLs from compiled Tauri config (replacing `BackendConfig`'s
  `Info.plist` reads); if Clerk key or Convex URL is missing → `is_misconfigured = true` and the
  Account tab is hidden downstream (settings-account-app.md auth state machine).
- **Given** Clerk in the webview (`@clerk/clerk-react`), **When** authenticated, **Then** the JWT
  is forwarded as `Bearer` to Convex over HTTP (`reqwest`), `users:upsertFromAuth` runs (3-try),
  and `account:get` is subscribed; derived state exposes `tier` (none/pro/max),
  `budget_credits = plan.monthlyBudgetCredits + user.purchasedCredits`,
  `remaining_credits = max(0, budget - spentThisPeriod)`,
  `ai_allowed = signed_in && !is_misconfigured`. **Unauthenticated/Convex-unreachable degrades to
  signed-out state — never an app error** (OQ-9 / R-4).
- **Given** the Anthropic API key, **When** saved/loaded/deleted, **Then** it uses the `keyring`
  crate (Windows Credential Manager / Linux Secret Service) at service `palmier-pro`, account
  **`anthropic-api-key`** (ruling #5 — **not** `palmier-pro-anthropic-api-key`); save/delete emit
  an `anthropic-api-key-changed` event; DEBUG reads `ANTHROPIC_API_KEY` env first. A round-trip
  unit test proves save→load returns the same key under account `anthropic-api-key`.
- **Given** a billing/feedback action, **When** invoked, **Then** Convex actions
  (`billing:createCheckoutSession`/`createTopOffCheckoutSession`/`createPortalSession`,
  `feedback:send`) are called and any returned URL is opened **only if** scheme=https AND host ∈
  {checkout.stripe.com, billing.stripe.com} (port the allowlist verbatim — security control,
  settings-account-app.md gotcha).

**Implementation context.**
- Crate: `palmier-auth`. Holds Clerk token cache, account/plan/credit state, billing calls,
  OS-keyring CRUD, misconfigured guard (settings-account-app.md "Mapping to FOUNDATION crates").
- Reference: `Account/AccountService.swift`, `Account/BackendConfig.swift`,
  `Utilities/KeychainStore.swift`, `Agent/Clients/AnthropicClient.swift` (`AnthropicKeychain`,
  account `"anthropic-api-key"`). Map ClerkKit/ConvexMobile → `@clerk/clerk-react` + `reqwest`;
  Keychain Security framework → `keyring` crate.
- Section: settings-account-app.md "Auth/account state machine" + "Token/keychain" + ruling #5.

**Dependencies.** E1-S1 (boot step 4 calls `configure()`). The Convex HTTP transport here is the
**M1 read-only slice** (config + auth state + key storage); the full Convex HTTP+WebSocket client
(generation live queries) is **Spike S-2 / Epic 9** — do not implement WebSocket subscriptions here.
**Parallel-safe?** Yes — own crate `palmier-auth` + the webview Clerk integration in
`src-ui/settings` account fragments (S9 owns the settings UI; coordinate the account-fragment file
boundary with S9, otherwise independent). Run in its own worktree.

---

### E1-S7 — Project registry + create/open/autosave-on-home lifecycle (`palmier-tauri` orchestration over `palmier-project`)

**Intent.** As a user (Priya, UJ-4), I want New/Open/Save-As and a recent-projects registry that
auto-saves the project I switch away from, so my work is tracked and never lost on navigation.

**Acceptance criteria.**
- **Given** the registry file, **When** loaded/mutated, **Then** it lives at
  `%APPDATA%\PalmierProWin\project-registry.json` (ruling #3 — **`project-registry.json`**, the
  reference name) as a JSON array of `ProjectEntry { id, url, created_date, last_opened_date }`;
  every mutation writes the whole array **atomically** (write-temp + atomic rename);
  `sorted_entries` = by `last_opened_date` **desc** (newest-first, FR-2).
- **Given** `register(url)`, **When** the URL already exists (standardized/normalized for dedup),
  **Then** bump `last_opened_date = now`; else append a new entry (new UUID, created+lastOpened =
  now). `remove` deletes the entry; **`delete` trashes the bundle** via the `trash` crate (Recycle
  Bin on Windows / XDG trash on Linux) then removes the entry; `update_url(old,new)` rewrites url +
  bumps lastOpened (called on rename/Save-As) — replicate verbatim (project-io.md "Registry").
- **Given** New, **When** chosen via the file dialog (defaulting into `~/Documents/Palmier Pro`-
  equivalent storage dir), **Then** a project is created, saved, and `register`ed. **Given** Open,
  **When** chosen, **Then** the bundle opens, its window shows, and it is `register`ed. (Bundle
  read/write itself is **Epic 2 / `palmier-project`**; Epic 1 owns the registry + lifecycle
  orchestration that calls it.)
- **Given** an active edited project, **When** the user returns to Home (`show_home`), **Then** the
  project is **force-autosaved before** its window hides and it re-registers (project-io.md
  "Autosave": `isDocumentEdited → autosave` before ordering-out) — this is FR-2's "switching
  auto-saves the previous".
- Samples are **NOT** registry-tracked (project-io.md gotcha) — verified by a test that opening a
  sample does not add a registry entry.

**Implementation context.**
- Crate: `palmier-tauri` (lifecycle commands: `new_project`/`open_project`/`show_home`,
  file-dialog via `tauri-plugin-dialog`) orchestrating `palmier-project`'s `ProjectRegistry` +
  bundle reader. The `ProjectRegistry` + `ProjectEntry` + `MediaResolver` path logic **live in
  `palmier-project`** (Epic 2's crate) per project-io.md "Mapping"; Epic 1 may land the registry
  module itself if Epic 2 has not, but the bundle-serde shapes are Epic 2's.
- Reference: `Project/ProjectRegistry.swift` (+ `ProjectRegistryDisk` actor), `App/AppState.swift`
  (`createNewProject`/`openProject`/`openProjectFromPanel`/`showHome`). Map `NSDocumentController`/
  `NSSave|OpenPanel` → `tauri-plugin-dialog`; `FileManager.trashItem` → `trash` crate;
  `URL.standardizedFileURL` → lexical path normalizer (NOT `canonicalize` — fails on
  non-existent paths, project-io.md "macOS/Apple APIs to replace").
- Section: project-io.md "Registry" + "Create" + "Open" + "Autosave"; FOUNDATION §6.1.

**Dependencies.** E1-S1; **cross-epic: Epic 2 (E2 / `palmier-project` bundle read/write + serde
model)** for actual save/open. Epic 1 can land the registry + lifecycle orchestration against an
Epic-2 trait/stub, but real round-trip save/open requires Epic 2.
**Parallel-safe?** Mostly — the `ProjectRegistry` module is self-contained in `palmier-project`;
the lifecycle commands add to `palmier-tauri`. Shares `palmier-project` with Epic 2 — coordinate
crate ownership (Epic 2 owns the bundle/serde; this story owns registry + lifecycle). Run in its
own worktree but sequence the `palmier-project` crate-scaffold handshake with Epic 2.

---

### E1-S8 — Sample project list + resolve + materialization (`palmier-project` SampleProjectService)

**Intent.** As a new user, I want to open a bundled sample from the Home carousel so I can explore
a real project immediately, with download progress.

**Acceptance criteria.**
- **Given** the Home carousel, **When** it loads, **Then** `GET {convexHttpUrl}/v1/samples` returns
  `[Summary { slug, title, posterUrl? }]`; **Convex-unreachable degrades to an empty carousel, not
  an error** (OQ-9 / R-4). If Convex is blocked at build time, a captured fixture payload is used.
- **Given** a sample is chosen, **When** resolved, **Then** `GET /v1/samples/resolve?slug=<slug>`
  returns `{ title, project, manifest, generationLog?, posterUrl?, downloads:[{id,relativePath,url}],
  chat:[{name,url}] }`, and a `.palmier` bundle is materialized at
  `%APPDATA%\PalmierProWin\Samples\<safeSlug>\<safeTitle>.palmier` (`safeName` strips `/ : \`),
  clearing any stale slug dir first, creating `media/`, and writing **the reference filenames**:
  `project`→`project.json`, `manifest`→`media.json`, optional `generationLog`→`generation-log.json`,
  optional poster→`thumbnail.jpg` (ruling #3, FR-4).
- **Given** the downloads, **When** materializing, **Then** media entries use server `relativePath`
  AS-IS (already `media/<file>`), chat entries get `relativePath = "chat/<name>"`; all download
  **concurrently** (`tokio JoinSet`) with progress reported as completed/total (`on_progress(0..1)`)
  on the UI; any failure removes the whole slug dir and surfaces the error (project-io.md "Sample
  materialization").
- **Given** a resolved sample, **When** opened, **Then** it opens and round-trips (FR-4) and is
  **not** registry-tracked. `cachedURL(slug)` returns the first `*.palmier` in the slug dir to skip
  re-download.

**Implementation context.**
- Crate: `palmier-project` (`SampleProjectService`), surfaced to `src-ui/home` carousel via Tauri
  commands/events.
- Reference: `Project/SampleProjectService.swift`. Map `URLSession.download` → `reqwest`;
  `withThrowingTaskGroup` → `tokio JoinSet`; ApplicationSupport samples dir → `dirs` crate
  (`%APPDATA%\PalmierProWin\Samples\<slug>\`).
- Section: project-io.md "Sample materialization"; FOUNDATION §6.1 sample flow; FR-4.

**Dependencies.** E1-S1, E1-S6 (Convex HTTP transport / `convexHttpUrl` config), and **cross-epic
Epic 2** (`Timeline`/`MediaManifest` serde shapes the resolved `project`/`manifest` decode into).
The **round-trip fidelity** of the materialized bundle depends on **Spike S-1b** (sample-payload
Date encoding) landing for Epic 2's serde lock — until then, materialization + open works on the
provisional per-field serde with the round-trip regression gate (R-6); Epic 1 materializes the
files regardless.
**Parallel-safe?** Yes for the service + download orchestration (own file in `palmier-project`);
the serde decode of `project`/`manifest` shares `palmier-model` types with Epic 2 (consume, don't
define). Run in its own worktree.

---

### E1-S9 — Settings (5 tabs) + Help/MCP-instructions + Feedback UI (`src-ui/settings`)

**Intent.** As a user, I want the 5 settings tabs, the Help/Shortcuts/MCP-instructions window, and
the feedback dialog, so I can manage account, prefs, models, agent key, and storage exactly as on
the Mac app.

**Acceptance criteria.**
- **Given** Settings opens, **When** rendered, **Then** the `SettingsTab` set is
  **Account/General/Models/Agent/Storage**; the **Account tab is hidden when `is_misconfigured`**;
  General always shows Notifications + Privacy panes (settings-account-app.md "Settings tabs").
- **General → Privacy:** "Send anonymous crash and error reports" toggles
  `io.palmier.pro.telemetry.enabled` (ruling #6) and shows **"Restart Palmier Pro to apply"** when
  the value differs from `enabled_for_current_launch` (launch-snapshotted, restart-required, FR-42).
  **General → Notifications:** toggles `io.palmier.pro.notifications.enabled` (absent ⇒ ON).
- **Agent tab:** Anthropic key `SecureField` (placeholder `sk-ant-...` or masked `•`×36 + last 4),
  Save when draft non-empty else trash when a key exists, persisting via E1-S6's keyring
  (`anthropic-api-key`). MCP section: green/grey dot + "Running on 127.0.0.1:19789" / "Stopped"
  reflecting **actual server liveness** (not just the pref — settings-account-app.md gotcha), a
  toggle to `set_mcp_enabled`, and a "Setup instructions" link to the Help MCP tab. (The live MCP
  server is Epic 7; this story reads a liveness stub returning the pref + start-result.)
- **Models / Storage / Account tabs** render per settings-account-app.md "Settings tabs"
  (Models: per-model enable toggles via model catalog; Storage: cache/index/model byte sizes +
  clear buttons; Account: signed-in subscription+credits vs signed-out plan cards + "Sign in with
  Google", `TopOffField` range **$5–$1000**, default top-off $20).
- **Help window (2 tabs):** Shortcuts tab matches the §6.1 menu table; MCP tab shows endpoint
  `http://127.0.0.1:19789/mcp` + Cursor/Claude Desktop/Claude Code/Codex install snippets verbatim
  (settings-account-app.md "Help / MCP instructions content"). **Feedback dialog:** message +
  may-contact + optional email + optional screenshot (PNG base64) → `feedback:send` (E1-S6).

**Implementation context.**
- Crate/surface: `src-ui/settings` (React port of 5 tabs + Help + Feedback), calling `palmier-auth`
  (account/billing/key) + `palmier-telemetry` (privacy snapshot) + settings commands. Tokens from
  AppTheme (FOUNDATION §9; computed hexes `#F29933`/`#F5EFE4` per ruling #21 — full Inspector/
  toolbar token work is Epic 12, this story uses the shared token set).
- Reference: `Settings/*` (`SettingsView`, `AccountPane`, `NotificationsPane`, `PrivacyPane`,
  `ModelsPane`, `AgentPane`, `StoragePane`), `Help/*` (`HelpView`, `ShortcutsPane`,
  `MCPInstructionsPane`, `FeedbackView`, `FeedbackScreenshot`). Map per settings-account-app.md
  "Mapping to FOUNDATION crates" (`src-ui/settings`).
- Section: settings-account-app.md "Settings tabs" + "Help / MCP instructions content";
  FOUNDATION §6.15.

**Dependencies.** E1-S1, E1-S2 (privacy snapshot), E1-S6 (account/key/billing/feedback),
E1-S4 (settings/help/feedback windows exist). The MCP status row reads an Epic-7 liveness stub.
**Parallel-safe?** Yes — own surface `src-ui/settings/*`; depends on S6/S2 via defined command
contracts. The Account fragment file boundary is shared with S6's Clerk integration — coordinate
that one file. Run in its own worktree.

---

### E1-S10 — Tauri updater glue + update badge (`palmier-update`)

**Intent.** As a user, I want the app to check for and surface updates via the Tauri Ed25519
updater so I get the same "update available" UX the Mac app had with Sparkle.

**Acceptance criteria.**
- **Given** boot / "Check for Updates" menu item, **When** invoked, **Then** the Tauri 2 updater
  (Ed25519-signed JSON manifest) checks the manifest URL and surfaces `update_available` +
  `update_version` via a Tauri event, consumed by an update-badge UI (FR-43).
- **Given** no signed feed / dev build, **When** the updater initializes, **Then** it **silently
  disables** (no spurious "check failed" UI) — replicating Sparkle's "no-op unless `.app` +
  `SUFeedURL`" behavior (settings-account-app.md updater gotcha).
- **Given** v1 channel policy, **When** configured, **Then** a single **`stable`** channel is used
  (OQ-1-update working decision); manifest URL is build/backend config (OQ-9). EdDSA signing model
  matches the reference.

**Implementation context.**
- Crate: `palmier-update` (Tauri updater glue exposing `update_available`/`update_version` +
  "check now" command; gate on signed-manifest presence), + the badge in `src-ui/app`.
- Reference: `App/Updater.swift` (`SPUStandardUpdaterController` wrapper), `App/UpdateBadgeView.swift`.
  Map Sparkle → Tauri 2 updater (same EdDSA model); `updateAvailable`/`updateVersion` → Tauri event.
- Section: settings-account-app.md "Updater" + "macOS/Apple APIs to replace" (Sparkle → Tauri
  updater); FR-43.

**Dependencies.** E1-S1, E1-S3 ("Check for Updates" menu item dispatches here).
**Parallel-safe?** Yes — own crate `palmier-update` + a self-contained badge component; the menu
item (S3) and boot hook (S1) call a `pub fn` defined here. Run in its own worktree.

---

## Story summary & sequencing

- **Foundational, must land first:** **E1-S1** (boot skeleton). Everything depends on it.
- **Sequential-after-S1 (touch S1's builder):** **E1-S3** (menu registration). Then E1-S10's menu
  item depends on S3.
- **Parallel-safe siblings (own crate/surface, only thin boot-call coordination with S1):**
  E1-S2 (`palmier-telemetry`), E1-S4 (windows + `src-ui/home` shell), E1-S5 (fonts/constants),
  E1-S6 (`palmier-auth`), E1-S7 (registry/lifecycle — coordinate `palmier-project` ownership with
  Epic 2), E1-S8 (samples — consumes Epic 2 model types + E1-S6 transport), E1-S9 (`src-ui/settings`
  — consumes S6/S2/S4), E1-S10 (`palmier-update`).
- **Cross-epic dependency:** real Save/Open round-trip (E1-S7) and sample round-trip fidelity
  (E1-S8) require **Epic 2** (`palmier-project` bundle I/O + `palmier-model` serde, gated for
  fidelity by Spike S-1b). Epic 1 lands the orchestration + materialization against Epic 2's types;
  byte-identical round-trip is asserted in Epic 2 (SM-7 / SM-1b).

**Epic-1 exit (M1 contribution):** boot < 3 s offline (SM-1), all §6.1 shortcuts fire (FR-3),
registry round-trips newest-first + trash-on-delete (FR-2), a resolved sample materializes with the
reference filenames + opens (FR-4), settings/account/telemetry/updater scaffolding present
(FR-1/§6.15/§6.16). The §11.3 hand-editing e2e (the M1 exit gate) builds on this shell once Epics
2/3/5/6 land.
