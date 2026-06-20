//! OS-keyring storage for the Anthropic API key.
//!
//! Ports `Utilities/KeychainStore.swift` + `AnthropicKeychain`
//! (`Agent/Clients/AnthropicClient.swift`). The macOS Security-framework keychain
//! becomes the `keyring` crate: Windows Credential Manager / Linux Secret Service
//! (settings-account-app.md "macOS/Apple APIs to replace").
//!
//! Ruling #5 (phase0-reconciliation.md): the keyring **account** name is
//! `anthropic-api-key` (NOT FOUNDATION's `palmier-pro-anthropic-api-key`) — a wrong
//! name silently loses the user's saved key. The **service** name is `palmier-pro`.
//!
//! Save/delete emit an [`KeyChange`] event (reference posts `anthropicAPIKeyChanged`
//! via `NotificationCenter`; the agent backend listens to re-pick its client).
//! In DEBUG builds, `load` honors the `ANTHROPIC_API_KEY` env var first, matching
//! `AnthropicKeychain.load()`.
//!
//! Network/OS access is behind the [`KeyStore`] trait so the round-trip is testable
//! with [`InMemoryKeyStore`] (no real Credential Manager / Secret Service needed).

use crate::error::AuthError;

/// Keyring service name (shared across all stored secrets for this app).
pub const KEYRING_SERVICE: &str = "palmier-pro";

/// Keyring account name for the Anthropic API key (ruling #5: `anthropic-api-key`).
pub const ANTHROPIC_KEY_ACCOUNT: &str = "anthropic-api-key";

/// Emitted by [`AnthropicKeyStore`] when the stored key changes (save or delete),
/// porting the reference `anthropicAPIKeyChanged` notification. The agent backend
/// re-selects its client (BYOK vs proxied) on this signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyChange {
    /// A key was saved (created or updated).
    Saved,
    /// A key was deleted.
    Deleted,
}

/// Abstraction over the OS keyring so the state machine is testable without a live
/// Credential Manager / Secret Service. Implemented by [`OsKeyStore`] (real) and
/// [`InMemoryKeyStore`] (tests).
pub trait KeyStore: Send + Sync {
    /// Store `value` under `(service, account)`, creating or overwriting.
    fn set(&self, service: &str, account: &str, value: &str) -> Result<(), AuthError>;
    /// Load the value under `(service, account)`. `Ok(None)` if absent.
    fn get(&self, service: &str, account: &str) -> Result<Option<String>, AuthError>;
    /// Delete the entry under `(service, account)`. Deleting a missing entry is Ok.
    fn delete(&self, service: &str, account: &str) -> Result<(), AuthError>;
}

/// Real OS keyring backend (`keyring` crate). Windows Credential Manager on Windows,
/// Secret Service / libsecret on Linux.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyStore;

impl KeyStore for OsKeyStore {
    fn set(&self, service: &str, account: &str, value: &str) -> Result<(), AuthError> {
        let entry =
            keyring::Entry::new(service, account).map_err(|e| AuthError::Keyring(e.to_string()))?;
        entry
            .set_password(value)
            .map_err(|e| AuthError::Keyring(e.to_string()))
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>, AuthError> {
        let entry =
            keyring::Entry::new(service, account).map_err(|e| AuthError::Keyring(e.to_string()))?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AuthError::Keyring(e.to_string())),
        }
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), AuthError> {
        let entry =
            keyring::Entry::new(service, account).map_err(|e| AuthError::Keyring(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AuthError::Keyring(e.to_string())),
        }
    }
}

/// Anthropic-key façade over a [`KeyStore`]. Fixes the service/account to the
/// reference values and applies the trim-empty-⇒-absent and DEBUG-env semantics of
/// `AnthropicKeychain`. Generic over the backend so tests inject [`InMemoryKeyStore`].
pub struct AnthropicKeyStore<S: KeyStore = OsKeyStore> {
    store: S,
}

impl AnthropicKeyStore<OsKeyStore> {
    /// Backed by the real OS keyring.
    #[must_use]
    pub fn os() -> Self {
        Self {
            store: OsKeyStore,
        }
    }
}

impl<S: KeyStore> AnthropicKeyStore<S> {
    /// Wrap an arbitrary [`KeyStore`] backend (used by tests).
    pub fn with_store(store: S) -> Self {
        Self { store }
    }

    /// Save the Anthropic key under service `palmier-pro` / account
    /// `anthropic-api-key`. Returns [`KeyChange::Saved`] so the caller can emit the
    /// `anthropic-api-key-changed` event.
    pub fn save(&self, key: &str) -> Result<KeyChange, AuthError> {
        self.store
            .set(KEYRING_SERVICE, ANTHROPIC_KEY_ACCOUNT, key)?;
        Ok(KeyChange::Saved)
    }

