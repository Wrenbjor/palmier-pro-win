//! # palmier-telemetry
//!
//! Crash reporting + categorized logging for the Palmier Pro Windows/Linux port
//! (FOUNDATION §6.16; E1-S2). Ports the macOS `Telemetry.swift` + `Log.swift`
//! subsystems (settings-account-app.md "Telemetry/logging").
//!
//! Provides, behind one public [`init`] call the Tauri boot path (E1-S1, boot
//! steps 1–2) invokes:
//!
//! 1. **Sentry** (Rust SDK) crash reporting — started **only if** the telemetry
//!    pref is ON (`io.palmier.pro.telemetry.enabled`, absent ⇒ ON, ruling #6)
//!    **and** a non-empty build-injected DSN is present. Options match the
//!    reference: `send_default_pii=false`, `traces_sample_rate=0.1`,
//!    app-hang `8.0`s, `attach_stacktrace=true`, env development/production,
//!    release `palmier-pro-win@<version>+<build>`.
//! 2. **`tracing-subscriber`** with the reference categorized targets
//!    (`app/editor/export/preview/mcp/generation/project/transcription/search`),
//!    a **daily-rotated, 7-day-retained** `palmier.log`, and a stderr mirror.
//!    When Sentry is on, a tracing→Sentry layer maps `warning` ⇒ breadcrumb and
//!    `error`+ ⇒ capture.
//! 3. A **panic/crash hook** writing `crashes/<timestamp>.log` and forwarding to
//!    Sentry.
//!
//! Log/crash paths are per-OS: `%LOCALAPPDATA%\PalmierProWin\Logs\` on Windows,
//! `~/.local/state/palmier-pro/logs/` on Linux (crashes in a `crashes/` subdir).
//!
//! The enabled flag is **snapshotted at launch** ([`pref::LaunchSnapshot`]);
//! toggling it in settings requires a restart (the settings UI reads the
//! snapshot via [`TelemetryHandle::launch_snapshot`]).
//!
//! ## DSN
//! The DSN is **build-injected** (FOUNDATION §6.16: `tauri.conf.json` →
//! `PALMIER_SENTRY_DSN`); this crate never hardcodes it. The boot path reads it
//! from build config/env and passes it in via [`TelemetryConfig`].

pub mod config;
pub mod crash;
pub mod logging;
pub mod paths;
pub mod pref;
pub mod sentry_init;

pub use config::{Environment, TelemetryConfig};
pub use pref::{LaunchSnapshot, TELEMETRY_PREF_KEY};

/// Reference log category targets (settings-account-app.md "Telemetry/logging",
/// FOUNDATION §6.16). Order is load-bearing for the parity test.
pub const CATEGORIES: &[&str] = &[
    "app",
    "editor",
    "export",
    "preview",
    "mcp",
    "generation",
    "project",
    "transcription",
    "search",
];

/// Live telemetry handle returned by [`init`]. **Must be kept alive for the
/// process lifetime** — dropping it flushes + stops the log writer and the
/// Sentry client. The boot path stores it on a long-lived value (e.g. Tauri
/// managed state).
#[must_use = "dropping the handle flushes/stops logging and Sentry"]
pub struct TelemetryHandle {
    _sentry_guard: Option<sentry_init::SentryGuard>,
    _logging_guard: logging::LoggingGuard,
    launch_snapshot: LaunchSnapshot,
    sentry_active: bool,
    file_logging: bool,
}

impl TelemetryHandle {
    /// The launch snapshot of the telemetry pref (absent ⇒ ON). The settings UI
    /// compares the live pref against this to show "Restart required".
    #[must_use]
    pub fn launch_snapshot(&self) -> LaunchSnapshot {
        self.launch_snapshot
    }

    /// Whether telemetry was enabled for this launch.
    #[must_use]
    pub fn enabled_for_current_launch(&self) -> bool {
        self.launch_snapshot.enabled_for_current_launch()
    }

    /// Whether the Sentry client was actually started this launch.
    #[must_use]
    pub fn sentry_active(&self) -> bool {
        self.sentry_active
    }

    /// Whether file logging (rotated `palmier.log`) is active (false ⇒ stderr
    /// only, e.g. no resolvable app data dir).
    #[must_use]
    pub fn file_logging(&self) -> bool {
        self.file_logging
    }
}

