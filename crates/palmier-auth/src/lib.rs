//! # palmier-auth
//!
//! Auth + account/credit state + secret storage for the Palmier Pro Windows/Linux
//! port. Ports `Account/AccountService.swift`, `Account/BackendConfig.swift`,
//! `Utilities/KeychainStore.swift`, and `AnthropicKeychain`
//! (`Agent/Clients/AnthropicClient.swift`) — see `docs/reference/settings-account-app.md`
//! "Mapping to FOUNDATION crates".
//!
//! Responsibilities (E1-S6):
//! - **Build-time config** ([`AuthConfig`]) — Clerk publishable key + Convex
//!   deployment/HTTP URLs; `is_misconfigured` when key or Convex URL is missing.
//! - **Clerk JWT cache** ([`TokenCache`]) — in-memory, forwarded as `Bearer` to
//!   Convex, refreshed every 5 min (FOUNDATION §8.2).
//! - **Account state machine** ([`AccountState`]) — tier (none/pro/max), credits,
//!   `ai_allowed`, misconfigured guard; degrades to signed-out when Convex is
//!   unreachable (OQ-9 / R-4).
//! - **OS-keyring storage** ([`AnthropicKeyStore`]) — the Anthropic API key under
//!   service `palmier-pro` / account `anthropic-api-key` (ruling #5), via the
//!   `keyring` crate (Windows Credential Manager / Linux Secret Service).
//! - **Convex transport** ([`ConvexBackend`]) — account/billing/feedback calls
//!   behind a trait so the state machine is testable without a live backend.
//!
//! ## Boot integration (the `init` fn the Tauri boot will call)
//! E1-S1's boot step 4 calls [`Auth::init`] with the build-time [`AuthConfig`].
//! `init` is infallible w.r.t. configuration: a misconfigured backend yields an
//! [`Auth`] whose [`Auth::is_misconfigured`] is `true` (Account tab hidden
//! downstream) rather than an error — never a boot stall. The full Convex
//! HTTP+WebSocket client (generation live queries) is Spike S-2 / Epic 9; this crate
//! is the M1 read-only slice (config + auth state + key storage + billing/feedback).
//!
//! `palmier-tauri` is intentionally NOT modified by this story — E1-S1 left the
//! client-config hook; wiring it is a later integration touch.

pub mod account;
pub mod config;
pub mod convex;
pub mod error;
pub mod keyring;
pub mod token;

pub use account::{
    top_off_limits, AccountPlan, AccountResponse, AccountState, AccountTier, AccountUser, AuthState,
    AvailablePlan,
};
pub use config::{AuthConfig, AuthConfigBuilder};
pub use convex::{
    validate_billing_url, ConvexBackend, FeedbackRequest, HttpConvexBackend, ALLOWED_BILLING_HOSTS,
};
pub use error::AuthError;
pub use keyring::{
    AnthropicKeyStore, InMemoryKeyStore, KeyChange, KeyStore, OsKeyStore, ANTHROPIC_KEY_ACCOUNT,
    KEYRING_SERVICE,
};
pub use token::{TokenCache, REFRESH_INTERVAL_MS};

/// Back-compat alias for the keyring account name (the skeleton exported
/// `KEYRING_ACCOUNT`; canonical name is [`ANTHROPIC_KEY_ACCOUNT`]).
pub const KEYRING_ACCOUNT: &str = ANTHROPIC_KEY_ACCOUNT;

/// The auth subsystem handle the Tauri boot owns.
///
/// Bundles the config, the Clerk JWT cache, the account state machine, and the
/// Anthropic-key store (OS keyring). The optional Convex backend is built only when
/// configured; when misconfigured it is `None` and every account-gated getter
/// reports the signed-out / misconfigured state.
///
/// Construct via [`Auth::init`] (boot step 4) or [`Auth::builder`] (tests / custom
/// backends).
pub struct Auth {
    config: AuthConfig,
    token: TokenCache,
    account: AccountState,
    key_store: AnthropicKeyStore<OsKeyStore>,
    convex: Option<Box<dyn ConvexBackend>>,
}

