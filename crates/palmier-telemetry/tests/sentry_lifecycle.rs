//! E12-S12 acceptance: Sentry init gating + launch-snapshot lifecycle.
//!
//! Asserts the three story-required conditions (epic-12-polish-release.md §E12-S12):
//!   1. DSN-empty ⇒ Sentry not initialized (even if the pref is ON).
//!   2. Pref-off-at-launch ⇒ Sentry not initialized even if DSN is present.
//!   3. Toggling the pref at runtime does NOT start/stop Sentry until restart —
//!      the launch snapshot is honored (the start decision is fixed at launch).
//!
//! These are end-to-end over the public crate API (`TelemetryConfig` +
//! `LaunchSnapshot`), exercising the exact gate the boot path uses
//! (`should_start_sentry`), not internal helpers.

use palmier_telemetry::config::TelemetryConfig;
use palmier_telemetry::pref::LaunchSnapshot;

const TEST_DSN: &str = "https://publickey@o0.ingest.sentry.example/1";

/// AC 1: a non-empty pref but empty/absent DSN must NOT start Sentry.
#[test]
fn dsn_empty_does_not_start_sentry() {
    // Absent DSN (pref defaults ON via absent⇒ON).
    let absent = TelemetryConfig::new(None, None, "1.0.0", "sha");
    assert!(absent.telemetry_enabled(), "absent pref ⇒ ON (ruling #6)");
    assert!(
        !absent.should_start_sentry(),
        "no DSN ⇒ Sentry must not start even with telemetry ON"
    );

    // Empty / whitespace DSN, pref explicitly ON.
    for dsn in ["", "   ", "\t\n"] {
        let cfg = TelemetryConfig::new(Some(dsn.into()), Some(true), "1.0.0", "sha");
        assert!(
            !cfg.should_start_sentry(),
            "empty DSN {dsn:?} ⇒ Sentry must not start"
        );
    }
}

/// AC 2: pref OFF at launch ⇒ Sentry not initialized even with a valid DSN.
#[test]
fn pref_off_at_launch_does_not_start_sentry_with_dsn() {
    let cfg = TelemetryConfig::new(Some(TEST_DSN.into()), Some(false), "1.0.0", "sha");
    assert!(cfg.effective_dsn().is_some(), "DSN is present and valid");
    assert!(
        !cfg.should_start_sentry(),
        "opted-out at launch ⇒ Sentry must not start even with a DSN"
    );
}

/// Sanity: the only state in which Sentry starts is ON + non-empty DSN.
#[test]
fn sentry_starts_only_with_enabled_and_dsn() {
    let cfg = TelemetryConfig::new(Some(TEST_DSN.into()), Some(true), "1.0.0", "sha");
    assert!(cfg.should_start_sentry());
    // ...and absent pref (ON) + DSN also starts.
    let cfg_absent = TelemetryConfig::new(Some(TEST_DSN.into()), None, "1.0.0", "sha");
    assert!(cfg_absent.should_start_sentry());
}

/// AC 3: the launch snapshot is the source of truth for Sentry's lifecycle.
/// Toggling the *live* pref does NOT change whether Sentry would be running —
/// only a restart (which re-captures the snapshot) does.
#[test]
fn runtime_toggle_does_not_change_sentry_until_restart() {
    // --- Launched with telemetry ON + DSN: Sentry started this launch. ---
    let launched_on = TelemetryConfig::new(Some(TEST_DSN.into()), Some(true), "1.0.0", "sha");
    let snap_on = launched_on.launch_snapshot();
    assert!(launched_on.should_start_sentry(), "launched ON ⇒ Sentry on");

    // User toggles telemetry OFF in settings mid-session. The launch snapshot is
    // immutable, so it still reports ON and flags a restart as required.
    assert!(
        snap_on.enabled_for_current_launch(),
        "snapshot stays ON for the current launch despite the live toggle"
    );
    assert!(
        snap_on.restart_required_for(Some(false)),
        "toggling OFF this session ⇒ restart required (Sentry not stopped live)"
    );

    // --- Launched with telemetry OFF + DSN: Sentry never started this launch. ---
    let launched_off = TelemetryConfig::new(Some(TEST_DSN.into()), Some(false), "1.0.0", "sha");
    let snap_off = launched_off.launch_snapshot();
    assert!(
        !launched_off.should_start_sentry(),
        "launched OFF ⇒ Sentry stays off"
    );

    // User toggles telemetry ON mid-session. Snapshot stays OFF; Sentry is NOT
    // started live — restart required.
    assert!(!snap_off.enabled_for_current_launch());
    assert!(
        snap_off.restart_required_for(Some(true)),
        "toggling ON this session ⇒ restart required (Sentry not started live)"
    );

    // The post-restart decision uses the NEW pref value, re-snapshotted.
    let after_restart = LaunchSnapshot::capture(Some(true));
    assert!(after_restart.enabled_for_current_launch());
    assert!(
        !after_restart.restart_required_for(Some(true)),
        "after restart the snapshot matches the live pref ⇒ no further restart"
    );
}

/// The release name carried into Sentry options matches the reference format
/// `palmier-pro-win@<version>+<git_sha>` (FOUNDATION §6.16).
#[test]
fn release_name_matches_reference_format() {
    let cfg = TelemetryConfig::new(Some(TEST_DSN.into()), Some(true), "0.4.2", "deadbeef");
    assert_eq!(cfg.release_name(), "palmier-pro-win@0.4.2+deadbeef");
}
