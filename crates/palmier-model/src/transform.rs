//! `Transform` (center-based) and `Crop` (normalized edge insets).
//!
//! Ported 1:1 from the macOS reference `Sources/PalmierPro/Models/Timeline.swift`
//! (`struct Transform`, `struct Crop`). See docs/reference/timeline-model.md
//! "Data model" + Port risks "Transform storage is center-based".
//!
//! ## Reconciliation ruling #7 (docs/phase0-reconciliation.md)
//!
//! `Transform` is stored **center-based** — the persisted JSON keys are
//! `centerX / centerY / width / height / rotation / flipHorizontal / flipVertical`
//! (camelCase, exactly as Swift's derived `Codable` writes them). FOUNDATION §5.4
//! says the stored field is `top_left`; the reference is the parity authority and
//! stores center, so we store center and expose `top_left()` as a *computed*
//! accessor. Porting to a top-left stored field would break round-trip of every
//! existing project file.
//!
//! Legacy top-left-ish projects carried `x` / `y` keys; on decode they migrate to
//! center via **`centerX = oldX + width − 0.5`** (and the `y` analogue) — verbatim
//! from the reference `Transform.init(from:)`. A naive top-left field is forbidden.

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use crate::interpolation::{lerp, KeyframeInterpolatable};

/// Per-clip placement on the canvas, stored **center-based** in normalized
/// `0..1` canvas space (ruling #7).
///
/// Wire keys (camelCase, matching the Swift `Codable`):
/// `centerX, centerY, width, height, rotation, flipHorizontal, flipVertical`.
/// Legacy `x` / `y` keys are accepted on decode and migrated to center.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Center X in normalized canvas space (`0.5` = canvas center).
    pub center_x: f64,
    /// Center Y in normalized canvas space (`0.5` = canvas center).
    pub center_y: f64,
    /// Width as a fraction of the canvas width.
    pub width: f64,
    /// Height as a fraction of the canvas height.
    pub height: f64,
    /// Rotation in degrees, positive = clockwise (matches the reference comment).
    pub rotation: f64,
    /// Mirror horizontally.
    pub flip_horizontal: bool,
    /// Mirror vertically.
    pub flip_vertical: bool,
}

impl Default for Transform {
    /// Reference default: centered, full-canvas, no rotation/flip
    /// (`centerX=0.5, centerY=0.5, width=1, height=1`).
    fn default() -> Self {
        Transform {
            center_x: 0.5,
            center_y: 0.5,
            width: 1.0,
            height: 1.0,
            rotation: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
        }
    }
}

impl Transform {
    /// Computed top-left corner `(centerX - width/2, centerY - height/2)`
    /// (reference `Transform.topLeft`). This is the inverse of the center
    /// storage; the timeline/preview/export read it but never persist it.
    pub fn top_left(self) -> (f64, f64) {
        (self.center_x - self.width / 2.0, self.center_y - self.height / 2.0)
    }

    /// Computed center `(centerX, centerY)` (reference `Transform.center`).
    pub fn center(self) -> (f64, f64) {
        (self.center_x, self.center_y)
    }

    /// Construct from a top-left corner + size (reference
    /// `init(topLeft:width:height:)`): `centerX = tl.x + w/2`.
    pub fn from_top_left(top_left: (f64, f64), width: f64, height: f64) -> Self {
        Transform {
            center_x: top_left.0 + width / 2.0,
            center_y: top_left.1 + height / 2.0,
            width,
            height,
            ..Transform::default()
        }
    }
}

/// Field set shared by serialize + the manual deserialize visitor.
#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "camelCase")]
enum TransformField {
    CenterX,
    CenterY,
    Width,
    Height,
    Rotation,
    FlipHorizontal,
    FlipVertical,
    // Legacy top-left keys (ruling #7 migration source).
    X,
    Y,
    // Any other key is ignored (lenient decode, matches Swift's keyed container).
    #[serde(other)]
    Other,
}

