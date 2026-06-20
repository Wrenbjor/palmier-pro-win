//! Telemetry boot configuration.
//!
//! Built by the Tauri boot path (E1-S1, boot steps 1–2) and passed to
//! [`init`](crate::init). The DSN is **build-injected** (FOUNDATION §6.16: via
//! `tauri.conf.json` → `PALMIER_SENTRY_DSN`); this crate never hardcodes it.

use crate::pref::{enabled_from_pref, LaunchSnapshot};

/// Sentry runtime environment tag (reference: `development` in debug builds,
/// `production` in release).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// Debug builds.
    Development,
    /// Release builds.
    Production,
}

impl Environment {
    /// Pick the environment from the build profile (`cfg!(debug_assertions)`).
    #[must_use]
    pub fn from_build() -> Self {
        if cfg!(debug_assertions) {
            Self::Development
        } else {
            Self::Production
        }
    }

    /// The Sentry environment string.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }
}

/// Configuration for [`init`](crate::init), assembled by the boot path.
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Sentry DSN, build-injected (`PALMIER_SENTRY_DSN`). Empty/whitespace ⇒
    /// Sentry stays disabled even if the pref is ON (reference: "DSN non-empty").
    pub dsn: Option<String>,
    /// Telemetry-enabled pref as read from `settings.json`
    /// (`io.palmier.pro.telemetry.enabled`): `None` when the key is absent
    /// (⇒ ON, ruling #6).
    pub telemetry_pref: Option<bool>,
    /// App semantic version (reference release name `palmier-pro-win@<version>`).
    pub version: String,
    /// Build identifier appended to the release name (git sha / build number).
    pub build: String,
    /// Sentry environment; defaults to [`Environment::from_build`].
    pub environment: Environment,
}

impl TelemetryConfig {
    /// Construct a config from build-injected values, defaulting the environment
    /// from the build profile.
    #[must_use]
    pub fn new(
        dsn: Option<String>,
        telemetry_pref: Option<bool>,
        version: impl Into<String>,
        build: impl Into<String>,
    ) -> Self {
        Self {
            dsn,
            telemetry_pref,
            version: version.into(),
            build: build.into(),
            environment: Environment::from_build(),
        }
    }

    /// The launch snapshot of the telemetry-enabled flag (absent ⇒ ON).
    #[must_use]
    pub fn launch_snapshot(&self) -> LaunchSnapshot {
        LaunchSnapshot::capture(self.telemetry_pref)
    }

    /// Whether telemetry is enabled for this launch (pref absent ⇒ ON).
    #[must_use]
    pub fn telemetry_enabled(&self) -> bool {
        enabled_from_pref(self.telemetry_pref)
    }

    /// The non-empty, trimmed DSN, if one is configured. Whitespace-only or
    /// missing DSNs return `None` (Sentry then stays off).
    #[must_use]
    pub fn effective_dsn(&self) -> Option<&str> {
        self.dsn
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
    }

    /// Whether Sentry should actually start: telemetry enabled **and** a
    /// non-empty DSN present (reference: both conditions required).
    #[must_use]
    pub fn should_start_sentry(&self) -> bool {
        self.telemetry_enabled() && self.effective_dsn().is_some()
    }

    /// The Sentry release name: `palmier-pro-win@<version>+<build>`
    /// (FOUNDATION §6.16).
    #[must_use]
    pub fn release_name(&self) -> String {
        format!("palmier-pro-win@{}+{}", self.version, self.build)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(dsn: Option<&str>, pref: Option<bool>) -> TelemetryConfig {
        TelemetryConfig::new(dsn.map(str::to_owned), pref, "1.2.3", "abc1234")
    }

    #[test]
    fn release_name_format() {
        assert_eq!(cfg(None, None).release_name(), "palmier-pro-win@1.2.3+abc1234");
    }

    #[test]
    fn telemetry_absent_pref_is_on() {
        assert!(cfg(None, None).telemetry_enabled());
    }

    #[test]
    fn telemetry_opt_out_is_off() {
        assert!(!cfg(None, Some(false)).telemetry_enabled());
    }

    #[test]
    fn empty_dsn_is_not_effective() {
        assert!(cfg(Some(""), None).effective_dsn().is_none());
        assert!(cfg(Some("   "), None).effective_dsn().is_none());
        assert!(cfg(None, None).effective_dsn().is_none());
    }

    #[test]
    fn dsn_is_trimmed() {
        assert_eq!(cfg(Some("  https://x@y/1  "), None).effective_dsn(), Some("https://x@y/1"));
    }

    #[test]
    fn sentry_starts_only_with_enabled_and_dsn() {
        // enabled + dsn ⇒ start.
        assert!(cfg(Some("https://k@o/1"), None).should_start_sentry());
        // enabled but no dsn ⇒ no start.
        assert!(!cfg(None, None).should_start_sentry());
        // dsn but opted-out ⇒ no start.
        assert!(!cfg(Some("https://k@o/1"), Some(false)).should_start_sentry());
    }
}
