//! # palmier-transcribe
//!
//! whisper.cpp wrapper with word/segment alignment and caption building
//! (FOUNDATION §4, §6.9). Wraps `whisper-rs` (CPU/CUDA/Vulkan/DirectML backends);
//! that heavy dep is added per-story, not in this skeleton.

/// Placeholder for the transcription subsystem.
pub fn placeholder() -> &'static str {
    "palmier-transcribe"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-transcribe");
    }
}
