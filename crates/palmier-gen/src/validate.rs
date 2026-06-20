//! Per-model & reference validation (E9-S4; reference `*ModelConfig.validate`,
//! `VideoGenerationSubmission.InputAssets.validate`). Pure — zero Convex
//! dependency.
//!
//! Each `validate_*` returns a **human-readable error string or `None`**, exactly
//! like the reference `unsupportedValue(...)` path. These are the same validators
//! the form runs to block submit and the tool bodies run before reaching Convex.

use crate::catalog::{AudioModel, ImageModel, UpscaleModel, VideoModel};

/// Reference `unsupportedValue(model:field:value:allowed:)`.
#[must_use]
pub fn unsupported_value(display_name: &str, field: &str, value: &str, allowed: &[String]) -> String {
    format!(
        "{display_name} does not support {field} '{value}'. Valid: {}.",
        allowed.join(", ")
    )
}

/// Validate video params against caps (reference `VideoModelConfig.validate`).
#[must_use]
pub fn validate_video(
    model: &VideoModel<'_>,
    duration: i32,
    aspect_ratio: &str,
    resolution: Option<&str>,
) -> Option<String> {
    let name = &model.entry.display_name;
    if !model.caps.durations.is_empty() && !model.caps.durations.contains(&duration) {
        let allowed: Vec<String> = model.caps.durations.iter().map(|d| format!("{d}s")).collect();
        return Some(unsupported_value(name, "duration", &format!("{duration}s"), &allowed));
    }
    if !model.caps.aspect_ratios.is_empty()
        && !aspect_ratio.is_empty()
        && !model.caps.aspect_ratios.contains(&aspect_ratio.to_string())
    {
        return Some(unsupported_value(name, "aspect ratio", aspect_ratio, &model.caps.aspect_ratios));
    }
    if let (Some(allowed), Some(r)) = (model.caps.resolutions.as_ref(), resolution) {
        if !r.is_empty() && !allowed.contains(&r.to_string()) {
            return Some(unsupported_value(name, "resolution", r, allowed));
        }
    }
    None
}

/// Validate image params against caps (reference `ImageModelConfig.validate`).
/// `num_images` should already be clamped via [`clamp_num_images`] for the
/// effective value, but this re-checks the `[1, max_images]` bound for an
/// explicit out-of-range request.
#[must_use]
pub fn validate_image(
    model: &ImageModel<'_>,
    aspect_ratio: &str,
    resolution: Option<&str>,
    quality: Option<&str>,
    image_ref_count: i32,
    num_images: i32,
) -> Option<String> {
    let name = &model.entry.display_name;
    if !model.caps.aspect_ratios.is_empty()
        && !aspect_ratio.is_empty()
        && !model.caps.aspect_ratios.contains(&aspect_ratio.to_string())
    {
        return Some(unsupported_value(name, "aspect ratio", aspect_ratio, &model.caps.aspect_ratios));
    }
    if let (Some(allowed), Some(r)) = (model.caps.resolutions.as_ref(), resolution) {
        if !r.is_empty() && !allowed.contains(&r.to_string()) {
            return Some(unsupported_value(name, "resolution", r, allowed));
        }
    }
    if let (Some(allowed), Some(q)) = (model.caps.qualities.as_ref(), quality) {
        if !q.is_empty() && !allowed.contains(&q.to_string()) {
            return Some(unsupported_value(name, "quality", q, allowed));
        }
    }
    if image_ref_count > 0 && !model.caps.supports_image_reference {
        return Some(format!("{name} does not accept reference images."));
    }
    let max = model.max_images();
    if num_images < 1 || num_images > max {
        let plural = if max == 1 { "" } else { "s" };
        return Some(format!(
            "{name} supports 1…{max} image{plural} per request (got {num_images})."
        ));
    }
    None
}

