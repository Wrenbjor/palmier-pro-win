//! Profanity censoring (E10-S3).
//!
//! Clean-room parity port of the reference's `censorProfanity` path, which sets the
//! Apple `SpeechTranscriber` option `.etiquetteReplacements`
//! (`Transcription.swift` line 113). Apple's etiquette replacement masks a profane
//! word by substituting a **bracketed redaction** in the transcript (the literal word
//! list and replacement strings live inside Apple's framework, not in the reference
//! source — there is nothing verbatim to copy). The story therefore directs us to
//! "replace matched words with bracketed equivalents, or use Whisper token
//! suppression"; whisper-rs has no first-class token-suppression API today, so this
//! module implements the **bracketed-replacement** path post-transcription.
//!
//! Behavior:
//! - A whole-word, case-insensitive match against a clean-room base profanity set is
//!   replaced with its **bracketed equivalent** — the canonical etiquette form is the
//!   word wrapped in square brackets (e.g. `damn` → `[damn]`). Surrounding punctuation
//!   and whitespace are preserved (`"Damn!"` → `"[damn]!"`).
//! - Matching is on alphabetic-token boundaries so substrings are never censored
//!   (`"assassin"` and `"class"` are untouched).
//! - A clean transcript (no matches) is returned **unchanged**.
//!
//! The censor runs over a whole [`TranscriptionResult`]: `text`, each segment `text`,
//! and each word `text` are passed through the same token replacer, so the redaction is
//! consistent across all three views the downstream pipeline reads.

use crate::model::{TranscriptionResult, TranscriptionSegment, TranscriptionWord};

/// Clean-room base set of censored words (lowercase, alphabetic).
///
/// This is a small, deliberately conservative seed list — NOT a copy of Apple's
/// (closed-source) etiquette list, which the port cannot see. It exists so the
/// `censor_profanity` flag has deterministic, testable behavior with bracketed
/// replacement; the exact membership is a clean-room choice, not a parity oracle (the
/// reference's list is unobservable). Callers can supply their own set via
/// [`censor_result_with`] / [`censor_text_with`].
const BASE_PROFANITY: &[&str] = &[
    "damn", "hell", "crap", "ass", "bastard", "bitch", "shit", "fuck", "piss", "dick",
    "cunt", "asshole", "bullshit", "goddamn", "motherfucker",
];

/// Wrap a matched word in its **bracketed equivalent** — the etiquette redaction form.
///
/// The reference relies on Apple to choose the replacement string; we use the
/// canonical bracketed form `[word]` (lowercased, matching how an etiquette mask reads
/// regardless of the source casing).
fn bracket(word: &str) -> String {
    format!("[{}]", word.to_ascii_lowercase())
}

/// Replace whole-word, case-insensitive matches of `censored` in `text` with their
/// bracketed equivalents, preserving all non-alphabetic characters (punctuation,
/// whitespace) in place.
///
/// A "word" is a maximal run of Unicode alphabetic characters; matching compares the
/// ASCII-lowercased run against the (already-lowercased) `censored` set. Non-matching
/// runs and all separators are emitted verbatim, so a clean string is byte-identical to
/// its input.
#[must_use]
pub fn censor_text_with(text: &str, censored: &[&str]) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(text.len());
    let mut word = String::new();

    // Flush the accumulated alphabetic run, censoring it if it matches.
    let flush = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        let lowered = word.to_ascii_lowercase();
        if censored.iter().any(|c| *c == lowered) {
            out.push_str(&bracket(word));
        } else {
            out.push_str(word);
        }
        word.clear();
    };

    for ch in text.chars() {
        if ch.is_alphabetic() {
            word.push(ch);
        } else {
            flush(&mut word, &mut out);
            out.push(ch);
        }
    }
    flush(&mut word, &mut out);
    out
}

/// Censor `text` against the built-in [`BASE_PROFANITY`] set.
#[must_use]
pub fn censor_text(text: &str) -> String {
    censor_text_with(text, BASE_PROFANITY)
}

