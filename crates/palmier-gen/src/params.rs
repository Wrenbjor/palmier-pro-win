//! Generation params — the byte-faithful wire contract sent to
//! `generations:submit` (E9-S5; reference `*GenerationParams.encode` +
//! `BackendGenerationParams`). Pure encode — zero Convex dependency.
//!
//! Each variant emits a **`kind` discriminator** (`video|image|audio|upscale`)
//! and **omits empty reference arrays / nil fields** (reference
//! `encodeIfPresent` + the `if !arr.isEmpty` guards). Field names are
//! **byte-identical** to the reference (`startFrameURL`, `referenceImageURLs`,
//! `generateAudio`, `numImages`, `durationSeconds`, …) — a renamed field is a
//! silent backend break.

use serde::Serialize;

/// Video generation params (reference `VideoGenerationParams`).
#[derive(Debug, Clone, Default)]
pub struct VideoParams {
    pub prompt: String,
    pub duration: i32,
    pub aspect_ratio: String,
    pub resolution: Option<String>,
    pub source_video_url: Option<String>,
    pub start_frame_url: Option<String>,
    pub end_frame_url: Option<String>,
    pub reference_image_urls: Vec<String>,
    pub reference_video_urls: Vec<String>,
    pub reference_audio_urls: Vec<String>,
    pub generate_audio: bool,
}

impl Serialize for VideoParams {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("kind", "video")?;
        m.serialize_entry("prompt", &self.prompt)?;
        m.serialize_entry("duration", &self.duration)?;
        m.serialize_entry("aspectRatio", &self.aspect_ratio)?;
        if let Some(r) = &self.resolution {
            m.serialize_entry("resolution", r)?;
        }
        if let Some(u) = &self.source_video_url {
            m.serialize_entry("sourceVideoURL", u)?;
        }
        if let Some(u) = &self.start_frame_url {
            m.serialize_entry("startFrameURL", u)?;
        }
        if let Some(u) = &self.end_frame_url {
            m.serialize_entry("endFrameURL", u)?;
        }
        if !self.reference_image_urls.is_empty() {
            m.serialize_entry("referenceImageURLs", &self.reference_image_urls)?;
        }
        if !self.reference_video_urls.is_empty() {
            m.serialize_entry("referenceVideoURLs", &self.reference_video_urls)?;
        }
        if !self.reference_audio_urls.is_empty() {
            m.serialize_entry("referenceAudioURLs", &self.reference_audio_urls)?;
        }
        m.serialize_entry("generateAudio", &self.generate_audio)?;
        m.end()
    }
}

/// Image generation params (reference `ImageGenerationParams`).
#[derive(Debug, Clone, Default)]
pub struct ImageParams {
    pub prompt: String,
    pub aspect_ratio: String,
    pub resolution: Option<String>,
    pub quality: Option<String>,
    pub image_urls: Vec<String>,
    pub num_images: i32,
}

impl Serialize for ImageParams {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("kind", "image")?;
        m.serialize_entry("prompt", &self.prompt)?;
        m.serialize_entry("aspectRatio", &self.aspect_ratio)?;
        if let Some(r) = &self.resolution {
            m.serialize_entry("resolution", r)?;
        }
        if let Some(q) = &self.quality {
            m.serialize_entry("quality", q)?;
        }
        if !self.image_urls.is_empty() {
            m.serialize_entry("imageURLs", &self.image_urls)?;
        }
        m.serialize_entry("numImages", &self.num_images)?;
        m.end()
    }
}

/// Audio generation params (reference `AudioGenerationParams`).
#[derive(Debug, Clone, Default)]
pub struct AudioParams {
    pub prompt: String,
    pub voice: Option<String>,
    pub lyrics: Option<String>,
    pub style_instructions: Option<String>,
    pub instrumental: bool,
    pub duration_seconds: Option<i32>,
    pub video_url: Option<String>,
}

impl Serialize for AudioParams {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("kind", "audio")?;
        m.serialize_entry("prompt", &self.prompt)?;
        if let Some(v) = &self.voice {
            m.serialize_entry("voice", v)?;
        }
        if let Some(l) = &self.lyrics {
            m.serialize_entry("lyrics", l)?;
        }
        if let Some(s_) = &self.style_instructions {
            m.serialize_entry("styleInstructions", s_)?;
        }
        m.serialize_entry("instrumental", &self.instrumental)?;
        if let Some(d) = &self.duration_seconds {
            m.serialize_entry("durationSeconds", d)?;
        }
        if let Some(u) = &self.video_url {
            m.serialize_entry("videoURL", u)?;
        }
        m.end()
    }
}

