//! # palmier-auth
//!
//! Clerk token cache, Convex account/credit state, and OS-keyring storage for the
//! Anthropic API key (FOUNDATION §4, §6.13; E1-S6). Wraps `reqwest` (Convex HTTP)
//! and the `keyring` crate (Windows Credential Manager / Linux Secret Service);
//! those deps are added per-story, not in this skeleton.

/// OS-keyring account name for the Anthropic key (ruling #5: `anthropic-api-key`).
pub const KEYRING_ACCOUNT: &str = "anthropic-api-key";

#[cfg(test)]
mod tests {
    #[test]
    fn keyring_account_matches_ruling() {
        assert_eq!(super::KEYRING_ACCOUNT, "anthropic-api-key");
    }
}
