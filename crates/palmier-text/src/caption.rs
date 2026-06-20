//! Caption phrase splitting + duration distribution + minimum-duration cascade
//! (FOUNDATION §11.1; story **E10-S5**). This is the **parity oracle** (SM-13):
//! a byte-exact port of the macOS reference `CaptionBuilder`
//! (`Sources/PalmierPro/MediaPanel/CaptionsTab/CaptionBuilder.swift`), driven by
//! the 14 reference unit tests in `Tests/.../CaptionBuilderTests.swift`. Any
//! off-by-one here changes visible caption timing, so the algorithm is ported
//! verbatim from `docs/reference/transcription.md` §C.
//!
//! ## Scope (E10-S5 only)
//! This module owns the **pure phrase algorithm** — [`phrases`] and its private
//! helpers ([`split`], `break_once`, `break_on`, `break_at_mid_word`,
//! [`distribute`], [`enforce_min_duration`]). The `specs(...)` mapping
//! (phrase → `TextClipSpec`) and the `generate_captions` orchestration are
//! **E10-S6** and intentionally not implemented here.
//!
//! ## Grapheme-aware counts (parity trap)
//! Swift's `String.count` and `Character` are **extended grapheme clusters**, not
//! Unicode scalars or bytes. We use [`unicode_segmentation`] so that `text.count`
//! in the distribution weighting matches the reference exactly. Using `char`
//! (scalar) or byte counts would silently skew the proportional time split for any
//! text containing multi-scalar graphemes (emoji, combining marks, ZWJ sequences).

use unicode_segmentation::UnicodeSegmentation;

/// Reference `minDisplayDuration` (`AppTheme.Caption`, seconds). The default
/// minimum on-screen duration enforced by [`enforce_min_duration`] in the full
/// orchestration (E10-S6). Re-exported here as the parity constant.
pub const MIN_DISPLAY_DURATION: f64 = 0.7;

/// The `AppTheme.Caption` constant block (E10-S6). Ported verbatim from the macOS
/// reference `UI/AppTheme.swift` `enum Caption` + the
/// `ComponentSize.captionPreviewMaxTextWidthRatio` token. These drive the caption
/// **style config** (font-size clamps), the **placement** controls (position bounds,
/// center-snap), and the orchestration's `caption_line_fits` width gate. Carried
/// here so the algorithm crate that owns caption phrasing also owns its constants
/// (the frontend Captions tab mirrors these — `src-ui/.../CaptionsTab.tsx`).
pub mod caption_theme {
    /// Minimum on-screen duration per phrase, seconds (`AppTheme.Caption.minDisplayDuration`).
    pub const MIN_DISPLAY_DURATION: f64 = super::MIN_DISPLAY_DURATION;
    /// Default caption font size, pt (`defaultFontSize`).
    pub const DEFAULT_FONT_SIZE: f64 = 48.0;
    /// Minimum caption font size, pt (`minFontSize`).
    pub const MIN_FONT_SIZE: f64 = 12.0;
    /// Maximum caption font size, pt (`maxFontSize`).
    pub const MAX_FONT_SIZE: f64 = 300.0;
    /// Minimum normalized placement coordinate (`minPosition`).
    pub const MIN_POSITION: f64 = 0.0;
    /// Maximum normalized placement coordinate (`maxPosition`).
    pub const MAX_POSITION: f64 = 1.0;
    /// The value placement snaps to (`centerSnapValue` — the canvas center axis).
    pub const CENTER_SNAP_VALUE: f64 = 0.5;
    /// Snap threshold: placement snaps to [`CENTER_SNAP_VALUE`] within this distance
    /// (`centerSnapThreshold`).
    pub const CENTER_SNAP_THRESHOLD: f64 = 0.02;
    /// Default caption center, normalized `(x, y)` (`defaultCenter` — lower third).
    pub const DEFAULT_CENTER: (f64, f64) = (0.5, 0.9);
    /// Caption preview max text-width ratio: a caption line "fits" when its natural
    /// width is `<= timeline.width * this` (`ComponentSize.captionPreviewMaxTextWidthRatio`).
    pub const CAPTION_PREVIEW_MAX_TEXT_WIDTH_RATIO: f64 = 0.9;
}

