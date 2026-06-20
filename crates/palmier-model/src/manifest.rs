//! `MediaManifest` / `MediaManifestEntry` / `MediaSource` / `MediaFolder` —
//! the `media.json` serde shapes (story E2-S7).
//!
//! Ported 1:1 from the macOS reference `Sources/PalmierPro/Models/MediaManifest.swift`
//! and `Models/MediaFolder.swift`. See docs/reference/project-io.md "Media path
//! resolution" + Port risks "Encoder config is type-specific" / "Lenient decode is
//! load-bearing".
//!
//! ## `MediaSource` is externally-tagged
//!
//! Swift's derived `Codable` for `enum MediaSource { case external(absolutePath:),
//! case project(relativePath:) }` emits the **externally-tagged** JSON
//! `{"external":{"absolutePath":"…"}}` / `{"project":{"relativePath":"…"}}`. serde's
//! default enum representation is externally-tagged, producing exactly that — so we
//! do NOT add `#[serde(tag = …)]` / `untagged`. A flat or internally-tagged shape is
//! FORBIDDEN (Port risk: it breaks round-trip and sample import).
//!
//! ## Version default = 1 on decode, current version = 2 on encode
//!
//! The reference `MediaManifest.init(from:)` decodes a missing `version` as **1**
//! (legacy bundles predate the field), while `MediaManifest()` (and thus a freshly
//! written manifest) carries the **current version 2** (reference
//! `var version: Int = 2`). We mirror both: `#[serde(default = "default_version")]`
//! → 1 on decode-when-absent, and `MediaManifest::default()`/`::new()` → 2 on encode.
//! (docs/reference/project-io.md "manifest version default 1"; FOUNDATION §5.6 says
//! current 2 — both honored, decode-default 1 vs new-value 2.)
//!
//! ## Date fields route through the per-field codec seam (E2-S8)
//!
//! `cached_remote_url_expires_at` (and `GenerationInput::created_at`) serialize as
//! Apple reference-epoch doubles via `serde_date::apple_ref_epoch::option`, NOT a
//! single global Date format (Port risk "Encoder config is type-specific").

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::clip_type::ClipType;

fn default_version() -> u32 {
    // Reference `MediaManifest.init(from:)`: `decodeIfPresent(version) ?? 1`.
    // Legacy bundles with no `version` field decode as 1.
    1
}

/// The current manifest schema version written on encode (reference
/// `var version: Int = 2`).
pub const CURRENT_MANIFEST_VERSION: u32 = 2;

/// `media.json`: the project's media library — version + the asset entries +
/// the folder tree.
///
/// Lenient decode: every field is `#[serde(default)]`, so a `{}` manifest decodes
/// to `version = 1`, no entries, no folders.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaManifest {
    /// Schema version. **Decode-default 1** (legacy); **`new()`/`default()` → 2**
    /// (current, written on encode).
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<MediaManifestEntry>,
    #[serde(default)]
    pub folders: Vec<MediaFolder>,
}

impl MediaManifest {
    /// A fresh, empty manifest at the **current** version (2) — this is what gets
    /// written to disk on save.
    pub fn new() -> Self {
        MediaManifest {
            version: CURRENT_MANIFEST_VERSION,
            entries: Vec::new(),
            folders: Vec::new(),
        }
    }
}

impl Default for MediaManifest {
    /// Reference `MediaManifest()` carries the current version (2). NOTE this
    /// differs from the **decode** default (1) — see the module docs.
    fn default() -> Self {
        MediaManifest::new()
    }
}

/// One media-library entry (reference `MediaManifestEntry`).
///
/// `id`/`name`/`type`/`source`/`duration` are required (the reference decodes them
/// non-optionally); the rest are `decodeIfPresent` optionals mirrored with
/// `#[serde(default, skip_serializing_if = "Option::is_none")]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaManifestEntry {
    pub id: String,
    pub name: String,
    /// The media kind (reference key `type`).
    #[serde(rename = "type")]
    pub asset_type: ClipType,
    pub source: MediaSource,
    /// Duration in seconds (reference `duration: Double`).
    pub duration: f64,
    #[serde(
        rename = "generationInput",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub generation_input: Option<GenerationInput>,
    #[serde(rename = "sourceWidth", default, skip_serializing_if = "Option::is_none")]
    pub source_width: Option<i32>,
    #[serde(
        rename = "sourceHeight",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub source_height: Option<i32>,
    #[serde(rename = "sourceFPS", default, skip_serializing_if = "Option::is_none")]
    pub source_fps: Option<f64>,
    #[serde(rename = "hasAudio", default, skip_serializing_if = "Option::is_none")]
    pub has_audio: Option<bool>,
    #[serde(rename = "folderId", default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    #[serde(
        rename = "cachedRemoteURL",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cached_remote_url: Option<String>,
    /// Apple reference-epoch double via the E2-S8 codec seam (project/media/log
    /// Dates are numeric seconds-since-2001, NOT iso8601).
    #[serde(
        rename = "cachedRemoteURLExpiresAt",
        with = "crate::serde_date::apple_ref_epoch::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cached_remote_url_expires_at: Option<OffsetDateTime>,
}

