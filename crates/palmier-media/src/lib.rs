//! # palmier-media
//!
//! FFmpeg decode/encode, thumbnail extraction, and audio waveform generation
//! (FOUNDATION §4, §6.2). Wraps `ffmpeg-next` (decode/encode) and `symphonia`
//! (audio decode); those heavy deps are added per-story, not in this skeleton.

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
