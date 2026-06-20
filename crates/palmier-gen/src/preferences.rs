//! `ModelPreferences` — user-disabled model ids (E9-S8; reference
//! `Catalog/ModelPreferences.swift`). The macOS `UserDefaults` key
//! `disabledModelIds` becomes a settings-store JSON value; here we own the
//! in-memory set + the serde shape the settings layer persists. Disabled models
//! are hidden from the form / `list_models`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The settings key the disabled-model ids persist under (reference
/// `defaultsKey = "disabledModelIds"`).
pub const DISABLED_MODEL_IDS_KEY: &str = "disabledModelIds";

/// User model preferences (reference `ModelPreferences`). Holds the set of
/// disabled model ids; `is_enabled` is the form/`list_models` filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModelPreferences {
    disabled_ids: BTreeSet<String>,
}

impl ModelPreferences {
    /// Empty preferences (nothing disabled).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from a persisted id list (settings store).
    #[must_use]
    pub fn from_ids(ids: impl IntoIterator<Item = String>) -> Self {
        Self {
            disabled_ids: ids.into_iter().collect(),
        }
    }

    /// Whether a model is enabled (reference `isEnabled`).
    #[must_use]
    pub fn is_enabled(&self, id: &str) -> bool {
        !self.disabled_ids.contains(id)
    }

    /// Enable/disable a model (reference `setEnabled`).
    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if enabled {
            self.disabled_ids.remove(id);
        } else {
            self.disabled_ids.insert(id.to_string());
        }
    }

    /// The disabled ids (for persistence).
    #[must_use]
    pub fn disabled_ids(&self) -> &BTreeSet<String> {
        &self.disabled_ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_by_default_and_toggles() {
        let mut p = ModelPreferences::new();
        assert!(p.is_enabled("veo"));
        p.set_enabled("veo", false);
        assert!(!p.is_enabled("veo"));
        p.set_enabled("veo", true);
        assert!(p.is_enabled("veo"));
    }

    #[test]
    fn round_trips_through_settings_json() {
        let p = ModelPreferences::from_ids(["a".into(), "b".into()]);
        let json = serde_json::to_string(&p).unwrap();
        // Serializes transparently as a JSON array of ids.
        assert!(json.contains("\"a\"") && json.contains("\"b\""));
        let back: ModelPreferences = serde_json::from_str(&json).unwrap();
        assert!(!back.is_enabled("a"));
        assert!(back.is_enabled("c"));
    }
}
