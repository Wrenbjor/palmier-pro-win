//! Model catalog — `CatalogEntry` decode, the per-kind caps, the `ModelCatalog`
//! registry, and `ModelRegistry::by_id` (E9-S2; reference
//! `Catalog/ModelCatalog.swift`).
//!
//! `CatalogEntry` has a **custom decoder keyed on `kind`** (`video|image|audio|
//! upscale`) selecting which `*Caps` struct decodes from `uiCapabilities`. The
//! decode + the `apply()` partition are **pure** — the live `models:list`
//! subscription wiring is the transport's job ([`crate::transport`]); here we own
//! the registry the form, cost, and validation read.
//!
//! Pricing fields decode exactly (reference `CatalogEntry`):
//! `creditsPerSecond`, `audioDiscountRate`, `creditsPerImage` (all
//! `Map<String, f64>` keyed by resolution / `"res|quality"` / quality, with `""`
//! as the **default key**), `audioPricing` (tagged enum
//! `perThousandChars|perSecond|flat`), `creditsPerSecondUpscale`.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The four model kinds (reference `CatalogEntry.Kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelCategory {
    Video,
    Image,
    Audio,
    Upscale,
}

/// Audio pricing model (reference `CatalogEntry.AudioPricing`, tagged on `mode`).
#[derive(Debug, Clone, PartialEq)]
pub enum AudioPricing {
    PerThousandChars { rate: f64 },
    PerSecond { rate: f64 },
    Flat { price: f64 },
}

impl<'de> Deserialize<'de> for AudioPricing {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            mode: String,
            #[serde(default)]
            rate: Option<f64>,
            #[serde(default)]
            price: Option<f64>,
        }
        let raw = Raw::deserialize(deserializer)?;
        match raw.mode.as_str() {
            "perThousandChars" => Ok(AudioPricing::PerThousandChars {
                rate: raw.rate.ok_or_else(|| serde::de::Error::missing_field("rate"))?,
            }),
            "perSecond" => Ok(AudioPricing::PerSecond {
                rate: raw.rate.ok_or_else(|| serde::de::Error::missing_field("rate"))?,
            }),
            "flat" => Ok(AudioPricing::Flat {
                price: raw.price.ok_or_else(|| serde::de::Error::missing_field("price"))?,
            }),
            other => Err(serde::de::Error::custom(format!(
                "Unknown audio pricing mode '{other}'"
            ))),
        }
    }
}

/// Per-kind capabilities decoded from `uiCapabilities` based on `kind`.
#[derive(Debug, Clone, PartialEq)]
pub enum Capabilities {
    Video(VideoCaps),
    Image(ImageCaps),
    Audio(AudioCaps),
    Upscale(UpscaleCaps),
}

/// Video caps (reference `VideoCaps`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoCaps {
    pub durations: Vec<i32>,
    #[serde(default)]
    pub resolutions: Option<Vec<String>>,
    pub aspect_ratios: Vec<String>,
    pub supports_first_frame: bool,
    pub supports_last_frame: bool,
    pub max_reference_images: i32,
    pub max_reference_videos: i32,
    pub max_reference_audios: i32,
    #[serde(default)]
    pub max_total_references: Option<i32>,
    #[serde(default)]
    pub max_combined_video_ref_seconds: Option<f64>,
    #[serde(default)]
    pub max_combined_audio_ref_seconds: Option<f64>,
    pub frames_and_references_exclusive: bool,
    pub reference_tag_noun: String,
    pub requires_source_video: bool,
    pub requires_reference_image: bool,
}

/// Image caps (reference `ImageCaps`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCaps {
    #[serde(default)]
    pub resolutions: Option<Vec<String>>,
    pub aspect_ratios: Vec<String>,
    #[serde(default)]
    pub qualities: Option<Vec<String>>,
    pub supports_image_reference: bool,
    pub max_images: i32,
}

/// Audio caps (reference `AudioCaps`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioCaps {
    /// `tts` | `music` | `sfx`.
    pub category: String,
    #[serde(default)]
    pub voices: Option<Vec<String>>,
    #[serde(default)]
    pub default_voice: Option<String>,
    pub supports_lyrics: bool,
    pub supports_instrumental: bool,
    pub supports_style_instructions: bool,
    #[serde(default)]
    pub durations: Option<Vec<i32>>,
    pub min_prompt_length: i32,
    /// `text` | `video`.
    #[serde(default)]
    pub inputs: Option<Vec<String>>,
    #[serde(default)]
    pub prompt_label: Option<String>,
    #[serde(default)]
    pub min_seconds: Option<i32>,
    #[serde(default)]
    pub max_seconds: Option<i32>,
}

