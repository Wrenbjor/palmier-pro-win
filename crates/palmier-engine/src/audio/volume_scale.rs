//! `VolumeScale` — dB ↔ linear-amplitude conversion for the audio mixer.
//!
//! Verbatim port of the macOS reference `VolumeScale`
//! (`Sources/PalmierPro/Inspector/InspectorView.swift`, lines ~1072–1085).
//!
//! ## Reconciliation ruling #9 (docs/phase0-reconciliation.md)
//!
//! The dB range is **−60…+15** (amplification above 0 dB is allowed), NOT the
//! −120…0 that FOUNDATION §5.3/§5.5 claims. The mixer must accept >0 dB gain. The
//! Inspector field/scale that surfaces this lives in Epic 12; this is just the math
//! the mixer applies. Below the floor we snap to **true 0** (hard mute), exactly as
//! the reference `linearFromDb` does (`guard db > floorDb else { return 0 }`).

/// Floor of the dB scale. At or below this, linear gain is hard-muted to `0.0`.
pub const FLOOR_DB: f64 = -60.0;

/// Ceiling of the dB scale (+15 dB amplification headroom, ruling #9).
pub const CEILING_DB: f64 = 15.0;

/// Map a linear amplitude multiplier to dB (clamped to `[FLOOR_DB, CEILING_DB]`).
///
/// Mirrors `VolumeScale.dbFromLinear`. `linear <= 0` returns `FLOOR_DB`.
#[inline]
pub fn db_from_linear(linear: f64) -> f64 {
    if linear > 0.0 {
        (20.0 * linear.log10()).clamp(FLOOR_DB, CEILING_DB)
    } else {
        FLOOR_DB
    }
}

/// Map dB to a linear amplitude multiplier.
///
/// Mirrors `VolumeScale.linearFromDb`. **Below the floor returns `0.0`** (hard mute,
/// not a tiny epsilon) so a keyframe parked at `-∞` produces true silence. dB above
/// the ceiling is clamped to `CEILING_DB` first.
#[inline]
pub fn linear_from_db(db: f64) -> f64 {
    if db > FLOOR_DB {
        10f64.powf(db.min(CEILING_DB) / 20.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_round_trips() {
        // 0 dB == linear 1.0
        assert!((linear_from_db(0.0) - 1.0).abs() < 1e-12);
        assert!((db_from_linear(1.0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn floor_is_hard_mute() {
        // Ruling #9: at/below the floor, gain snaps to true 0.
        assert_eq!(linear_from_db(FLOOR_DB), 0.0);
        assert_eq!(linear_from_db(FLOOR_DB - 10.0), 0.0);
        assert_eq!(linear_from_db(-1000.0), 0.0);
    }

    #[test]
    fn amplification_allowed_above_unity() {
        // Ruling #9: >0 dB must amplify (reference allows up to +15 dB).
        assert!(linear_from_db(6.0) > 1.0);
        // +15 dB ~= 5.623x
        assert!((linear_from_db(15.0) - 5.623_413_25).abs() < 1e-6);
        // Clamped above the ceiling.
        assert_eq!(linear_from_db(100.0), linear_from_db(CEILING_DB));
    }

    #[test]
    fn db_from_linear_clamps_to_range() {
        assert_eq!(db_from_linear(0.0), FLOOR_DB);
        assert_eq!(db_from_linear(-1.0), FLOOR_DB);
        assert_eq!(db_from_linear(1e9), CEILING_DB);
    }

    #[test]
    fn half_amplitude_is_about_minus_six_db() {
        // 20*log10(0.5) ~= -6.0206 dB
        assert!((db_from_linear(0.5) - (-6.020_599_91)).abs() < 1e-6);
    }
}
