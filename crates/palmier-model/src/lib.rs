//! # palmier-model
//!
//! Core data model: `Timeline`, `Track`, `Clip`, `Keyframe`, `MediaAsset` and the
//! pure sampling / geometry / computed-property math over them (FOUNDATION §4, §5).
//!
//! This crate is the data foundation every later epic consumes. It holds only pure
//! serde shapes and pure functions — no filesystem, no async, no GPU.
//!
//! ## Parity authority
//!
//! The macOS reference (`../palmier-pro/Sources/PalmierPro/`) is the parity
//! authority (docs/phase0-reconciliation.md). Where FOUNDATION and the reference
//! disagree, the reference wins; each such case is cited inline by its reconciliation
//! ruling number. Wire (serde) representations mirror the reference's Swift `Codable`
//! encodings so reference- / Convex-authored projects round-trip byte-identically.
//!
//! Story E2-S1 lands the leaf enums every other shape depends on: [`ClipType`],
//! [`Interpolation`], and [`AnimatableProperty`], plus the [`smoothstep`] / [`lerp`]
//! helpers and the [`KeyframeInterpolatable`] trait. Later Epic 2 stories add
//! `Transform`/`Crop` (E2-S2), keyframes + sampling (E2-S3), `VolumeScale` (E2-S4),
//! and the `Timeline`/`Track`/`Clip` shapes.

mod animatable_property;
mod clip_type;
mod interpolation;

pub use animatable_property::AnimatableProperty;
pub use clip_type::ClipType;
pub use interpolation::{lerp, smoothstep, Interpolation, KeyframeInterpolatable};

/// Crate marker retained for the workspace skeleton smoke tests
/// (`palmier-tauri`, `palmier-project` reference it). Real model types are the
/// re-exports above.
pub const CRATE_NAME: &str = "palmier-model";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_is_set() {
        assert_eq!(CRATE_NAME, "palmier-model");
    }
}
