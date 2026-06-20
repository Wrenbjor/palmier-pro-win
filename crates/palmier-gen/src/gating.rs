//! Credit gating — the **advisory** `can_generate` + affordability math (E9-S8;
//! reference `GenerationView` cost gating). The Convex `generations:submit`
//! mutation is the **real** gate (ruling #24 / FR-34); never treat the client
//! gate as authoritative.
//!
//! Reads budget/credit state from `palmier-auth`'s [`AccountState`]:
//! - `remaining_credits = max(0, budget_credits - spent_credits)` where
//!   `budget_credits = plan.monthlyBudgetCredits + user.purchasedCredits`,
//!   `spent_credits = user.spentCreditsThisPeriod`.
//! - `can_generate = signed_in && tier_allows && has_remaining_credits` —
//!   advisory only.

use palmier_auth::AccountState;

/// The advisory generation gate (reference `canGenerate`). True only when signed
/// in, AI-allowed (configured), and there are remaining credits. **Advisory** —
/// the server mutation is the real gate.
#[must_use]
pub fn can_generate(account: &AccountState) -> bool {
    account.is_signed_in() && account.ai_allowed() && account.has_credits()
}

/// Why generation is blocked, for the UI/tool messaging (FR-34).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateBlock {
    /// Signed out (or misconfigured) — "Sign in".
    SignedOut,
    /// Signed in but no remaining credits — "Out of credits".
    OutOfCredits,
}

impl GateBlock {
    /// The reference-style block reason for a non-`can_generate` account, or
    /// `None` when generation is allowed.
    #[must_use]
    pub fn for_account(account: &AccountState) -> Option<GateBlock> {
        if !account.is_signed_in() || !account.ai_allowed() {
            Some(GateBlock::SignedOut)
        } else if !account.has_credits() {
            Some(GateBlock::OutOfCredits)
        } else {
            None
        }
    }

    /// A human-readable message (reference "Sign in" / "Out of credits").
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            GateBlock::SignedOut => {
                "Sign in to Palmier and subscribe to generate media."
            }
            GateBlock::OutOfCredits => "Out of credits — top up to keep generating.",
        }
    }
}

/// Whether an estimated cost is affordable against the account's remaining
/// credits (reference `canAffordGeneration`):
/// - budget unknown → `true` (advisory; server decides);
/// - else `estimated_cost <= remaining` (or `remaining > 0` when cost unknown).
#[must_use]
pub fn can_afford(account: &AccountState, estimated_cost: Option<i64>) -> bool {
    // Budget unknown (no account snapshot yet) → advisory true.
    let Some(_budget) = account.budget_credits() else {
        return true;
    };
    let remaining = account.remaining_credits();
    match estimated_cost {
        Some(cost) => cost <= remaining,
        None => remaining > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_auth::{
        AccountPlan, AccountResponse, AccountTier, AccountUser, AuthState,
    };

    fn signed_in(budget: i64, purchased: i64, spent: i64) -> AccountState {
        let mut s = AccountState::new(false);
        s.set_auth_state(AuthState::Authenticated);
        s.set_account(AccountResponse {
            user: AccountUser {
                tier: AccountTier::Pro,
                spent_credits_this_period: Some(spent),
                purchased_credits: Some(purchased),
                ..Default::default()
            },
            plan: Some(AccountPlan {
                tier: AccountTier::Pro,
                monthly_price_usd: 20,
                monthly_budget_credits: Some(budget),
            }),
        });
        s
    }

    #[test]
    fn gate_truth_table() {
        // Signed out → blocked (Sign in).
        let out = AccountState::new(false);
        assert!(!can_generate(&out));
        assert_eq!(GateBlock::for_account(&out), Some(GateBlock::SignedOut));

        // Signed in, no credits → blocked (Out of credits).
        let broke = signed_in(100, 0, 100);
        assert!(!can_generate(&broke));
        assert_eq!(GateBlock::for_account(&broke), Some(GateBlock::OutOfCredits));

        // Signed in with credits → allowed.
        let rich = signed_in(1000, 0, 100);
        assert!(can_generate(&rich));
        assert_eq!(GateBlock::for_account(&rich), None);
    }

    #[test]
    fn affordability_unknown_budget_is_true() {
        // No account snapshot → budget unknown → advisory true.
        let loading = AccountState::new(false);
        assert!(can_afford(&loading, Some(999_999)));
    }

    #[test]
    fn affordability_compares_cost_to_remaining() {
        let acct = signed_in(1000, 0, 100); // remaining = 900
        assert!(can_afford(&acct, Some(900)));
        assert!(can_afford(&acct, Some(500)));
        assert!(!can_afford(&acct, Some(901)));
        // unknown cost but credits remain → true.
        assert!(can_afford(&acct, None));
    }
}