impl AudioCaps {
    /// Effective inputs (reference `inputs ?? ["text"]`).
    #[must_use]
    pub fn effective_inputs(&self) -> Vec<String> {
        self.inputs
            .clone()
            .unwrap_or_else(|| vec!["text".to_string()])
    }

    /// Whether this model takes a video input (reference `inputs.contains(.video)`).
    #[must_use]
    pub fn accepts_video(&self) -> bool {
        self.effective_inputs().iter().any(|i| i == "video")
    }
}

/// Upscale caps (reference `UpscaleCaps`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpscaleCaps {
    /// `Fast` | `Medium` | `Slow`.
    pub speed: String,
    pub p75_duration_seconds: i32,
    /// `video` | `image`.
    pub supported_types: Vec<String>,
}

/// One model-catalog row (reference `CatalogEntry`). Custom-decoded: `kind`
/// drives which `*Caps` decodes from `uiCapabilities`.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
    pub id: String,
    pub kind: ModelCategory,
    pub display_name: String,
    pub allowed_endpoints: Vec<String>,
    pub response_shape: String,
    pub caps: Capabilities,
    pub credits_per_second: Option<BTreeMap<String, f64>>,
    pub audio_discount_rate: Option<BTreeMap<String, f64>>,
    pub credits_per_image: Option<BTreeMap<String, f64>>,
    pub qualities: Option<Vec<String>>,
    pub audio_pricing: Option<AudioPricing>,
    pub credits_per_second_upscale: Option<f64>,
}

impl<'de> Deserialize<'de> for CatalogEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // Decode the kind + scalars first, then re-decode `uiCapabilities`
        // against the kind-selected caps struct from the captured raw value.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            id: String,
            kind: ModelCategory,
            display_name: String,
            #[serde(default)]
            allowed_endpoints: Vec<String>,
            #[serde(default)]
            response_shape: String,
            ui_capabilities: serde_json::Value,
            #[serde(default)]
            credits_per_second: Option<BTreeMap<String, f64>>,
            #[serde(default)]
            audio_discount_rate: Option<BTreeMap<String, f64>>,
            #[serde(default)]
            credits_per_image: Option<BTreeMap<String, f64>>,
            #[serde(default)]
            qualities: Option<Vec<String>>,
            #[serde(default)]
            audio_pricing: Option<AudioPricing>,
            #[serde(default)]
            credits_per_second_upscale: Option<f64>,
        }

        let raw = Raw::deserialize(deserializer)?;

        // The caps structs carry `#[serde(rename_all = "camelCase")]`, so they
        // decode straight from the captured `uiCapabilities` JSON value.
        fn caps_decode<T: serde::de::DeserializeOwned, E: serde::de::Error>(
            v: serde_json::Value,
        ) -> Result<T, E> {
            serde_json::from_value::<T>(v).map_err(|e| E::custom(e.to_string()))
        }

        let caps = match raw.kind {
            ModelCategory::Video => Capabilities::Video(caps_decode(raw.ui_capabilities)?),
            ModelCategory::Image => Capabilities::Image(caps_decode(raw.ui_capabilities)?),
            ModelCategory::Audio => Capabilities::Audio(caps_decode(raw.ui_capabilities)?),
            ModelCategory::Upscale => Capabilities::Upscale(caps_decode(raw.ui_capabilities)?),
        };

        Ok(CatalogEntry {
            id: raw.id,
            kind: raw.kind,
            display_name: raw.display_name,
            allowed_endpoints: raw.allowed_endpoints,
            response_shape: raw.response_shape,
            caps,
            credits_per_second: raw.credits_per_second,
            audio_discount_rate: raw.audio_discount_rate,
            credits_per_image: raw.credits_per_image,
            qualities: raw.qualities,
            audio_pricing: raw.audio_pricing,
            credits_per_second_upscale: raw.credits_per_second_upscale,
        })
    }
}

impl CatalogEntry {
    /// The video config view if this is a video model.
    #[must_use]
    pub fn as_video(&self) -> Option<VideoModel<'_>> {
        match &self.caps {
            Capabilities::Video(caps) => Some(VideoModel { entry: self, caps }),
            _ => None,
        }
    }
    /// The image config view if this is an image model.
    #[must_use]
    pub fn as_image(&self) -> Option<ImageModel<'_>> {
        match &self.caps {
            Capabilities::Image(caps) => Some(ImageModel { entry: self, caps }),
            _ => None,
        }
    }
    /// The audio config view if this is an audio model.
    #[must_use]
    pub fn as_audio(&self) -> Option<AudioModel<'_>> {
        match &self.caps {
            Capabilities::Audio(caps) => Some(AudioModel { entry: self, caps }),
            _ => None,
        }
    }
    /// The upscale config view if this is an upscale model.
    #[must_use]
    pub fn as_upscale(&self) -> Option<UpscaleModel<'_>> {
        match &self.caps {
            Capabilities::Upscale(caps) => Some(UpscaleModel { entry: self, caps }),
            _ => None,
        }
    }
}

