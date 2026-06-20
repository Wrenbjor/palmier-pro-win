//! Telemetry preference gate.
//!
//! The reference app stores `io.palmier.pro.telemetry.enabled` as a
//! UserDefaults boolean with **absent ⇒ ON** semantics (ruling #6). Toggling it
//! is **launch-snapshotted** (`enabledForCurrentLaunch`): a change only takes
//! effect after restart, so the UI shows a "Restart required" note.
//!
//! On Windows/Linux the pref lives in `settings.json` (owned by the Tauri
//! settings module, E1-S1), not a macOS preference domain. This crate does not
//! read `settings.json` itself — the boot path passes the resolved value in via
//! [`TelemetryConfig`](crate::TelemetryConfig). This module only encodes the
//! **default + snapshot** semantics so they live in one place and are testable.

/// The settings key for the telemetry opt-out toggle (reference parity).
pub const TELEMETRY_PREF_KEY: &str = "io.palmier.pro.telemetry.enabled";

/// Resolve the telemetry-enabled value with reference **absent ⇒ ON** semantics.
///
/// `raw` is the value as read from `settings.json` for [`TELEMETRY_PREF_KEY`]:
/// `Some(true)`/`Some(false)` when present, `None` when the key is absent.
/// A missing key means the user has never opted out, so telemetry defaults ON.
#[must_use]
pub fn enabled_from_pref(raw: Option<bool>) -> bool {
    raw.unwrap_or(true)
}

/// A launch-time snapshot of the telemetry-enabled flag.
///
/// Captured once at boot and never mutated for the life of the process; a
/// settings toggle changes the stored pref but not this snapshot, mirroring the
/// reference `enabledForCurrentLaunch` and the restart-required UX (ruling #6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LaunchSnapshot {
    enabled: bool,
}

impl LaunchSnapshot {
    /// Snapshot the effective enabled state from the raw stored pref value.
    #[must_use]
    pub fn capture(raw_pref: Option<bool>) -> Self {
        Self {
            enabled: enabled_from_pref(raw_pref),
        }
    }

    /// Snapshot from an already-resolved boolean (e.g. when the boot path has
    /// applied the absent⇒ON default itself).
    #[must_use]
    pub fn from_resolved(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Whether telemetry is enabled **for the current launch**.
    #[must_use]
    pub fn enabled_for_current_launch(self) -> bool {
        self.enabled
    }

    /// Whether the live pref value differs from this launch snapshot — i.e.
    /// whether a restart is required for the change to take effect. The settings
    /// UI uses this to decide whether to show "Restart Palmier Pro to apply".
    #[must_use]
    pub fn restart_required_for(self, live_pref: Option<bool>) -> bool {
        enabled_from_pref(live_pref) != self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_pref_defaults_on() {
        // ruling #6: absent ⇒ ON.
        assert!(enabled_from_pref(None));
    }

    #[test]
    fn present_pref_is_honored() {
        assert!(enabled_from_pref(Some(true)));
        assert!(!enabled_from_pref(Some(false)));
    }

    #[test]
    fn snapshot_absent_is_on() {
        let snap = LaunchSnapshot::capture(None);
        assert!(snap.enabled_for_current_launch());
    }

    #[test]
    fn snapshot_opt_out_is_off() {
        let snap = LaunchSnapshot::capture(Some(false));
        assert!(!snap.enabled_for_current_launch());
    }

    #[test]
    fn restart_required_when_live_differs_from_snapshot() {
        // Launched OFF (user had opted out)...
        let snap = LaunchSnapshot::capture(Some(false));
        // ...then toggled ON in settings this session → restart needed.
        assert!(snap.restart_required_for(Some(true)));
        // Toggling back to the launch value clears the requirement.
        assert!(!snap.restart_required_for(Some(false)));
    }

    #[test]
    fn restart_required_respects_absent_default() {
        // Launched with no key (ON by default)...
        let snap = LaunchSnapshot::capture(None);
        // ...clearing the key again is still ON → no restart needed.
        assert!(!snap.restart_required_for(None));
        // Opting out this session differs from the ON snapshot → restart needed.
        assert!(snap.restart_required_for(Some(false)));
    }
}