/// Upscale generation params (reference `UpscaleGenerationParams`).
#[derive(Debug, Clone, Default)]
pub struct UpscaleParams {
    pub source_url: String,
    pub duration_seconds: i32,
}

impl Serialize for UpscaleParams {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(None)?;
        m.serialize_entry("kind", "upscale")?;
        m.serialize_entry("sourceURL", &self.source_url)?;
        m.serialize_entry("durationSeconds", &self.duration_seconds)?;
        m.end()
    }
}

/// The tagged params union (reference `BackendGenerationParams`). Encodes as the
/// inner variant's params (the `kind` discriminator lives in the variant encode).
#[derive(Debug, Clone)]
pub enum BackendGenerationParams {
    Video(VideoParams),
    Image(ImageParams),
    Audio(AudioParams),
    Upscale(UpscaleParams),
}

impl Serialize for BackendGenerationParams {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            BackendGenerationParams::Video(p) => p.serialize(s),
            BackendGenerationParams::Image(p) => p.serialize(s),
            BackendGenerationParams::Audio(p) => p.serialize(s),
            BackendGenerationParams::Upscale(p) => p.serialize(s),
        }
    }
}

impl BackendGenerationParams {
    /// Render to a `serde_json::Value` (the transport submits this as the
    /// `params` arg). Goes through serde so the omission/discriminator rules hold.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("generation params serialize")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_emits_kind_and_omits_empty_refs_and_nils() {
        let p = VideoParams {
            prompt: "a cat".into(),
            duration: 4,
            aspect_ratio: "16:9".into(),
            resolution: None,
            generate_audio: true,
            ..Default::default()
        };
        let v = BackendGenerationParams::Video(p).to_json();
        assert_eq!(v["kind"], "video");
        assert_eq!(v["prompt"], "a cat");
        assert_eq!(v["generateAudio"], true);
        // nil resolution + empty ref arrays are omitted.
        assert!(v.get("resolution").is_none());
        assert!(v.get("referenceImageURLs").is_none());
        assert!(v.get("startFrameURL").is_none());
    }

    #[test]
    fn video_includes_refs_and_frames_when_present() {
        let p = VideoParams {
            prompt: "x".into(),
            duration: 8,
            aspect_ratio: "9:16".into(),
            resolution: Some("1080p".into()),
            start_frame_url: Some("https://cdn/sf.png".into()),
            reference_image_urls: vec!["https://cdn/r1.png".into()],
            generate_audio: false,
            ..Default::default()
        };
        let v = BackendGenerationParams::Video(p).to_json();
        assert_eq!(v["resolution"], "1080p");
        assert_eq!(v["startFrameURL"], "https://cdn/sf.png");
        assert_eq!(v["referenceImageURLs"][0], "https://cdn/r1.png");
        assert_eq!(v["generateAudio"], false);
    }

    #[test]
    fn image_emits_kind_numimages_and_omits_empty_image_urls() {
        let p = ImageParams {
            prompt: "p".into(),
            aspect_ratio: "1:1".into(),
            resolution: Some("4K".into()),
            quality: Some("high".into()),
            image_urls: vec![],
            num_images: 2,
        };
        let v = BackendGenerationParams::Image(p).to_json();
        assert_eq!(v["kind"], "image");
        assert_eq!(v["numImages"], 2);
        assert_eq!(v["quality"], "high");
        assert!(v.get("imageURLs").is_none());
    }

    #[test]
    fn audio_emits_kind_and_optional_fields() {
        let p = AudioParams {
            prompt: "speak".into(),
            voice: Some("Rachel".into()),
            instrumental: false,
            duration_seconds: Some(10),
            ..Default::default()
        };
        let v = BackendGenerationParams::Audio(p).to_json();
        assert_eq!(v["kind"], "audio");
        assert_eq!(v["voice"], "Rachel");
        assert_eq!(v["instrumental"], false);
        assert_eq!(v["durationSeconds"], 10);
        assert!(v.get("lyrics").is_none());
        assert!(v.get("videoURL").is_none());
    }

    #[test]
    fn upscale_emits_kind_source_and_duration() {
        let p = UpscaleParams {
            source_url: "https://cdn/src.mp4".into(),
            duration_seconds: 12,
        };
        let v = BackendGenerationParams::Upscale(p).to_json();
        assert_eq!(v["kind"], "upscale");
        assert_eq!(v["sourceURL"], "https://cdn/src.mp4");
        assert_eq!(v["durationSeconds"], 12);
    }
}
