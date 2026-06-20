//! GENERATE tool bodies — `generate_video`, `generate_image`, `generate_audio`,
//! `upscale_media` (E9-S11; reference `ToolExecutor+Generate.swift` →
//! `GenerationService.generate`).
//!
//! Each body: (1) checks the advisory `can_generate` gate (signed-in + credits;
//! ruling #24 — the server mutation is the real gate); (2) resolves the model
//! against the live catalog (defaulting to the first available); (3) validates
//! params per-model via `palmier-gen`; (4) assembles the `GenerationInput` +
//! `BackendGenerationParams` and submits through the [`GenerationGateway`] seam,
//! returning the **placeholder asset id(s)** immediately (the reference's
//! synchronous `placeholders[0].id`).
//!
//! When the backend isn't configured (no Convex) or `can_generate` is false, the
//! body returns the reference "sign in / backend not available" [`ToolResult`] —
//! never a crash (FR-34 / SM-11 edge).
//!
//! ## The [`GenerationGateway`] seam
//! The async lifecycle (transport, upload, subscribe, download) lives in
//! `palmier-gen` and is wired by the host (`palmier-tauri`) — the synchronous tool
//! path can't spawn it directly. So the executor holds an optional
//! `GenerationGateway`: the body does the pure catalog/validation/gating work and
//! hands a validated [`GenerationSubmission`] to the gateway, which kicks off the
//! lifecycle and returns the placeholder id(s). This keeps the bodies fully
//! unit-testable with a [mock gateway](crate::generate::tests).

use serde_json::{json, Value};

use palmier_gen::{
    clamp_num_images, validate_audio, validate_image, validate_upscale, validate_video,
    validate_video_references, AudioParams, BackendGenerationParams, ImageParams, ModelCatalog,
    ReferenceCounts, UpscaleParams, VideoParams,
};
use palmier_model::{ClipType, GenerationInput};

use crate::editor::EditorState;
use crate::result::ToolResult;

/// The reference "backend not available" message surfaced when generation isn't
/// configured / the user is signed out (reference + FR-34).
pub const BACKEND_NOT_AVAILABLE: &str =
    "AI generation is not available. Sign in to Palmier and subscribe to a plan with \
     remaining credits, then try again.";

/// A fully-validated generation request handed to the [`GenerationGateway`]. The
/// gateway owns the async lifecycle; the tool body owns validation + assembly.
pub struct GenerationSubmission {
    /// The recorded inputs (attached to the placeholder assets).
    pub gen_input: GenerationInput,
    /// The placeholder asset type (`video`/`image`/`audio`).
    pub asset_type: ClipType,
    /// The byte-faithful params JSON union submitted to `generations:submit`.
    pub params: BackendGenerationParams,
    /// `numImages` (clamped `[1,4]`) — 1 for non-image kinds.
    pub num_images: i32,
    /// Display name (defaults to first 30 chars of the prompt).
    pub name: Option<String>,
    /// Target folder id (validated to exist by the gateway/host).
    pub folder_id: Option<String>,
    /// Placeholder file extension (`mp4`/`png`/`mp3`).
    pub file_extension: String,
}

/// The seam the generate tool bodies submit through. Implemented by the host
/// (`palmier-tauri`) over `palmier-gen::GenerationService`, and by a mock in
/// tests. `submit` returns the placeholder asset id(s) synchronously (the
/// reference returns `placeholders[0].id`); the lifecycle runs detached.
pub trait GenerationGateway: Send + Sync {
    /// The live model catalog (for default-model resolution + validation). When
    /// the catalog hasn't synced, it is empty / `is_loaded() == false`.
    fn catalog(&self) -> &ModelCatalog;

    /// Submit a validated generation. Returns the placeholder asset id(s) created
    /// (id `[0]` is the primary, returned to the agent). `Err` carries a
    /// human-readable reason (e.g. backend not configured).
    fn submit(&self, submission: GenerationSubmission) -> Result<Vec<String>, String>;
}

/// String arg helper.
fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

/// i32 arg helper.
fn int_arg(args: &Value, key: &str) -> Option<i32> {
    args.get(key).and_then(Value::as_i64).map(|n| n as i32)
}