/// Validate audio params against caps (reference `AudioModelConfig.validate(params:)`):
/// min prompt length, voice allowlist, duration allowlist.
#[must_use]
pub fn validate_audio(
    model: &AudioModel<'_>,
    prompt: &str,
    voice: Option<&str>,
    duration_seconds: Option<i32>,
) -> Option<String> {
    let name = &model.entry.display_name;
    let prompt_len = prompt.trim().chars().count() as i32;
    if prompt_len < model.caps.min_prompt_length {
        return Some(format!(
            "{name} requires prompt ≥ {} characters (got {prompt_len}).",
            model.caps.min_prompt_length
        ));
    }
    if let (Some(allowed), Some(v)) = (model.caps.voices.as_ref(), voice) {
        if !v.is_empty() && !allowed.contains(&v.to_string()) {
            let mut shown: Vec<String> = allowed.iter().take(6).cloned().collect();
            if allowed.len() > 6 {
                shown.push("…".to_string());
            }
            return Some(unsupported_value(name, "voice", v, &shown));
        }
    }
    if let (Some(allowed), Some(d)) = (model.caps.durations.as_ref(), duration_seconds) {
        if !allowed.contains(&d) {
            let allowed_s: Vec<String> = allowed.iter().map(|x| format!("{x}s")).collect();
            return Some(unsupported_value(name, "duration", &format!("{d}s"), &allowed_s));
        }
    }
    None
}

/// Validate that an upscale model supports the asset's clip type (`video`/`image`).
#[must_use]
pub fn validate_upscale(model: &UpscaleModel<'_>, clip_type: &str) -> Option<String> {
    if model.caps.supported_types.iter().any(|t| t == clip_type) {
        None
    } else {
        Some(format!(
            "{} does not support upscaling {clip_type} assets. Supported: {}.",
            model.entry.display_name,
            model.caps.supported_types.join(", ")
        ))
    }
}

/// Reference counts to validate for a video request (reference
/// `VideoGenerationSubmission.InputAssets`).
#[derive(Debug, Clone, Default)]
pub struct ReferenceCounts {
    pub image_count: i32,
    pub video_count: i32,
    pub audio_count: i32,
    pub combined_video_seconds: f64,
    pub combined_audio_seconds: f64,
    pub has_start_or_end_frame: bool,
}

/// Validate the reference set for a video request (reference
/// `InputAssets.validate(for:)`): per-type max counts, combined-duration caps,
/// total-references cap, and frames-vs-references exclusivity.
#[must_use]
pub fn validate_video_references(model: &VideoModel<'_>, refs: &ReferenceCounts) -> Option<String> {
    let name = &model.entry.display_name;
    if refs.image_count > model.caps.max_reference_images {
        return Some(format!(
            "{name} accepts at most {} reference image(s) (got {}).",
            model.caps.max_reference_images, refs.image_count
        ));
    }
    if refs.video_count > model.caps.max_reference_videos {
        return Some(format!(
            "{name} accepts at most {} reference video(s) (got {}).",
            model.caps.max_reference_videos, refs.video_count
        ));
    }
    if refs.audio_count > model.caps.max_reference_audios {
        return Some(format!(
            "{name} accepts at most {} reference audio(s) (got {}).",
            model.caps.max_reference_audios, refs.audio_count
        ));
    }
    if let Some(max_total) = model.caps.max_total_references {
        let total = refs.image_count + refs.video_count + refs.audio_count;
        if total > max_total {
            return Some(format!(
                "{name} accepts at most {max_total} total reference(s) (got {total})."
            ));
        }
    }
    if let Some(cap) = model.caps.max_combined_video_ref_seconds {
        if refs.combined_video_seconds > cap {
            return Some(format!(
                "{name} accepts at most {cap}s of combined reference video (got {:.0}s).",
                refs.combined_video_seconds
            ));
        }
    }
    if let Some(cap) = model.caps.max_combined_audio_ref_seconds {
        if refs.combined_audio_seconds > cap {
            return Some(format!(
                "{name} accepts at most {cap}s of combined reference audio (got {:.0}s).",
                refs.combined_audio_seconds
            ));
        }
    }
    if model.caps.frames_and_references_exclusive
        && refs.has_start_or_end_frame
        && (refs.image_count + refs.video_count + refs.audio_count) > 0
    {
        return Some(format!(
            "{name} cannot combine first/last frames with reference assets — use one or the other."
        ));
    }
    None
}

