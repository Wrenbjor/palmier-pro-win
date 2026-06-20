//! Model availability + effective-model selection.
//!
//! Ports `AgentService.availableModels` / `effectiveModel` (`agent-panel.md`
//! lines 53-54, FR-32, reconciliation ruling #20).
//!
//! ## Tiering
//! - **BYOK** (an Anthropic key is present): all three models.
//! - **Signed-in, paid**: **catalog-driven**, default Sonnet 4.6; the Convex
//!   catalog MAY enable Opus 4.8 (ruling #20 — NOT the reference's hard-coded
//!   `[sonnet46]`). The catalog list is injected; absent → just the default.
//! - **Signed-in, free**: Haiku 4.5 only.
//! - **No backend**: nothing available.
//!
//! ## Effective model
//! `effective_model = persisted model if in available list, else first available,
//! else Sonnet 4.6` (reference). Persisted under config key `"agentModel"`
//! ([`AGENT_MODEL_CONFIG_KEY`]).

use crate::event::AnthropicModel;

/// App-config key the picked model persists under (reference `UserDefaults`
/// `"agentModel"` → app config store; `agent-panel.md` lines 54, 175).
pub const AGENT_MODEL_CONFIG_KEY: &str = "agentModel";

/// The default paid/fallback model when nothing else applies (reference
/// `effectiveModel` final fallback + paid default).
pub const DEFAULT_MODEL: AnthropicModel = AnthropicModel::Sonnet46;

/// The backend kind selected for the agent (see [`crate::client`]).
///
/// `availableModels` keys off this + the paid flag + the (optional) Convex model
/// catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tier {
    /// BYOK: an Anthropic key is present → all three models.
    Byok,
    /// Signed-in. `paid` gates Sonnet (+ catalog) vs Haiku-only.
    SignedIn {
        /// Whether the user is on a paid tier (reference `isPaid`).
        paid: bool,
        /// Catalog-enabled paid models from Convex (ruling #20). Empty → only the
        /// default Sonnet for paid. Ignored for free.
        catalog: Vec<AnthropicModel>,
    },
    /// No backend (no key, not signed in) → no models available.
    None,
}

/// The list of models the user may choose, given their [`Tier`] (reference
/// `availableModels`).
///
/// - BYOK → all three, in reference order.
/// - Signed-in paid → `[Sonnet46]` unioned with any catalog models (deduped,
///   Sonnet-first), per ruling #20.
/// - Signed-in free → `[Haiku45]`.
/// - None → empty.
#[must_use]
pub fn available_models(tier: &Tier) -> Vec<AnthropicModel> {
    match tier {
        Tier::Byok => AnthropicModel::ALL.to_vec(),
        Tier::SignedIn { paid: true, catalog } => {
            // Default Sonnet first, then any catalog-enabled extras (e.g. Opus),
            // de-duplicated, preserving catalog order for the extras.
            let mut models = vec![DEFAULT_MODEL];
            for &m in catalog {
                if !models.contains(&m) {
                    models.push(m);
                }
            }
            models
        }
        Tier::SignedIn { paid: false, .. } => vec![AnthropicModel::Haiku45],
        Tier::None => Vec::new(),
    }
}

/// The model the agent will actually use (reference `effectiveModel`).
///
/// `persisted` if it is in `available`, else the first available model, else
/// [`DEFAULT_MODEL`] (when nothing is available).
#[must_use]
pub fn effective_model(persisted: Option<AnthropicModel>, available: &[AnthropicModel]) -> AnthropicModel {
    if let Some(p) = persisted
        && available.contains(&p)
    {
        return p;
    }
    available.first().copied().unwrap_or(DEFAULT_MODEL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byok_has_all_three() {
        assert_eq!(available_models(&Tier::Byok), AnthropicModel::ALL.to_vec());
    }

    #[test]
    fn signed_paid_default_is_sonnet_only_without_catalog() {
        let models = available_models(&Tier::SignedIn {
            paid: true,
            catalog: vec![],
        });
        assert_eq!(models, vec![AnthropicModel::Sonnet46]);
    }

    #[test]
    fn signed_paid_catalog_can_enable_opus_ruling_20() {
        // Ruling #20: paid is catalog-driven (NOT hard-coded [sonnet46]).
        let models = available_models(&Tier::SignedIn {
            paid: true,
            catalog: vec![AnthropicModel::Opus48],
        });
        assert_eq!(models, vec![AnthropicModel::Sonnet46, AnthropicModel::Opus48]);
    }

    #[test]
    fn signed_paid_catalog_dedupes_default() {
        let models = available_models(&Tier::SignedIn {
            paid: true,
            catalog: vec![AnthropicModel::Sonnet46, AnthropicModel::Opus48],
        });
        // Sonnet not duplicated.
        assert_eq!(models, vec![AnthropicModel::Sonnet46, AnthropicModel::Opus48]);
    }

    #[test]
    fn signed_free_is_haiku_only() {
        let models = available_models(&Tier::SignedIn {
            paid: false,
            catalog: vec![AnthropicModel::Opus48], // ignored for free
        });
        assert_eq!(models, vec![AnthropicModel::Haiku45]);
    }

    #[test]
    fn none_has_no_models() {
        assert!(available_models(&Tier::None).is_empty());
    }

    #[test]
    fn effective_model_keeps_persisted_when_available() {
        let avail = available_models(&Tier::Byok);
        assert_eq!(
            effective_model(Some(AnthropicModel::Opus48), &avail),
            AnthropicModel::Opus48
        );
    }

    #[test]
    fn effective_model_falls_back_to_first_available() {
        // Persisted Opus, but only Haiku available (free) → first available.
        let avail = available_models(&Tier::SignedIn {
            paid: false,
            catalog: vec![],
        });
        assert_eq!(
            effective_model(Some(AnthropicModel::Opus48), &avail),
            AnthropicModel::Haiku45
        );
    }

    #[test]
    fn effective_model_defaults_when_nothing_available() {
        assert_eq!(effective_model(None, &[]), DEFAULT_MODEL);
        assert_eq!(
            effective_model(Some(AnthropicModel::Haiku45), &[]),
            DEFAULT_MODEL
        );
    }
}