impl Serialize for Transform {
    /// Encodes the **center-based** keys only (never the legacy `x`/`y`), in the
    /// reference's declaration order, so re-encoding a decoded project is
    /// byte-stable for clips authored center-based.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_struct("Transform", 7)?;
        s.serialize_field("centerX", &self.center_x)?;
        s.serialize_field("centerY", &self.center_y)?;
        s.serialize_field("width", &self.width)?;
        s.serialize_field("height", &self.height)?;
        s.serialize_field("rotation", &self.rotation)?;
        s.serialize_field("flipHorizontal", &self.flip_horizontal)?;
        s.serialize_field("flipVertical", &self.flip_vertical)?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for Transform {
    /// Ports the reference `Transform.init(from:)` exactly, including the legacy
    /// `x`/`y` → center migration (ruling #7): when `centerX` is absent but `x`
    /// is present, `centerX = oldX + width − 0.5` (and the `y` analogue). Every
    /// field is optional and defaults to the reference value.
    fn deserialize<D>(deserializer: D) -> Result<Transform, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TransformVisitor;

        impl<'de> Visitor<'de> for TransformVisitor {
            type Value = Transform;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a Transform object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Transform, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut center_x: Option<f64> = None;
                let mut center_y: Option<f64> = None;
                let mut width: Option<f64> = None;
                let mut height: Option<f64> = None;
                let mut rotation: Option<f64> = None;
                let mut flip_horizontal: Option<bool> = None;
                let mut flip_vertical: Option<bool> = None;
                let mut legacy_x: Option<f64> = None;
                let mut legacy_y: Option<f64> = None;

                while let Some(key) = map.next_key::<TransformField>()? {
                    match key {
                        TransformField::CenterX => center_x = Some(map.next_value()?),
                        TransformField::CenterY => center_y = Some(map.next_value()?),
                        TransformField::Width => width = Some(map.next_value()?),
                        TransformField::Height => height = Some(map.next_value()?),
                        TransformField::Rotation => rotation = Some(map.next_value()?),
                        TransformField::FlipHorizontal => {
                            flip_horizontal = Some(map.next_value()?)
                        }
                        TransformField::FlipVertical => {
                            flip_vertical = Some(map.next_value()?)
                        }
                        TransformField::X => legacy_x = Some(map.next_value()?),
                        TransformField::Y => legacy_y = Some(map.next_value()?),
                        // Unknown keys: consume and ignore (lenient decode).
                        TransformField::Other => {
                            let _ = map.next_value::<de::IgnoredAny>()?;
                        }
                    }
                }

                // Reference reads width/height first (default 1) because the
                // legacy migration uses width/height.
                let w = width.unwrap_or(1.0);
                let h = height.unwrap_or(1.0);

                // ruling #7: prefer centerX; else migrate legacy x via
                // centerX = oldX + width − 0.5; else default 0.5.
                let cx = match center_x {
                    Some(cx) => cx,
                    None => match legacy_x {
                        Some(old_x) => old_x + w - 0.5,
                        None => 0.5,
                    },
                };
                let cy = match center_y {
                    Some(cy) => cy,
                    None => match legacy_y {
                        Some(old_y) => old_y + h - 0.5,
                        None => 0.5,
                    },
                };

                Ok(Transform {
                    center_x: cx,
                    center_y: cy,
                    width: w,
                    height: h,
                    rotation: rotation.unwrap_or(0.0),
                    flip_horizontal: flip_horizontal.unwrap_or(false),
                    flip_vertical: flip_vertical.unwrap_or(false),
                })
            }
        }

        deserializer.deserialize_map(TransformVisitor)
    }
}

/// Per-clip crop as edge insets in normalized (`0..1`) source coordinates
/// (reference `struct Crop`). All four insets default to `0` (no crop).
///
/// Wire keys are the bare field names `left/top/right/bottom` (Swift's derived
/// `Codable`). Lenient decode: any missing inset defaults to `0`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Crop {
    #[serde(default)]
    pub left: f64,
    #[serde(default)]
    pub top: f64,
    #[serde(default)]
    pub right: f64,
    #[serde(default)]
    pub bottom: f64,
}

