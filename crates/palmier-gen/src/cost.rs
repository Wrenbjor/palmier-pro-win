//! `CostEstimator` — pure credit math (E9-S3; reference
//! `Catalog/CostEstimator.swift`). Zero Convex dependency.
//!
//! Credit rounding is **ceil**: `ceil_credits(x) = if x <= 0 { 0 } else
//! { x.ceil() as i64 }` (carry-forward: float→credit rounding is `ceil`,
//! replicated exactly so displayed estimates match the reference to the credit).

use crate::catalog::{
    AudioModel, AudioPricing, ImageModel, ModelCatalog, UpscaleModel, VideoModel,
};
use palmier_model::GenerationInput;

/// `ceilCredits` — non-positive → 0, else round up (reference).
#[must_use]
pub fn ceil_credits(credits: f64) -> i64 {
    if credits <= 0.0 {
        0
    } else {
        credits.ceil() as i64
    }
}

/// Look up a rate by key, falling back to the `""` default key (reference
/// `resolvedRate`).
fn resolved_rate(dict: &std::collections::BTreeMap<String, f64>, key: Option<&str>) -> Option<f64> {
    if let Some(k) = key {
        if let Some(v) = dict.get(k) {
            return Some(*v);
        }
    }
    dict.get("").copied()
}

/// Video cost (reference `videoCost`): `ceil(rate * duration)`,
/// `rate = creditsPerSecond[resolution] ?? creditsPerSecond[""]`, multiplied by
/// `audioDiscountRate` when `!generate_audio`.
#[must_use]
pub fn video_cost(
    model: &VideoModel<'_>,
    duration_seconds: i32,
    resolution: Option<&str>,
    generate_audio: bool,
) -> Option<i64> {
    let cps = model.entry.credits_per_second.as_ref()?;
    if cps.is_empty() || duration_seconds <= 0 {
        return None;
    }
    let mut rate = resolved_rate(cps, resolution)?;
    if !generate_audio {
        if let Some(discount) = model.entry.audio_discount_rate.as_ref() {
            if let Some(d) = resolved_rate(discount, resolution) {
                rate *= d;
            }
        }
    }
    Some(ceil_credits(rate * f64::from(duration_seconds)))
}

/// Image cost (reference `imageCost`): 2D `["res|quality"]` lookup → quality-only
/// → resolution lookup, `* num_images`.
#[must_use]
pub fn image_cost(
    model: &ImageModel<'_>,
    resolution: Option<&str>,
    quality: Option<&str>,
    num_images: i32,
) -> Option<i64> {
    let cpi = model.entry.credits_per_image.as_ref()?;
    if cpi.is_empty() {
        return None;
    }
    let count = f64::from(num_images.max(1));
    // 2D matrix lookup first (e.g. GPT Image 2 varies on both axes).
    if let (Some(r), Some(q)) = (resolution, quality) {
        if let Some(price) = cpi.get(&format!("{r}|{q}")) {
            return Some(ceil_credits(price * count));
        }
    }
    // Quality-only lookup when the model varies on quality but not resolution.
    if model.caps.qualities.is_some() {
        if let Some(q) = quality {
            if let Some(price) = cpi.get(q) {
                return Some(ceil_credits(price * count));
            }
        }
    }
    let rate = resolved_rate(cpi, resolution)?;
    Some(ceil_credits(rate * count))
}

/// Audio cost (reference `audioCost`): `perThousandChars` → `rate*chars/1000`;
/// `perSecond` → `rate*secs`; `flat` → flat amount.
#[must_use]
pub fn audio_cost(
    model: &AudioModel<'_>,
    prompt: &str,
    duration_seconds: Option<i32>,
) -> Option<i64> {
    match model.entry.audio_pricing.as_ref()? {
        AudioPricing::PerThousandChars { rate } => {
            let chars = prompt.chars().count();
            if chars == 0 {
                return None;
            }
            Some(ceil_credits(rate * (chars as f64 / 1000.0)))
        }
        AudioPricing::PerSecond { rate } => {
            let secs = duration_seconds?;
            if secs <= 0 {
                return None;
            }
            Some(ceil_credits(rate * f64::from(secs)))
        }
        AudioPricing::Flat { price } => Some(ceil_credits(*price)),
    }
}

/// Upscale cost (reference `upscaleCost`): `creditsPerSecondUpscale * max(1, secs)`.
#[must_use]
pub fn upscale_cost(model: &UpscaleModel<'_>, duration_seconds: i32) -> Option<i64> {
    let rate = model.entry.credits_per_second_upscale?;
    let d = duration_seconds.max(1);
    Some(ceil_credits(rate * f64::from(d)))
}

