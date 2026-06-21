//! `VolumeScale` — linear ↔ dB conversions and the distinct dB ranges.
//!
//! Ported 1:1 from the macOS reference `Inspector/InspectorView.swift:1072`
//! (`enum VolumeScale`). See docs/reference/timeline-model.md "VolumeScale" +
//! Port risks "Volume dB floor is -60".
//!
//! ## Reconciliation ruling #9 (docs/phase0-reconciliation.md)
//!
//! The editing dB range is **−60 … +15** (NOT FOUNDATION §5.3/§5.5's −120…0). The
//! ceiling of +15 dB allows amplification (>0 dB gain). The reference is the parity
//! authority, so we use −60 / +15.
//!
//! ### The three distinct dB constant pairs (ruling #9 — keep them SEPARATE)
//!
//! The reference uses three different dB ranges for three different purposes. They
//! must NOT be collapsed into one constant:
//!
//! 1. **`VolumeScale` editing range** — `FLOOR_DB = −60`, `CEILING_DB = +15`
//!    (this module). The inspector volume field + linear/dB math.
//! 2. **Rubber-band DRAW axis** — `RUBBER_BAND_TOP_DB = +6`, `RUBBER_BAND_BOTTOM_DB
//!    = −60` (reference `ClipRenderer.volumeRubberBand{Top,Bottom}Db`). The on-canvas
//!    audio rubber-band's vertical axis; consumed by Epic 3 timeline rendering.
//! 3. **Keyframe-storage dB floor** — **CONFIRMED = −60** (E12-S1). Volume keyframe
//!    values are stored in dB (`Clip.volume_track`). The reference applies **no
//!    separate storage-level clamp**: the only floor on a stored volume-keyframe dB
//!    value is the inspector field's editing range `[−60, +15]`
//!    (`ScrubbableNumberField range: VolumeScale.floorDb...ceilingDb`). The write
//!    path (`writeVolume` → `upsertKeyframe(in: \.volumeTrack, value: valueDb)`) and
//!    `KeyframeTrack` itself do not clamp. Therefore the effective storage floor is
//!    [`FLOOR_DB`] (−60), and FOUNDATION §5.3/§5.5's claim of a **−120** storage
//!    floor is **incorrect** — do NOT adopt −120, and do NOT silently set a 0
//!    ceiling. See [`KEYFRAME_STORAGE_FLOOR_DB`] below.
//!
//! ### E12-S1 decision (ruling #9)
//!
//! Field clamps to **[−60, +15]**; the keyframe-storage floor is **confirmed −60**
//! (no distinct −120 floor exists in the reference). This [`VolumeScale`] is the
//! **single source of truth** consumed by both the Inspector volume field (E12-S5)
//! and the keyframe lane (E12-S8); neither may re-derive these literals.
//!
//! (A fourth, unrelated normalization constant `dbRange = 50` exists in the
//! reference waveform renderer `ClipRenderer.drawWaveform`; it normalizes sample
//! loudness for drawing and is not a volume scale — it belongs to Epic 3 rendering,
//! not this module.)

/// Linear ↔ dB conversions for clip volume (reference `enum VolumeScale`).
///
/// Volume keyframe values are stored in **dB**; the static `Clip.volume` is
/// **linear**. This type bridges the two.
pub struct VolumeScale;

impl VolumeScale {
    /// Editing-range floor (ruling #9): below this, audio is hard-muted.
    pub const FLOOR_DB: f64 = -60.0;
    /// Editing-range ceiling (ruling #9): +15 dB allows amplification.
    pub const CEILING_DB: f64 = 15.0;

    /// Rubber-band DRAW axis top (reference
    /// `ClipRenderer.volumeRubberBandTopDb`). Constant #2 of three — the on-canvas
    /// audio rubber-band's vertical axis, consumed by Epic 3 rendering. Distinct
    /// from [`CEILING_DB`]; do NOT collapse them.
    pub const RUBBER_BAND_TOP_DB: f64 = 6.0;
    /// Rubber-band DRAW axis bottom (reference
    /// `ClipRenderer.volumeRubberBandBottomDb`).
    pub const RUBBER_BAND_BOTTOM_DB: f64 = -60.0;

