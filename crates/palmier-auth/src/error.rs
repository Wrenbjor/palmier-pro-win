//! Error type for `palmier-auth`.

/// Errors surfaced by the auth crate. Network/Convex failures are deliberately
/// recoverable: the state machine degrades to signed-out rather than erroring the
/// app (OQ-9 / R-4) — these variants are for the caller's logging, not for aborting
/// boot.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// OS-keyring backend failure (Credential Manager / Secret Service).
    #[error("keyring error: {0}")]
    Keyring(String),

    /// Convex HTTP transport / decode failure. Treated as "unreachable" by the
    /// state machine — degrades to signed-out, never an app error.
    #[error("convex transport error: {0}")]
    Convex(String),

    /// A returned billing/portal URL failed the https + allowlisted-host guard
    /// (security control ported verbatim — settings-account-app.md gotcha).
    #[error("refused to open untrusted URL: {0}")]
    UntrustedUrl(String),

    /// The service is misconfigured (missing Clerk key or Convex URL).
    #[error("auth is misconfigured (missing Clerk key or Convex URL)")]
    Misconfigured,
}
