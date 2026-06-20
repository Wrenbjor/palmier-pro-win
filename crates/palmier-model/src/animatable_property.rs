//! `AnimatableProperty` — identifies which clip property a keyframe lane drives.
//!
//! Ported from the macOS reference `Sources/PalmierPro/Models/Keyframe.swift`
//! (`enum AnimatableProperty: String, CaseIterable`). In the reference this enum is
//! a UI selector (which inspector lane / stamp button is active) and is NOT itself
//! persisted to `project.json`; it is `CaseIterable` but not `Codable`.
//!
//! We still derive serde here so it round-trips losslessly (story E2-S1 names a
//! serde round-trip test for each enum) and so MCP/tool payloads that reference a
//! property by name share one canonical wire spelling. Wire form is the lowercase
//! bare case name, matching the Swift `String` raw values.

use serde::{Deserialize, Serialize};

/// Which animatable clip property a keyframe track / inspector lane targets.
///
/// Wire values: `"opacity"`, `"position"`, `"scale"`, `"rotation"`, `"crop"`,
/// `"volume"` (the Swift `String` raw values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnimatableProperty {
    Opacity,
    Position,
    Scale,
    Rotation,
    Crop,
    Volume,
}

impl AnimatableProperty {
    /// All animatable properties, in reference declaration order (Swift `CaseIterable`).
    pub const ALL: [AnimatableProperty; 6] = [
        AnimatableProperty::Opacity,
        AnimatableProperty::Position,
        AnimatableProperty::Scale,
        AnimatableProperty::Rotation,
        AnimatableProperty::Crop,
        AnimatableProperty::Volume,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_each_variant() {
        let cases = [
            (AnimatableProperty::Opacity, "\"opacity\""),
            (AnimatableProperty::Position, "\"position\""),
            (AnimatableProperty::Scale, "\"scale\""),
            (AnimatableProperty::Rotation, "\"rotation\""),
            (AnimatableProperty::Crop, "\"crop\""),
            (AnimatableProperty::Volume, "\"volume\""),
        ];
        for (variant, wire) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, wire, "wire encoding for {variant:?}");
            let back: AnimatableProperty = serde_json::from_str(&json).unwrap();
            assert_eq!(back, variant, "round-trip for {variant:?}");
        }
    }

    #[test]
    fn all_has_six_distinct_variants() {
        assert_eq!(AnimatableProperty::ALL.len(), 6);
    }
}
