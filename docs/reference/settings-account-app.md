---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# settings-account-app — reference port notes

## Purpose
Implementation-level reference for the macOS subsystems that map to `palmier-auth`, `palmier-update`,
`palmier-telemetry`, and `src-ui/settings`: app boot/lifecycle, window config, main menu + shortcuts,
the 5 Settings tabs (Account/General/Models/Agent/Storage), Clerk auth + Convex billing, token/keychain
handling, the Sparkle updater, Sentry telemetry + crash logging, notifications, and Help/MCP-instructions
content. Derived read-only from `../palmier-pro/Sources/PalmierPro/{App,Settings,Account,Help,Toolbar,Telemetry,Utilities}`.

## Key types & files (cite paths under Sources/PalmierPro/...)
- `App/main.swift` — entry point; boot order (see below).
- `App/AppDelegate.swift` — `applicationDidFinishLaunching`, menu-action selectors (showSettings/showKeyboardShortcuts/showMCPInstructions/showFeedback/showTutorial), reopen handling.
- `App/MainMenu.swift` — `MainMenuBuilder.buildMenu()`; all menus + shortcuts; `EditorActions` @objc responder-chain protocol.
- `App/Updater.swift` — Sparkle `SPUStandardUpdaterController` wrapper (`Updater.shared`, @Observable, `updateAvailable`/`updateVersion`).
- `App/UpdateBadgeView.swift` — UI badge bound to `Updater.shared`.
- `App/AppState.swift` — `AppState.shared`; project lifecycle (new/open/sample), MCP service start/stop, notification reveal, autosave-on-home.
- `App/AppNotifications.swift` — `UNUserNotificationCenter` wrapper; `generationComplete(...)`, enabled pref `io.palmier.pro.notifications.enabled`.
- `Settings/SettingsView.swift` — `SettingsTab` enum (account/general/models/agent/storage), sidebar+detail, `SettingsWindowController.shared`, `SettingsToggleRow`.
- `Settings/AccountPane.swift` — signed-in (subscription+credits) vs signed-out (plan cards + Sign in with Google).
- `Settings/{NotificationsPane,PrivacyPane}.swift` — the two rows under "General".
- `Settings/ModelsPane.swift` — per-model enable toggles via `ModelPreferences.shared` + `ModelCatalog.shared`.
- `Settings/AgentPane.swift` — Anthropic key SecureField + MCP server status/toggle.
- `Settings/StoragePane.swift` — cache/index/model byte sizes + clear buttons; `DiskCache.rootDirectory`.
- `Account/AccountService.swift` — `AccountService.shared`; Clerk + Convex; auth state machine, plans, account, billing actions, feedback.
- `Account/BackendConfig.swift` — reads `Info.plist` keys: `PalmierClerkPublishableKey`, `PalmierConvexDeploymentURL`, `PalmierConvexHttpURL`.
- `Account/{AccountPopoverCard,CreditSummaryView,IdentityViews,TopOffField}.swift` — account UI fragments.
- `Help/{HelpView,ShortcutsPane,MCPInstructionsPane,FeedbackView,FeedbackScreenshot}.swift` — Help window (2 tabs) + feedback window.
- `Toolbar/ToolbarView.swift` — editor toolbar (undo/redo, pointer/razor, split/trim, add-text, log-mapped zoom slider).
- `Telemetry/Telemetry.swift` — Sentry init + breadcrumb/capture/trace; enabled pref `io.palmier.pro.telemetry.enabled`.
- `Utilities/Log.swift` — categorized `os.Logger` + signal/exception crash handler → `~/Library/Logs/PalmierPro/crash.log`.
- `Utilities/KeychainStore.swift` — generic-password Keychain CRUD (service = `bundleIdentifier` ?? `io.palmier.pro`).
- `Utilities/{Constants,BundledFonts,DiskCache,...}.swift` — layout/defaults constants, CoreText font registration.
- `Agent/Clients/AnthropicClient.swift` — `AnthropicKeychain` (account = `"anthropic-api-key"`; DEBUG env override `ANTHROPIC_API_KEY`).
- `Agent/MCP/MCPService.swift` — `MCPService.port = 19789`; enabled pref `io.palmier.pro.mcp.enabled` (default true).

