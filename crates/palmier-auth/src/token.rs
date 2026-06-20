//! Clerk JWT cache with a 5-minute refresh window.
//!
//! Clerk lives in the webview (`@clerk/clerk-react`); after each sign-in event the
//! JWT is forwarded to Rust via a Tauri command and stored here in memory, then
//! sent to Convex as a `Bearer` token (settings-account-app.md "Auth/account state
//! machine"; FOUNDATION §8.2). Clerk refreshes the session silently; FOUNDATION
//! §8.2 specifies "we refresh the cached JWT every 5 min", so a token older than
//! [`TokenCache::REFRESH_INTERVAL`] is reported stale and the boot/agent layer
//! re-pulls a fresh JWT from the webview before using it.
//!
//! The clock is injected (a `now_ms` closure) so the 5-minute logic is unit-tested
//! deterministically without sleeping.

/// How long a cached JWT is considered fresh before a refresh pull is needed
/// (FOUNDATION §8.2: refresh every 5 minutes).
pub const REFRESH_INTERVAL_MS: u64 = 5 * 60 * 1000;

/// A cached Clerk JWT plus the wall-clock time (ms since epoch) it was stored.
#[derive(Debug, Clone)]
struct CachedToken {
    jwt: String,
    stored_at_ms: u64,
}

/// In-memory Clerk JWT cache. Holds at most one token; `set` overwrites.
///
/// `now_ms` is the clock source (defaults to system time via [`TokenCache::new`];
/// tests use [`TokenCache::with_clock`] to drive it deterministically).
pub struct TokenCache {
    token: Option<CachedToken>,
    now_ms: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl std::fmt::Debug for TokenCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenCache")
            .field("has_token", &self.token.is_some())
            .finish()
    }
}

impl TokenCache {
    /// New empty cache using the system wall clock.
    #[must_use]
    pub fn new() -> Self {
        Self::with_clock(system_now_ms)
    }

    /// New empty cache with an injected clock (for tests / non-default time sources).
    #[must_use]
    pub fn with_clock(now_ms: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self {
            token: None,
            now_ms: Box::new(now_ms),
        }
    }

    /// Store a freshly-pulled JWT, stamping it with the current clock time.
    /// Empty/whitespace JWTs clear the cache (signed-out / cleared session).
    pub fn set(&mut self, jwt: impl Into<String>) {
        let jwt = jwt.into();
        if jwt.trim().is_empty() {
            self.clear();
            return;
        }
        let stored_at_ms = (self.now_ms)();
        self.token = Some(CachedToken { jwt, stored_at_ms });
    }

    /// Drop the cached token (sign-out / auth state `.unauthenticated`).
    pub fn clear(&mut self) {
        self.token = None;
    }

    /// Is any token cached at all?
    #[must_use]
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// Age of the cached token in ms, or `None` if no token is cached.
    #[must_use]
    pub fn age_ms(&self) -> Option<u64> {
        self.token
            .as_ref()
            .map(|t| (self.now_ms)().saturating_sub(t.stored_at_ms))
    }

    /// True when there is no token, or the cached token is older than the
    /// [`REFRESH_INTERVAL_MS`] window (FOUNDATION §8.2). The caller should pull a
    /// fresh JWT from the webview before forwarding it as `Bearer` to Convex.
    #[must_use]
    pub fn needs_refresh(&self) -> bool {
        match self.age_ms() {
            None => true,
            Some(age) => age >= REFRESH_INTERVAL_MS,
        }
    }

    /// The cached JWT regardless of staleness (the caller decides whether to act on
    /// [`Self::needs_refresh`] first).
    #[must_use]
    pub fn jwt(&self) -> Option<&str> {
        self.token.as_ref().map(|t| t.jwt.as_str())
    }

    /// The cached JWT only if still fresh (`!needs_refresh`); `None` if absent or
    /// stale. Convenience for "give me a Bearer token I can use without refreshing".
    #[must_use]
    pub fn fresh_jwt(&self) -> Option<&str> {
        if self.needs_refresh() {
            None
        } else {
            self.jwt()
        }
    }
}

impl Default for TokenCache {
    fn default() -> Self {
        Self::new()
    }
}

/// System wall clock in ms since the Unix epoch. Monotonicity is not required —
/// `age_ms` uses `saturating_sub` so a clock step-back cannot underflow.
fn system_now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// A controllable clock for deterministic refresh-window tests.
    fn fake_clock() -> (Arc<AtomicU64>, impl Fn() -> u64 + Send + Sync + 'static) {
        let now = Arc::new(AtomicU64::new(0));
        let handle = now.clone();
        (now, move || handle.load(Ordering::SeqCst))
    }

    #[test]
    fn empty_cache_needs_refresh_and_has_no_token() {
        let cache = TokenCache::with_clock(|| 0);
        assert!(!cache.has_token());
        assert!(cache.needs_refresh());
        assert_eq!(cache.fresh_jwt(), None);
    }

    #[test]
    fn fresh_token_is_usable_until_five_minutes() {
        let (now, clock) = fake_clock();
        let mut cache = TokenCache::with_clock(clock);
        cache.set("jwt-abc");

        // t = 0 -> fresh.
        assert!(cache.has_token());
        assert!(!cache.needs_refresh());
        assert_eq!(cache.fresh_jwt(), Some("jwt-abc"));

        // t = 4m59s -> still fresh.
        now.store(REFRESH_INTERVAL_MS - 1000, Ordering::SeqCst);
        assert!(!cache.needs_refresh());
        assert_eq!(cache.fresh_jwt(), Some("jwt-abc"));
    }

    #[test]
    fn token_goes_stale_at_exactly_five_minutes() {
        let (now, clock) = fake_clock();
        let mut cache = TokenCache::with_clock(clock);
        cache.set("jwt-abc");

        // t = exactly 5m -> stale (>= boundary).
        now.store(REFRESH_INTERVAL_MS, Ordering::SeqCst);
        assert!(cache.needs_refresh());
        // Raw jwt still readable, but fresh_jwt withholds it.
        assert_eq!(cache.jwt(), Some("jwt-abc"));
        assert_eq!(cache.fresh_jwt(), None);

        // Re-pulling a JWT re-stamps the time -> fresh again.
        cache.set("jwt-def");
        assert!(!cache.needs_refresh());
        assert_eq!(cache.fresh_jwt(), Some("jwt-def"));
    }

    #[test]
    fn setting_empty_jwt_clears_cache() {
        let mut cache = TokenCache::with_clock(|| 0);
        cache.set("jwt-abc");
        cache.set("   ");
        assert!(!cache.has_token());
        assert!(cache.needs_refresh());
    }

    #[test]
    fn clock_step_back_does_not_underflow_age() {
        let (now, clock) = fake_clock();
        now.store(10_000, Ordering::SeqCst);
        let mut cache = TokenCache::with_clock(clock);
        cache.set("jwt-abc");
        // Clock jumps backwards.
        now.store(0, Ordering::SeqCst);
        assert_eq!(cache.age_ms(), Some(0));
        assert!(!cache.needs_refresh());
    }
}
