//! # palmier-media
//!
//! FFmpeg decode/encode, thumbnail extraction, and audio waveform generation
//! (FOUNDATION §4, §6.2). Wraps `ffmpeg-next` (decode/encode) and `symphonia`
//! (audio decode); those heavy deps are added per-story, not in this skeleton.
//!
//! The [`cache`] module (story E4-S2) provides the media visual-cache
//! infrastructure — SHA256 cache key, cache-directory resolution, and the
//! concurrency gates + in-flight dedup — that the thumbnail/waveform pipelines
//! (E4-S3/S4/S5) build on. It deliberately pulls no media-decode dependency.
//!
//! The [`clip`] and [`metadata`] modules (story E4-S1) provide the import-time
//! classification + probing layer:
//! * [`clip`] — the case-insensitive ClipType **extension gate** and the Lottie
//!   content **second-gate** sniff (`classify_path`).
//! * [`metadata`] — the lightweight, **pure-Rust** asset metadata loader
//!   (`load_metadata`) returning duration / width / height / fps / has_audio.
//!   It links NO system FFmpeg — fields needing a full decoder are `None` with a
//!   `// TODO(ffmpeg)` note for the E4-S3+ decode stories.

pub mod cache;
pub mod clip;
pub mod metadata;

pub use clip::{
    classify_path, clip_type_for_extension, clip_type_for_path, is_lottie, is_lottie_bytes,
    ClipType,
};
pub use metadata::{load_metadata, load_metadata_as, AssetMetadata, MetadataError};

/// Placeholder for the media subsystem.
pub fn placeholder() -> &'static str {
    "palmier-media"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-media");
    }
}