/// A transcript segment: the raw text plus its `[start, end]` window in seconds.
///
/// The phrase algorithm consumes only these three fields; this is the minimal
/// input shape (the richer serde `TranscriptionSegment` from E10-S1 can be
/// converted into one of these at the call site in E10-S6).
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Transcribed text for this segment.
    pub text: String,
    /// Segment start time, seconds.
    pub start: f64,
    /// Segment end time, seconds.
    pub end: f64,
}

impl Segment {
    /// Construct a segment from its text and `[start, end]` window (seconds).
    pub fn new(text: impl Into<String>, start: f64, end: f64) -> Self {
        Self { text: text.into(), start, end }
    }
}

/// A screen-ready caption phrase: display text plus its `[start, end]` timing in
/// seconds. Mirrors the reference `CaptionBuilder.Phrase`.
#[derive(Debug, Clone, PartialEq)]
pub struct Phrase {
    /// Display text of the phrase.
    pub text: String,
    /// Phrase start time, seconds.
    pub start: f64,
    /// Phrase end time, seconds.
    pub end: f64,
}

impl Phrase {
    /// Construct a phrase from its text and `[start, end]` window (seconds).
    pub fn new(text: impl Into<String>, start: f64, end: f64) -> Self {
        Self { text: text.into(), start, end }
    }
}

/// Caption text casing (ruling #18: **auto / upper / lower only** — the reference
/// has no title-case, so "title" is rejected). Kept here as the small enum the
/// E10-S6 casing step parses; lives with the caption algorithm it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionCase {
    /// Leave the text as transcribed.
    Auto,
    /// Uppercase the caption text.
    Upper,
    /// Lowercase the caption text.
    Lower,
}

impl CaptionCase {
    /// Parse a casing token, rejecting `"title"` (ruling #18 — no title-case in
    /// the reference). Returns `None` for any unrecognized value (including
    /// `"title"`), so callers fall back to their default.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "upper" => Some(Self::Upper),
            "lower" => Some(Self::Lower),
            // "title" is explicitly NOT supported (ruling #18).
            _ => None,
        }
    }
}

