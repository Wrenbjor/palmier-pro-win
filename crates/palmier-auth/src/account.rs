//! Account / plan / credit state machine.
//!
//! Ports `Account/AccountService.swift`. The macOS service was an `@Observable`
//! actor driven by Clerk + Convex live subscriptions; here it is a plain state
//! struct that the Convex layer (or the webview, via Tauri commands) feeds. The
//! derived getters (`tier`, `budget_credits`, `remaining_credits`, `ai_allowed`)
//! match the reference exactly (settings-account-app.md "Auth/account state machine").
//!
//! Degradation rule (OQ-9 / R-4): an unauthenticated session OR an unreachable
//! Convex backend degrades to [`AuthState::Unauthenticated`] — never an app error.

use serde::Deserialize;

/// Subscription tier (reference `AccountTier`). Decodes from the Convex
/// `account:get` payload's `user.tier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountTier {
    /// Free / no paid plan.
    #[default]
    None,
    /// Pro plan.
    Pro,
    /// Max plan.
    Max,
}

impl AccountTier {
    /// Any paid tier (reference `isPaid`).
    #[must_use]
    pub fn is_paid(self) -> bool {
        self != AccountTier::None
    }

    /// Human label (reference `planLabel`).
    #[must_use]
    pub fn plan_label(self) -> &'static str {
        match self {
            AccountTier::None => "Free",
            AccountTier::Pro => "Pro plan",
            AccountTier::Max => "Max plan",
        }
    }
}

/// `user` object from `account:get` (reference `AccountUser`). Credit fields are
/// optional and default to 0 in the derived getters.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountUser {
    pub email: Option<String>,
    pub name: Option<String>,
    pub image: Option<String>,
    #[serde(default)]
    pub tier: AccountTier,
    #[serde(rename = "currentPeriodEnd")]
    pub current_period_end: Option<f64>,
    #[serde(rename = "cancelAtPeriodEnd")]
    pub cancel_at_period_end: Option<bool>,
    #[serde(rename = "spentCreditsThisPeriod")]
    pub spent_credits_this_period: Option<i64>,
    #[serde(rename = "purchasedCredits")]
    pub purchased_credits: Option<i64>,
}

/// `plan` object from `account:get` (reference `AccountPlan`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountPlan {
    #[serde(default)]
    pub tier: AccountTier,
    #[serde(rename = "monthlyPriceUsd")]
    pub monthly_price_usd: i64,
    #[serde(rename = "monthlyBudgetCredits")]
    pub monthly_budget_credits: Option<i64>,
}

/// Full `account:get` response (reference `AccountResponse`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AccountResponse {
    pub user: AccountUser,
    pub plan: Option<AccountPlan>,
}

/// One entry from `billing:listPlans` (reference `AvailablePlan`).
#[derive(Debug, Clone, Deserialize)]
pub struct AvailablePlan {
    #[serde(default)]
    pub tier: AccountTier,
    #[serde(rename = "monthlyPriceUsd")]
    pub monthly_price_usd: i64,
    #[serde(rename = "discountedMonthlyPriceUsd")]
    pub discounted_monthly_price_usd: Option<i64>,
    #[serde(rename = "monthlyBudgetCredits")]
    pub monthly_budget_credits: Option<i64>,
}

/// Top-off (buy-more-credits) dollar limits (reference `TopOffLimits`).
pub mod top_off_limits {
    /// Minimum top-off amount in USD.
    pub const MIN_DOLLARS: i64 = 5;
    /// Maximum top-off amount in USD.
    pub const MAX_DOLLARS: i64 = 1000;
    /// Default top-off amount in USD (settings-account-app.md: default $20).
    pub const DEFAULT_DOLLARS: i64 = 20;
}

/// Clerk/Convex auth observation state (reference `AuthState`). Mirrors the three
/// states the macOS service tracked from `convex.authState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthState {
    /// Restoring a cached session / waiting on Clerk.
    #[default]
    Loading,
    /// Signed in with an active session.
    Authenticated,
    /// Signed out (or degraded from an unreachable backend).
    Unauthenticated,
}

/// The account state machine. Holds the live auth state, the latest `account:get`
/// snapshot, available plans, and `is_misconfigured`; exposes the reference's
/// derived getters. Network is fed in from the Convex layer — this struct contains
/// no I/O, so it is fully unit-testable.
#[derive(Debug, Clone)]
pub struct AccountState {
    is_misconfigured: bool,
    is_loading: bool,
    auth_state: AuthState,
    account: Option<AccountResponse>,
    available_plans: Vec<AvailablePlan>,
    last_error: Option<String>,
}

