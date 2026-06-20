//! Build-time backend configuration.
//!
//! Ports `Account/BackendConfig.swift`. On macOS these came from `Info.plist`
//! keys (`PalmierClerkPublishableKey`, `PalmierConvexDeploymentURL`,
//! `PalmierConvexHttpURL`); on the Windows/Linux port they arrive from compiled
//! Tauri config (`tauri.conf.json` + compile-time env — see settings-account-app.md
//! "macOS/Apple APIs to replace"). The boot path (E1-S1 step 4) hands these to
//! [`crate::Auth::init`].
//!
//! `is_misconfigured` mirrors the reference: if the Clerk publishable key OR the
//! Convex deployment URL is missing, the service is misconfigured and the Account
//! tab is hidden downstream (settings-account-app.md "Auth/account state machine").

use url::Url;

/// Backend configuration read from build-time Tauri config.
///
/// Mirrors `BackendConfig`: `clerk_publishable_key` + `convex_deployment_url`
/// (WebSocket/deployment endpoint) + `convex_http_url` (REST endpoint used for
/// `/v1/...` calls). The HTTP URL is optional in the reference (`isConfigured`
/// only checks key + deployment URL) but the M1 read-only slice needs it for the
/// Convex HTTP transport, so we surface it separately.
#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    /// Clerk publishable key. Empty/whitespace is treated as absent.
    pub clerk_publishable_key: Option<String>,
    /// Convex deployment URL (WebSocket base; full client is Spike S-2 / Epic 9).
    pub convex_deployment_url: Option<Url>,
    /// Convex HTTP base URL (`/v1/...` REST calls + agent stream proxy).
    pub convex_http_url: Option<Url>,
}

impl AuthConfig {
    /// Builder seeded with nothing configured (⇒ misconfigured).
    pub fn builder() -> AuthConfigBuilder {
        AuthConfigBuilder::default()
    }

    /// Reference `BackendConfig.isConfigured` parity: configured iff a Clerk key
    /// AND a Convex deployment URL are present. When `false`, downstream hides the
    /// Account tab and `ai_allowed` is forced off.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.clerk_publishable_key.is_some() && self.convex_deployment_url.is_some()
    }

    /// Inverse of [`Self::is_configured`] — the field the state machine exposes as
    /// `is_misconfigured` (settings-account-app.md auth state machine).
    #[must_use]
    pub fn is_misconfigured(&self) -> bool {
        !self.is_configured()
    }
}

/// Normalize a raw string config value: trims, and maps empty ⇒ `None`
/// (reference `BackendConfig.string` rejected empty Info.plist values).
fn non_empty(value: impl Into<String>) -> Option<String> {
    let trimmed = value.into().trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Builder for [`AuthConfig`]. Accepts raw `&str` values (as they arrive from the
/// Tauri compile-time config) and applies the reference empty-⇒-absent semantics.
#[derive(Debug, Default, Clone)]
pub struct AuthConfigBuilder {
    clerk_publishable_key: Option<String>,
    convex_deployment_url: Option<String>,
    convex_http_url: Option<String>,
}

impl AuthConfigBuilder {
    /// Set the Clerk publishable key (empty/whitespace ⇒ treated as absent).
    #[must_use]
    pub fn clerk_publishable_key(mut self, key: impl Into<String>) -> Self {
        self.clerk_publishable_key = non_empty(key);
        self
    }

    /// Set the Convex deployment URL string (empty/whitespace ⇒ absent).
    #[must_use]
    pub fn convex_deployment_url(mut self, url: impl Into<String>) -> Self {
        self.convex_deployment_url = non_empty(url);
        self
    }

    /// Set the Convex HTTP base URL string (empty/whitespace ⇒ absent).
    #[must_use]
    pub fn convex_http_url(mut self, url: impl Into<String>) -> Self {
        self.convex_http_url = non_empty(url);
        self
    }

    /// Build the [`AuthConfig`]. A non-empty-but-unparseable URL is dropped to
    /// `None` (reference `URL(string:)` likewise yields nil on a bad string),
    /// which surfaces as `is_misconfigured`.
    #[must_use]
    pub fn build(self) -> AuthConfig {
        AuthConfig {
            clerk_publishable_key: self.clerk_publishable_key,
            convex_deployment_url: self
                .convex_deployment_url
                .and_then(|s| Url::parse(&s).ok()),
            convex_http_url: self.convex_http_url.and_then(|s| Url::parse(&s).ok()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_values_are_treated_as_absent() {
        let cfg = AuthConfig::builder()
            .clerk_publishable_key("   ")
            .convex_deployment_url("")
            .build();
        assert!(cfg.clerk_publishable_key.is_none());
        assert!(cfg.convex_deployment_url.is_none());
        assert!(cfg.is_misconfigured());
    }

    #[test]
    fn configured_requires_key_and_deployment_url() {
        // Key only -> still misconfigured.
        let cfg = AuthConfig::builder()
            .clerk_publishable_key("pk_test_abc")
            .build();
        assert!(cfg.is_misconfigured());

        // Key + deployment URL -> configured (parity with isConfigured).
        let cfg = AuthConfig::builder()
            .clerk_publishable_key("pk_test_abc")
            .convex_deployment_url("https://example.convex.cloud")
            .build();
        assert!(cfg.is_configured());
        assert!(!cfg.is_misconfigured());
    }

    #[test]
    fn unparseable_url_drops_to_none() {
        let cfg = AuthConfig::builder()
            .clerk_publishable_key("pk_test_abc")
            .convex_deployment_url("not a url")
            .build();
        assert!(cfg.convex_deployment_url.is_none());
        assert!(cfg.is_misconfigured());
    }
}