    /// Keyframe-storage dB floor — constant #3 of three. **CONFIRMED = [`FLOOR_DB`]
    /// (−60) by E12-S1.**
    ///
    /// The reference stores volume keyframes as raw `Double` dB with **no
    /// storage-level clamp** (`writeVolume` → `upsertKeyframe(value: valueDb)`;
    /// `KeyframeTrack<Double>` does not clamp). The only floor applied to a value
    /// that can enter the track is the inspector field's editing range
    /// `[FLOOR_DB, CEILING_DB]` = `[−60, +15]`. Hence the effective storage floor is
    /// −60 — there is **no** −120 floor (FOUNDATION §5.3/§5.5's −120 is wrong) and
    /// **no** 0 ceiling. Equals [`FLOOR_DB`] by design (single source of truth), not
    /// coincidence.
    pub const KEYFRAME_STORAGE_FLOOR_DB: f64 = Self::FLOOR_DB;

    /// dB from a linear amplitude (reference `dbFromLinear`):
    /// `linear > 0 ? clamp(20·log10(linear), FLOOR, CEILING) : FLOOR`.
    ///
    /// A non-positive linear value (silence / mute) maps to the floor.
    pub fn db_from_linear(linear: f64) -> f64 {
        if linear > 0.0 {
            (20.0 * linear.log10()).clamp(Self::FLOOR_DB, Self::CEILING_DB)
        } else {
            Self::FLOOR_DB
        }
    }

