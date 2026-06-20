//! `Interpolation`, `smoothstep`, `lerp`, and the `KeyframeInterpolatable` trait.
//!
//! Ported from the macOS reference `Sources/PalmierPro/Models/Keyframe.swift`.
//! See docs/reference/timeline-model.md "Keyframes & sampling".
//!
//! Wire representation: the Swift enum is `enum Interpolation: String` with
//! lowercase bare cases (`linear`, `hold`, `smooth`). We mirror that with serde
//! `rename_all = "lowercase"`.

use serde::{Deserialize, Serialize};

/// How a keyframe segment interpolates from this keyframe to the next.
///
/// Reconciliation ruling #8 (docs/phase0-reconciliation.md): **`Smooth` is the
/// default** — both the reference `Keyframe.interpolationOut = .smooth` and the
/// ruling override FOUNDATION §5.2/§5.5 which wrongly say "linear". `Default` and
/// the serde `default` attribute on `Keyframe::interpolation_out` (E2-S3) both
/// resolve to `Smooth` so a missing/absent field decodes to `Smooth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interpolation {
    Linear,
    Hold,
    Smooth,
}

impl Interpolation {
    /// All interpolation modes, in reference declaration order (Swift `CaseIterable`).
    pub const ALL: [Interpolation; 3] =
        [Interpolation::Linear, Interpolation::Hold, Interpolation::Smooth];
}

impl Default for Interpolation {
    /// Ruling #8: the default interpolation is `Smooth`, not `Linear`.
    fn default() -> Self {
        Interpolation::Smooth
    }
}

/// Hermite smoothstep easing: `t*t*(3 - 2*t)`.
///
/// Verbatim from the reference `smoothstep(_:)`. Defined on `[0, 1]`; the sampling
/// code (E2-S3) only ever feeds it a normalized `raw` in that range.
#[inline]
pub fn smoothstep(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

/// Linear interpolation `a + (b - a) * t`. Mirrors the reference
/// `Double.keyframeInterpolate(_:_:t:)`.
#[inline]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// A value that can be keyframe-interpolated between two endpoints.
///
/// Port of the Swift `KeyframeInterpolatable` protocol. This story provides the
/// `f64` (Swift `Double`) implementation as the stub; later stories add
/// `AnimPair` (position/scale) and `Crop` impls (E2-S2/E2-S3).
pub trait KeyframeInterpolatable: Sized {
    /// Interpolate from `a` to `b` by normalized factor `t` (`0` → `a`, `1` → `b`).
    fn keyframe_interpolate(a: Self, b: Self, t: f64) -> Self;
}

impl KeyframeInterpolatable for f64 {
    #[inline]
    fn keyframe_interpolate(a: f64, b: f64, t: f64) -> f64 {
        lerp(a, b, t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_each_variant() {
        let cases = [
            (Interpolation::Linear, "\"linear\""),
            (Interpolation::Hold, "\"hold\""),
            (Interpolation::Smooth, "\"smooth\""),
        ];
        for (variant, wire) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, wire, "wire encoding for {variant:?}");
            let back: Interpolation = serde_json::from_str(&json).unwrap();
            assert_eq!(back, variant, "round-trip for {variant:?}");
        }
    }

    #[test]
    fn default_is_smooth() {
        // Ruling #8: default interpolation is Smooth.
        assert_eq!(Interpolation::default(), Interpolation::Smooth);
    }

    /// Helper struct proving an absent serde field decodes to `Smooth` via the
    /// `#[serde(default)]` attribute (the exact pattern `Keyframe` uses in E2-S3).
    #[derive(Debug, Deserialize, PartialEq)]
    struct HasInterp {
        #[serde(default)]
        interpolation_out: Interpolation,
    }

    #[test]
    fn serde_default_absent_field_is_smooth() {
        // Ruling #8: when the field is absent from JSON, it must default to Smooth.
        let decoded: HasInterp = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded.interpolation_out, Interpolation::Smooth);

        // And an explicitly-present value still wins.
        let decoded: HasInterp =
            serde_json::from_str(r#"{"interpolation_out":"linear"}"#).unwrap();
        assert_eq!(decoded.interpolation_out, Interpolation::Linear);
    }

    #[test]
    fn smoothstep_boundaries_and_midpoint() {
        assert_eq!(smoothstep(0.0), 0.0);
        assert_eq!(smoothstep(1.0), 1.0);
        assert_eq!(smoothstep(0.5), 0.5);
    }

    #[test]
    fn lerp_and_f64_interpolatable() {
        assert_eq!(lerp(0.0, 10.0, 0.0), 0.0);
        assert_eq!(lerp(0.0, 10.0, 1.0), 10.0);
        assert_eq!(lerp(0.0, 10.0, 0.25), 2.5);
        // Trait stub for f64 delegates to lerp.
        assert_eq!(f64::keyframe_interpolate(2.0, 6.0, 0.5), 4.0);
    }
}
