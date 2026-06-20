//! Engine-independent transcription result model (E10-S1).
//!
//! Clean-room parity port of the macOS reference
//! `Sources/PalmierPro/Transcription/Transcription.swift` — the
//! `TranscriptionWord` / `TranscriptionSegment` / `TranscriptionResult` model and
//! `offsetting`. Times are plain `f64` **source seconds** (the reference's `CMTime`
//! @ timescale 600 collapses to `f64.seconds` here).
//!
//! The Whisper engine (E10-S2) and the disk+memory transcript cache (E10-S4) build
//! on these shapes; serde derives below are what the JSON disk cache round-trips.

use serde::{Deserialize, Serialize};

/// One token the transcriber aligned to an audio time range.
///
/// **Deviation from FOUNDATION §6.9 (reference wins — parity authority):** FOUNDATION
/// types the timestamps as non-optional `f64` (the common case). The reference
/// `Transcription.swift` models them as `Double?` because Whisper does not always
/// emit a time range for every run, and downstream code (`filter` / `spokenWordCount`)
/// relies on `None` words being skippable. We keep `Option<f64>` to preserve those
/// `filter`/skip semantics exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptionWord {
    pub text: String,
    pub start: Option<f64>,
    pub end: Option<f64>,
}

/// One natural utterance the transcriber endpointed on its own (pause/sentence
/// boundary). `text` carries the model's punctuation and casing. Unlike words,
/// segment timestamps are always present (non-optional `f64`), matching the reference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptionSegment {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// A full transcript: flattened `text`, detected `language` (BCP-47, optional),
/// per-token `words`, and endpointed `segments`. All timestamps are source seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptionResult {
    pub text: String,
    pub language: Option<String>,
    pub words: Vec<TranscriptionWord>,
    pub segments: Vec<TranscriptionSegment>,
}

impl TranscriptionResult {
    /// Shift every timestamp back into source time after transcribing an extracted
    /// range. Adds `by` to each segment `start`/`end` and to each word `start`/`end`
    /// that is present (`None` word timestamps are left untouched — parity with the
    /// reference `Optional.map` skip).
    ///
    /// No-op when `by == 0.0`: returns an identical clone (reference `guard offset != 0`).
    #[must_use]
    pub fn offsetting(&self, by: f64) -> TranscriptionResult {
        if by == 0.0 {
            return self.clone();
        }
        TranscriptionResult {
            text: self.text.clone(),
            language: self.language.clone(),
            words: self
                .words
                .iter()
                .map(|w| TranscriptionWord {
                    text: w.text.clone(),
                    start: w.start.map(|s| s + by),
                    end: w.end.map(|e| e + by),
                })
                .collect(),
            segments: self
                .segments
                .iter()
                .map(|s| TranscriptionSegment {
                    text: s.text.clone(),
                    start: s.start + by,
                    end: s.end + by,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populated() -> TranscriptionResult {
        TranscriptionResult {
            text: "Hello world".to_string(),
            language: Some("en-US".to_string()),
            words: vec![
                TranscriptionWord {
                    text: "Hello".to_string(),
                    start: Some(0.0),
                    end: Some(0.5),
                },
                // A word with no aligned time range — exercises the `None` skip path.
                TranscriptionWord {
                    text: "world".to_string(),
                    start: None,
                    end: None,
                },
            ],
            segments: vec![TranscriptionSegment {
                text: "Hello world".to_string(),
                start: 0.0,
                end: 1.0,
            }],
        }
    }

    #[test]
    fn offsetting_zero_is_identity() {
        let r = populated();
        // No-op when by == 0.0 — must round-trip byte-identical (PartialEq).
        assert_eq!(r.offsetting(0.0), r);
    }

    #[test]
    fn offsetting_shifts_present_timestamps_and_skips_none() {
        let r = populated().offsetting(2.0);
        assert_eq!(r.words[0].start, Some(2.0));
        assert_eq!(r.words[0].end, Some(2.5));
        // None word timestamps stay None.
        assert_eq!(r.words[1].start, None);
        assert_eq!(r.words[1].end, None);
        assert_eq!(r.segments[0].start, 2.0);
        assert_eq!(r.segments[0].end, 3.0);
        // text / language are carried through unchanged.
        assert_eq!(r.text, "Hello world");
        assert_eq!(r.language.as_deref(), Some("en-US"));
    }

    #[test]
    fn serde_round_trips_byte_identically() {
        let r = populated();
        let json = serde_json::to_string(&r).expect("serialize");
        let back: TranscriptionResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, r);
        // Re-serialize and compare bytes to confirm a stable round-trip
        // (including the `None`-timestamped word).
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2);
    }
}
