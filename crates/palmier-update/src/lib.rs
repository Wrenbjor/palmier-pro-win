//! # palmier-update
//!
//! Tauri 2 updater glue — Ed25519-signed manifest check, `update_available` /
//! `update_version` surfacing, single `stable` channel (FOUNDATION §4, §8.4; E1-S10).
//!
//! Ports `App/Updater.swift` (`SPUStandardUpdaterController` wrapper, `Updater.shared`
//! `@Observable` with `updateAvailable`/`updateVersion`) and `App/UpdateBadgeView.swift`
//! onto the Tauri 2 updater (same EdDSA signing model). See
//! `docs/reference/settings-account-app.md` "Updater" + "macOS/Apple APIs to replace".
//!
//! ## Silent-disable gate (the load-bearing parity behavior)
//! Sparkle's `Updater.init` **no-ops** unless the app runs from a `.app` bundle AND
//! `SUFeedURL` is present in `Info.plist`; otherwise dev builds would surface spurious
//! "check failed" UI. The Tauri equivalent here: the updater is **active only when a
//! signed feed is configured** — i.e. an `Updater::config` resolves a non-empty
//! manifest endpoint *and* the Tauri updater plugin is present (a pubkey is baked into
//! `tauri.conf.json`'s `plugins.updater`). When no signed feed is configured (the
//! common dev case), [`Updater::check_now`] **silently disables**: it returns
//! [`CheckOutcome::Disabled`] without any network call or error UI.
//!
//! ## Channel
//! A single **`stable`** channel for v1 (OQ-1 working decision). The manifest URL is
//! build/backend config (OQ-9): read from `PALMIER_UPDATE_MANIFEST_URL` (runtime env
//! override first, then the compile-time `option_env!`). Empty/whitespace ⇒ no feed ⇒
//! updater disabled.
//!
//! ## Split of responsibilities
//! This crate owns the **gate + config resolution + result/event types**, which are
//! `tauri`-free and fully unit-testable. The actual `app.updater()?.check()` call (which
//! needs an `AppHandle` and the `tauri-plugin-updater`) lives in `palmier-tauri`'s
//! `update` module, which calls [`Updater::feed`] to decide whether to run at all and
//! emits [`UpdateEvent`] over Tauri. Keeping the network/plugin call out of this crate
//! avoids pulling the Tauri runtime into the UAC-sensitive `palmier-update` test binary
//! (the crate name trips Windows installer-detection — see `build.rs`).

use serde::{Deserialize, Serialize};

/// v1 update channel (OQ-1 working decision).
pub const CHANNEL: &str = "stable";

/// Tauri event name carrying the [`UpdateEvent`] payload to the frontend update badge.
/// `palmier-tauri` emits this; `src-ui/app`'s badge listens. Mirrors the reference
/// `Updater.shared` `@Observable` → SwiftUI binding.
pub const UPDATE_EVENT: &str = "update://status";

/// Resolved updater configuration: the signed-feed manifest endpoint, if any.
///
/// `None` ⇒ no signed feed ⇒ the updater silently disables (dev-build parity with
/// Sparkle's `.app`+`SUFeedURL` gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updater {
    /// The Ed25519-signed JSON manifest URL, or `None` when no feed is configured.
    feed: Option<String>,
}

impl Updater {
    /// Resolve the updater config from build/runtime config.
    ///
    /// Reads `PALMIER_UPDATE_MANIFEST_URL`: a runtime env var (lets ops/CI point at a
    /// feed without a rebuild) first, then the compile-time `option_env!`. An
    /// empty/whitespace value ⇒ `None` ⇒ disabled.
    #[must_use]
    pub fn from_build_config() -> Self {
        let feed = std::env::var("PALMIER_UPDATE_MANIFEST_URL")
            .ok()
            .or_else(|| option_env!("PALMIER_UPDATE_MANIFEST_URL").map(str::to_owned))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        Self { feed }
    }

    /// Construct with an explicit feed (tests / custom wiring). An empty/whitespace
    /// feed normalizes to `None` (disabled).
    #[must_use]
    pub fn with_feed(feed: Option<impl Into<String>>) -> Self {
        let feed = feed
            .map(Into::into)
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        Self { feed }
    }

    /// The configured signed-feed manifest URL, if any.
    #[must_use]
    pub fn feed(&self) -> Option<&str> {
        self.feed.as_deref()
    }

    /// Whether the updater is active: true only when a signed feed is configured.
    /// When false, [`check_now`](Self::check_now) silently disables (no network, no
    /// "check failed" UI) — the Sparkle dev-build parity behavior.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.feed.is_some()
    }

    /// Decide the pre-flight outcome for a check.
    ///
    /// `palmier-tauri` calls this before touching `tauri-plugin-updater`:
    /// - `Disabled` ⇒ return immediately, emit nothing actionable (badge stays hidden).
    /// - `Enabled(url)` ⇒ run `app.updater()?.check()` against `url`, then map its
    ///   result into an [`UpdateEvent`] via [`UpdateEvent::from_check`].
    #[must_use]
    pub fn preflight(&self) -> CheckOutcome {
        match &self.feed {
            Some(url) => CheckOutcome::Enabled(url.clone()),
            None => CheckOutcome::Disabled,
        }
    }
}

