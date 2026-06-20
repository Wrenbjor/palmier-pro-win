//! `ClipType` — the kind of media a track/clip carries.
//!
//! Ported 1:1 from the macOS reference `Sources/PalmierPro/Models/ClipType.swift`.
//! See docs/reference/timeline-model.md "ClipType compatibility (cross-track move rules)".
//!
//! Wire representation: the Swift enum is `enum ClipType: String` with lowercase
//! bare cases (`video`, `audio`, `image`, `text`, `lottie`). We mirror that exactly
//! with serde `rename_all = "lowercase"` so projects authored by the reference (or
//! the Convex sample server) round-trip byte-identically.

use serde::{Deserialize, Serialize};

/// The kind of media a clip / track holds.
///
/// Wire values are the lowercase case names (`"video"`, `"audio"`, …), matching
/// `Models/ClipType.swift` (`enum ClipType: String`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipType {
    Video,
    Audio,
    Image,
    Text,
    Lottie,
}

impl ClipType {
    /// All clip types, in reference declaration order (mirrors Swift `CaseIterable`).
    pub const ALL: [ClipType; 5] = [
        ClipType::Video,
        ClipType::Audio,
        ClipType::Image,
        ClipType::Text,
        ClipType::Lottie,
    ];

    /// A *visual* type renders to the canvas: `video | image | text | lottie`.
    /// Audio is the only non-visual type. (Reference `ClipType.isVisual`.)
    pub fn is_visual(self) -> bool {
        matches!(
            self,
            ClipType::Video | ClipType::Image | ClipType::Text | ClipType::Lottie
        )
    }

    /// Whether a clip of `self` may live on / move onto a track of `other`.
    ///
    /// Reconciliation ruling #12 (docs/phase0-reconciliation.md) and reference
    /// `ClipType.isCompatible(with:)`: **ALL visual types are interchangeable**
    /// across visual tracks — video/image/text/lottie are mutually compatible;
    /// audio is compatible only with audio.
    ///
    /// This intentionally does NOT restrict text/lottie to their own type, even
    /// though FOUNDATION §6.3 wrongly states that. The reference is the parity
    /// authority (phase0-reconciliation: reference wins on conflict).
    pub fn is_compatible(self, other: ClipType) -> bool {
        self == other || (self.is_visual() && other.is_visual())
    }

    /// Map a (case-sensitive, no leading dot) file extension to a `ClipType`.
    ///
    /// Mirrors `init?(fileExtension:)` in `Models/ClipType.swift`. Note the
    /// reference maps both `json` and `lottie` to `lottie`. Unknown extensions
    /// return `None`.
    pub fn from_file_extension(ext: &str) -> Option<ClipType> {
        match ext {
            "mov" | "mp4" | "m4v" => Some(ClipType::Video),
            "mp3" | "wav" | "aac" | "m4a" => Some(ClipType::Audio),
            "png" | "jpg" | "jpeg" | "tiff" | "heic" | "webp" => Some(ClipType::Image),
            "json" | "lottie" => Some(ClipType::Lottie),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip_each_variant() {
        // Wire form is the lowercase bare case name, matching the Swift String enum.
        let cases = [
            (ClipType::Video, "\"video\""),
            (ClipType::Audio, "\"audio\""),
            (ClipType::Image, "\"image\""),
            (ClipType::Text, "\"text\""),
            (ClipType::Lottie, "\"lottie\""),
        ];
        for (variant, wire) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, wire, "wire encoding for {variant:?}");
            let back: ClipType = serde_json::from_str(&json).unwrap();
            assert_eq!(back, variant, "round-trip for {variant:?}");
        }
    }

    #[test]
    fn is_visual_matrix() {
        assert!(ClipType::Video.is_visual());
        assert!(ClipType::Image.is_visual());
        assert!(ClipType::Text.is_visual());
        assert!(ClipType::Lottie.is_visual());
        assert!(!ClipType::Audio.is_visual());
    }

    #[test]
    fn is_compatible_truth_table() {
        // Ruling #12: every visual type is interchangeable with every other visual
        // type; audio is compatible only with audio.
        let visual = [
            ClipType::Video,
            ClipType::Image,
            ClipType::Text,
            ClipType::Lottie,
        ];
        for &a in &visual {
            for &b in &visual {
                assert!(
                    a.is_compatible(b),
                    "{a:?} should be compatible with {b:?} (all visual interchangeable)"
                );
            }
            // Visual is never compatible with audio.
            assert!(!a.is_compatible(ClipType::Audio), "{a:?} vs audio");
            assert!(!ClipType::Audio.is_compatible(a), "audio vs {a:?}");
        }
        // Audio only with audio.
        assert!(ClipType::Audio.is_compatible(ClipType::Audio));
    }

    #[test]
    fn from_file_extension_representative() {
        assert_eq!(ClipType::from_file_extension("mov"), Some(ClipType::Video));
        assert_eq!(ClipType::from_file_extension("mp4"), Some(ClipType::Video));
        assert_eq!(ClipType::from_file_extension("m4v"), Some(ClipType::Video));
        assert_eq!(ClipType::from_file_extension("mp3"), Some(ClipType::Audio));
        assert_eq!(ClipType::from_file_extension("wav"), Some(ClipType::Audio));
        assert_eq!(ClipType::from_file_extension("aac"), Some(ClipType::Audio));
        assert_eq!(ClipType::from_file_extension("m4a"), Some(ClipType::Audio));
        assert_eq!(ClipType::from_file_extension("png"), Some(ClipType::Image));
        assert_eq!(ClipType::from_file_extension("jpg"), Some(ClipType::Image));
        assert_eq!(ClipType::from_file_extension("jpeg"), Some(ClipType::Image));
        assert_eq!(ClipType::from_file_extension("tiff"), Some(ClipType::Image));
        assert_eq!(ClipType::from_file_extension("heic"), Some(ClipType::Image));
        assert_eq!(ClipType::from_file_extension("webp"), Some(ClipType::Image));
        // Both json and lottie map to lottie (reference behavior).
        assert_eq!(ClipType::from_file_extension("json"), Some(ClipType::Lottie));
        assert_eq!(
            ClipType::from_file_extension("lottie"),
            Some(ClipType::Lottie)
        );
        // Unknown / wrong-case extensions are rejected (reference is case-sensitive,
        // bare extension with no leading dot).
        assert_eq!(ClipType::from_file_extension("MOV"), None);
        assert_eq!(ClipType::from_file_extension(".mov"), None);
        assert_eq!(ClipType::from_file_extension("txt"), None);
        assert_eq!(ClipType::from_file_extension(""), None);
    }
}
