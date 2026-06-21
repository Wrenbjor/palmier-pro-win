//! Tauri updater glue (E1-S10).
//!
//! Bridges [`palmier_update`] (the `tauri`-free gate + event types) to the Tauri
//! runtime + `tauri-plugin-updater`. Ports `App/Updater.swift` (Sparkle wrapper) and
//! drives `App/UpdateBadgeView.swift`'s frontend equivalent
//! (`src-ui/src/app/update-badge.ts(x)`).
//!
//! ## Behavior
//! - Boot touches the updater (reference `AppDelegate` "touch `Updater.shared`") via
//!   [`check_on_boot`], which runs a check only when a signed feed is configured.
//! - The "Check for Updates" menu item + the `check_for_updates` command both call
//!   [`check_now`].
//! - When **no signed feed** is configured (dev builds / unsigned), the updater
//!   **silently disables**: it emits an [`UpdateEvent::disabled`] (badge hidden) and
//!   never touches the network or the plugin — replicating Sparkle's "no-op unless
//!   `.app` + `SUFeedURL`" gate (settings-account-app.md updater gotcha).
//!
//! ## Why the plugin call is feature-gated
//! `tauri-plugin-updater` requires a baked Ed25519 pubkey in `tauri.conf.json`
//! (`plugins.updater.pubkey`) to initialize. The dev/CI build has no signed feed, so
//! the plugin is wired behind the `updater-plugin` cargo feature (off by default). With
//! the feature off, [`check_now`] resolves to the silent-disable path (and, if a feed
//! *is* configured without the plugin, logs that the build lacks updater support rather
//! than failing). A release build that ships a feed enables `updater-plugin` and the
//! real `app.updater()?.check()` runs. This keeps the default build green with no
//! signing material while preserving the full glue.
//!
//! ## E12-S13 placeholders — HUMAN-OWNED RELEASE BLOCKERS
//! `tauri.conf.json` ships a `plugins.updater` block with **placeholder** values that
//! a human must replace before public release:
//!   - `pubkey`: the literal `PLACEHOLDER_REPLACE_WITH_REAL_TAURI_ED25519_PUBKEY_…`.
//!     Generate the real keypair with `tauri signer generate`; commit the **public**
//!     key here and keep the **private** key in CI secrets
//!     (`TAURI_SIGNING_PRIVATE_KEY` / `…_PASSWORD`).
//!   - `endpoints[0]`: the placeholder host `https://updates.palmier.io/win/latest.json`
//!     (OQ-9 — the Convex/manifest backend does not exist yet).
//! Until BOTH are real (and `updater-plugin` is enabled), the updater stays silently
//! disabled — exactly the Sparkle-without-`SUFeedURL` no-op (E12-S13 AC).

use palmier_update::{CheckOutcome, UpdateEvent, Updater, UPDATE_EVENT};
use tauri::{AppHandle, Emitter, Runtime};

/// Emit the current update status to the frontend badge over [`UPDATE_EVENT`].
fn emit<R: Runtime>(app: &AppHandle<R>, event: &UpdateEvent) {
    if let Err(err) = app.emit(UPDATE_EVENT, event) {
        tracing::error!(target: "app", error = %err, "failed to emit update status");
    }
}

/// Run an update check now (menu "Check for Updates" + the `check_for_updates` command).
///
/// Resolves the feed via [`Updater::from_build_config`]; if disabled, emits
/// [`UpdateEvent::disabled`] and returns without any network call. If enabled, runs the
/// plugin check (when built with `updater-plugin`) and emits the mapped status.
pub fn check_now<R: Runtime>(app: &AppHandle<R>) {
    let updater = Updater::from_build_config();
    match updater.preflight() {
        CheckOutcome::Disabled => {
            tracing::info!(
                target: "app",
                "updater disabled (no signed feed configured) — silent no-op (Sparkle parity)"
            );
            emit(app, &UpdateEvent::disabled());
        }
        CheckOutcome::Enabled(manifest_url) => {
            tracing::info!(
                target: "app",
                channel = palmier_update::CHANNEL,
                manifest = %manifest_url,
                "checking for updates"
            );
            run_plugin_check(app, &manifest_url);
        }
    }
}

/// Boot-time touch of the updater (reference `AppDelegate` touches `Updater.shared`,
/// which starts Sparkle's background check). Only runs when a feed is configured; a dev
/// build with no feed stays completely silent.
pub fn check_on_boot<R: Runtime>(app: &AppHandle<R>) {
    let updater = Updater::from_build_config();
    if updater.is_enabled() {
        check_now(app);
    } else {
        // No feed: keep the badge hidden, do nothing else.
        emit(app, &UpdateEvent::disabled());
    }
}

/// Run the actual `tauri-plugin-updater` check when the `updater-plugin` feature is on.
///
/// With the feature **on**, calls `app.updater_builder().endpoint(url).build()?.check()`
/// and emits the mapped [`UpdateEvent`]. With the feature **off** (the default dev/CI
/// build, which has no signing pubkey), logs that updater support is not compiled in and
/// emits the no-update state — never an error.
#[cfg(feature = "updater-plugin")]
fn run_plugin_check<R: Runtime>(app: &AppHandle<R>, manifest_url: &str) {
    use tauri_plugin_updater::UpdaterExt;

    let app = app.clone();
    let manifest_url = manifest_url.to_string();
    tauri::async_runtime::spawn(async move {
        let result = (|| async {
            let endpoint = manifest_url
                .parse()
                .map_err(|e| format!("bad manifest URL: {e}"))?;
            let updater = app
                .updater_builder()
                .endpoints(vec![endpoint])
                .map_err(|e| e.to_string())?
                .build()
                .map_err(|e| e.to_string())?;
            updater.check().await.map_err(|e| e.to_string())
        })()
        .await;

        match result {
            Ok(Some(update)) => {
                tracing::info!(target: "app", version = %update.version, "update available");
                emit(&app, &UpdateEvent::from_check(Some(update.version)));
            }
            Ok(None) => {
                tracing::info!(target: "app", "no update available");
                emit(&app, &UpdateEvent::none_available());
            }
            Err(err) => {
                // A feed is configured but the check failed (offline/transient). Keep
                // the badge hidden; this is recoverable, not a fatal error.
                tracing::warn!(target: "app", error = %err, "update check failed");
                emit(&app, &UpdateEvent::none_available());
            }
        }
    });
}

/// Stub used when the `updater-plugin` feature is off (default build). A feed *was*
/// configured (otherwise we'd never reach here), but this build was compiled without the
/// updater plugin / signing material, so we cannot verify a signed manifest. Log + emit
/// the no-update state rather than pretending or erroring.
#[cfg(not(feature = "updater-plugin"))]
fn run_plugin_check<R: Runtime>(app: &AppHandle<R>, manifest_url: &str) {
    tracing::warn!(
        target: "app",
        manifest = %manifest_url,
        "a signed-update feed is configured but this build lacks the `updater-plugin` \
         feature (no signing pubkey baked in) — skipping the real check"
    );
    emit(app, &UpdateEvent::none_available());
}