/// Re-derive cost from a stored [`GenerationInput`] via the catalog (reference
/// `cost(for:)`) — used on rerun and for the persisted generation log.
#[must_use]
pub fn cost_for(catalog: &ModelCatalog, gen_input: &GenerationInput) -> Option<i64> {
    let entry = catalog.by_id(&gen_input.model)?;
    if let Some(m) = entry.as_video() {
        video_cost(
            &m,
            gen_input.duration,
            gen_input.resolution.as_deref(),
            gen_input.generate_audio.unwrap_or(true),
        )
    } else if let Some(m) = entry.as_image() {
        image_cost(
            &m,
            gen_input.resolution.as_deref(),
            gen_input.quality.as_deref(),
            gen_input.num_images.unwrap_or(1),
        )
    } else if let Some(m) = entry.as_audio() {
        // Pass duration only when the model is duration-priced or video-input.
        let duration = if m.caps.durations.is_some() || m.caps.accepts_video() {
            Some(gen_input.duration)
        } else {
            None
        };
        audio_cost(&m, &gen_input.prompt, duration)
    } else if let Some(m) = entry.as_upscale() {
        upscale_cost(&m, gen_input.duration)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::CatalogEntry;

    fn catalog() -> ModelCatalog {
        let entries: Vec<CatalogEntry> = serde_json::from_value(serde_json::json!([
            {
                "id": "veo", "kind": "video", "displayName": "Veo",
                "allowedEndpoints": [], "responseShape": "video",
                "uiCapabilities": {
                    "durations": [4, 8], "resolutions": ["720p", "1080p"],
                    "aspectRatios": ["16:9"], "supportsFirstFrame": true,
                    "supportsLastFrame": false, "maxReferenceImages": 0,
                    "maxReferenceVideos": 0, "maxReferenceAudios": 0,
                    "framesAndReferencesExclusive": false, "referenceTagNoun": "Image",
                    "requiresSourceVideo": false, "requiresReferenceImage": false
                },
                "creditsPerSecond": { "": 10.0, "1080p": 20.0 },
                "audioDiscountRate": { "": 0.5 }
            },
            {
                "id": "img", "kind": "image", "displayName": "Img",
                "allowedEndpoints": [], "responseShape": "images",
                "uiCapabilities": {
                    "resolutions": ["2K", "4K"], "aspectRatios": ["1:1"],
                    "qualities": ["low", "high"], "supportsImageReference": true,
                    "maxImages": 4
                },
                "creditsPerImage": { "": 5.0, "4K|high": 11.0, "high": 7.0 }
            },
            {
                "id": "tts", "kind": "audio", "displayName": "TTS",
                "allowedEndpoints": [], "responseShape": "audio",
                "uiCapabilities": {
                    "category": "tts", "supportsLyrics": false,
                    "supportsInstrumental": false, "supportsStyleInstructions": false,
                    "minPromptLength": 1, "inputs": ["text"]
                },
                "audioPricing": { "mode": "perThousandChars", "rate": 30.0 }
            },
            {
                "id": "ups", "kind": "upscale", "displayName": "Ups",
                "allowedEndpoints": [], "responseShape": "upscaledImage",
                "uiCapabilities": {
                    "speed": "Fast", "p75DurationSeconds": 30,
                    "supportedTypes": ["video"]
                },
                "creditsPerSecondUpscale": 4.0
            }
        ]))
        .unwrap();
        let mut c = ModelCatalog::new();
        c.apply(entries);
        c
    }

    #[test]
    fn ceil_rounds_up_and_floors_at_zero() {
        assert_eq!(ceil_credits(0.0), 0);
        assert_eq!(ceil_credits(-3.0), 0);
        assert_eq!(ceil_credits(0.1), 1);
        assert_eq!(ceil_credits(10.0), 10);
        assert_eq!(ceil_credits(10.01), 11);
    }

    #[test]
    fn video_uses_resolution_then_default_key() {
        let c = catalog();
        let m = c.by_id("veo").unwrap().as_video().unwrap();
        // 1080p key present: 20 * 4 = 80.
        assert_eq!(video_cost(&m, 4, Some("1080p"), true), Some(80));
        // 720p not in map -> default "" = 10 * 8 = 80.
        assert_eq!(video_cost(&m, 8, Some("720p"), true), Some(80));
    }

    #[test]
    fn video_applies_audio_discount_when_no_audio() {
        let c = catalog();
        let m = c.by_id("veo").unwrap().as_video().unwrap();
        // default rate 10, discount 0.5, duration 4 -> ceil(20) = 20.
        assert_eq!(video_cost(&m, 4, Some("720p"), false), Some(20));
    }

    #[test]
    fn image_2d_then_quality_then_resolution() {
        let c = catalog();
        let m = c.by_id("img").unwrap().as_image().unwrap();
        // 2D "4K|high" = 11 * 2 imgs = 22.
        assert_eq!(image_cost(&m, Some("4K"), Some("high"), 2), Some(22));
        // quality-only "high" = 7 (no "2K|high" key).
        assert_eq!(image_cost(&m, Some("2K"), Some("high"), 1), Some(7));
        // resolution/default fallback "" = 5.
        assert_eq!(image_cost(&m, Some("2K"), Some("low"), 1), Some(5));
    }

    #[test]
    fn audio_per_thousand_chars() {
        let c = catalog();
        let m = c.by_id("tts").unwrap().as_audio().unwrap();
        // 100 chars * 30 / 1000 = 3.0 -> 3.
        let prompt = "x".repeat(100);
        assert_eq!(audio_cost(&m, &prompt, None), Some(3));
        // empty prompt -> None.
        assert_eq!(audio_cost(&m, "", None), None);
    }

    #[test]
    fn upscale_floors_seconds_at_one() {
        let c = catalog();
        let m = c.by_id("ups").unwrap().as_upscale().unwrap();
        // max(1, 0) = 1 * 4 = 4.
        assert_eq!(upscale_cost(&m, 0), Some(4));
        assert_eq!(upscale_cost(&m, 3), Some(12));
    }

    #[test]
    fn cost_for_reruns_via_registry() {
        let c = catalog();
        let gi = GenerationInput {
            prompt: "a".into(),
            model: "veo".into(),
            duration: 4,
            aspect_ratio: "16:9".into(),
            resolution: Some("1080p".into()),
            quality: None,
            image_urls: None,
            num_images: None,
            voice: None,
            lyrics: None,
            style_instructions: None,
            instrumental: None,
            generate_audio: Some(true),
            reference_image_urls: None,
            reference_video_urls: None,
            reference_audio_urls: None,
            image_url_asset_ids: None,
            reference_image_asset_ids: None,
            reference_video_asset_ids: None,
            reference_audio_asset_ids: None,
            created_at: None,
        };
        assert_eq!(cost_for(&c, &gi), Some(80));
    }
}