/// A borrowed video model config (entry + caps) — the validation/cost view
/// (reference `VideoModelConfig`).
pub struct VideoModel<'a> {
    pub entry: &'a CatalogEntry,
    pub caps: &'a VideoCaps,
}
/// A borrowed image model config (reference `ImageModelConfig`).
pub struct ImageModel<'a> {
    pub entry: &'a CatalogEntry,
    pub caps: &'a ImageCaps,
}
/// A borrowed audio model config (reference `AudioModelConfig`).
pub struct AudioModel<'a> {
    pub entry: &'a CatalogEntry,
    pub caps: &'a AudioCaps,
}
/// A borrowed upscale model config (reference `UpscaleModelConfig`).
pub struct UpscaleModel<'a> {
    pub entry: &'a CatalogEntry,
    pub caps: &'a UpscaleCaps,
}

impl ImageModel<'_> {
    /// Clamp `maxImages` to `[1,4]` (reference `max(1, min(4, caps.maxImages))`).
    #[must_use]
    pub fn max_images(&self) -> i32 {
        self.caps.max_images.clamp(1, 4)
    }
}

/// The partitioned model registry (reference `ModelCatalog` + `ModelRegistry`).
/// Built by [`ModelCatalog::apply`] from a decoded catalog payload; pure, no I/O.
#[derive(Debug, Clone, Default)]
pub struct ModelCatalog {
    entries: Vec<CatalogEntry>,
    by_id: BTreeMap<String, usize>,
    is_loaded: bool,
}

impl ModelCatalog {
    /// A fresh, empty (not-loaded) catalog.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild the registry from a decoded `models:list` payload (reference
    /// `apply`). Sets `is_loaded = true`. Partition queries are `video()` etc.
    pub fn apply(&mut self, entries: Vec<CatalogEntry>) {
        self.by_id.clear();
        for (i, e) in entries.iter().enumerate() {
            self.by_id.insert(e.id.clone(), i);
        }
        self.entries = entries;
        self.is_loaded = true;
    }