## Core behaviors & algorithms (concrete — downstream story/dev agents implement from this)
**Boot order (`main.swift`, before `app.run()`):**
1. `Log.bootstrap()` — installs crash handler (NSSetUncaughtExceptionHandler + signal handlers for SIGSEGV/SIGABRT/SIGBUS/SIGILL/SIGFPE/SIGTRAP), opens `crash.log` fd `O_WRONLY|O_CREAT|O_APPEND`, logs launch pid.
2. `Telemetry.start()` — snapshots `enabledForCurrentLaunch` from pref (default true); starts Sentry only if enabled AND DSN non-empty.
3. `BundledFonts.register()` — CoreText-register all `.ttf/.otf` under `Resources/Fonts/` (`.process` scope).
4. `AccountService.shared.configure()` — Clerk + Convex init (idempotent via `didConfigure`).
5. `ModelCatalog.shared.configure()`.
6. Set `NSInitialToolTipDelay = 10` (UserDefaults).
7. Build `NSApplication`, attach `AppDelegate`, set `app.mainMenu`, `app.run()`.

**`applicationDidFinishLaunching`:** `setActivationPolicy(.regular)` + `activate`; touch `Updater.shared` (starts Sparkle); `HomeWindowController.shared.showWindow`; `AppNotifications.configure()`; `AppState.shared.startMCPService()`. `applicationShouldOpenUntitledFile → false`. Reopen with no windows → `showHome()`.

**Window config (replicate exactly):**
- Settings window: content 980×640, min 760×480, autosave name `PalmierProSettings-v2`, dark, transparent titlebar, fullSizeContentView, movable-by-background. SettingsView frame min 760×480 / ideal 980×640.
- Help window: content 900×560, min 820×520, autosave `PalmierProHelp-v1`. HelpView min 820×520.
- Feedback window: 480×480, min 480×420, `isReleasedWhenClosed=false`.
- (FOUNDATION §6.1 gives Home 1200×1200 / min 760×480 and Project 1600×1000 / min 960×600 — those windows live outside this subtree; not contradicted here.)

**Project lifecycle (`AppState`):** New = `NSSavePanel` (allowedContentType `io.palmier.project`, default name "Untitled Project", dir `~/Documents/Palmier Pro`) → create `VideoProject`, save, register in `ProjectRegistry`. Open = `NSOpenPanel` (single file, packages-not-traversed). `showHome()` autosaves the active project (if `isDocumentEdited`) before ordering its windows out. `openSample(slug:)` uses `SampleProjectService` cache-or-materialize. Notification click → `revealGeneratedAssetFromNotification` activates app, finds project by `projectPath` or `assetId`, selects+reveals the asset in the media panel.

**Settings tabs:**
- Account tab hidden when `account.isMisconfigured` (`visibleTabs` filter); General always shows NotificationsPane + PrivacyPane.
- AccountPane: `isLoading` → "Loading…"; signed-in+paid → subscription section (planLabel, "Cancels <date>" in orange if `cancelAtPeriodEnd`, Manage subscription) + credits section (Remaining card with `CreditSummaryView` + "Resets <date>"; Buy-more card with `TopOffField`, range $5–$1000); signed-in+free → plan cards (Pro primary / Max secondary) from `availablePlans`; signed-out → "Sign in with Google". `lastError` shown red.
- ModelsPane: search field; sections Image/Video/Audio from `ModelCatalog.{image,video,audio}`; per-row toggle via `ModelPreferences.shared.isEnabled/setEnabled(id)`.
- AgentPane: SecureField with placeholder `sk-ant-...` or masked key (`•`×36 + last 4); Save when draft non-empty else trash button when key exists; loads/saves via `AnthropicKeychain`. MCP section: green/grey dot + "Running on 127.0.0.1:19789" or "Stopped"; toggle → `appState.setMCPEnabled`; "Setup instructions" → Help MCP tab.
- StoragePane: Cache row (sum of `[ImageVideoGenerator.cache, MediaVisualCache.diskCache]`, path shown `~`-relativized, "Clear cache"); Media-search section (toggle → `VisualModelLoader.setEnabled`, index bytes from `EmbeddingStore.directory`, model bytes from `ModelDownloader.modelsDir`, clear/remove buttons).
- PrivacyPane: "Send anonymous crash and error reports" toggle → `Telemetry.isEnabled`; shows "Restart Palmier Pro to apply" when value ≠ `enabledForCurrentLaunch`.

