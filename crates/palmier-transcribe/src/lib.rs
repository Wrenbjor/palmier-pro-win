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

mod cache;
mod error;
mod locale;
mod model;
mod profanity;

pub use cache::TranscriptCache;
pub use error::TranscriptionError;
pub use model::{TranscriptionResult, TranscriptionSegment, TranscriptionWord};

// E10-S3: locale resolution (prefer user → OS auto-detect → error) and profanity
// censoring (bracketed-equivalent replacement). The engine (E10-S2) consumes
// `resolve_locale_en` for `.en` models and `censor_result` when `censor_profanity`;
// orchestration (E10-S6) consumes the general `resolve_locale` / `LocaleTag` surface.
pub use locale::{
    best_supported_locale, english_only_supported, match_locale, os_candidate_locales,
    resolve_locale, resolve_locale_en, LocaleTag,
};
pub use profanity::{censor_result, censor_result_with, censor_text, censor_text_with};
