//! FOUNDATION §6.1 boot sequence (E1-S1).
//!
//! Ports the reference `App/main.swift` + `App/AppDelegate.swift` boot order onto
//! the Tauri 2 lifecycle. The reference order (verbatim from `main.swift`):
//!
//! ```text
//! Log.bootstrap()            // 1. crash handler (Sentry + native panic hook)
//! Telemetry.start()          // 2. tracing/Sentry subscriber
//! BundledFonts.register()    // (fonts; E1-S5 owns the real impl)
//! AccountService.configure() // 4. Clerk + Convex clients from build-time config
//! ModelCatalog.configure()   // 5. /v1/models — async, non-blocking
//! // 6. start MCP server if settings.mcp_enabled  (AppDelegate.startMCPService)
//! // 7. show Home window                          (AppDelegate.showWindow)
//! ```
//!
//! Steps 1–4 + 7 run **synchronously** on the boot path; step 5 (`ModelCatalog`)
//! is `tokio::spawn`ed and **never awaited before window show** so a slow/offline
//! Convex never stalls cold start (SM-1, FR-1).
//!
//! In this story the real subsystems are wired where they exist:
//! - **crash handler + tracing** (steps 1–2): owned by `palmier_telemetry::init`,
//!   which installs Sentry (if enabled + DSN), the categorized daily-rotated file
//!   subscriber, **and** the crash hook. E1-S3's integration removed the boot's
//!   own stub `tracing_subscriber` so telemetry owns subscriber setup — the
//!   returned [`palmier_telemetry::TelemetryHandle`] is held for the process
//!   lifetime in Tauri managed state.
//! - **client config** (step 4): `palmier_auth::Auth::init(config)` is called and
//!   the [`palmier_auth::Auth`] handle is stored in Tauri managed state.
//! - **model catalog** (step 5): the async fetch is a logged stub.
//! - **MCP start** (step 6): a no-op hook returning the pref; real server is Epic 7.

use crate::settings::{self, Settings};

/// Build-injected Sentry DSN (FOUNDATION §6.16: `tauri.conf.json` →
/// `PALMIER_SENTRY_DSN`). Read at compile time; absent ⇒ `None` ⇒ Sentry stays
/// off even when the telemetry pref is ON (reference: DSN must be non-empty).
/// A runtime `PALMIER_SENTRY_DSN` env var overrides the compile-time value (lets
/// CI/ops point at a DSN without a rebuild).
fn sentry_dsn() -> Option<String> {
    std::env::var("PALMIER_SENTRY_DSN")
        .ok()
        .or_else(|| option_env!("PALMIER_SENTRY_DSN").map(str::to_owned))
        .filter(|d| !d.trim().is_empty())
}

/// Build-injected Clerk publishable key (`PALMIER_CLERK_PUBLISHABLE_KEY`).
fn clerk_publishable_key() -> Option<String> {
    build_config_value("PALMIER_CLERK_PUBLISHABLE_KEY")
}

/// Build-injected Convex deployment (WebSocket) URL (`PALMIER_CONVEX_DEPLOYMENT_URL`).
fn convex_deployment_url() -> Option<String> {
    build_config_value("PALMIER_CONVEX_DEPLOYMENT_URL")
}

/// Build-injected Convex HTTP base URL (`PALMIER_CONVEX_HTTP_URL`).
fn convex_http_url() -> Option<String> {
    build_config_value("PALMIER_CONVEX_HTTP_URL")
}

/// Read a build-time config value: runtime env first (ops/CI override), then the
/// compile-time `option_env!`. Empty/whitespace ⇒ `None`.
fn build_config_value(key: &str) -> Option<String> {
    let from_env = std::env::var(key).ok();
    let from_compile = match key {
        "PALMIER_CLERK_PUBLISHABLE_KEY" => option_env!("PALMIER_CLERK_PUBLISHABLE_KEY"),
        "PALMIER_CONVEX_DEPLOYMENT_URL" => option_env!("PALMIER_CONVEX_DEPLOYMENT_URL"),
        "PALMIER_CONVEX_HTTP_URL" => option_env!("PALMIER_CONVEX_HTTP_URL"),
        _ => None,
    }
    .map(str::to_owned);
    from_env
        .or(from_compile)
        .filter(|v| !v.trim().is_empty())
}