/// The recorded inputs of an AI generation (reference `GenerationInput`). All
/// fields except `prompt`/`model`/`duration`/`aspect_ratio` are optional.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationInput {
    pub prompt: String,
    pub model: String,
    pub duration: i32,
    #[serde(rename = "aspectRatio")]
    pub aspect_ratio: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(rename = "imageURLs", default, skip_serializing_if = "Option::is_none")]
    pub image_urls: Option<Vec<String>>,
    /// Image-only.
    #[serde(rename = "numImages", default, skip_serializing_if = "Option::is_none")]
    pub num_images: Option<i32>,
    /// Audio-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lyrics: Option<String>,
    #[serde(
        rename = "styleInstructions",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub style_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instrumental: Option<bool>,
    /// Video-only.
    #[serde(
        rename = "generateAudio",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub generate_audio: Option<bool>,
    #[serde(
        rename = "referenceImageURLs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reference_image_urls: Option<Vec<String>>,
    #[serde(
        rename = "referenceVideoURLs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reference_video_urls: Option<Vec<String>>,
    #[serde(
        rename = "referenceAudioURLs",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reference_audio_urls: Option<Vec<String>>,
    #[serde(
        rename = "imageURLAssetIds",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub image_url_asset_ids: Option<Vec<String>>,
    #[serde(
        rename = "referenceImageAssetIds",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reference_image_asset_ids: Option<Vec<String>>,
    #[serde(
        rename = "referenceVideoAssetIds",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reference_video_asset_ids: Option<Vec<String>>,
    #[serde(
        rename = "referenceAudioAssetIds",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub reference_audio_asset_ids: Option<Vec<String>>,
    /// Apple reference-epoch double (E2-S8 codec seam).
    #[serde(
        rename = "createdAt",
        with = "crate::serde_date::apple_ref_epoch::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

/// Where the backing file of a media entry lives (reference `enum MediaSource`).
///
/// **Externally-tagged** — serde's default enum repr matches Swift's derived
/// Codable exactly:
/// - `MediaSource::External { absolute_path }` → `{"external":{"absolutePath":"…"}}`
/// - `MediaSource::Project  { relative_path }` → `{"project":{"relativePath":"…"}}`
///
/// The variant names are lowercased to `external`/`project` (`rename_all`) and the
/// inner keys are the exact Swift label spellings (`absolutePath` / `relativePath`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaSource {
    /// An absolute path outside the project bundle.
    External {
        #[serde(rename = "absolutePath")]
        absolute_path: String,
    },
    /// A path under the project bundle (already includes the `media/` segment).
    Project {
        #[serde(rename = "relativePath")]
        relative_path: String,
    },
}

/// A folder in the media-library tree (reference `MediaFolder`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaFolder {
    pub id: String,
    pub name: String,
    /// Parent folder id (`None` = top level). Reference key `parentFolderId`.
    #[serde(
        rename = "parentFolderId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_source_external_round_trips_externally_tagged() {
        let s = MediaSource::External {
            absolute_path: "/abs/path/clip.mov".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        // Exact externally-tagged shape matching Swift's derived Codable.
        assert_eq!(json, r#"{"external":{"absolutePath":"/abs/path/clip.mov"}}"#);
        let back: MediaSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn media_source_project_round_trips_externally_tagged() {
        let s = MediaSource::Project {
            relative_path: "media/clip.mov".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"project":{"relativePath":"media/clip.mov"}}"#);
        let back: MediaSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn manifest_missing_version_decodes_to_1() {
        // Reference decode default: a manifest without `version` decodes as 1.
        let json = r#"{"entries":[],"folders":[]}"#;
        let m: MediaManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.version, 1);

        // Empty `{}` likewise.
        let m2: MediaManifest = serde_json::from_str("{}").unwrap();
        assert_eq!(m2.version, 1);
        assert!(m2.entries.is_empty());
        assert!(m2.folders.is_empty());
    }

    #[test]
    fn fresh_manifest_uses_current_version_2() {
        // new()/default() carry the CURRENT version written on encode.
        assert_eq!(MediaManifest::new().version, 2);
        assert_eq!(MediaManifest::default().version, 2);
        let json = serde_json::to_string(&MediaManifest::new()).unwrap();
        assert!(json.contains("\"version\":2"), "{json}");
    }

    #[test]
    fn manifest_entry_round_trips_byte_stable_no_date() {
        // A representative entry with no Date fields (Date round-trip is S8's gate).
        let entry = MediaManifestEntry {
            id: "asset-1".into(),
            name: "Clip One".into(),
            asset_type: ClipType::Video,
            source: MediaSource::Project {
                relative_path: "media/clip.mov".into(),
            },
            duration: 12.5,
            generation_input: None,
            source_width: Some(1920),
            source_height: Some(1080),
            source_fps: Some(30.0),
            has_audio: Some(true),
            folder_id: None,
            cached_remote_url: None,
            cached_remote_url_expires_at: None,
        };
        let mut m = MediaManifest::new();
        m.entries.push(entry);
        m.folders.push(MediaFolder {
            id: "f1".into(),
            name: "Footage".into(),
            parent_id: None,
        });

        let json = serde_json::to_string(&m).unwrap();
        let back: MediaManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
        // Re-encode is byte-stable.
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    #[test]
    fn manifest_ignores_unknown_fields() {
        let json = r#"{"version":2,"entries":[],"folders":[],"futureField":true}"#;
        let m: MediaManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.version, 2);
    }

    #[test]
    fn media_folder_parent_id_omitted_when_none() {
        let f = MediaFolder {
            id: "f1".into(),
            name: "Top".into(),
            parent_id: None,
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(!json.contains("parentFolderId"), "{json}");
        let back: MediaFolder = serde_json::from_str(&json).unwrap();
        assert_eq!(back, f);
    }
}