**Auth/account state machine (`AccountService`):** `configure()` returns `isMisconfigured=true` if Clerk key or Convex URL missing. Else `Clerk.configure(publishableKey, redirectUrl: "palmier://callback", scheme: "palmier")`, build `ConvexClientWithAuth(deploymentUrl, ClerkConvexAuthProvider())`, subscribe `billing:listPlans`. Auth observation: wait up to 50×100ms for `Clerk.shared.isLoaded`; then for each `convex.authState`: `.loading`→isLoading; `.authenticated`→`provisionAndSubscribe()` (3-try `users:upsertFromAuth` mutation with email/name/image, then subscribe `account:get`); `.unauthenticated`→clear. Derived: `tier` from `account.user.tier` (none/pro/max); `budgetCredits = plan.monthlyBudgetCredits + user.purchasedCredits`; `remainingCredits = max(0, budget - spentCreditsThisPeriod)`; `aiAllowed = isSignedIn && !isMisconfigured`. Billing actions call Convex actions `billing:createCheckoutSession`/`createTopOffCheckoutSession`/`createPortalSession` → URL opened only if scheme=https AND host ∈ {checkout.stripe.com, billing.stripe.com}. Feedback → Convex action `feedback:send` (message, mayContact, appVersion, osVersion, optional email + screenshotPngBase64).

**Token/keychain:** `KeychainStore` = generic password, service=bundleId, accessible=`AfterFirstUnlock`, update-then-add. Anthropic key account=`"anthropic-api-key"`; on save/delete posts `anthropicAPIKeyChanged`; DEBUG reads `ANTHROPIC_API_KEY` env first. (FOUNDATION §6.13 names the key `palmier-pro-anthropic-api-key` — reconcile; account string differs.)

**Updater:** `Updater.init` no-ops unless running from a `.app` bundle AND `SUFeedURL` Info key present; else builds `SPUStandardUpdaterController(startingUpdater:true, delegate:self)` and `checkForUpdateInformation()`. Delegate sets `updateAvailable`/`updateVersion` from `SUAppcastItem.displayVersionString`. Menu "Check for Updates…" → `controller.checkForUpdates`.

**Telemetry/logging:** Sentry options: `sendDefaultPii=false`, env development(DEBUG)/production, `tracesSampleRate=0.1`, `appHangTimeoutInterval=8.0`, `attachStacktrace=true`, `enableCaptureFailedRequests=false`, `enableUncaughtNSExceptionReporting=true`, releaseName `palmier-pro@<CFBundleShortVersionString>+<CFBundleVersion>`. `Log` categories: app/editor/export/preview/mcp/generation/project/transcription/search; `warning`→Sentry breadcrumb, `error`/`fault`→Sentry capture-message; all levels mirror to stderr. Crash handler is async-signal-safe (write/backtrace/fsync/raise only).