/// Clamp `num_images` to `[1, 4]` (reference numImages clamp).
#[must_use]
pub fn clamp_num_images(num_images: i32) -> i32 {
    num_images.clamp(1, 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogEntry, ModelCatalog};

    fn catalog() -> ModelCatalog {
        let entries: Vec<CatalogEntry> = serde_json::from_value(serde_json::json!([
            {
                "id": "veo", "kind": "video", "displayName": "Veo",
                "allowedEndpoints": [], "responseShape": "video",
                "uiCapabilities": {
                    "durations": [4, 8], "resolutions": ["720p", "1080p"],
                    "aspectRatios": ["16:9", "9:16"], "supportsFirstFrame": true,
                    "supportsLastFrame": false, "maxReferenceImages": 2,
                    "maxReferenceVideos": 1, "maxReferenceAudios": 0,
                    "maxTotalReferences": 2, "maxCombinedVideoRefSeconds": 10.0,
                    "framesAndReferencesExclusive": true, "referenceTagNoun": "Image",
                    "requiresSourceVideo": false, "requiresReferenceImage": false
                }
            },
            {
                "id": "img", "kind": "image", "displayName": "Img",
                "allowedEndpoints": [], "responseShape": "images",
                "uiCapabilities": {
                    "resolutions": ["2K", "4K"], "aspectRatios": ["1:1"],
                    "qualities": ["low", "high"], "supportsImageReference": false,
                    "maxImages": 4
                }
            }
        ]))
        .unwrap();
        let mut c = ModelCatalog::new();
        c.apply(entries);
        c
    }

    #[test]
    fn video_rejects_bad_duration_resolution_aspect() {
        let c = catalog();
        let m = c.by_id("veo").unwrap().as_video().unwrap();
        assert!(validate_video(&m, 5, "16:9", Some("720p")).is_some());
        assert!(validate_video(&m, 4, "4:3", Some("720p")).is_some());
        assert!(validate_video(&m, 4, "16:9", Some("8k")).is_some());
        assert!(validate_video(&m, 4, "16:9", Some("720p")).is_none());
    }

    #[test]
    fn image_rejects_refs_when_unsupported_and_clamps_num() {
        let c = catalog();
        let m = c.by_id("img").unwrap().as_image().unwrap();
        // ref images on a model that doesn't support them.
        assert!(validate_image(&m, "1:1", Some("2K"), Some("low"), 1, 1).is_some());
        // num_images 0 and 5 are out of [1, max].
        assert!(validate_image(&m, "1:1", Some("2K"), Some("low"), 0, 0).is_some());
        assert!(validate_image(&m, "1:1", Some("2K"), Some("low"), 0, 5).is_some());
        assert!(validate_image(&m, "1:1", Some("2K"), Some("low"), 0, 4).is_none());
    }

    #[test]
    fn num_images_clamp() {
        assert_eq!(clamp_num_images(0), 1);
        assert_eq!(clamp_num_images(5), 4);
        assert_eq!(clamp_num_images(3), 3);
    }

    #[test]
    fn video_reference_overcount_and_exclusivity() {
        let c = catalog();
        let m = c.by_id("veo").unwrap().as_video().unwrap();
        // over per-type image count.
        let mut refs = ReferenceCounts {
            image_count: 3,
            ..Default::default()
        };
        assert!(validate_video_references(&m, &refs).is_some());
        // over combined video seconds.
        refs = ReferenceCounts {
            video_count: 1,
            combined_video_seconds: 20.0,
            ..Default::default()
        };
        assert!(validate_video_references(&m, &refs).is_some());
        // frames + references mutual exclusion.
        refs = ReferenceCounts {
            image_count: 1,
            has_start_or_end_frame: true,
            ..Default::default()
        };
        assert!(validate_video_references(&m, &refs).is_some());
        // a clean single-image ref passes.
        refs = ReferenceCounts {
            image_count: 1,
            ..Default::default()
        };
        assert!(validate_video_references(&m, &refs).is_none());
    }
}