/// Initialize telemetry from `config`. Call **once**, early on the boot path
/// (E1-S1 boot steps 1–2). Order within:
///
/// 1. Start Sentry (installs its panic + backtrace integrations) — only when
///    [`TelemetryConfig::should_start_sentry`].
/// 2. Install the tracing subscriber (file rotation + stderr + Sentry layer).
/// 3. Install the panic hook (writes `crashes/<timestamp>.log`, then chains into
///    Sentry's hook from step 1).
///
/// Returns a [`TelemetryHandle`] the caller must keep alive.
pub fn init(config: &TelemetryConfig) -> TelemetryHandle {
    let launch_snapshot = config.launch_snapshot();
    let debug = matches!(config.environment, Environment::Development);

    // 1. Sentry first, so its panic hook is in place before we chain ours.
    let sentry_guard = if config.should_start_sentry() {
        sentry_init::start(config)
    } else {
        None
    };
    let sentry_active = sentry_guard.is_some();

    // 2. Tracing subscriber: file rotation + stderr + (optional) Sentry layer.
    let log_dir = paths::log_dir();
    let logging_guard = logging::init_logging(log_dir.as_deref(), debug, sentry_active);
    let file_logging = logging_guard.file_logging;

    // 3. Panic hook: local crash file + forward to Sentry's hook.
    crash::install(paths::crash_dir());

    tracing::info!(
        target: "app",
        version = %config.version,
        build = %config.build,
        environment = config.environment.as_str(),
        sentry = sentry_active,
        file_logging,
        telemetry_enabled = launch_snapshot.enabled_for_current_launch(),
        "telemetry initialized"
    );

    TelemetryHandle {
        _sentry_guard: sentry_guard,
        _logging_guard: logging_guard,
        launch_snapshot,
        sentry_active,
        file_logging,
    }
}

/// Backward-compatible boot shim for the call site E1-S1 reserved
/// (`palmier_telemetry::start(enabled, dsn)` in `palmier-tauri`'s `boot.rs`).
///
/// Bridges the simple boot bool/DSN to the full [`init`] path: builds a minimal
/// [`TelemetryConfig`] (version/build from this crate's package version as a
/// placeholder — the full integration touch will pass the real Tauri build
/// config) and runs [`init`]. **Do not** extend `palmier-tauri` to call this;
/// the richer [`init`] is the API the integration step should adopt. The handle
/// is intentionally leaked here (`Box::leak`) so Sentry + the log writer stay
/// alive for the process even though the boot stub keeps no handle — the
/// integration step should switch to holding the [`TelemetryHandle`] instead.
///
/// ## Integration caveat (orchestrator)
/// `boot.rs` already installs its own stderr `tracing_subscriber` *before*
/// calling this, so [`init`]'s `try_init` for the **file** subscriber no-ops and
/// rotated-file logging will not attach until the integration step removes the
/// stub subscriber and lets this crate own subscriber setup. Sentry, the crash
/// hook, and the launch snapshot are unaffected and work through this shim.
pub fn start(enabled: bool, dsn: Option<&str>) {
    let config = TelemetryConfig::new(
        dsn.map(str::to_owned),
        Some(enabled),
        env!("CARGO_PKG_VERSION"),
        "dev",
    );
    let handle = init(&config);
    // Keep telemetry alive for the process lifetime (boot stub holds no handle).
    Box::leak(Box::new(handle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_are_the_nine_reference_targets() {
        assert_eq!(CATEGORIES.len(), 9);
        assert_eq!(CATEGORIES[0], "app");
        assert_eq!(CATEGORIES[8], "search");
    }

    #[test]
    fn init_without_dsn_runs_and_keeps_sentry_off() {
        // No DSN ⇒ Sentry stays off, but logging + crash hook still install.
        // (Runs the real subscriber init; if another test already set the
        // global subscriber, init_logging no-ops, which is fine.)
        let cfg = TelemetryConfig::new(None, None, "0.1.0", "test");
        let handle = init(&cfg);
        assert!(!handle.sentry_active());
        // Absent pref ⇒ enabled for current launch (ruling #6).
        assert!(handle.enabled_for_current_launch());
    }

    #[test]
    fn start_shim_compiles_and_runs() {
        // The E1-S1 boot call site: `start(enabled, dsn)`. No DSN ⇒ no Sentry,
        // and it must not panic.
        start(true, None);
    }
}