impl Auth {
    /// Boot entry point (E1-S1 step 4). Configures the auth subsystem from the
    /// build-time [`AuthConfig`].
    ///
    /// Infallible w.r.t. configuration: a missing Clerk key or Convex URL produces a
    /// misconfigured [`Auth`] (Account tab hidden downstream) rather than an error —
    /// the boot path is never blocked (FR-1 / OQ-9 / R-4). When configured, a real
    /// [`HttpConvexBackend`] is attached over the Convex HTTP URL; a transport-build
    /// failure also degrades to no backend rather than failing boot.
    #[must_use]
    pub fn init(config: AuthConfig) -> Self {
        let misconfigured = config.is_misconfigured();
        let convex: Option<Box<dyn ConvexBackend>> = if misconfigured {
            None
        } else {
            // Prefer the explicit HTTP URL; fall back to the deployment URL.
            config
                .convex_http_url
                .clone()
                .or_else(|| config.convex_deployment_url.clone())
                .and_then(|url| HttpConvexBackend::new(url).ok())
                .map(|b| Box::new(b) as Box<dyn ConvexBackend>)
        };
        Self {
            account: AccountState::new(misconfigured),
            token: TokenCache::new(),
            key_store: AnthropicKeyStore::os(),
            config,
            convex,
        }
    }

    /// Builder for tests / injecting a custom (mock) Convex backend.
    #[must_use]
    pub fn builder(config: AuthConfig) -> AuthBuilder {
        AuthBuilder {
            config,
            convex: None,
        }
    }

    /// The build-time config.
    #[must_use]
    pub fn config(&self) -> &AuthConfig {
        &self.config
    }

    /// True when the Clerk key or Convex URL is missing (Account tab hidden).
    #[must_use]
    pub fn is_misconfigured(&self) -> bool {
        self.config.is_misconfigured()
    }

    /// The Clerk JWT cache (mutable — the webview pushes fresh JWTs here).
    pub fn token_mut(&mut self) -> &mut TokenCache {
        &mut self.token
    }

    /// The Clerk JWT cache (read-only).
    #[must_use]
    pub fn token(&self) -> &TokenCache {
        &self.token
    }

    /// The account state machine (read-only).
    #[must_use]
    pub fn account(&self) -> &AccountState {
        &self.account
    }

    /// The account state machine (mutable — the Convex layer feeds it).
    pub fn account_mut(&mut self) -> &mut AccountState {
        &mut self.account
    }

    /// The Anthropic-key store (OS keyring, account `anthropic-api-key`).
    #[must_use]
    pub fn anthropic_key(&self) -> &AnthropicKeyStore<OsKeyStore> {
        &self.key_store
    }

    /// The attached Convex backend, if configured.
    #[must_use]
    pub fn convex(&self) -> Option<&dyn ConvexBackend> {
        self.convex.as_deref()
    }
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("is_misconfigured", &self.is_misconfigured())
            .field("has_convex", &self.convex.is_some())
            .finish()
    }
}

/// Builder for [`Auth`] (tests / custom backends).
pub struct AuthBuilder {
    config: AuthConfig,
    convex: Option<Box<dyn ConvexBackend>>,
}

impl AuthBuilder {
    /// Inject a Convex backend (e.g. a mock). Overrides the default HTTP backend.
    #[must_use]
    pub fn convex_backend(mut self, backend: Box<dyn ConvexBackend>) -> Self {
        self.convex = Some(backend);
        self
    }