impl AccountState {
    /// New state for a given misconfigured flag. When misconfigured, the machine is
    /// permanently signed-out and not loading (reference `configure()` early-out:
    /// `isMisconfigured = true; isLoading = false`).
    #[must_use]
    pub fn new(is_misconfigured: bool) -> Self {
        Self {
            is_misconfigured,
            is_loading: !is_misconfigured,
            auth_state: AuthState::Loading,
            account: None,
            available_plans: Vec::new(),
            last_error: None,
        }
    }

    // --- transitions (driven by the Convex/auth layer) ---

    /// Apply an auth-state transition (reference `convex.authState` switch).
    /// `.authenticated` keeps any existing account snapshot until `account:get`
    /// arrives; `.unauthenticated` clears the account (reference `clearAccount`).
    pub fn set_auth_state(&mut self, state: AuthState) {
        if self.is_misconfigured {
            // Misconfigured never authenticates.
            self.auth_state = AuthState::Unauthenticated;
            self.is_loading = false;
            return;
        }
        self.auth_state = state;
        match state {
            AuthState::Loading => self.is_loading = true,
            AuthState::Authenticated => self.is_loading = false,
            AuthState::Unauthenticated => {
                self.clear_account();
                self.is_loading = false;
            }
        }
    }

    /// Feed a fresh `account:get` snapshot (reference account subscription value).
    /// Clears `last_error` on success, matching the reference.
    pub fn set_account(&mut self, account: AccountResponse) {
        self.account = Some(account);
        self.last_error = None;
    }

    /// Feed the `billing:listPlans` result (reference plans subscription value).
    pub fn set_available_plans(&mut self, plans: Vec<AvailablePlan>) {
        self.available_plans = plans;
    }

    /// Record a recoverable error (reference `lastError`). Does NOT change auth state.
    pub fn set_last_error(&mut self, error: impl Into<String>) {
        self.last_error = Some(error.into());
    }

    /// Drop the account snapshot (reference `clearAccount`).
    pub fn clear_account(&mut self) {
        self.account = None;
    }

    // --- derived getters (reference parity) ---

    /// True only when configured and authenticated (reference `isSignedIn`).
    #[must_use]
    pub fn is_signed_in(&self) -> bool {
        !self.is_misconfigured && self.auth_state == AuthState::Authenticated
    }

    /// `aiAllowed = isSignedIn && !isMisconfigured` (reference).
    #[must_use]
    pub fn ai_allowed(&self) -> bool {
        self.is_signed_in() && !self.is_misconfigured
    }

    /// Effective tier — `account.user.tier` or [`AccountTier::None`] (reference `tier`).
    #[must_use]
    pub fn tier(&self) -> AccountTier {
        self.account
            .as_ref()
            .map(|a| a.user.tier)
            .unwrap_or_default()
    }

    /// Credits spent this period (reference `spentCredits`).
    #[must_use]
    pub fn spent_credits(&self) -> i64 {
        self.account
            .as_ref()
            .and_then(|a| a.user.spent_credits_this_period)
            .unwrap_or(0)
    }

    /// `budgetCredits = plan.monthlyBudgetCredits + user.purchasedCredits` —
    /// `None` until an account snapshot exists (reference returns nil pre-account).
    #[must_use]
    pub fn budget_credits(&self) -> Option<i64> {
        let account = self.account.as_ref()?;
        let tier_budget = account
            .plan
            .as_ref()
            .and_then(|p| p.monthly_budget_credits)
            .unwrap_or(0);
        let purchased = account.user.purchased_credits.unwrap_or(0);
        Some(tier_budget + purchased)
    }

    /// `remainingCredits = max(0, budget - spent)` (reference).
    #[must_use]
    pub fn remaining_credits(&self) -> i64 {
        (self.budget_credits().unwrap_or(0) - self.spent_credits()).max(0)
    }

    /// `hasCredits = remainingCredits > 0` (reference).
    #[must_use]
    pub fn has_credits(&self) -> bool {
        self.remaining_credits() > 0
    }

    /// Whether the service is misconfigured (Account tab hidden downstream).
    #[must_use]
    pub fn is_misconfigured(&self) -> bool {
        self.is_misconfigured
    }

    /// Whether the machine is still loading the initial auth/account state.
    #[must_use]
    pub fn is_loading(&self) -> bool {
        self.is_loading
    }

    /// Current auth observation state.
    #[must_use]
    pub fn auth_state(&self) -> AuthState {
        self.auth_state
    }

    /// Latest recoverable error, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// Latest account snapshot, if any.
    #[must_use]
    pub fn account(&self) -> Option<&AccountResponse> {
        self.account.as_ref()
    }

