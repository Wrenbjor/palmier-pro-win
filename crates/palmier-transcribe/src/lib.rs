//! # palmier-transcribe
//!
//! whisper.cpp wrapper with word/segment alignment and caption building
//! (FOUNDATION §4, §6.9). Wraps `whisper-rs` (CPU/CUDA/Vulkan/DirectML backends);
//! that heavy dep is added per-story, not in this skeleton.
//!
//! E10-S1 lands the engine-independent layer: the [`TranscriptionResult`] data
//! model (+ [`TranscriptionResult::offsetting`]) and the [`TranscriptionError`]
//! taxonomy. The FFmpeg extraction + whisper.cpp engine run (E10-S2), locale
//! resolution (E10-S3), and disk+memory cache (E10-S4) build on these shapes.

mod error;
mod model;

pub use error::TranscriptionError;
pub use model::{TranscriptionResult, TranscriptionSegment, TranscriptionWord};