    /// Linear amplitude from dB (reference `linearFromDb`):
    /// `db > FLOOR ? 10^(min(db, CEILING)/20) : 0`.
    ///
    /// At or below the floor, returns `0.0` (hard mute). The reference clamps `db`
    /// to the ceiling *inside* the power (`min(db, ceilingDb)`) — we replicate that
    /// exactly so a stored value above +15 dB does not over-amplify.
    pub fn linear_from_db(db: f64) -> f64 {
        if db > Self::FLOOR_DB {
            10f64.powf(db.min(Self::CEILING_DB) / 20.0)
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_gain_is_zero_db() {
        assert_eq!(VolumeScale::db_from_linear(1.0), 0.0);
        assert_eq!(VolumeScale::linear_from_db(0.0), 1.0);
    }

    #[test]
    fn hard_mute_below_floor() {
        // linear 0 (or negative) → floor dB.
        assert_eq!(VolumeScale::db_from_linear(0.0), VolumeScale::FLOOR_DB);
        assert_eq!(VolumeScale::db_from_linear(-1.0), VolumeScale::FLOOR_DB);
        // db at/below floor → linear 0 (hard mute).
        assert_eq!(VolumeScale::linear_from_db(-60.0), 0.0);
        assert_eq!(VolumeScale::linear_from_db(-120.0), 0.0);
        // Strictly above floor is NOT muted.
        assert!(VolumeScale::linear_from_db(-59.9) > 0.0);
    }

    #[test]
    fn ceiling_clamps_db_from_linear() {
        // 20·log10(10) = 20 dB, which clamps to the +15 ceiling.
        assert_eq!(VolumeScale::db_from_linear(10.0), VolumeScale::CEILING_DB);
        // A very loud linear value still clamps to +15.
        assert_eq!(VolumeScale::db_from_linear(1000.0), VolumeScale::CEILING_DB);
    }

    #[test]
    fn floor_clamps_db_from_linear() {
        // A tiny positive linear value (−∞ dB mathematically) clamps to the floor.
        assert_eq!(VolumeScale::db_from_linear(1e-12), VolumeScale::FLOOR_DB);
    }

    #[test]
    fn linear_from_db_clamps_above_ceiling_inside_power() {
        // Reference applies min(db, ceiling) before pow: a stored +30 dB behaves
        // like +15 dB, not 10^1.5.
        let at_ceiling = VolumeScale::linear_from_db(15.0);
        let above_ceiling = VolumeScale::linear_from_db(30.0);
        assert_eq!(at_ceiling, above_ceiling);
        // +15 dB = 10^0.75 ≈ 5.623.
        assert!((at_ceiling - 10f64.powf(0.75)).abs() < 1e-12);
    }

    #[test]
    fn round_trip_linear_db_linear_on_unit_interval() {
        // For x in (0, 1] whose dB is STRICTLY above the −60 floor,
        // linear_from_db(db_from_linear(x)) ≈ x. (x=0.001 → exactly −60 dB, which
        // the hard-mute floor maps back to 0 — that boundary is asserted
        // separately in `round_trip_hits_floor_at_exactly_minus_60`.)
        // 0.01 → 20·log10(0.01) = −40 dB, well above the floor.
        for &x in &[1.0, 0.5, 0.25, 0.1, 0.01, 0.005] {
            let round = VolumeScale::linear_from_db(VolumeScale::db_from_linear(x));
            assert!(
                (round - x).abs() < 1e-9,
                "round-trip failed for x={x}: got {round}"
            );
        }
    }

    #[test]
    fn round_trip_hits_floor_at_exactly_minus_60() {
        // linear 0.001 → 20·log10(0.001) = exactly −60 dB (the floor). Because
        // linear_from_db hard-mutes at/below the floor (`db > FLOOR_DB` is false at
        // exactly −60), the value does NOT round-trip — it collapses to 0. This is
        // faithful reference behavior, not a bug: the floor is a hard mute.
        assert_eq!(VolumeScale::db_from_linear(0.001), VolumeScale::FLOOR_DB);
        assert_eq!(VolumeScale::linear_from_db(VolumeScale::FLOOR_DB), 0.0);
    }

    // ---- E12-S1 acceptance-criteria assertions ----

    #[test]
    fn e12_s1_mute_path_floor_and_true_mute() {
        // dbFromLinear(0.0) == floor → the "−∞ dB" render path; stores linear 0.0
        // (true mute) on the inverse.
        assert_eq!(VolumeScale::db_from_linear(0.0), VolumeScale::FLOOR_DB);
        assert_eq!(VolumeScale::linear_from_db(VolumeScale::FLOOR_DB), 0.0);
    }

    #[test]
    fn e12_s1_unity_is_zero_db() {
        // dbFromLinear(1.0) == 0.0 dB.
        assert_eq!(VolumeScale::db_from_linear(1.0), 0.0);
    }

    #[test]
    fn e12_s1_amplification_above_unity_is_representable() {
        // linearFromDb(15.0) == 10^(15/20) — amplification > 0 dB is representable;
        // it is NOT clamped down to 1.0.
        let expected = 10f64.powf(15.0 / 20.0);
        assert_eq!(VolumeScale::linear_from_db(15.0), expected);
        assert!(expected > 1.0, "ceiling gain must amplify (>1.0 linear)");
    }

    #[test]
    fn e12_s1_keyframe_storage_floor_is_minus_60_not_minus_120() {
        // E12-S1 decision: the keyframe-storage floor equals the editing floor
        // (−60). There is NO distinct −120 storage floor (FOUNDATION §5.3/§5.5 is
        // wrong) and NO 0 ceiling.
        assert_eq!(VolumeScale::KEYFRAME_STORAGE_FLOOR_DB, -60.0);
        assert_eq!(VolumeScale::KEYFRAME_STORAGE_FLOOR_DB, VolumeScale::FLOOR_DB);
        assert_ne!(VolumeScale::KEYFRAME_STORAGE_FLOOR_DB, -120.0);
    }

    #[test]
    fn three_distinct_db_constant_pairs_are_separate() {
        // ruling #9: the three pairs must not be collapsed.
        // 1) VolumeScale editing range.
        assert_eq!(VolumeScale::FLOOR_DB, -60.0);
        assert_eq!(VolumeScale::CEILING_DB, 15.0);
        // 2) Rubber-band draw axis (distinct top of +6).
        assert_eq!(VolumeScale::RUBBER_BAND_TOP_DB, 6.0);
        assert_eq!(VolumeScale::RUBBER_BAND_BOTTOM_DB, -60.0);
        // The rubber-band top differs from the editing ceiling.
        assert_ne!(VolumeScale::RUBBER_BAND_TOP_DB, VolumeScale::CEILING_DB);
    }
}