/// App semantic version, from the Tauri binary's package version (the source of
/// truth for `tauri.conf.json`'s `version`). Reference release name component.
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Build identifier appended to the Sentry release name. Prefers a CI-injected
/// `PALMIER_BUILD_ID` (git sha / build number); falls back to the build profile.
fn build_id() -> String {
    std::env::var("PALMIER_BUILD_ID")
        .ok()
        .filter(|b| !b.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                "dev".to_string()
            } else {
                "release".to_string()
            }
        })
}

/// The outcome of running the synchronous boot steps (1–6). Carried into the
/// Tauri `setup` hook so the window-show step (7) can read it and the long-lived
/// handles (telemetry, auth) can move into Tauri managed state.
///
/// Not `Clone`/`Debug`: it owns the [`palmier_telemetry::TelemetryHandle`]
/// (dropping it flushes logging + Sentry) and the [`palmier_auth::Auth`] handle.
pub struct BootContext {
    pub settings: Settings,
    /// Result of the MCP start hook (step 6): the effective enabled state.
    pub mcp_started: bool,
    /// Telemetry handle — **must outlive the process**. The boot path moves this
    /// into Tauri managed state so it lives as long as the app. Dropping it stops
    /// Sentry + flushes the rotated-file log writer.
    pub telemetry: palmier_telemetry::TelemetryHandle,
    /// Auth subsystem (Clerk JWT cache + account state + Anthropic-key store +
    /// Convex backend). Moved into Tauri managed state so commands can reach it.
    pub auth: palmier_auth::Auth,
}

/// Steps 1–2 — install crash handling + tracing via `palmier_telemetry::init`.
///
/// **The telemetry crate owns subscriber setup.** Earlier (E1-S1) the boot path
/// installed its own stub stderr `tracing_subscriber` *before* telemetry, which
/// made `palmier_telemetry::init`'s file subscriber a no-op (a global subscriber
/// was already set) — so rotated-file logging never attached. E1-S3 removed that
/// stub: this is now the **first and only** subscriber install on the boot path,
/// so the categorized daily-rotated file logger (FOUNDATION §6.16) attaches.
///
/// Returns the [`palmier_telemetry::TelemetryHandle`] the caller must keep alive
/// for the whole process (held in Tauri managed state).
pub fn init_telemetry(telemetry_pref: bool) -> palmier_telemetry::TelemetryHandle {
    // Build the real config: build-injected DSN, the launch-time telemetry pref
    // (absent-⇒-ON already resolved in `Settings`), and real version/build so the
    // Sentry release name is `palmier-pro-win@<version>+<build>`.
    let config = palmier_telemetry::TelemetryConfig::new(
        sentry_dsn(),
        Some(telemetry_pref),
        app_version(),
        build_id(),
    );
    // `init` installs (1) Sentry [if enabled + DSN], (2) the tracing subscriber
    // [file rotation + stderr + Sentry layer], (3) the crash/panic hook. This is
    // the seam E1-S2/E1-S6 flagged: telemetry — not boot — owns subscriber setup.
    let handle = palmier_telemetry::init(&config);

    // Record the FIRST telemetry init's `file_logging` result for the seam test.
    // Because the global tracing subscriber can be installed only once per
    // process, only the first `init` across a test binary actually attaches the
    // file layer; capturing it here lets the seam test assert deterministically
    // (whichever test runs first) that telemetry — with no boot-owned stub
    // subscriber shadowing it — does attach the rotated file logger.
    #[cfg(test)]
    {
        let _ = tests::FIRST_FILE_LOGGING.set(handle.file_logging());
    }

    tracing::debug!(
        target: "app",
        file_logging = handle.file_logging(),
        sentry = handle.sentry_active(),
        "boot 1-2/7: telemetry initialized (crash hook + categorized rotated logging)"
    );
    handle
}

/// Step 3 — read settings from `settings.json` (absent ⇒ defaults).
pub fn read_settings() -> Settings {
    let s = match settings::settings_path() {
        Some(path) => Settings::read_from(&path),
        None => {
            tracing::warn!(target: "app", "could not resolve settings dir; using defaults");
            Settings::default()
        }
    };
    tracing::debug!(
        target: "app",
        mcp = s.mcp_enabled,
        notifications = s.notifications_enabled,
        telemetry = s.telemetry_enabled,
        has_seen_welcome = s.has_seen_welcome,
        "boot 3/7: settings read"
    );
    s
}