/// String-array arg helper.
fn str_array_arg(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The default display name (first 30 chars of the prompt) — reference default.
fn default_name(name: Option<&str>, prompt: &str) -> Option<String> {
    name.map(str::to_string)
        .or_else(|| Some(prompt.chars().take(30).collect()))
}

/// Gate + gateway lookup shared by all four bodies. Returns the gateway when
/// generation is allowed, else the reference "backend not available" result.
fn require_generation(state: &EditorState) -> Result<&dyn GenerationGateway, ToolResult> {
    // Advisory gate (ruling #24): signed in AND has credits. The server mutation
    // is the real enforcer; this only short-circuits the obvious blocked path.
    if !state.can_generate {
        return Err(ToolResult::error(BACKEND_NOT_AVAILABLE));
    }
    match state.generation_gateway() {
        Some(g) => Ok(g),
        None => Err(ToolResult::error(BACKEND_NOT_AVAILABLE)),
    }
}

/// Build an empty-ish [`GenerationInput`] with the common fields set.
fn base_gen_input(prompt: &str, model: &str, duration: i32, aspect_ratio: &str) -> GenerationInput {
    GenerationInput {
        prompt: prompt.to_string(),
        model: model.to_string(),
        duration,
        aspect_ratio: aspect_ratio.to_string(),
        resolution: None,
        quality: None,
        image_urls: None,
        num_images: None,
        voice: None,
        lyrics: None,
        style_instructions: None,
        instrumental: None,
        generate_audio: None,
        reference_image_urls: None,
        reference_video_urls: None,
        reference_audio_urls: None,
        image_url_asset_ids: None,
        reference_image_asset_ids: None,
        reference_video_asset_ids: None,
        reference_audio_asset_ids: None,
        created_at: None,
    }
}

/// Render the placeholder-id(s) result. The agent gets the primary id; image
/// generations also list the rest (reference returns the primary + the count).
fn placeholders_result(ids: Vec<String>) -> ToolResult {
    let primary = ids.first().cloned().unwrap_or_default();
    let body = if ids.len() > 1 {
        json!({ "mediaRef": primary, "mediaRefs": ids, "status": "generating" })
    } else {
        json!({ "mediaRef": primary, "status": "generating" })
    };
    ToolResult::ok(serde_json::to_string(&body).unwrap())
}

// ─────────────────────────────────────────────────────────────────────────────
// generate_video
// ─────────────────────────────────────────────────────────────────────────────

/// `generate_video` — async AI video generation. Returns a placeholder id
/// immediately (reference `ToolExecutor.generateVideo`).
pub fn generate_video(state: &mut EditorState, args: &Value) -> ToolResult {
    let gateway = match require_generation(state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    let catalog = gateway.catalog();

    let Some(prompt) = str_arg(args, "prompt") else {
        return ToolResult::error("Missing required 'prompt'");
    };
    // Default to the first available video model.
    let model_id = match str_arg(args, "model") {
        Some(m) => m.to_string(),
        None => match catalog.first_video_id() {
            Some(m) => m.to_string(),
            None => return ToolResult::error(BACKEND_NOT_AVAILABLE),
        },
    };
    let Some(model) = catalog.by_id(&model_id).and_then(|e| e.as_video()) else {
        return ToolResult::error(format!("Unknown video model '{model_id}'. Call list_models."));
    };

    let duration = int_arg(args, "duration").unwrap_or(0);
    let aspect_ratio = str_arg(args, "aspectRatio").unwrap_or("");
    let resolution = str_arg(args, "resolution");
    let generate_audio = args
        .get("generateAudio")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    // Per-model param validation.
    if let Some(err) = validate_video(&model, duration, aspect_ratio, resolution) {
        return ToolResult::error(err);
    }

    // Reference-count validation (counts only — the host resolves the bytes).
    let ref_images = str_array_arg(args, "referenceImageMediaRefs");
    let ref_videos = str_array_arg(args, "referenceVideoMediaRefs");
    let ref_audios = str_array_arg(args, "referenceAudioMediaRefs");
    let has_frame =
        str_arg(args, "startFrameMediaRef").is_some() || str_arg(args, "endFrameMediaRef").is_some();
    let refs = ReferenceCounts {
        image_count: ref_images.len() as i32,
        video_count: ref_videos.len() as i32,
        audio_count: ref_audios.len() as i32,
        has_start_or_end_frame: has_frame,
        ..Default::default()
    };
    if let Some(err) = validate_video_references(&model, &refs) {
        return ToolResult::error(err);
    }

    let mut gen_input = base_gen_input(prompt, &model_id, duration, aspect_ratio);
    gen_input.resolution = resolution.map(str::to_string);
    gen_input.generate_audio = Some(generate_audio);

    // Params: the reference URLs are resolved by the host/gateway from the asset
    // ids during the lifecycle (uploaded then stamped in). The tool body emits the
    // base params; the gateway fills referenceImageURLs etc. after upload.
    let params = BackendGenerationParams::Video(VideoParams {
        prompt: prompt.to_string(),
        duration,
        aspect_ratio: aspect_ratio.to_string(),
        resolution: resolution.map(str::to_string),
        generate_audio,
        ..Default::default()
    });

    submit(gateway, GenerationSubmission {
        gen_input,
        asset_type: ClipType::Video,
        params,
        num_images: 1,
        name: default_name(str_arg(args, "name"), prompt),
        folder_id: str_arg(args, "folderId").map(str::to_string),
        file_extension: "mp4".to_string(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// generate_image
// ─────────────────────────────────────────────────────────────────────────────

/// `generate_image` — async AI image generation. Returns placeholder id(s)
/// immediately (reference `ToolExecutor.generateImage`).
pub fn generate_image(state: &mut EditorState, args: &Value) -> ToolResult {
    let gateway = match require_generation(state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    let catalog = gateway.catalog();

    let Some(prompt) = str_arg(args, "prompt") else {
        return ToolResult::error("Missing required 'prompt'");
    };
    let model_id = match str_arg(args, "model") {
        Some(m) => m.to_string(),
        None => match catalog.first_image_id() {
            Some(m) => m.to_string(),
            None => return ToolResult::error(BACKEND_NOT_AVAILABLE),
        },
    };
    let Some(model) = catalog.by_id(&model_id).and_then(|e| e.as_image()) else {
        return ToolResult::error(format!("Unknown image model '{model_id}'. Call list_models."));
    };

    let aspect_ratio = str_arg(args, "aspectRatio").unwrap_or("");
    let resolution = str_arg(args, "resolution");
    let quality = str_arg(args, "quality");
    let ref_images = str_array_arg(args, "referenceMediaRefs");
    // numImages isn't a tool param in the schema (UI-only); default 1, clamp.
    let num_images = clamp_num_images(int_arg(args, "numImages").unwrap_or(1));

    if let Some(err) = validate_image(
        &model,
        aspect_ratio,
        resolution,
        quality,
        ref_images.len() as i32,
        num_images,
    ) {
        return ToolResult::error(err);
    }

    let mut gen_input = base_gen_input(prompt, &model_id, 0, aspect_ratio);
    gen_input.resolution = resolution.map(str::to_string);
    gen_input.quality = quality.map(str::to_string);
    gen_input.num_images = Some(num_images);

    let params = BackendGenerationParams::Image(ImageParams {
        prompt: prompt.to_string(),
        aspect_ratio: aspect_ratio.to_string(),
        resolution: resolution.map(str::to_string),
        quality: quality.map(str::to_string),
        image_urls: vec![],
        num_images,
    });

    submit(gateway, GenerationSubmission {
        gen_input,
        asset_type: ClipType::Image,
        params,
        num_images,
        name: default_name(str_arg(args, "name"), prompt),
        folder_id: str_arg(args, "folderId").map(str::to_string),
        file_extension: "png".to_string(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// generate_audio
// ─────────────────────────────────────────────────────────────────────────────

/// `generate_audio` — async AI audio (TTS / music / video-to-music). Returns a
/// placeholder id immediately (reference `ToolExecutor.generateAudio`). Note:
/// `prompt` is OPTIONAL (video-to-music).
pub fn generate_audio(state: &mut EditorState, args: &Value) -> ToolResult {
    let gateway = match require_generation(state) {
        Ok(g) => g,
        Err(r) => return r,
    };
    let catalog = gateway.catalog();

    let prompt = str_arg(args, "prompt").unwrap_or("");
    let model_id = match str_arg(args, "model") {
        Some(m) => m.to_string(),
        None => match catalog.first_audio_id() {
            Some(m) => m.to_string(),
            None => return ToolResult::error(BACKEND_NOT_AVAILABLE),
        },
    };
    let Some(model) = catalog.by_id(&model_id).and_then(|e| e.as_audio()) else {
        return ToolResult::error(format!("Unknown audio model '{model_id}'. Call list_models."));
    };

    let voice = str_arg(args, "voice");
    let lyrics = str_arg(args, "lyrics");
    let style = str_arg(args, "styleInstructions");
    let instrumental = args.get("instrumental").and_then(Value::as_bool).unwrap_or(false);
    let duration = int_arg(args, "duration");

    if let Some(err) = validate_audio(&model, prompt, voice, duration) {
        return ToolResult::error(err);
    }

    let mut gen_input = base_gen_input(prompt, &model_id, duration.unwrap_or(0), "");
    gen_input.voice = voice.map(str::to_string);
    gen_input.lyrics = lyrics.map(str::to_string);
    gen_input.style_instructions = style.map(str::to_string);
    gen_input.instrumental = Some(instrumental);

    let params = BackendGenerationParams::Audio(AudioParams {
        prompt: prompt.to_string(),
        voice: voice.map(str::to_string),
        lyrics: lyrics.map(str::to_string),
        style_instructions: style.map(str::to_string),
        instrumental,
        duration_seconds: duration,
        video_url: None,
    });

    let name = default_name(str_arg(args, "name"), prompt)
        .filter(|n| !n.is_empty())
        .or_else(|| Some(model.entry.display_name.clone()));

    submit(gateway, GenerationSubmission {
        gen_input,
        asset_type: ClipType::Audio,
        params,
        num_images: 1,
        name,
        folder_id: str_arg(args, "folderId").map(str::to_string),
        file_extension: "mp3".to_string(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// upscale_media
// ─────────────────────────────────────────────────────────────────────────────

/// `upscale_media` — AI upscale of an existing video/image asset. Returns a
/// placeholder id immediately (reference `ToolExecutor.upscaleMedia`).
pub fn upscale_media(state: &mut EditorState, args: &Value) -> ToolResult {
    let gateway = match require_generation(state) {
        Ok(g) => g,
        Err(r) => return r,
    };

    let Some(media_ref) = str_arg(args, "mediaRef") else {
        return ToolResult::error("Missing required 'mediaRef'");
    };
    // Resolve the source asset to learn its type + duration for cost/validation.
    let Some(asset) = state.library.assets.iter().find(|a| a.id == media_ref) else {
        return ToolResult::error(format!("Unknown media asset '{media_ref}'. Call get_media."));
    };
    let clip_type = asset.asset_type;
    let duration_seconds = asset.duration_seconds.max(1.0) as i32;
    let asset_name = asset.name.clone();
    let type_str = match clip_type {
        ClipType::Video => "video",
        ClipType::Image => "image",
        _ => return ToolResult::error("upscale_media only supports video or image assets."),
    };

    let catalog = gateway.catalog();
    let model_id = match str_arg(args, "model") {
        Some(m) => m.to_string(),
        None => match catalog.first_upscale_id_for(type_str) {
            Some(m) => m.to_string(),
            None => return ToolResult::error(BACKEND_NOT_AVAILABLE),
        },
    };
    let Some(model) = catalog.by_id(&model_id).and_then(|e| e.as_upscale()) else {
        return ToolResult::error(format!("Unknown upscale model '{model_id}'. Call list_models."));
    };
    if let Some(err) = validate_upscale(&model, type_str) {
        return ToolResult::error(err);
    }

    // The placeholder dest extension follows the source type.
    let file_extension = if clip_type == ClipType::Video { "mp4" } else { "png" };
    let mut gen_input = base_gen_input("", &model_id, duration_seconds, "");
    // The source URL is resolved + uploaded by the host during the lifecycle; the
    // base params carry an empty source (filled after upload).
    gen_input.image_url_asset_ids = Some(vec![media_ref.to_string()]);

    let params = BackendGenerationParams::Upscale(UpscaleParams {
        source_url: String::new(),
        duration_seconds,
    });

    submit(gateway, GenerationSubmission {
        gen_input,
        asset_type: clip_type,
        params,
        num_images: 1,
        name: Some(format!("Upscaled {asset_name}")),
        folder_id: str_arg(args, "folderId").map(str::to_string),
        file_extension: file_extension.to_string(),
    })
}

/// Submit through the gateway and render the placeholder-id(s) result, mapping a
/// gateway error to the reference "backend not available" / typed error.
fn submit(gateway: &dyn GenerationGateway, submission: GenerationSubmission) -> ToolResult {
    match gateway.submit(submission) {
        Ok(ids) if !ids.is_empty() => placeholders_result(ids),
        Ok(_) => ToolResult::error(BACKEND_NOT_AVAILABLE),
        Err(msg) => ToolResult::error(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use palmier_gen::CatalogEntry;
    use palmier_model::{MediaAsset, MediaLibrary, MediaSource};

    fn catalog() -> ModelCatalog {
        let entries: Vec<CatalogEntry> = serde_json::from_value(serde_json::json!([
            {
                "id": "veo", "kind": "video", "displayName": "Veo",
                "allowedEndpoints": [], "responseShape": "video",
                "uiCapabilities": {
                    "durations": [4, 8], "resolutions": ["720p", "1080p"],
                    "aspectRatios": ["16:9", "9:16"], "supportsFirstFrame": true,
                    "supportsLastFrame": false, "maxReferenceImages": 2,
                    "maxReferenceVideos": 0, "maxReferenceAudios": 0,
                    "framesAndReferencesExclusive": true, "referenceTagNoun": "Image",
                    "requiresSourceVideo": false, "requiresReferenceImage": false
                },
                "creditsPerSecond": { "": 10.0 }
            },
            {
                "id": "img", "kind": "image", "displayName": "Img",
                "allowedEndpoints": [], "responseShape": "images",
                "uiCapabilities": {
                    "resolutions": ["2K"], "aspectRatios": ["1:1"],
                    "qualities": ["high"], "supportsImageReference": true, "maxImages": 4
                },
                "creditsPerImage": { "": 5.0 }
            },
            {
                "id": "tts", "kind": "audio", "displayName": "TTS",
                "allowedEndpoints": [], "responseShape": "audio",
                "uiCapabilities": {
                    "category": "tts", "supportsLyrics": false, "supportsInstrumental": false,
                    "supportsStyleInstructions": false, "minPromptLength": 1, "inputs": ["text"]
                },
                "audioPricing": { "mode": "perThousandChars", "rate": 30.0 }
            },
            {
                "id": "ups", "kind": "upscale", "displayName": "Ups",
                "allowedEndpoints": [], "responseShape": "upscaledImage",
                "uiCapabilities": {
                    "speed": "Fast", "p75DurationSeconds": 30, "supportedTypes": ["video", "image"]
                },
                "creditsPerSecondUpscale": 4.0
            }
        ]))
        .unwrap();
        let mut c = ModelCatalog::new();
        c.apply(entries);
        c
    }

    /// A mock gateway recording the submissions and returning scripted ids.
    struct MockGateway {
        catalog: ModelCatalog,
        submissions: Mutex<Vec<GenerationSubmission>>,
        configured: bool,
    }
    impl MockGateway {
        fn new(configured: bool) -> Self {
            Self {
                catalog: catalog(),
                submissions: Mutex::new(Vec::new()),
                configured,
            }
        }
    }
    impl GenerationGateway for MockGateway {
        fn catalog(&self) -> &ModelCatalog {
            &self.catalog
        }
        fn submit(&self, submission: GenerationSubmission) -> Result<Vec<String>, String> {
            if !self.configured {
                return Err(BACKEND_NOT_AVAILABLE.to_string());
            }
            let n = submission.num_images.max(1) as usize;
            self.submissions.lock().unwrap().push(submission);
            Ok((0..n).map(|i| format!("ph-{i}")).collect())
        }
    }

    fn state_with_gateway(can_generate: bool, configured: bool) -> EditorState {
        let mut s = EditorState::new();
        s.can_generate = can_generate;
        s.set_generation_gateway(Box::new(MockGateway::new(configured)));
        s
    }

    fn parse(r: &ToolResult) -> Value {
        match &r.content[0] {
            crate::result::Block::Text(s) => serde_json::from_str(s).unwrap_or(json!({ "_raw": s })),
            _ => panic!("text"),
        }
    }

    #[test]
    fn generate_video_happy_path_returns_placeholder() {
        let mut s = state_with_gateway(true, true);
        let r = generate_video(
            &mut s,
            &json!({ "prompt": "a cat", "duration": 4, "aspectRatio": "16:9", "resolution": "720p" }),
        );
        assert!(!r.is_error);
        let v = parse(&r);
        assert_eq!(v["mediaRef"], "ph-0");
        assert_eq!(v["status"], "generating");
    }

    #[test]
    fn generate_image_returns_n_placeholders() {
        let mut s = state_with_gateway(true, true);
        let r = generate_image(
            &mut s,
            &json!({ "prompt": "a logo", "aspectRatio": "1:1", "numImages": 3 }),
        );
        assert!(!r.is_error);
        let v = parse(&r);
        assert_eq!(v["mediaRefs"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn generate_audio_prompt_optional_for_default_model() {
        let mut s = state_with_gateway(true, true);
        // minPromptLength is 1, so an empty prompt is rejected for tts; pass one.
        let r = generate_audio(&mut s, &json!({ "prompt": "hello world" }));
        assert!(!r.is_error, "{:?}", r.content);
    }

    #[test]
    fn upscale_media_resolves_asset_type() {
        let mut s = state_with_gateway(true, true);
        // Seed a video asset to upscale.
        let asset = MediaAsset::new(
            "vid-1",
            "Clip",
            ClipType::Video,
            MediaSource::External { absolute_path: "/x.mov".into() },
            10.0,
        );
        s.library.assets.push(asset);
        let r = upscale_media(&mut s, &json!({ "mediaRef": "vid-1" }));
        assert!(!r.is_error, "{:?}", r.content);
        let v = parse(&r);
        assert_eq!(v["status"], "generating");
    }

    #[test]
    fn blocked_when_cannot_generate() {
        let mut s = state_with_gateway(false, true);
        let r = generate_video(&mut s, &json!({ "prompt": "x", "duration": 4, "aspectRatio": "16:9" }));
        assert!(r.is_error);
        match &r.content[0] {
            crate::result::Block::Text(t) => assert!(t.contains("Sign in")),
            _ => panic!(),
        }
    }

    #[test]
    fn not_configured_returns_backend_unavailable() {
        // can_generate true but the gateway reports not-configured.
        let mut s = state_with_gateway(true, false);
        let r = generate_video(&mut s, &json!({ "prompt": "x", "duration": 4, "aspectRatio": "16:9" }));
        assert!(r.is_error);
    }

    #[test]
    fn no_gateway_at_all_returns_backend_unavailable() {
        let mut s = EditorState::new();
        s.can_generate = true; // gate passes but no gateway wired
        let r = generate_video(&mut s, &json!({ "prompt": "x", "duration": 4, "aspectRatio": "16:9" }));
        assert!(r.is_error);
    }

    #[test]
    fn invalid_duration_rejected_before_submit() {
        let mut s = state_with_gateway(true, true);
        let r = generate_video(
            &mut s,
            &json!({ "prompt": "x", "duration": 5, "aspectRatio": "16:9" }),
        );
        assert!(r.is_error);
        match &r.content[0] {
            crate::result::Block::Text(t) => assert!(t.contains("duration")),
            _ => panic!(),
        }
    }

    #[test]
    fn unknown_model_rejected() {
        let mut s = state_with_gateway(true, true);
        let r = generate_video(
            &mut s,
            &json!({ "prompt": "x", "model": "nope", "duration": 4, "aspectRatio": "16:9" }),
        );
        assert!(r.is_error);
    }
}
