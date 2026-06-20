//! Sentry Rust SDK initialization with reference option parity.
//!
//! Reference (settings-account-app.md "Telemetry/logging", constants confirmed
//! in the story): `send_default_pii = false`, `traces_sample_rate = 0.1`,
//! app-hang timeout `8.0`s (where supported), `attach_stacktrace = true`,
//! environment `development`(debug)/`production`, release
//! `palmier-pro-win@<version>+<build>`. The DSN is build-injected; Sentry only
//! starts when the pref is ON **and** the DSN is non-empty (enforced by the
//! caller via [`TelemetryConfig::should_start_sentry`]).

use crate::config::TelemetryConfig;

/// Reference Sentry constants (exposed for assertions / parity tests).
pub mod constants {
    /// Performance trace sampling rate.
    pub const TRACES_SAMPLE_RATE: f32 = 0.1;
    /// App-hang detection timeout, seconds (reference `appHangTimeoutInterval`).
    /// The Rust SDK has no app-hang option; recorded for parity + a future
    /// native-handler wiring. Not silently dropped — see `app_hang_timeout`.
    pub const APP_HANG_TIMEOUT_SECS: f64 = 8.0;
    /// Reference `sendDefaultPii`.
    pub const SEND_DEFAULT_PII: bool = false;
    /// Reference `attachStacktrace`.
    pub const ATTACH_STACKTRACE: bool = true;
}

/// The app-hang timeout (reference parity). The Sentry Rust SDK does not expose
/// an app-hang option, so this value is carried for documentation/parity and is
/// available to a future native app-hang watchdog rather than being dropped.
#[must_use]
pub fn app_hang_timeout() -> std::time::Duration {
    std::time::Duration::from_secs_f64(constants::APP_HANG_TIMEOUT_SECS)
}

/// A live Sentry guard. Dropping it flushes + shuts down the client, so the boot
/// path keeps it alive for the process lifetime.
pub type SentryGuard = sentry::ClientInitGuard;

/// Start Sentry from `config`. Returns `Some(guard)` when started, `None` when
/// telemetry is disabled or no DSN is present (caller asserts this via
/// [`TelemetryConfig::should_start_sentry`]).
///
/// Installs the SDK's panic + backtrace integrations; the crate's own panic hook
/// ([`crate::crash`]) is installed **after** this so it chains into Sentry's.
#[must_use]
pub fn start(config: &TelemetryConfig) -> Option<SentryGuard> {
    let dsn = config.effective_dsn()?;
    if !config.telemetry_enabled() {
        return None;
    }

    let options = sentry::ClientOptions {
        dsn: dsn.parse().ok(),
        release: Some(config.release_name().into()),
        environment: Some(config.environment.as_str().into()),
        send_default_pii: constants::SEND_DEFAULT_PII,
        traces_sample_rate: constants::TRACES_SAMPLE_RATE,
        attach_stacktrace: constants::ATTACH_STACKTRACE,
        ..Default::default()
    };

    Some(sentry::init(options))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_reference() {
        assert!((constants::TRACES_SAMPLE_RATE - 0.1).abs() < f32::EPSILON);
        assert!((constants::APP_HANG_TIMEOUT_SECS - 8.0).abs() < f64::EPSILON);
        assert!(!constants::SEND_DEFAULT_PII);
        assert!(constants::ATTACH_STACKTRACE);
    }

    #[test]
    fn app_hang_timeout_is_eight_seconds() {
        assert_eq!(app_hang_timeout(), std::time::Duration::from_secs(8));
    }

    #[test]
    fn no_start_without_dsn() {
        let cfg = TelemetryConfig::new(None, None, "1.0.0", "build1");
        assert!(start(&cfg).is_none());
    }

    #[test]
    fn no_start_when_opted_out() {
        let cfg = TelemetryConfig::new(Some("https://k@o.example/1".into()), Some(false), "1.0.0", "b");
        assert!(start(&cfg).is_none());
    }
}