/// Grapheme-cluster count of `text` (Swift `String.count` / `Character` parity).
///
/// NOT a `char`/scalar or byte count — see the module docs. `true` selects
/// **extended** grapheme clusters (matching Swift's `Character`).
fn grapheme_count(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Trim ASCII/Unicode whitespace the way Swift's
/// `trimmingCharacters(in: .whitespaces)` does for these inputs (leading/trailing
/// spaces and tabs). The reference set `.whitespaces` is horizontal whitespace
/// (space + tab), which `char::is_whitespace`-based `trim` covers as a superset;
/// for the caption inputs (space-separated text) the behavior is identical.
fn trimmed(text: &str) -> &str {
    text.trim()
}

/// Split a transcript segment into screen-ready phrases and time them.
///
/// `phrases = enforce_min_duration(distribute(split(text, fits), start, end), min_duration)`.
///
/// - `fits` decides whether a candidate piece is short enough to display as one
///   phrase (the reference passes a width/length predicate, e.g. `|s| s.count <= 6`).
/// - `min_duration` is the per-phrase floor applied by [`enforce_min_duration`].
pub fn phrases(segment: &Segment, fits: impl Fn(&str) -> bool, min_duration: f64) -> Vec<Phrase> {
    let pieces = split(&segment.text, &fits);
    let timed = distribute(&pieces, segment.start, segment.end);
    enforce_min_duration(timed, min_duration)
}

/// Recursively break `text` into pieces that each satisfy `fits`.
///
/// - Trim whitespace; empty → `[]`.
/// - If `fits(t)` → `[t]` (done).
/// - Else `parts = break_once(t)`; if `parts.len() <= 1` → `[t]` (an unbreakable
///   over-long single word is kept as-is).
/// - Else recurse into each part and flatten.
fn split(text: &str, fits: &impl Fn(&str) -> bool) -> Vec<String> {
    let t = trimmed(text);
    if t.is_empty() {
        return Vec::new();
    }
    if fits(t) {
        return vec![t.to_string()];
    }
    let parts = break_once(t);
    if parts.len() <= 1 {
        // A single over-long word: keep it.
        return vec![t.to_string()];
    }
    parts.iter().flat_map(|p| split(p, fits)).collect()
}

/// Break once at the best boundary present: sentence (`.!?`), then clause
/// (`,;:`), then the midpoint word.
fn break_once(text: &str) -> Vec<String> {
    if let Some(pieces) = break_on(text, ".!?") {
        return pieces;
    }
    if let Some(pieces) = break_on(text, ",;:") {
        return pieces;
    }
    break_at_mid_word(text)
}

/// Split `text` **after** a delimiter, but only when the next character is a space
/// or end-of-string — so "U.S." and "3.14" stay intact.
///
/// Walk the characters accumulating `current`; when `current`'s last char is in
/// `delimiters` AND the next is a break point, push the trimmed `current` and
/// reset. Push the trailing `tail`. Returns the pieces **only if** more than one
/// was produced, else `None`.
///
/// Iterates over Unicode scalars (`char`); delimiters here are all ASCII
/// punctuation, so scalar iteration matches the reference `Array(text)`
/// (`[Character]`) behavior for these inputs while being unambiguous about the
/// "next char is a space" check.
fn break_on(text: &str, delimiters: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut pieces: Vec<String> = Vec::new();
    let mut current = String::new();
    for (i, &c) in chars.iter().enumerate() {
        current.push(c);
        let next_is_break = i + 1 >= chars.len() || chars[i + 1] == ' ';
        if delimiters.contains(c) && next_is_break {
            let piece = current.trim();
            if !piece.is_empty() {
                pieces.push(piece.to_string());
            }
            current.clear();
        }
    }
    let tail = current.trim();
    if !tail.is_empty() {
        pieces.push(tail.to_string());
    }
    if pieces.len() > 1 { Some(pieces) } else { None }
}

/// Split on spaces into words; if `<= 1` word → `[text]`; else split at
/// `mid = words.len() / 2` (integer division) and rejoin each half with spaces.
fn break_at_mid_word(text: &str) -> Vec<String> {
    // Swift `text.split(separator: " ")` drops empty subsequences (collapses
    // runs of spaces and ignores leading/trailing ones); `split_whitespace`
    // would also catch tabs, but inputs here are space-delimited so the result
    // is identical. Use an explicit ' ' filter to mirror the reference exactly.
    let words: Vec<&str> = text.split(' ').filter(|w| !w.is_empty()).collect();
    if words.len() <= 1 {
        return vec![text.to_string()];
    }
    let mid = words.len() / 2;
    vec![words[..mid].join(" "), words[mid..].join(" ")]
}

/// Share the segment's time across pieces proportionally by **grapheme count**,
/// back to back.
///
/// - `total = Σ max(text.count, 1)`; `span = max(end - start, 0)`; `t = start`.
/// - Each piece: `dur = span * max(text.count, 1) / total`; phrase `(text, t, t + dur)`;
///   then `t += dur`.
///
/// `text.count` is the grapheme-cluster count (Swift `Character` parity), NOT a
/// byte or scalar count.
fn distribute(texts: &[String], start: f64, end: f64) -> Vec<Phrase> {
    if texts.is_empty() {
        return Vec::new();
    }
    let total: usize = texts.iter().map(|t| grapheme_count(t).max(1)).sum();
    let span = (end - start).max(0.0);
    let mut phrases = Vec::with_capacity(texts.len());
    let mut t = start;
    for text in texts {
        let weight = grapheme_count(text).max(1);
        let dur = span * (weight as f64) / (total as f64);
        phrases.push(Phrase::new(text.clone(), t, t + dur));
        t += dur;
    }
    phrases
}

/// Give each phrase a floor duration, shifting later phrases so they don't
/// overlap. Forward pass over indices:
///
/// - If `phrase[i].end - phrase[i].start < min_duration` →
///   `phrase[i].end = phrase[i].start + min_duration`.
/// - If a next phrase exists and `phrase[i+1].start < phrase[i].end`, add
///   `shift = phrase[i].end - phrase[i+1].start` to **both** `phrase[i+1].start`
///   and `.end`. This cascades and **can push the final end past the segment end
///   — it is deliberately NOT clamped back**.
fn enforce_min_duration(mut phrases: Vec<Phrase>, min_duration: f64) -> Vec<Phrase> {
    let n = phrases.len();
    for i in 0..n {
        if phrases[i].end - phrases[i].start < min_duration {
            phrases[i].end = phrases[i].start + min_duration;
        }
        if i + 1 < n && phrases[i + 1].start < phrases[i].end {
            let shift = phrases[i].end - phrases[i + 1].start;
            phrases[i + 1].start += shift;
            phrases[i + 1].end += shift;
        }
    }
    phrases
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(text: &str, start: f64, end: f64) -> Segment {
        Segment::new(text, start, end)
    }

    // ---- Ported verbatim from CaptionBuilderTests.swift (phrase algorithm) ----
    // These mirror the reference `@Test` cases 1:1 (same inputs, same `fits`
    // predicate, same expected text/start/end). The remaining `specs(...)` tests
    // in the reference suite belong to E10-S6 and are ported there.

    #[test]
    fn keeps_segment_whole_when_it_fits() {
        let p = phrases(&segment("Hello there", 1.0, 2.0), |_| true, 0.0);
        assert_eq!(p, vec![Phrase::new("Hello there", 1.0, 2.0)]);
    }

    #[test]
    fn splits_at_sentence_boundary() {
        let p = phrases(&segment("One. Two.", 0.0, 8.0), |s| grapheme_count(s) <= 5, 0.0);
        assert_eq!(p.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["One.", "Two."]);
        assert_eq!(p.iter().map(|x| x.start).collect::<Vec<_>>(), [0.0, 4.0]);
        assert_eq!(p.iter().map(|x| x.end).collect::<Vec<_>>(), [4.0, 8.0]);
    }

    #[test]
    fn splits_at_clause_when_no_sentence() {
        let p = phrases(&segment("alpha, beta", 0.0, 2.0), |s| grapheme_count(s) <= 6, 0.0);
        assert_eq!(p.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["alpha,", "beta"]);
    }

    #[test]
    fn splits_at_mid_word_when_no_punctuation() {
        let p = phrases(&segment("a b c d", 0.0, 4.0), |s| grapheme_count(s) <= 3, 0.0);
        assert_eq!(p.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["a b", "c d"]);
    }

    #[test]
    fn keeps_punctuated_tokens_intact() {
        let p = phrases(&segment("U.S. army here", 0.0, 6.0), |s| grapheme_count(s) <= 6, 0.0);
        assert_eq!(p.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["U.S.", "army", "here"]);
    }

    #[test]
    fn distributes_time_by_character_count() {
        let p = phrases(&segment("aaaa bb", 0.0, 6.0), |s| grapheme_count(s) <= 4, 0.0);
        assert_eq!(p.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["aaaa", "bb"]);
        assert_eq!(p.iter().map(|x| x.start).collect::<Vec<_>>(), [0.0, 4.0]);
        assert_eq!(p.iter().map(|x| x.end).collect::<Vec<_>>(), [4.0, 6.0]);
    }

    #[test]
    fn enforces_minimum_duration_and_shifts() {
        let p = phrases(&segment("aa bbbb", 0.0, 6.0), |s| grapheme_count(s) <= 4, 3.0);
        assert_eq!(p.iter().map(|x| x.start).collect::<Vec<_>>(), [0.0, 3.0]);
        assert_eq!(p.iter().map(|x| x.end).collect::<Vec<_>>(), [3.0, 7.0]);
    }

    #[test]
    fn keeps_overlong_single_word() {
        let p = phrases(&segment("supercalifragilistic", 0.0, 1.0), |_| false, 0.0);
        assert_eq!(p.iter().map(|x| x.text.as_str()).collect::<Vec<_>>(), ["supercalifragilistic"]);
    }

    // ---- E10-S5-specific guards (ruling #18 + grapheme parity) ----

    #[test]
    fn caption_case_rejects_title() {
        assert_eq!(CaptionCase::parse("auto"), Some(CaptionCase::Auto));
        assert_eq!(CaptionCase::parse("upper"), Some(CaptionCase::Upper));
        assert_eq!(CaptionCase::parse("lower"), Some(CaptionCase::Lower));
        // Ruling #18: no title-case in the reference.
        assert_eq!(CaptionCase::parse("title"), None);
        assert_eq!(CaptionCase::parse("Title"), None);
        assert_eq!(CaptionCase::parse(""), None);
    }

    #[test]
    fn grapheme_count_is_cluster_aware() {
        // A flag emoji is one grapheme cluster but multiple scalars/bytes.
        // Distribution must weight by clusters, not scalars/bytes.
        let flag = "\u{1F1FA}\u{1F1F8}"; // 🇺🇸 — 1 grapheme, 2 scalars, 8 bytes.
        assert_eq!(grapheme_count(flag), 1);
        assert_eq!(flag.chars().count(), 2);
        assert_eq!(flag.len(), 8);
    }
}