    /// Load the Anthropic key. In DEBUG builds the `ANTHROPIC_API_KEY` env var wins
    /// (parity with `AnthropicKeychain.load()`); otherwise reads the keyring.
    /// Empty/whitespace stored values are treated as absent (reference trims + drops
    /// empties).
    pub fn load(&self) -> Result<Option<String>, AuthError> {
        #[cfg(debug_assertions)]
        {
            if let Ok(env) = std::env::var("ANTHROPIC_API_KEY") {
                let trimmed = env.trim();
                if !trimmed.is_empty() {
                    return Ok(Some(trimmed.to_string()));
                }
            }
        }
        let raw = self.store.get(KEYRING_SERVICE, ANTHROPIC_KEY_ACCOUNT)?;
        Ok(raw.and_then(|v| {
            let t = v.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        }))
    }

    /// Delete the stored Anthropic key. Returns [`KeyChange::Deleted`].
    pub fn delete(&self) -> Result<KeyChange, AuthError> {
        self.store
            .delete(KEYRING_SERVICE, ANTHROPIC_KEY_ACCOUNT)?;
        Ok(KeyChange::Deleted)
    }

    /// Whether a non-empty key is available (drives BYOK client selection in
    /// `palmier-agent`). Defers to [`Self::load`], so the DEBUG `ANTHROPIC_API_KEY`
    /// env override counts as "has key" — `load` is the single source of truth.
    pub fn has_key(&self) -> Result<bool, AuthError> {
        Ok(self.load()?.is_some())
    }
}

/// In-memory [`KeyStore`] for tests — a `(service, account) -> value` map. Proves the
/// save→load round-trip without touching the real OS keyring.
#[derive(Debug, Default)]
pub struct InMemoryKeyStore {
    map: std::sync::Mutex<std::collections::HashMap<(String, String), String>>,
}

impl InMemoryKeyStore {
    /// New empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl KeyStore for InMemoryKeyStore {
    fn set(&self, service: &str, account: &str, value: &str) -> Result<(), AuthError> {
        self.map
            .lock()
            .unwrap()
            .insert((service.to_string(), account.to_string()), value.to_string());
        Ok(())
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>, AuthError> {
        Ok(self
            .map
            .lock()
            .unwrap()
            .get(&(service.to_string(), account.to_string()))
            .cloned())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), AuthError> {
        self.map
            .lock()
            .unwrap()
            .remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_name_matches_ruling_5() {
        // A wrong name silently loses the saved key (ruling #5).
        assert_eq!(ANTHROPIC_KEY_ACCOUNT, "anthropic-api-key");
        assert_ne!(ANTHROPIC_KEY_ACCOUNT, "palmier-pro-anthropic-api-key");
        assert_eq!(KEYRING_SERVICE, "palmier-pro");
    }

    #[test]
    fn round_trip_save_load_delete() {
        let store = AnthropicKeyStore::with_store(InMemoryKeyStore::new());
        assert_eq!(store.load().unwrap(), None);
        assert!(!store.has_key().unwrap());

        assert_eq!(store.save("sk-ant-secret-key").unwrap(), KeyChange::Saved);
        // Save -> load returns the same key under account `anthropic-api-key`.
        assert_eq!(store.load().unwrap().as_deref(), Some("sk-ant-secret-key"));
        assert!(store.has_key().unwrap());

        assert_eq!(store.delete().unwrap(), KeyChange::Deleted);
        assert_eq!(store.load().unwrap(), None);
        assert!(!store.has_key().unwrap());
    }

    #[test]
    fn save_overwrites_existing_key() {
        let store = AnthropicKeyStore::with_store(InMemoryKeyStore::new());
        store.save("sk-ant-first").unwrap();
        store.save("sk-ant-second").unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some("sk-ant-second"));
    }

    #[test]
    fn stored_under_exact_reference_account() {
        // Independently confirm the underlying backend key is (palmier-pro, anthropic-api-key).
        let backend = InMemoryKeyStore::new();
        backend
            .set(KEYRING_SERVICE, ANTHROPIC_KEY_ACCOUNT, "sk-ant-x")
            .unwrap();
        let store = AnthropicKeyStore::with_store(backend);
        assert_eq!(store.load().unwrap().as_deref(), Some("sk-ant-x"));
    }

    #[test]
    fn whitespace_only_stored_value_reads_as_absent() {
        let backend = InMemoryKeyStore::new();
        backend
            .set(KEYRING_SERVICE, ANTHROPIC_KEY_ACCOUNT, "   ")
            .unwrap();
        let store = AnthropicKeyStore::with_store(backend);
        assert_eq!(store.load().unwrap(), None);
    }

    #[test]
    fn deleting_absent_key_is_ok() {
        let store = AnthropicKeyStore::with_store(InMemoryKeyStore::new());
        assert_eq!(store.delete().unwrap(), KeyChange::Deleted);
    }
}
