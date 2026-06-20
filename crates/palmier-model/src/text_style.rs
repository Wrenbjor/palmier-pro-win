//! `TextStyle` — minimal serde shape for text-clip styling (story E2-S5).
//!
//! Ported from the macOS reference `Sources/PalmierPro/Models/TextStyle.swift`.
//! E2-S5 needs only the serde shape so a text clip carrying a `text_style`
//! round-trips; full text styling / rendering is Epic 5/10. The fields and
//! defaults mirror the reference exactly (lenient, missing-key-tolerant decode).
//!
//! Wire keys are the bare camelCase-ish Swift field names. `TextStyle` itself,
//! `RGBA`, `Shadow`, and `Fill` all use Swift's *derived* `Codable` (bare field
//! names); only `TextStyle` has a hand-written tolerant decode in the reference,
//! which `#[serde(default)]` on every field reproduces.

use serde::{Deserialize, Serialize};

/// RGBA color in sRGB, components in `0..1`. Reference `TextStyle.RGBA`
/// (default opaque white).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    #[serde(default = "one")]
    pub r: f64,
    #[serde(default = "one")]
    pub g: f64,
    #[serde(default = "one")]
    pub b: f64,
    #[serde(default = "one")]
    pub a: f64,
}

fn one() -> f64 {
    1.0
}

impl Default for Rgba {
    fn default() -> Self {
        Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
    }
}

impl Rgba {
    pub fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Rgba { r, g, b, a }
    }
}

/// Text horizontal alignment. Reference `TextStyle.Alignment` (`enum: String`).
/// Default is `Center` (reference `TextStyle.alignment = .center`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlignment {
    Left,
    #[default]
    Center,
    Right,
}

/// Drop shadow. Reference `TextStyle.Shadow`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shadow {
    #[serde(default = "bool_true")]
    pub enabled: bool,
    /// Alpha doubles as opacity (reference comment).
    #[serde(default = "shadow_color")]
    pub color: Rgba,
    #[serde(rename = "offsetX", default)]
    pub offset_x: f64,
    #[serde(rename = "offsetY", default = "neg_two")]
    pub offset_y: f64,
    #[serde(default = "six")]
    pub blur: f64,
}

fn bool_true() -> bool {
    true
}
fn neg_two() -> f64 {
    -2.0
}
fn six() -> f64 {
    6.0
}
fn shadow_color() -> Rgba {
    Rgba::new(0.0, 0.0, 0.0, 0.6)
}

impl Default for Shadow {
    fn default() -> Self {
        Shadow {
            enabled: true,
            color: shadow_color(),
            offset_x: 0.0,
            offset_y: -2.0,
            blur: 6.0,
        }
    }
}

/// Toggleable solid color — text-box background and border. Reference
/// `TextStyle.Fill`. Default = disabled with opaque-white color (`enabled:
/// false`, `Rgba::default()`), which the derive reproduces.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Fill {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub color: Rgba,
}

/// Text-clip typography + appearance. Reference `struct TextStyle` (lenient
/// decode — every field defaults). Wire keys: `fontName, fontSize, fontScale,
/// color, alignment, shadow, background, border`.
///
/// `Clone` (not `Copy`) — `font_name` is heap-backed. `Clip` derives `Clone`,
/// so this is sufficient.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    #[serde(rename = "fontName", default = "default_font_name")]
    pub font_name: FontName,
    #[serde(rename = "fontSize", default = "ninety_six")]
    pub font_size: f64,
    #[serde(rename = "fontScale", default = "one")]
    pub font_scale: f64,
    #[serde(default)]
    pub color: Rgba,
    #[serde(default)]
    pub alignment: TextAlignment,
    #[serde(default)]
    pub shadow: Shadow,
    #[serde(default = "background_default")]
    pub background: Fill,
    #[serde(default = "border_default")]
    pub border: Fill,
}

/// Font name. A small `Clone` newtype over `String` (kept distinct so callers
/// don't depend on it being a bare `String`). The reference default is
/// `"Helvetica-Bold"`; real font handling is Epic 5/10.
pub type FontName = arrayvec_lite::SmallStr;

fn ninety_six() -> f64 {
    96.0
}
fn default_font_name() -> FontName {
    FontName::from_str("Helvetica-Bold")
}
fn background_default() -> Fill {
    Fill {
        enabled: false,
        color: Rgba::new(0.0, 0.0, 0.0, 0.6),
    }
}
fn border_default() -> Fill {
    Fill {
        enabled: false,
        color: Rgba::new(0.0, 0.0, 0.0, 1.0),
    }
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle {
            font_name: default_font_name(),
            font_size: 96.0,
            font_scale: 1.0,
            color: Rgba::default(),
            alignment: TextAlignment::Center,
            shadow: Shadow::default(),
            background: background_default(),
            border: border_default(),
        }
    }
}

/// A tiny inline-string shim so `TextStyle` (and therefore `Clip`'s optional
/// `text_style`) can stay `Clone` without pulling a heavy dependency. Font names
/// are short; the reference default fits easily.
mod arrayvec_lite {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Heap-backed but `Clone` small string. Kept as a newtype over `String`
    /// (not literally `Copy`) — `TextStyle`/`Clip` derive `Clone`, not `Copy`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SmallStr(String);

    impl SmallStr {
        pub fn from_str(s: &str) -> Self {
            SmallStr(s.to_string())
        }
        pub fn as_str(&self) -> &str {
            &self.0
        }
    }

    impl Serialize for SmallStr {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&self.0)
        }
    }
    impl<'de> Deserialize<'de> for SmallStr {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            Ok(SmallStr(String::deserialize(d)?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_reference() {
        let s = TextStyle::default();
        assert_eq!(s.font_name.as_str(), "Helvetica-Bold");
        assert_eq!(s.font_size, 96.0);
        assert_eq!(s.font_scale, 1.0);
        assert_eq!(s.alignment, TextAlignment::Center);
        assert!(s.shadow.enabled);
        assert_eq!(s.shadow.offset_y, -2.0);
        assert!(!s.background.enabled);
        assert_eq!(s.background.color, Rgba::new(0.0, 0.0, 0.0, 0.6));
        assert!(!s.border.enabled);
    }

    #[test]
    fn empty_object_decodes_to_defaults() {
        let s: TextStyle = serde_json::from_str("{}").unwrap();
        assert_eq!(s, TextStyle::default());
    }

    #[test]
    fn round_trips_explicit_values() {
        let json = r#"{"fontName":"Arial","fontSize":48.0,"fontScale":2.0,"color":{"r":0.5,"g":0.5,"b":0.5,"a":1.0},"alignment":"left","shadow":{"enabled":false,"color":{"r":0.0,"g":0.0,"b":0.0,"a":0.6},"offsetX":1.0,"offsetY":1.0,"blur":3.0},"background":{"enabled":true,"color":{"r":1.0,"g":1.0,"b":1.0,"a":1.0}},"border":{"enabled":false,"color":{"r":0.0,"g":0.0,"b":0.0,"a":1.0}}}"#;
        let s: TextStyle = serde_json::from_str(json).unwrap();
        assert_eq!(s.font_name.as_str(), "Arial");
        assert_eq!(s.font_size, 48.0);
        assert_eq!(s.alignment, TextAlignment::Left);
        assert!(s.background.enabled);
        // decode → encode → decode is stable.
        let re = serde_json::to_string(&s).unwrap();
        let s2: TextStyle = serde_json::from_str(&re).unwrap();
        assert_eq!(s, s2);
    }
}