    /// Available plans (`billing:listPlans`).
    #[must_use]
    pub fn available_plans(&self) -> &[AvailablePlan] {
        &self.available_plans
    }

    /// Find the available plan for a tier (reference `availablePlan(for:)`).
    #[must_use]
    pub fn available_plan(&self, tier: AccountTier) -> Option<&AvailablePlan> {
        self.available_plans.iter().find(|p| p.tier == tier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paid_account(tier: AccountTier, budget: i64, purchased: i64, spent: i64) -> AccountResponse {
        AccountResponse {
            user: AccountUser {
                tier,
                spent_credits_this_period: Some(spent),
                purchased_credits: Some(purchased),
                ..Default::default()
            },
            plan: Some(AccountPlan {
                tier,
                monthly_price_usd: 20,
                monthly_budget_credits: Some(budget),
            }),
        }
    }

    #[test]
    fn misconfigured_is_never_signed_in() {
        let mut s = AccountState::new(true);
        assert!(s.is_misconfigured());
        assert!(!s.is_loading());
        // Even an authenticated transition cannot sign in a misconfigured service.
        s.set_auth_state(AuthState::Authenticated);
        assert!(!s.is_signed_in());
        assert!(!s.ai_allowed());
        assert_eq!(s.auth_state(), AuthState::Unauthenticated);
    }

    #[test]
    fn loading_to_authenticated_to_unauthenticated() {
        let mut s = AccountState::new(false);
        assert_eq!(s.auth_state(), AuthState::Loading);
        assert!(s.is_loading());
        assert!(!s.is_signed_in());

        s.set_auth_state(AuthState::Authenticated);
        assert!(s.is_signed_in());
        assert!(s.ai_allowed());
        assert!(!s.is_loading());

        s.set_account(paid_account(AccountTier::Pro, 1000, 200, 300));
        assert_eq!(s.tier(), AccountTier::Pro);
        // budget = 1000 + 200, remaining = 1200 - 300.
        assert_eq!(s.budget_credits(), Some(1200));
        assert_eq!(s.remaining_credits(), 900);
        assert!(s.has_credits());

        // Signing out clears the account snapshot.
        s.set_auth_state(AuthState::Unauthenticated);
        assert!(!s.is_signed_in());
        assert!(s.account().is_none());
        assert_eq!(s.tier(), AccountTier::None);
        assert_eq!(s.budget_credits(), None);
        assert_eq!(s.remaining_credits(), 0);
        assert!(!s.has_credits());
    }

    #[test]
    fn remaining_credits_never_negative() {
        let mut s = AccountState::new(false);
        s.set_auth_state(AuthState::Authenticated);
        // spent exceeds budget.
        s.set_account(paid_account(AccountTier::Pro, 100, 0, 500));
        assert_eq!(s.remaining_credits(), 0);
        assert!(!s.has_credits());
    }

    #[test]
    fn tier_decodes_lowercase() {
        let resp: AccountResponse =
            serde_json::from_str(r#"{"user":{"tier":"max"}}"#).unwrap();
        assert_eq!(resp.user.tier, AccountTier::Max);
        assert!(resp.user.tier.is_paid());
    }

    #[test]
    fn account_get_payload_decodes_with_camelcase_fields() {
        let json = r#"{
            "user": {
                "email": "sam@example.com",
                "name": "Sam",
                "tier": "pro",
                "spentCreditsThisPeriod": 150,
                "purchasedCredits": 50,
                "cancelAtPeriodEnd": false
            },
            "plan": { "tier": "pro", "monthlyPriceUsd": 20, "monthlyBudgetCredits": 1000 }
        }"#;
        let resp: AccountResponse = serde_json::from_str(json).unwrap();
        let mut s = AccountState::new(false);
        s.set_auth_state(AuthState::Authenticated);
        s.set_account(resp);
        assert_eq!(s.tier(), AccountTier::Pro);
        assert_eq!(s.budget_credits(), Some(1050));
        assert_eq!(s.remaining_credits(), 900);
    }

    #[test]
    fn last_error_does_not_change_auth_state() {
        let mut s = AccountState::new(false);
        s.set_auth_state(AuthState::Authenticated);
        s.set_last_error("convex unreachable");
        assert!(s.is_signed_in());
        assert_eq!(s.last_error(), Some("convex unreachable"));
        // A fresh account snapshot clears the error.
        s.set_account(AccountResponse::default());
        assert_eq!(s.last_error(), None);
    }

    #[test]
    fn top_off_limits_match_reference() {
        assert_eq!(top_off_limits::MIN_DOLLARS, 5);
        assert_eq!(top_off_limits::MAX_DOLLARS, 1000);
        assert_eq!(top_off_limits::DEFAULT_DOLLARS, 20);
    }
}