/// Step 4 — configure Clerk + Convex clients from build-time config.
///
/// Builds the [`palmier_auth::AuthConfig`] from the build-injected Clerk key +
/// Convex URLs and calls [`palmier_auth::Auth::init`]. `Auth::init` is infallible
/// w.r.t. configuration: a missing key/URL yields a misconfigured `Auth` (Account
/// tab hidden downstream) rather than an error, and it does **no** network I/O on
/// the boot path — so this step is offline-safe and cannot stall cold start
/// (FR-1 / SM-1 / OQ-9 / R-4).
pub fn configure_clients() -> palmier_auth::Auth {
    let config = palmier_auth::AuthConfig::builder()
        .clerk_publishable_key(clerk_publishable_key().unwrap_or_default())
        .convex_deployment_url(convex_deployment_url().unwrap_or_default())
        .convex_http_url(convex_http_url().unwrap_or_default())
        .build();
    let auth = palmier_auth::Auth::init(config);
    tracing::debug!(
        target: "app",
        misconfigured = auth.is_misconfigured(),
        has_convex = auth.convex().is_some(),
        "boot 4/7: Clerk + Convex configured (palmier-auth)"
    );
    auth
}

/// Step 5 — load the model catalog (`/v1/models`), **spawned async, non-blocking**.
///
/// The fetch is `tokio::spawn`ed and never awaited before window show. A failed
/// fetch degrades to a cached/empty catalog and only logs — it must never stall
/// cold start (FR-1 / SM-1; decoupled from R-4 / OQ-9). Real fetch is a later epic.
pub fn spawn_model_catalog_load(handle: &tauri::async_runtime::RuntimeHandle) {
    handle.spawn(async {
        // Stub for the async `/v1/models` GET (24h-cached). Real impl is later;
        // an unreachable Convex here must degrade to empty, never panic/stall.
        tracing::info!(
            target: "generation",
            "boot 5/7: model-catalog load spawned (async, non-blocking; stubbed fetch)"
        );
    });
}

/// Step 6 — start the MCP server if `settings.mcp_enabled` (default true).
///
/// No-op/stub hook in this epic; the real local MCP server on
/// `127.0.0.1:19789` is Epic 7 (`palmier-mcp`). Returns the effective started
/// state (here: simply the pref) so the Settings Agent-tab liveness row (E1-S9)
/// has a value to read.
pub fn start_mcp_if_enabled(settings: &Settings) -> bool {
    if settings.mcp_enabled {
        let _ = palmier_mcp::DEFAULT_BIND;
        tracing::info!(
            target: "mcp",
            bind = palmier_mcp::DEFAULT_BIND,
            "boot 6/7: MCP start hook (stub; real server is Epic 7 / palmier-mcp)"
        );
        true
    } else {
        tracing::info!(target: "mcp", "boot 6/7: MCP disabled by settings; not started");
        false
    }
}