impl Default for Updater {
    fn default() -> Self {
        Self::from_build_config()
    }
}

/// Pre-flight result of [`Updater::preflight`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// No signed feed — silently disabled (dev build / unsigned).
    Disabled,
    /// A signed feed is configured; run the plugin check against this manifest URL.
    Enabled(String),
}

impl CheckOutcome {
    /// True when a check should actually run.
    #[must_use]
    pub fn should_check(&self) -> bool {
        matches!(self, CheckOutcome::Enabled(_))
    }
}

/// The update status pushed to the frontend badge over the [`UPDATE_EVENT`] Tauri event.
///
/// Ports the reference `Updater.shared.updateAvailable` / `.updateVersion` observable
/// pair into a single serializable payload the badge consumes. `available=false` keeps
/// the badge hidden (the reference badge only shows when `updateAvailable`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateEvent {
    /// Whether an update is available (badge visible only when true).
    pub available: bool,
    /// The available version's display string (reference `SUAppcastItem.displayVersionString`),
    /// `None` when no update / disabled.
    pub version: Option<String>,
    /// Whether the updater is enabled at all (a configured signed feed). When false the
    /// badge stays hidden and "Check for Updates" is a no-op — surfaced so the Settings/
    /// menu UI can reflect "updates not configured" in dev builds without erroring.
    pub enabled: bool,
}

impl UpdateEvent {
    /// The "no update" / hidden-badge state for an **enabled** updater that checked and
    /// found nothing (or is mid-check).
    #[must_use]
    pub fn none_available() -> Self {
        Self {
            available: false,
            version: None,
            enabled: true,
        }
    }

    /// The disabled state (no signed feed): badge hidden, updater off. This is what a
    /// dev build surfaces — never an error.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            available: false,
            version: None,
            enabled: false,
        }
    }

    /// An "update available" state carrying the version (reference sets
    /// `updateAvailable=true; updateVersion=<displayVersionString>`).
    #[must_use]
    pub fn available(version: impl Into<String>) -> Self {
        Self {
            available: true,
            version: Some(version.into()),
            enabled: true,
        }
    }

    /// Map a Tauri-plugin check result into an [`UpdateEvent`].
    ///
    /// `palmier-tauri` passes `Some(version)` when `app.updater()?.check()` yields an
    /// update, or `None` when it returns no update. Either way the updater was enabled
    /// (a feed was configured), so `enabled=true`.
    #[must_use]
    pub fn from_check(version: Option<String>) -> Self {
        match version {
            Some(v) => Self::available(v),
            None => Self::none_available(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_is_stable() {
        assert_eq!(CHANNEL, "stable");
    }

    #[test]
    fn no_feed_disables_updater_silently() {
        // The common dev case: no PALMIER_UPDATE_MANIFEST_URL configured.
        let u = Updater::with_feed(None::<String>);
        assert!(!u.is_enabled());
        assert_eq!(u.preflight(), CheckOutcome::Disabled);
        assert!(!u.preflight().should_check());
        assert!(u.feed().is_none());
    }

    #[test]
    fn empty_or_whitespace_feed_normalizes_to_disabled() {
        assert!(!Updater::with_feed(Some("")).is_enabled());
        assert!(!Updater::with_feed(Some("   ")).is_enabled());
        assert!(!Updater::with_feed(Some("\t\n")).is_enabled());
    }

    #[test]
    fn configured_feed_enables_check() {
        let u = Updater::with_feed(Some("https://updates.example.com/stable/manifest.json"));
        assert!(u.is_enabled());
        assert_eq!(
            u.preflight(),
            CheckOutcome::Enabled(
                "https://updates.example.com/stable/manifest.json".to_string()
            )
        );
        assert!(u.preflight().should_check());
        assert_eq!(
            u.feed(),
            Some("https://updates.example.com/stable/manifest.json")
        );
    }

    #[test]
    fn feed_is_trimmed() {
        let u = Updater::with_feed(Some("  https://x/manifest.json  "));
        assert_eq!(u.feed(), Some("https://x/manifest.json"));
    }

    #[test]
    fn update_event_states() {
        let disabled = UpdateEvent::disabled();
        assert!(!disabled.enabled);
        assert!(!disabled.available);
        assert!(disabled.version.is_none());

        let none = UpdateEvent::none_available();
        assert!(none.enabled);
        assert!(!none.available);

        let avail = UpdateEvent::available("1.2.3");
        assert!(avail.enabled);
        assert!(avail.available);
        assert_eq!(avail.version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn from_check_maps_plugin_result() {
        assert_eq!(
            UpdateEvent::from_check(Some("2.0.0".to_string())),
            UpdateEvent::available("2.0.0")
        );
        assert_eq!(UpdateEvent::from_check(None), UpdateEvent::none_available());
    }

    #[test]
    fn update_event_serializes_for_frontend() {
        let json = serde_json::to_string(&UpdateEvent::available("3.1.0")).unwrap();
        assert!(json.contains("\"available\":true"));
        assert!(json.contains("\"version\":\"3.1.0\""));
        assert!(json.contains("\"enabled\":true"));
    }
}