**Help / MCP instructions content (`MCPInstructionsPane`):** endpoint `http://127.0.0.1:19789/mcp`. Cursor: deep-link `cursor://anysphere.cursor-deeplink/mcp/install?name=palmier-pro&config=<base64(urlencoded({"type":"http","url":endpoint}))>` + manual `~/.cursor/mcp.json`. Claude Desktop: "Install" opens bundled `palmier-pro.mcpb` from `Bundle.resourceURL`; manual JSON uses `npx -y mcp-remote <endpoint> --allow-http --transport http-only`. Claude Code: `claude mcp add --transport http palmier-pro <endpoint>`. Codex: `codex mcp add palmier-pro --url <endpoint>`.

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `NSApplication`/`AppDelegate`/`NSApp.activate`/`setActivationPolicy` → Tauri app lifecycle (`tauri::Builder`, `setup`, window-show). No activation-policy concept.
- `NSMenu`/`NSMenuItem`/`#selector` responder chain → Tauri `Menu`/`MenuItem` + accelerator strings; Cmd→Ctrl, `Cmd+F` fullscreen → F11 (per FOUNDATION §6.1). Editor actions dispatched as Tauri commands/events instead of `NSApp.sendAction`.
- `NSWindow`/`NSWindowController`/`setFrameAutosaveName`/`NSHostingController` → Tauri `WebviewWindow` configs (size, minSize, decorations, transparent titlebar via `titleBarStyle`); persist size/pos via `tauri-plugin-window-state` (replaces autosave names).
- `NSSavePanel`/`NSOpenPanel`/`UTType`/`NSDocumentController`/`NSDocument` autosave → `tauri-plugin-dialog` file pickers + custom `.palmier` directory-as-document handling (FOUNDATION §5.7); reimplement registry + autosave in `palmier-project`.
- Sparkle (`SPUStandardUpdaterController`, `SUAppcastItem`, `SUFeedURL`) → **Tauri 2 updater** (Ed25519-signed JSON manifest) in `palmier-update`. Same signing model (EdDSA). `updateAvailable`/`updateVersion` surfaced via Tauri event.
- Keychain Security framework (`SecItemAdd/Update/CopyMatching/Delete`, `kSecAttrAccessibleAfterFirstUnlock`) → `keyring` crate: Windows Credential Manager, Linux Secret Service/libsecret. Service `palmier-pro`, key `palmier-pro-anthropic-api-key` (FOUNDATION) / account `anthropic-api-key` (reference) — pick one, document.
- `UNUserNotificationCenter`/`UNMutableNotificationContent` → `tauri-plugin-notification` (Windows toast, Linux libnotify). Click-action userInfo (`assetId`/`projectPath`) → Tauri deep-link/event to reveal asset.
- `os.Logger`/`os_log` → `tracing` + `tracing-subscriber` (FOUNDATION §2.2). Stderr mirror trivially kept.
- Sentry Apple SDK → **Sentry Rust SDK** (backend) + Browser SDK (frontend); same option semantics (sample rate 0.1, pii off, release name format, environment).
- Crash handler (`NSSetUncaughtExceptionHandler`, POSIX `signal`/`backtrace`) → Rust panic hook + `signal-hook`/`backtrace` crate or rely on Sentry native crash handler; write `crash.log` under app data dir (not `~/Library/Logs`).
- `CoreText` `CTFontManagerRegisterFontURLs`/`NSFontManager.availableFontFamilies` → `fontdb` (system + bundled enumeration) in `palmier-text` (FOUNDATION §6.6).
- `NSWorkspace.shared.open(url)` → `tauri-plugin-opener` / `open` crate (keep the https + allowlisted-host guard).
- `NSPasteboard` (copy buttons) → Tauri clipboard plugin.
- `NSScreen`/screenshot capture (`FeedbackScreenshot.captureMainWindow`) → Tauri webview screenshot or `tauri::WebviewWindow` capture; PNG base64 to Convex.
- `ProcessInfo.operatingSystemVersion` / `CFBundle*` Info.plist reads (`BackendConfig`, version strings) → Tauri build-time config (`tauri.conf.json` + compile-time env) and `tauri::api` version.
- ClerkKit/ClerkConvex/ConvexMobile native SDKs → `@clerk/clerk-react` in webview (FOUNDATION §2.2) + Convex over HTTP via `reqwest` in `palmier-auth`/`palmier-gen`; OAuth redirect `palmier://callback` → Tauri deep-link plugin scheme `palmier`.

