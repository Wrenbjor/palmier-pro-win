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
//! helpers and the [`KeyframeInterpolatable`] trait. E2-S2 adds [`Transform`]
//! (center-based, ruling #7) + [`Crop`]; E2-S4 adds [`VolumeScale`] (linear↔dB,
//! ruling #9); E3-S1 adds the edit value types the engines consume
//! ([`FrameRange`], [`ClipShift`], [`GapSelection`], [`TimelineRangeSelection`])
//! and confirms [`ClipType::is_compatible`] (ruling #12).
//!
//! E2-S3 adds the keyframe storage + sampling ([`Keyframe`], [`KeyframeTrack`],
//! [`AnimPair`], the clip-relative↔absolute [`to_abs`]/[`to_offset`] seam);
//! E2-S5 adds the [`Clip`] core entity (all stored fields + the derived
//! `end_frame`/`source_frames_consumed`/… and the value-sampling math), plus a
//! minimal [`TextStyle`]; E2-S8 adds the per-field [`serde_date`] codec seam
//! (Apple reference-epoch doubles + ISO-8601). The `Timeline`/`Track` shapes are
//! E2-S6.

mod animatable_property;
mod clip;
mod clip_type;
mod edit_types;
mod interpolation;
mod keyframe;
pub mod serde_date;
mod text_style;
mod transform;
mod volume;

pub use animatable_property::AnimatableProperty;
pub use clip::{Clip, FadeEdge};
pub use clip_type::ClipType;
pub use edit_types::{ClipShift, FrameRange, GapSelection, TimelineRangeSelection};
pub use interpolation::{lerp, smoothstep, Interpolation, KeyframeInterpolatable};
pub use keyframe::{
    clamp_keyframes_to_duration, rescale_keyframes, to_abs, to_offset, AnimPair, Keyframe,
    KeyframeTrack,
};
pub use text_style::{Fill, FontName, Rgba, Shadow, TextAlignment, TextStyle};
pub use transform::{Crop, Transform};
pub use volume::VolumeScale;

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