/// Run the synchronous boot steps 1–6 in the reference order and return the
/// context the Tauri `setup` hook needs to perform step 7 (show Home window).
///
/// Step 5 (model catalog) is spawned onto the Tauri async runtime and not awaited.
pub fn run_sync_boot() -> BootContext {
    // The reference snapshots the telemetry pref *inside* `Telemetry.start()`
    // (step 2) from UserDefaults. Our settings are file-backed, so we read
    // `settings.json` *first* (a few-µs file read — it cannot stall boot) to get
    // the telemetry pref, then hand it to `palmier_telemetry::init`. This keeps
    // the reference's "telemetry snapshotted at launch" semantics: the pref read
    // here is the value `enabled_for_current_launch` reflects for the whole run.
    //
    // Crucially, there is **no** boot-owned `tracing_subscriber` install before
    // this point anymore — `palmier_telemetry::init` is the first and only
    // subscriber install, so its categorized daily-rotated file logger attaches
    // (the seam E1-S2/E1-S6 flagged; resolved in E1-S3).
    let settings = read_settings(); // 3. settings (read early for the telemetry pref)

    // 1-2. crash handler + tracing/Sentry — owned by palmier-telemetry.
    let telemetry = init_telemetry(settings.telemetry_enabled);

    // (fonts register before window build — reference order; E1-S5 real impl)
    palmier_text::register_bundled_fonts();

    let auth = configure_clients(); // 4. Clerk + Convex (palmier-auth)

    // 5. model catalog is spawned in the Tauri setup hook (needs the runtime
    //    handle); see `spawn_model_catalog_load`.

    let mcp_started = start_mcp_if_enabled(&settings); // 6. MCP start hook

    BootContext {
        settings,
        mcp_started,
        telemetry,
        auth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;
    use std::time::Instant;

    /// `file_logging` result of the **first** `init_telemetry` in this test
    /// binary (set in `init_telemetry` under `#[cfg(test)]`). The global tracing
    /// subscriber installs once per process, so only the first call attaches the
    /// file layer; this captures that one result for the deterministic seam test.
    pub(super) static FIRST_FILE_LOGGING: OnceLock<bool> = OnceLock::new();

    #[test]
    fn sync_boot_runs_in_order_and_returns_context() {
        let ctx = run_sync_boot();
        // Fresh box (no settings.json under the test env) ⇒ defaults: mcp ON.
        // We don't assert the exact pref (the box may have a real settings.json),
        // only that the MCP started state agrees with the pref it read.
        assert_eq!(ctx.mcp_started, ctx.settings.mcp_enabled);
    }

    /// Telemetry owns subscriber setup now: with **no** boot-owned stub
    /// `tracing_subscriber` installed before `palmier_telemetry::init`, that
    /// crate's `init` wins the one-shot global subscriber install and attaches
    /// the rotated file logger (FOUNDATION §6.16). `init_telemetry` records the
    /// **first** init's `file_logging` into `FIRST_FILE_LOGGING`; this test drives
    /// a telemetry init (so a value is always captured) then asserts the first one
    /// attached file logging.
    ///
    /// If a future regression re-introduces a stub subscriber on the boot path
    /// *before* telemetry, telemetry's `init` would lose the `try_init` race and
    /// report `file_logging == false` — flipping the captured value and failing
    /// here. This is the seam guard the story requires.
    #[test]
    fn telemetry_owns_subscriber_and_file_logging_attaches() {
        // Drive a telemetry init (pref ON, no DSN ⇒ Sentry off). Ensures
        // FIRST_FILE_LOGGING is set even if this test runs first.
        let handle = init_telemetry(true);
        assert!(
            handle.enabled_for_current_launch(),
            "telemetry pref ON ⇒ enabled for current launch"
        );

        // The first telemetry init across this process MUST have attached the
        // file logger (a log dir resolves on the dev/CI box). Telemetry owning the
        // subscriber is exactly what makes that true.
        let first = *FIRST_FILE_LOGGING
            .get()
            .expect("init_telemetry records the first file_logging result");
        assert!(
            first,
            "the first telemetry init did not attach file logging — a boot-owned \
             stub tracing_subscriber is shadowing palmier_telemetry::init (the seam \
             is broken)."
        );
    }

    /// Step 4 wires real auth: with no build-injected config it degrades to a
    /// misconfigured `Auth` (Account tab hidden) rather than erroring — boot is
    /// never blocked, and no network I/O happens on this path.
    #[test]
    fn configure_clients_returns_auth_offline_safe() {
        let auth = configure_clients();
        // The dev/CI box has no PALMIER_CLERK_* / PALMIER_CONVEX_* injected, so
        // this is misconfigured — and that is a valid, non-erroring state.
        assert!(auth.is_misconfigured());
        assert!(!auth.account().is_signed_in());
    }

    /// SM-1 proxy: the **synchronous** boot path (steps 1–6, everything that must
    /// complete before the Home window can show) is far under the 3 s cold-start
    /// budget even with Convex modeled as unreachable — the model-catalog fetch
    /// (step 5) is spawned, not awaited, so it cannot contribute to this time.
    #[test]
    fn sync_boot_path_is_well_under_sm1_budget() {
        let start = Instant::now();
        let _ctx = run_sync_boot();
        let elapsed = start.elapsed();
        // Generous ceiling: the real SM-1 target is < 3 s to the window on
        // NVMe/RTX-4060 HW including window paint. The pure sync boot logic must
        // be a tiny fraction of that. 2 s here catches a pathological regression
        // (e.g. someone awaiting the catalog) without flaking on a slow CI box.
        assert!(
            elapsed.as_secs_f64() < 2.0,
            "sync boot took {elapsed:?}, expected << SM-1's 3 s budget"
        );
    }
}