## Mapping to FOUNDATION crates
- **palmier-auth** ← `Account/AccountService.swift`, `Account/BackendConfig.swift`, `KeychainStore`, `AnthropicKeychain`. Holds: Clerk token cache (forwarded as `Bearer` to Convex), account/plan/credit state, billing action calls, OS-keyring API-key CRUD, misconfigured guard.
- **palmier-update** ← `App/Updater.swift`, `App/UpdateBadgeView.swift`. Tauri updater glue exposing `update_available`/`update_version` + "check now" command; gate on signed manifest presence.
- **palmier-telemetry** ← `Telemetry/Telemetry.swift`, `Utilities/Log.swift`. Sentry init from build config + `tracing` subscriber + categorized loggers + crash file. Honors privacy toggle (snapshot at launch, restart-required).
- **src-ui/settings** ← `Settings/*`, `Help/*`, `Toolbar/ToolbarView.swift`, `Account/*` UI fragments. React port of 5 settings tabs, Help window (Shortcuts + MCP tabs), feedback dialog, account cards. Tokens from AppTheme (FOUNDATION §9). MCP status/toggle and model-enable prefs call backend commands.

## Port risks & gotchas
- **Filename drift:** reference `Project` constants are `project.json` / `media.json` / `project-registry.json` / `generation-log.json`; FOUNDATION §5.7 says `timeline.json` / `manifest.json` / `registry.json` / `generation_log.json`. Pick the FOUNDATION names but know the reference's on-disk names differ (no migration since clean-room).
- **Settings persistence is UserDefaults-backed booleans** (telemetry/notifications/mcp all default *true* when key absent). Replicate the "absent = on" semantics exactly, and store under FOUNDATION's `settings.json` path (§6.1), not the macOS preference domain.
- **Telemetry + privacy are launch-snapshotted** (`enabledForCurrentLaunch`): toggling requires restart to take effect. Keep the "Restart required" UX.
- **Sparkle no-op unless `.app` + `SUFeedURL`** — Tauri updater similarly should silently disable when no signed feed configured (dev builds), to avoid spurious "check failed" UI.
- **Billing URL allowlist** ({checkout,billing}.stripe.com, https only) is a security control — port it verbatim into the `open` path.
- **Claude Desktop manual config diverges from `.mcpb`:** reference ships `palmier-pro.mcpb` AND documents an `npx mcp-remote ... --allow-http --transport http-only` JSON. FOUNDATION §6.14 leans on the `.mcpb`. Implement both; the manual flag set (`--allow-http`, `http-only`) is load-bearing for loopback http.
- **Anthropic key name mismatch** (account `anthropic-api-key` vs FOUNDATION `palmier-pro-anthropic-api-key`): a wrong choice silently loses the user's saved key. Decide once.
- **MCP enable toggle reads liveness, not the pref** (`mcpStatusRow` toggle get = `mcpService?.isRunning`). On Win, ensure the toggle reflects actual server state after start failures (port already bound).
- **Crash-log path** `~/Library/Logs/PalmierPro/crash.log` must move to `%APPDATA%\PalmierProWin\logs\` / `~/.local/state/palmier-pro/`.
- **Window autosave names** (`-v2`, `-v1`) imply prior schema bumps; pick equivalent state-keys and don't collide.
- **No haptics here** but Snap haptic (FOUNDATION §6.3) is the only `NSHapticFeedback` use; it's silent/no-op on Win/Linux (not in this subtree).

## Open questions
- Exact `ModelPreferences` storage format + default-enabled set (file not in this subtree; ModelsPane only calls `isEnabled/setEnabled`).
- `CreditSummaryView`/`TopOffField` exact formatting + min/step (range $5–$1000 confirmed; default top-off $20).
- Does `users:upsertFromAuth` / `account:get` Convex schema match the Windows Convex deployment, or is a parallel backend assumed? (FOUNDATION says same Convex.)
- Sample-project filenames inside materialized bundles (registry uses reference filenames) vs FOUNDATION's renamed bundle layout.
- Tutorial walkthrough (`editor.tour`) content — referenced from Help menu but defined outside this subtree.
