//! Transcription error taxonomy (E10-S1).
//!
//! Clean-room parity port of the reference `TranscriptionError` (Swift
//! `LocalizedError`). The user-facing strings match the reference
//! `errorDescription` messages verbatim. Variants that carry context in the
//! reference (`unsupportedLocale`, `modelInstallFailed`, `audioExtractionFailed`,
//! `analysisFailed`) keep their `String` payload so the message can interpolate it,
//! exactly as the reference does. `decodeFailed` is payload-free.
//!
//! Hand-rolled `Display` + `std::error::Error` (no `thiserror` in the workspace yet);
//! this keeps the model layer dependency-free per the scaffold scope.

use std::fmt;

/// Errors surfaced by the transcription pipeline. The `Display` string is the
/// user-facing message (parity with the reference `errorDescription`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptionError {
    /// On-device transcription unavailable for the given locale id.
    UnsupportedLocale(String),
    /// The on-device speech model could not be installed (reason).
    ModelInstallFailed(String),
    /// The engine produced a result that could not be parsed.
    DecodeFailed,
    /// Audio could not be extracted from the asset (reason).
    AudioExtractionFailed(String),
    /// The analysis/transcription step failed (reason).
    AnalysisFailed(String),
}

impl fmt::Display for TranscriptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscriptionError::UnsupportedLocale(id) => {
                write!(f, "On-device transcription is not available for {id}.")
            }
            TranscriptionError::ModelInstallFailed(reason) => {
                write!(f, "Could not install the on-device speech model: {reason}")
            }
            TranscriptionError::DecodeFailed => {
                write!(f, "Could not parse transcription result.")
            }
            TranscriptionError::AudioExtractionFailed(reason) => {
                write!(f, "Audio extraction failed: {reason}")
            }
            TranscriptionError::AnalysisFailed(reason) => {
                write!(f, "Transcription failed: {reason}")
            }
        }
    }
}

impl std::error::Error for TranscriptionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_match_reference_verbatim() {
        assert_eq!(
            TranscriptionError::UnsupportedLocale("zz-ZZ".to_string()).to_string(),
            "On-device transcription is not available for zz-ZZ."
        );
        assert_eq!(
            TranscriptionError::ModelInstallFailed("disk full".to_string()).to_string(),
            "Could not install the on-device speech model: disk full"
        );
        assert_eq!(
            TranscriptionError::DecodeFailed.to_string(),
            "Could not parse transcription result."
        );
        assert_eq!(
            TranscriptionError::AudioExtractionFailed("no track".to_string()).to_string(),
            "Audio extraction failed: no track"
        );
        assert_eq!(
            TranscriptionError::AnalysisFailed("oom".to_string()).to_string(),
            "Transcription failed: oom"
        );
    }
}