    /// Build the [`Auth`] handle.
    #[must_use]
    pub fn build(self) -> Auth {
        let misconfigured = self.config.is_misconfigured();
        Auth {
            account: AccountState::new(misconfigured),
            token: TokenCache::new(),
            key_store: AnthropicKeyStore::os(),
            config: self.config,
            convex: self.convex,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_account_matches_ruling() {
        assert_eq!(KEYRING_ACCOUNT, "anthropic-api-key");
        assert_eq!(ANTHROPIC_KEY_ACCOUNT, "anthropic-api-key");
    }

    #[test]
    fn init_with_missing_config_is_misconfigured_not_an_error() {
        let auth = Auth::init(AuthConfig::default());
        assert!(auth.is_misconfigured());
        assert!(auth.convex().is_none());
        assert!(!auth.account().is_signed_in());
        assert!(!auth.account().ai_allowed());
    }

    #[test]
    fn init_with_full_config_attaches_convex_backend() {
        let cfg = AuthConfig::builder()
            .clerk_publishable_key("pk_test_abc")
            .convex_deployment_url("https://example.convex.cloud")
            .convex_http_url("https://example.convex.site")
            .build();
        let auth = Auth::init(cfg);
        assert!(!auth.is_misconfigured());
        assert!(auth.convex().is_some());
    }

    #[test]
    fn builder_injects_mock_backend() {
        let cfg = AuthConfig::builder()
            .clerk_publishable_key("pk_test_abc")
            .convex_deployment_url("https://example.convex.cloud")
            .build();
        let auth = Auth::builder(cfg)
            .convex_backend(Box::new(MockConvexBackend::signed_in_pro()))
            .build();
        assert!(auth.convex().is_some());
        // The state machine drives off the mock without any live network.
        let plans = auth.convex().unwrap().list_plans().unwrap();
        assert_eq!(plans.len(), 2);
    }

    /// A mock [`ConvexBackend`] that proves the seam works without a live backend.
    struct MockConvexBackend {
        account: AccountResponse,
    }

    impl MockConvexBackend {
        fn signed_in_pro() -> Self {
            Self {
                account: AccountResponse {
                    user: AccountUser {
                        tier: AccountTier::Pro,
                        spent_credits_this_period: Some(100),
                        purchased_credits: Some(0),
                        ..Default::default()
                    },
                    plan: Some(AccountPlan {
                        tier: AccountTier::Pro,
                        monthly_price_usd: 20,
                        monthly_budget_credits: Some(1000),
                    }),
                },
            }
        }
    }

    impl ConvexBackend for MockConvexBackend {
        fn upsert_from_auth(
            &self,
            _jwt: &str,
            _email: Option<&str>,
            _name: Option<&str>,
            _image: Option<&str>,
        ) -> Result<(), AuthError> {
            Ok(())
        }
        fn get_account(&self, _jwt: &str) -> Result<AccountResponse, AuthError> {
            Ok(self.account.clone())
        }
        fn list_plans(&self) -> Result<Vec<AvailablePlan>, AuthError> {
            Ok(vec![
                AvailablePlan {
                    tier: AccountTier::Pro,
                    monthly_price_usd: 20,
                    discounted_monthly_price_usd: None,
                    monthly_budget_credits: Some(1000),
                },
                AvailablePlan {
                    tier: AccountTier::Max,
                    monthly_price_usd: 50,
                    discounted_monthly_price_usd: None,
                    monthly_budget_credits: Some(5000),
                },
            ])
        }
        fn create_checkout_session(
            &self,
            _jwt: &str,
            _tier: AccountTier,
        ) -> Result<String, AuthError> {
            Ok("https://checkout.stripe.com/c/pay/test".to_string())
        }
        fn create_top_off_checkout_session(
            &self,
            _jwt: &str,
            _dollars: i64,
        ) -> Result<String, AuthError> {
            Ok("https://checkout.stripe.com/c/pay/topoff".to_string())
        }
        fn create_portal_session(&self, _jwt: &str) -> Result<String, AuthError> {
            Ok("https://billing.stripe.com/p/session/test".to_string())
        }
        fn send_feedback(
            &self,
            _jwt: Option<&str>,
            _req: &FeedbackRequest,
        ) -> Result<(), AuthError> {
            Ok(())
        }
    }

    /// End-to-end (no network): feed a mock account snapshot through the state
    /// machine via the trait and assert the derived credit state, plus the billing
    /// URL allowlist on the returned checkout URL.
    #[test]
    fn mock_backend_drives_state_machine_and_billing_guard() {
        let cfg = AuthConfig::builder()
            .clerk_publishable_key("pk_test_abc")
            .convex_deployment_url("https://example.convex.cloud")
            .build();
        let mut auth = Auth::builder(cfg)
            .convex_backend(Box::new(MockConvexBackend::signed_in_pro()))
            .build();

        // Webview pushes a JWT; cache is fresh.
        auth.token_mut().set("clerk-jwt");
        assert!(!auth.token().needs_refresh());

        // Authenticate + pull account via the backend.
        auth.account_mut().set_auth_state(AuthState::Authenticated);
        let snapshot = auth
            .convex()
            .unwrap()
            .get_account(auth.token().jwt().unwrap())
            .unwrap();
        auth.account_mut().set_account(snapshot);

        assert!(auth.account().is_signed_in());
        assert_eq!(auth.account().tier(), AccountTier::Pro);
        assert_eq!(auth.account().remaining_credits(), 900);

        // A returned checkout URL passes the allowlist.
        let url = auth
            .convex()
            .unwrap()
            .create_checkout_session(auth.token().jwt().unwrap(), AccountTier::Pro)
            .unwrap();
        assert!(validate_billing_url(&url).is_ok());
    }
}