impl Default for Crop {
    fn default() -> Self {
        Crop {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        }
    }
}

impl Crop {
    /// No crop applied (all insets zero) — reference `Crop.isIdentity`.
    pub fn is_identity(self) -> bool {
        self.left == 0.0 && self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0
    }

    /// Visible width as a fraction of source width — reference
    /// `Crop.visibleWidthFraction = max(0, 1 - left - right)`.
    pub fn visible_width_fraction(self) -> f64 {
        (1.0 - self.left - self.right).max(0.0)
    }

    /// Visible height as a fraction of source height — reference
    /// `Crop.visibleHeightFraction = max(0, 1 - top - bottom)`.
    pub fn visible_height_fraction(self) -> f64 {
        (1.0 - self.top - self.bottom).max(0.0)
    }
}

impl KeyframeInterpolatable for Crop {
    /// 4-component-wise lerp (reference: `Crop` conforms to
    /// `KeyframeInterpolatable`, interpolating each inset independently).
    fn keyframe_interpolate(a: Crop, b: Crop, t: f64) -> Crop {
        Crop {
            left: lerp(a.left, b.left, t),
            top: lerp(a.top, b.top, t),
            right: lerp(a.right, b.right, t),
            bottom: lerp(a.bottom, b.bottom, t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_default_matches_reference() {
        let t = Transform::default();
        assert_eq!(t.center_x, 0.5);
        assert_eq!(t.center_y, 0.5);
        assert_eq!(t.width, 1.0);
        assert_eq!(t.height, 1.0);
        assert_eq!(t.rotation, 0.0);
        assert!(!t.flip_horizontal);
        assert!(!t.flip_vertical);
    }

    #[test]
    fn transform_center_keys_round_trip() {
        // A center-based object round-trips byte-stable in the reference key order.
        let json = r#"{"centerX":0.25,"centerY":0.75,"width":0.5,"height":0.5,"rotation":30.0,"flipHorizontal":true,"flipVertical":false}"#;
        let t: Transform = serde_json::from_str(json).unwrap();
        assert_eq!(t.center_x, 0.25);
        assert_eq!(t.center_y, 0.75);
        assert_eq!(t.width, 0.5);
        assert_eq!(t.height, 0.5);
        assert_eq!(t.rotation, 30.0);
        assert!(t.flip_horizontal);
        assert!(!t.flip_vertical);

        let reencoded = serde_json::to_string(&t).unwrap();
        assert_eq!(reencoded, json, "center-based Transform must round-trip byte-stable");
    }

    #[test]
    fn transform_legacy_xy_migrates_to_center() {
        // ruling #7: legacy {x,y,width,height} migrates via centerX = x + w − 0.5.
        // x=0.0, w=0.5  ->  centerX = 0.0 + 0.5 − 0.5 = 0.0
        // y=0.2, h=0.4  ->  centerY = 0.2 + 0.4 − 0.5 = 0.1
        let json = r#"{"x":0.0,"y":0.2,"width":0.5,"height":0.4}"#;
        let t: Transform = serde_json::from_str(json).unwrap();
        assert_eq!(t.center_x, 0.0);
        assert!((t.center_y - 0.1).abs() < 1e-12, "centerY migrated, got {}", t.center_y);
        assert_eq!(t.width, 0.5);
        assert_eq!(t.height, 0.4);
    }

    #[test]
    fn transform_legacy_default_width_when_absent() {
        // width/height default to 1 before migration is applied.
        // x=0.0, w=1 (default) -> centerX = 0.0 + 1 − 0.5 = 0.5
        let json = r#"{"x":0.0,"y":0.0}"#;
        let t: Transform = serde_json::from_str(json).unwrap();
        assert_eq!(t.center_x, 0.5);
        assert_eq!(t.center_y, 0.5);
        assert_eq!(t.width, 1.0);
        assert_eq!(t.height, 1.0);
    }

    #[test]
    fn transform_empty_object_is_default() {
        // Lenient decode: an empty object yields the full default.
        let t: Transform = serde_json::from_str("{}").unwrap();
        assert_eq!(t, Transform::default());
    }

    #[test]
    fn transform_centerx_wins_over_legacy_x() {
        // If both centerX and legacy x are present, centerX takes precedence.
        let json = r#"{"centerX":0.9,"x":0.1,"width":0.5,"height":0.5}"#;
        let t: Transform = serde_json::from_str(json).unwrap();
        assert_eq!(t.center_x, 0.9);
    }

    #[test]
    fn transform_ignores_unknown_keys() {
        let json = r#"{"centerX":0.5,"centerY":0.5,"unknownThing":42}"#;
        let t: Transform = serde_json::from_str(json).unwrap();
        assert_eq!(t.center_x, 0.5);
    }

    #[test]
    fn top_left_is_inverse_of_center_for_nontrivial_size() {
        // ruling #7: top_left = (centerX − w/2, centerY − h/2).
        let t = Transform {
            center_x: 0.6,
            center_y: 0.4,
            width: 0.3,
            height: 0.2,
            ..Transform::default()
        };
        let (tlx, tly) = t.top_left();
        assert!((tlx - (0.6 - 0.15)).abs() < 1e-12);
        assert!((tly - (0.4 - 0.10)).abs() < 1e-12);

        // Reconstructing from top-left recovers the same center.
        let back = Transform::from_top_left((tlx, tly), t.width, t.height);
        assert!((back.center_x - t.center_x).abs() < 1e-12);
        assert!((back.center_y - t.center_y).abs() < 1e-12);
    }

    #[test]
    fn crop_is_identity_and_visible_fractions() {
        let id = Crop::default();
        assert!(id.is_identity());
        assert_eq!(id.visible_width_fraction(), 1.0);
        assert_eq!(id.visible_height_fraction(), 1.0);

        let c = Crop {
            left: 0.1,
            top: 0.2,
            right: 0.3,
            bottom: 0.1,
        };
        assert!(!c.is_identity());
        // 1 − 0.1 − 0.3 = 0.6
        assert!((c.visible_width_fraction() - 0.6).abs() < 1e-12);
        // 1 − 0.2 − 0.1 = 0.7
        assert!((c.visible_height_fraction() - 0.7).abs() < 1e-12);

        // Over-crop clamps to 0, never negative.
        let over = Crop {
            left: 0.7,
            top: 0.0,
            right: 0.7,
            bottom: 0.0,
        };
        assert_eq!(over.visible_width_fraction(), 0.0);
    }

    #[test]
    fn crop_round_trips_and_defaults_missing_insets() {
        let json = r#"{"left":0.1,"top":0.2,"right":0.3,"bottom":0.4}"#;
        let c: Crop = serde_json::from_str(json).unwrap();
        assert_eq!(c.left, 0.1);
        assert_eq!(c.bottom, 0.4);

        // Missing insets default to 0.
        let partial: Crop = serde_json::from_str(r#"{"left":0.5}"#).unwrap();
        assert_eq!(partial.left, 0.5);
        assert_eq!(partial.top, 0.0);
        assert_eq!(partial.right, 0.0);
        assert_eq!(partial.bottom, 0.0);
    }

    #[test]
    fn crop_keyframe_interpolates_componentwise() {
        let a = Crop {
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
        };
        let b = Crop {
            left: 0.2,
            top: 0.4,
            right: 0.6,
            bottom: 0.8,
        };
        let mid = Crop::keyframe_interpolate(a, b, 0.5);
        assert!((mid.left - 0.1).abs() < 1e-12);
        assert!((mid.top - 0.2).abs() < 1e-12);
        assert!((mid.right - 0.3).abs() < 1e-12);
        assert!((mid.bottom - 0.4).abs() < 1e-12);
    }
}
