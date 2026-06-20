//! Source↔timeline frame rounding — the **one** rounding convention every engine
//! shares.
//!
//! Every source↔timeline conversion in the reference is Swift
//! `(x).rounded()`, whose default rule is `.toNearestOrAwayFromZero` — i.e.
//! **ties round away from zero** (`0.5 → 1`, `-0.5 → -1`, `2.5 → 3`). Rust's
//! [`f64::round`] has *exactly* this semantics, so this is a thin, intention-
//! revealing wrapper rather than a reimplementation.
//!
//! Reconciliation carry-forward (#12 frame rounding / docs/phase0-reconciliation.md
//! line 55, edit-engines.md lines 20-21): use `f64::round`, **never**
//! `round_ties_even` (banker's rounding). A mismatched rule drifts trims by ±1
//! frame and breaks split/trim round-trips (edit-engines.md lines 220-222).

/// Round `value` to the nearest integer, ties **away from zero**, as `i32`.
///
/// Matches Swift `Double.rounded()` (`.toNearestOrAwayFromZero`). This is the
/// only rounding any engine here uses for `round(Δ·speed)` / `round(Δ/speed)`.
#[inline]
pub fn round_ties_away(value: f64) -> i32 {
    // f64::round is ties-away-from-zero by definition — the exact rule we want.
    value.round() as i32
}

#[cfg(test)]
mod tests {
    use super::round_ties_away;

    #[test]
    fn ties_round_away_from_zero_not_to_even() {
        // The distinguishing cases vs banker's rounding (round_ties_even):
        assert_eq!(round_ties_away(0.5), 1, "0.5 → 1 (away), not 0 (even)");
        assert_eq!(round_ties_away(1.5), 2);
        assert_eq!(round_ties_away(2.5), 3, "2.5 → 3 (away), not 2 (even)");
        assert_eq!(round_ties_away(-0.5), -1, "-0.5 → -1 (away), not 0");
        assert_eq!(round_ties_away(-2.5), -3, "-2.5 → -3 (away), not -2");
    }

    #[test]
    fn non_tie_values_round_normally() {
        assert_eq!(round_ties_away(0.4), 0);
        assert_eq!(round_ties_away(0.6), 1);
        assert_eq!(round_ties_away(-0.6), -1);
        assert_eq!(round_ties_away(3.0), 3);
    }
}