/// Apply [`censor_text_with`] across a whole [`TranscriptionResult`] (the flattened
/// `text`, every segment `text`, and every word `text`), returning a new result.
/// Timestamps and `language` are carried through untouched.
#[must_use]
pub fn censor_result_with(result: &TranscriptionResult, censored: &[&str]) -> TranscriptionResult {
    TranscriptionResult {
        text: censor_text_with(&result.text, censored),
        language: result.language.clone(),
        words: result
            .words
            .iter()
            .map(|w| TranscriptionWord {
                text: censor_text_with(&w.text, censored),
                start: w.start,
                end: w.end,
            })
            .collect(),
        segments: result
            .segments
            .iter()
            .map(|s| TranscriptionSegment {
                text: censor_text_with(&s.text, censored),
                start: s.start,
                end: s.end,
            })
            .collect(),
    }
}

/// Censor a whole [`TranscriptionResult`] against the built-in [`BASE_PROFANITY`] set.
///
/// This is the seam the engine (E10-S2) calls when `censor_profanity == true`: run the
/// transcript through it before returning, so profane words surface as bracketed
/// redactions across `text` / segments / words.
#[must_use]
pub fn censor_result(result: &TranscriptionResult) -> TranscriptionResult {
    censor_result_with(result, BASE_PROFANITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brackets_a_known_profane_token() {
        assert_eq!(censor_text("what the hell"), "what the [hell]");
    }

    #[test]
    fn is_case_insensitive_and_preserves_punctuation() {
        assert_eq!(censor_text("Damn!"), "[damn]!");
        assert_eq!(censor_text("Oh, SHIT."), "Oh, [shit].");
    }

    #[test]
    fn clean_transcript_is_unchanged() {
        let clean = "the quick brown fox jumps over the lazy dog";
        assert_eq!(censor_text(clean), clean);
    }

    #[test]
    fn does_not_censor_substrings() {
        // "ass" is in the set but must not censor "class" / "assassin".
        let s = "the class president is no assassin";
        assert_eq!(censor_text(s), s);
    }

    #[test]
    fn standalone_substring_word_is_censored() {
        assert_eq!(censor_text("move your ass now"), "move your [ass] now");
    }

    #[test]
    fn custom_word_set_is_honored() {
        let out = censor_text_with("foo bar baz", &["bar"]);
        assert_eq!(out, "foo [bar] baz");
    }

    #[test]
    fn censors_across_whole_result() {
        let r = TranscriptionResult {
            text: "what the hell".to_string(),
            language: Some("en-US".to_string()),
            words: vec![
                TranscriptionWord {
                    text: "what".to_string(),
                    start: Some(0.0),
                    end: Some(0.2),
                },
                TranscriptionWord {
                    text: "the".to_string(),
                    start: Some(0.2),
                    end: Some(0.3),
                },
                TranscriptionWord {
                    text: "hell".to_string(),
                    start: Some(0.3),
                    end: Some(0.5),
                },
            ],
            segments: vec![TranscriptionSegment {
                text: "what the hell".to_string(),
                start: 0.0,
                end: 0.5,
            }],
        };
        let c = censor_result(&r);
        assert_eq!(c.text, "what the [hell]");
        assert_eq!(c.segments[0].text, "what the [hell]");
        assert_eq!(c.words[2].text, "[hell]");
        // Timestamps + language untouched.
        assert_eq!(c.words[2].start, Some(0.3));
        assert_eq!(c.language.as_deref(), Some("en-US"));
    }

    #[test]
    fn clean_result_round_trips_unchanged() {
        let r = TranscriptionResult {
            text: "hello world".to_string(),
            language: Some("en-US".to_string()),
            words: vec![TranscriptionWord {
                text: "hello".to_string(),
                start: Some(0.0),
                end: Some(0.5),
            }],
            segments: vec![TranscriptionSegment {
                text: "hello world".to_string(),
                start: 0.0,
                end: 1.0,
            }],
        };
        assert_eq!(censor_result(&r), r);
    }
}
