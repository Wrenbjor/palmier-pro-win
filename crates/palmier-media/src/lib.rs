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

pub mod cache;

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
