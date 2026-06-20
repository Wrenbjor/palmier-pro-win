//! Convex HTTP backend seam.
//!
//! The macOS app used the native `ConvexMobile` SDK (WebSocket live queries +
//! actions). The Windows/Linux port talks to Convex over HTTP via `reqwest`
//! (settings-account-app.md "macOS/Apple APIs to replace"; FOUNDATION §8.1). This
//! module is the **M1 read-only slice**: config + auth-state + key storage + the
//! billing/feedback action calls. The full HTTP+WebSocket client (generation live
//! queries) is Spike S-2 / Epic 9 and is deliberately NOT implemented here (E1-S6
//! dependency note).
//!
//! All network is behind the [`ConvexBackend`] trait so the account state machine
//! and billing flow are testable without a live backend ([`MockConvexBackend`]).
//! Unauthenticated / unreachable degrades to signed-out — never an app error
//! (OQ-9 / R-4).

use crate::account::{AccountResponse, AccountTier, AvailablePlan};
use crate::error::AuthError;
use url::Url;

/// Hosts a returned billing/portal URL is allowed to point at. Ported VERBATIM from
/// the reference `allowedBillingHosts` — this is a security control
/// (settings-account-app.md gotcha): a URL is only opened if scheme is `https` AND
/// host is in this set.
pub const ALLOWED_BILLING_HOSTS: [&str; 2] = ["checkout.stripe.com", "billing.stripe.com"];

/// Validate a billing/portal URL against the https + allowlisted-host guard.
/// Returns the parsed [`Url`] on success, or [`AuthError::UntrustedUrl`]. Ported from
/// reference `openInBrowser`.
pub fn validate_billing_url(raw: &str) -> Result<Url, AuthError> {
    let url = Url::parse(raw).map_err(|_| AuthError::UntrustedUrl(raw.to_string()))?;
    if url.scheme() != "https" {
        return Err(AuthError::UntrustedUrl(raw.to_string()));
    }
    match url.host_str() {
        Some(host) if ALLOWED_BILLING_HOSTS.contains(&host) => Ok(url),
        _ => Err(AuthError::UntrustedUrl(raw.to_string())),
    }
}

/// A feedback submission (reference `feedback:send` args).
#[derive(Debug, Clone)]
pub struct FeedbackRequest {
    pub message: String,
    pub may_contact: bool,
    pub email: Option<String>,
    pub screenshot_png_base64: Option<String>,
    pub app_version: String,
    pub os_version: String,
}

/// The Convex action/query surface this crate needs for M1. Implemented by
/// [`HttpConvexBackend`] (real `reqwest`) and [`MockConvexBackend`] (tests).
///
/// `auth_jwt`, where present, is the cached Clerk JWT forwarded as `Bearer`
/// (FOUNDATION §8.2). A `None` JWT means anonymous — calls that require auth should
/// surface a recoverable error, not panic.
pub trait ConvexBackend: Send + Sync {
    /// `users:upsertFromAuth` — idempotent provisioning on sign-in (reference runs it
    /// with up to 3 tries; the retry loop lives in the caller).
    fn upsert_from_auth(
        &self,
        auth_jwt: &str,
        email: Option<&str>,
        name: Option<&str>,
        image: Option<&str>,
    ) -> Result<(), AuthError>;

    /// `account:get` — current account snapshot (reference account subscription; M1
    /// is a one-shot HTTP fetch rather than a live subscription).
    fn get_account(&self, auth_jwt: &str) -> Result<AccountResponse, AuthError>;

    /// `billing:listPlans` — available plans for the signed-out plan cards.
    fn list_plans(&self) -> Result<Vec<AvailablePlan>, AuthError>;

    /// `billing:createCheckoutSession` — returns a checkout URL (caller MUST pass it
    /// through [`validate_billing_url`] before opening).
    fn create_checkout_session(&self, auth_jwt: &str, tier: AccountTier)
        -> Result<String, AuthError>;

    /// `billing:createTopOffCheckoutSession` — returns a top-off checkout URL.
    fn create_top_off_checkout_session(
        &self,
        auth_jwt: &str,
        dollars: i64,
    ) -> Result<String, AuthError>;

    /// `billing:createPortalSession` — returns a customer-portal URL.
    fn create_portal_session(&self, auth_jwt: &str) -> Result<String, AuthError>;

    /// `feedback:send` — submit feedback (optional auth).
    fn send_feedback(
        &self,
        auth_jwt: Option<&str>,
        request: &FeedbackRequest,
    ) -> Result<(), AuthError>;
}

/// Real Convex HTTP backend over `reqwest::blocking`. Holds the `/v1` base URL
/// (FOUNDATION §8.1). Reqwest is the locked transport for Convex HTTP (FOUNDATION
/// §2 / §8.1); the blocking client keeps this M1 read-only slice runtime-agnostic —
/// Epic 9's live-query client introduces the async WebSocket path.
#[derive(Debug, Clone)]
pub struct HttpConvexBackend {
    http_base: Url,
    client: reqwest::blocking::Client,
}