    /// Whether the catalog has been populated at least once (reference
    /// `isLoaded`). Offline/empty degrades to `false` so the form can report it.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        self.is_loaded
    }

    /// Resolve a model id to its entry (reference `ModelRegistry.byId`).
    #[must_use]
    pub fn by_id(&self, id: &str) -> Option<&CatalogEntry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    /// Whether a model id exists (reference `ModelRegistry.exists`).
    #[must_use]
    pub fn exists(&self, id: &str) -> bool {
        self.by_id.contains_key(id)
    }

    /// All video models (reference `ModelCatalog.video`).
    pub fn video(&self) -> impl Iterator<Item = VideoModel<'_>> {
        self.entries.iter().filter_map(CatalogEntry::as_video)
    }
    /// All image models.
    pub fn image(&self) -> impl Iterator<Item = ImageModel<'_>> {
        self.entries.iter().filter_map(CatalogEntry::as_image)
    }
    /// All audio models.
    pub fn audio(&self) -> impl Iterator<Item = AudioModel<'_>> {
        self.entries.iter().filter_map(CatalogEntry::as_audio)
    }
    /// All upscale models.
    pub fn upscale(&self) -> impl Iterator<Item = UpscaleModel<'_>> {
        self.entries.iter().filter_map(CatalogEntry::as_upscale)
    }

    /// All entries (for the form / `list_models`).
    #[must_use]
    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    /// The first video model id, used as the default when the request omits
    /// `model` (reference "Defaults to first available model").
    #[must_use]
    pub fn first_video_id(&self) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.kind == ModelCategory::Video)
            .map(|e| e.id.as_str())
    }
    /// The first image model id.
    #[must_use]
    pub fn first_image_id(&self) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.kind == ModelCategory::Image)
            .map(|e| e.id.as_str())
    }
    /// The first audio model id.
    #[must_use]
    pub fn first_audio_id(&self) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.kind == ModelCategory::Audio)
            .map(|e| e.id.as_str())
    }
    /// The first upscale model id that supports `clip_type` (`video`/`image`).
    #[must_use]
    pub fn first_upscale_id_for(&self, clip_type: &str) -> Option<&str> {
        self.entries.iter().find_map(|e| {
            let u = e.as_upscale()?;
            if u.caps.supported_types.iter().any(|t| t == clip_type) {
                Some(e.id.as_str())
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        serde_json::json!([
            {
                "id": "veo3.1-fast",
                "kind": "video",
                "displayName": "Veo 3.1 Fast",
                "allowedEndpoints": ["generate"],
                "responseShape": "video",
                "uiCapabilities": {
                    "durations": [4, 8],
                    "resolutions": ["720p", "1080p"],
                    "aspectRatios": ["16:9", "9:16"],
                    "supportsFirstFrame": true,
                    "supportsLastFrame": false,
                    "maxReferenceImages": 3,
                    "maxReferenceVideos": 0,
                    "maxReferenceAudios": 0,
                    "maxTotalReferences": 3,
                    "framesAndReferencesExclusive": true,
                    "referenceTagNoun": "Image",
                    "requiresSourceVideo": false,
                    "requiresReferenceImage": false
                },
                "creditsPerSecond": { "": 10.0, "1080p": 20.0 },
                "audioDiscountRate": { "": 0.8 }
            },
            {
                "id": "nano-banana-pro",
                "kind": "image",
                "displayName": "Nano Banana Pro",
                "allowedEndpoints": ["generate"],
                "responseShape": "images",
                "uiCapabilities": {
                    "resolutions": ["2K", "4K"],
                    "aspectRatios": ["16:9", "1:1"],
                    "qualities": ["low", "high"],
                    "supportsImageReference": true,
                    "maxImages": 4
                },
                "creditsPerImage": { "": 5.0, "4K|high": 12.0, "high": 8.0 }
            },
            {
                "id": "elevenlabs-tts-v3",
                "kind": "audio",
                "displayName": "ElevenLabs TTS",
                "allowedEndpoints": ["generate"],
                "responseShape": "audio",
                "uiCapabilities": {
                    "category": "tts",
                    "voices": ["Rachel", "Adam"],
                    "defaultVoice": "Rachel",
                    "supportsLyrics": false,
                    "supportsInstrumental": false,
                    "supportsStyleInstructions": false,
                    "minPromptLength": 1,
                    "inputs": ["text"]
                },
                "audioPricing": { "mode": "perThousandChars", "rate": 30.0 }
            },
            {
                "id": "seedvr-upscaler",
                "kind": "upscale",
                "displayName": "SeedVR Upscaler",
                "allowedEndpoints": ["generate"],
                "responseShape": "upscaledImage",
                "uiCapabilities": {
                    "speed": "Fast",
                    "p75DurationSeconds": 30,
                    "supportedTypes": ["video", "image"]
                },
                "creditsPerSecondUpscale": 4.0
            }
        ])
    }

    #[test]
    fn decodes_all_four_kinds() {
        let entries: Vec<CatalogEntry> = serde_json::from_value(fixture()).unwrap();
        assert_eq!(entries.len(), 4);
        assert!(matches!(entries[0].caps, Capabilities::Video(_)));
        assert!(matches!(entries[1].caps, Capabilities::Image(_)));
        assert!(matches!(entries[2].caps, Capabilities::Audio(_)));
        assert!(matches!(entries[3].caps, Capabilities::Upscale(_)));
    }

    #[test]
    fn audio_pricing_tagged_enum_decodes() {
        let entries: Vec<CatalogEntry> = serde_json::from_value(fixture()).unwrap();
        let audio = &entries[2];
        assert_eq!(
            audio.audio_pricing,
            Some(AudioPricing::PerThousandChars { rate: 30.0 })
        );
    }

    #[test]
    fn apply_partitions_and_populates_by_id() {
        let entries: Vec<CatalogEntry> = serde_json::from_value(fixture()).unwrap();
        let mut cat = ModelCatalog::new();
        assert!(!cat.is_loaded());
        cat.apply(entries);
        assert!(cat.is_loaded());
        assert_eq!(cat.video().count(), 1);
        assert_eq!(cat.image().count(), 1);
        assert_eq!(cat.audio().count(), 1);
        assert_eq!(cat.upscale().count(), 1);
        assert!(cat.exists("veo3.1-fast"));
        assert!(cat.by_id("nano-banana-pro").is_some());
        assert_eq!(cat.first_video_id(), Some("veo3.1-fast"));
        assert_eq!(cat.first_upscale_id_for("image"), Some("seedvr-upscaler"));
        assert_eq!(cat.first_upscale_id_for("audio"), None);
    }

    #[test]
    fn image_max_images_clamps_to_four() {
        let entries: Vec<CatalogEntry> = serde_json::from_value(fixture()).unwrap();
        let img = entries[1].as_image().unwrap();
        assert_eq!(img.max_images(), 4);
    }
}
