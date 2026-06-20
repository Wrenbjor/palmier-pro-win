//! `MediaAsset` (serde shape) + `GenerationStatus` (story E2-S7).
//!
//! The macOS reference `MediaAsset` (`Models/MediaAsset.swift`) is a `@MainActor`
//! runtime class (carries an `NSImage` thumbnail, `pendingDownloadURL`, etc.) that
//! is **never Codable directly** — it is projected to/from a `MediaManifestEntry`
//! (`init(entry:resolvedURL:)` / `toManifestEntry`). This module provides the
//! durable serde shape FOUNDATION §5.6 calls for (the persistable subset of the
//! runtime asset), so a platform layer can round-trip an asset's stored state
//! independent of the runtime-only fields. The path-resolution / internalize-on-save
//! heuristic (`MediaResolver`, `toManifestEntry`) is `palmier-project`'s (E2-S12);
//! this is the model shape only.
//!
//! See FOUNDATION §5.6 and docs/reference/project-io.md "Media path resolution".

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::clip_type::ClipType;
use crate::manifest::{GenerationInput, MediaSource};

/// The live generation state of an asset (reference
/// `MediaAsset.GenerationStatus`).
///
/// Externally-tagged like the reference enum; `failed` carries its message. This is
/// a runtime/UI state — it is **not** persisted in `media.json` (the reference class
/// resets it from `generationInput`/download state on load) — but it round-trips as
/// a serde shape so the agent/MCP layer (Epic 7) can report it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GenerationStatus {
    /// No active generation.
    #[default]
    None,
    Generating,
    Downloading,
    Rendering,
    /// Generation failed, with the error message.
    Failed(String),
}

/// The persistable subset of a runtime `MediaAsset` (FOUNDATION §5.6).
///
/// Mirrors the manifest-entry fields plus the live `generation_status`. Required:
/// `id`/`name`/`asset_type`/`source`/`duration_seconds`. Optionals are
/// `decodeIfPresent`-style (`#[serde(default, skip_serializing_if = …)]`). Dates use
/// the E2-S8 apple-epoch codec seam.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaAsset {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub asset_type: ClipType,
    pub source: MediaSource,
    /// Duration in seconds (reference `duration: Double`).
    #[serde(rename = "duration")]
    pub duration_seconds: f64,
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
    /// Reference default is `false` (`var hasAudio: Bool = false`).
    #[serde(rename = "hasAudio", default)]
    pub has_audio: bool,
    #[serde(rename = "folderId", default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    #[serde(
        rename = "generationInput",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub generation_input: Option<GenerationInput>,
    /// Live generation state (reference default `.none`).
    #[serde(rename = "generationStatus", default)]
    pub generation_status: GenerationStatus,
    #[serde(
        rename = "cachedRemoteURL",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cached_remote_url: Option<String>,
    /// Apple reference-epoch double (E2-S8 codec seam).
    #[serde(
        rename = "cachedRemoteURLExpiresAt",
        with = "crate::serde_date::apple_ref_epoch::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub cached_remote_url_expires_at: Option<OffsetDateTime>,
}

impl MediaAsset {
    /// Minimal constructor (reference `init(id:url:type:name:duration:…)` — here
    /// `hasAudio` defaults to `type == video`, matching the reference init).
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        asset_type: ClipType,
        source: MediaSource,
        duration_seconds: f64,
    ) -> Self {
        MediaAsset {
            id: id.into(),
            name: name.into(),
            asset_type,
            source,
            duration_seconds,
            source_width: None,
            source_height: None,
            source_fps: None,
            has_audio: asset_type == ClipType::Video,
            folder_id: None,
            generation_input: None,
            generation_status: GenerationStatus::None,
            cached_remote_url: None,
            cached_remote_url_expires_at: None,
        }
    }

    /// Whether this asset was AI-generated (reference `isGenerated`).
    pub fn is_generated(&self) -> bool {
        self.generation_input.is_some()
    }

    /// Whether a generation is currently in flight (reference `isGenerating`).
    pub fn is_generating(&self) -> bool {
        matches!(
            self.generation_status,
            GenerationStatus::Generating
                | GenerationStatus::Downloading
                | GenerationStatus::Rendering
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_status_default_is_none() {
        assert_eq!(GenerationStatus::default(), GenerationStatus::None);
    }

    #[test]
    fn generation_status_round_trips() {
        // Unit variants encode as bare strings; failed carries its message.
        let none = serde_json::to_string(&GenerationStatus::None).unwrap();
        assert_eq!(none, r#""none""#);
        assert_eq!(
            serde_json::to_string(&GenerationStatus::Generating).unwrap(),
            r#""generating""#
        );
        let failed = GenerationStatus::Failed("boom".into());
        let fj = serde_json::to_string(&failed).unwrap();
        assert_eq!(fj, r#"{"failed":"boom"}"#);
        let back: GenerationStatus = serde_json::from_str(&fj).unwrap();
        assert_eq!(back, failed);
    }

    #[test]
    fn media_asset_round_trips_byte_stable() {
        let mut a = MediaAsset::new(
            "asset-1",
            "Clip",
            ClipType::Video,
            MediaSource::Project {
                relative_path: "media/clip.mov".into(),
            },
            10.0,
        );
        a.source_width = Some(1920);
        a.source_height = Some(1080);
        a.generation_status = GenerationStatus::Rendering;

        let json = serde_json::to_string(&a).unwrap();
        let back: MediaAsset = serde_json::from_str(&json).unwrap();
        assert_eq!(a, back);
        assert_eq!(json, serde_json::to_string(&back).unwrap());
    }

    #[test]
    fn media_asset_lenient_decode_defaults() {
        // Only the required fields; optionals default, generation_status → none,
        // has_audio → false.
        let json = r#"{"id":"a1","name":"X","type":"image","source":{"project":{"relativePath":"media/x.png"}},"duration":5.0}"#;
        let a: MediaAsset = serde_json::from_str(json).unwrap();
        assert_eq!(a.asset_type, ClipType::Image);
        assert!(!a.has_audio);
        assert_eq!(a.generation_status, GenerationStatus::None);
        assert!(a.source_width.is_none());
        assert!(!a.is_generated());
        assert!(!a.is_generating());
    }

    #[test]
    fn new_sets_has_audio_for_video() {
        let v = MediaAsset::new(
            "a",
            "v",
            ClipType::Video,
            MediaSource::External {
                absolute_path: "/x.mov".into(),
            },
            1.0,
        );
        assert!(v.has_audio);
        let i = MediaAsset::new(
            "a",
            "i",
            ClipType::Image,
            MediaSource::External {
                absolute_path: "/x.png".into(),
            },
            1.0,
        );
        assert!(!i.has_audio);
    }
}