impl HttpConvexBackend {
    /// Construct against the Convex HTTP base URL (`PALMIER_CONVEX_HTTP_URL`).
    pub fn new(http_base: Url) -> Result<Self, AuthError> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .map_err(|e| AuthError::Convex(e.to_string()))?;
        Ok(Self { http_base, client })
    }

    fn url(&self, path: &str) -> Result<Url, AuthError> {
        self.http_base
            .join(path)
            .map_err(|e| AuthError::Convex(e.to_string()))
    }

    fn post_json(
        &self,
        path: &str,
        auth_jwt: Option<&str>,
        body: serde_json::Value,
    ) -> Result<reqwest::blocking::Response, AuthError> {
        let mut req = self.client.post(self.url(path)?).json(&body);
        if let Some(jwt) = auth_jwt {
            req = req.bearer_auth(jwt);
        }
        let resp = req.send().map_err(|e| AuthError::Convex(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AuthError::Convex(format!("status {}", resp.status())));
        }
        Ok(resp)
    }

    fn get(
        &self,
        path: &str,
        auth_jwt: Option<&str>,
    ) -> Result<reqwest::blocking::Response, AuthError> {
        let mut req = self.client.get(self.url(path)?);
        if let Some(jwt) = auth_jwt {
            req = req.bearer_auth(jwt);
        }
        let resp = req.send().map_err(|e| AuthError::Convex(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AuthError::Convex(format!("status {}", resp.status())));
        }
        Ok(resp)
    }
}

/// `{ "url": "..." }` envelope returned by the billing actions (reference `UrlResponse`).
#[derive(serde::Deserialize)]
struct UrlResponse {
    url: String,
}

impl ConvexBackend for HttpConvexBackend {
    fn upsert_from_auth(
        &self,
        auth_jwt: &str,
        email: Option<&str>,
        name: Option<&str>,
        image: Option<&str>,
    ) -> Result<(), AuthError> {
        let body = serde_json::json!({
            "email": email,
            "name": name,
            "image": image,
        });
        self.post_json("v1/users/upsertFromAuth", Some(auth_jwt), body)?;
        Ok(())
    }

    fn get_account(&self, auth_jwt: &str) -> Result<AccountResponse, AuthError> {
        self.get("v1/account", Some(auth_jwt))?
            .json::<AccountResponse>()
            .map_err(|e| AuthError::Convex(e.to_string()))
    }

    fn list_plans(&self) -> Result<Vec<AvailablePlan>, AuthError> {
        self.get("v1/billing/plans", None)?
            .json::<Vec<AvailablePlan>>()
            .map_err(|e| AuthError::Convex(e.to_string()))
    }

    fn create_checkout_session(
        &self,
        auth_jwt: &str,
        tier: AccountTier,
    ) -> Result<String, AuthError> {
        let tier_str = match tier {
            AccountTier::None => "none",
            AccountTier::Pro => "pro",
            AccountTier::Max => "max",
        };
        let resp = self.post_json(
            "v1/billing/createCheckoutSession",
            Some(auth_jwt),
            serde_json::json!({ "tier": tier_str }),
        )?;
        Ok(resp
            .json::<UrlResponse>()
            .map_err(|e| AuthError::Convex(e.to_string()))?
            .url)
    }

    fn create_top_off_checkout_session(
        &self,
        auth_jwt: &str,
        dollars: i64,
    ) -> Result<String, AuthError> {
        let resp = self.post_json(
            "v1/billing/createTopOffCheckoutSession",
            Some(auth_jwt),
            serde_json::json!({ "dollars": dollars }),
        )?;
        Ok(resp
            .json::<UrlResponse>()
            .map_err(|e| AuthError::Convex(e.to_string()))?
            .url)
    }

    fn create_portal_session(&self, auth_jwt: &str) -> Result<String, AuthError> {
        let resp = self.post_json(
            "v1/billing/createPortalSession",
            Some(auth_jwt),
            serde_json::json!({}),
        )?;
        Ok(resp
            .json::<UrlResponse>()
            .map_err(|e| AuthError::Convex(e.to_string()))?
            .url)
    }

    fn send_feedback(
        &self,
        auth_jwt: Option<&str>,
        request: &FeedbackRequest,
    ) -> Result<(), AuthError> {
        let mut body = serde_json::json!({
            "message": request.message,
            "mayContact": request.may_contact,
            "appVersion": request.app_version,
            "osVersion": request.os_version,
        });
        if let Some(email) = &request.email {
            body["email"] = serde_json::Value::String(email.clone());
        }
        if let Some(shot) = &request.screenshot_png_base64 {
            body["screenshotPngBase64"] = serde_json::Value::String(shot.clone());
        }
        self.post_json("v1/feedback", auth_jwt, body)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billing_url_allowlist_accepts_stripe_https() {
        assert!(validate_billing_url("https://checkout.stripe.com/c/pay/abc").is_ok());
        assert!(validate_billing_url("https://billing.stripe.com/p/session/xyz").is_ok());
    }

    #[test]
    fn billing_url_allowlist_rejects_off_host_and_http() {
        // Wrong scheme.
        assert!(validate_billing_url("http://checkout.stripe.com/c/pay").is_err());
        // Wrong host (lookalike).
        assert!(validate_billing_url("https://checkout.stripe.com.evil.com/c").is_err());
        assert!(validate_billing_url("https://evil.com/c").is_err());
        // Garbage.
        assert!(validate_billing_url("not a url").is_err());
        // Hosts list is exactly the two reference values.
        assert_eq!(
            ALLOWED_BILLING_HOSTS,
            ["checkout.stripe.com", "billing.stripe.com"]
        );
    }
}
