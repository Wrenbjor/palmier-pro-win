//! # Transcript keyword search — `TranscriptSearch::search` (E11-S8)
//!
//! Exact keyword search over **cached** transcript segments, so spoken hits work
//! with no model download and 100% keyword recall. Verbatim behavioral port of the
//! macOS reference `Sources/PalmierPro/Transcription/TranscriptSearch.swift`.
//!
//! ## Algorithm (parity-critical — matches the reference exactly)
//!  - **Term split:** `query` is split on whitespace; each token has its **edge
//!    punctuation stripped** (`"budget,"` → `"budget"`); empty tokens are dropped.
//!    Reference: `split(whereSeparator: \.isWhitespace).map(trim(punctuation)).filter(!empty)`.
//!  - **Match:** a segment matches iff it contains **all** terms, compared
//!    **case-insensitively AND diacritic-insensitively** — the reference's Swift
//!    `text.range(of: term, options: [.caseInsensitive, .diacriticInsensitive])`.
//!    We reproduce that fold with Unicode **NFD decomposition + strip combining
//!    marks + lowercase** on both the segment text and each term, then a plain
//!    substring containment check on the folded strings (so "café" matches "cafe"
//!    and "US" matches "us").
//!  - **Order + cap:** hits are emitted in **asset order, then segment order** within
//!    each asset, capped at `limit` (reference default 20). The reference returns as
//!    soon as `hits.count >= limit`, so the cap is applied during iteration.
//!
//! ## Disk-only read (no transcription at query time)
//! Segments come from the **merged E10-S4** [`palmier_transcribe::TranscriptCache`]
//! (E11-S7 is subsumed by it — there is no second cache). [`search`](TranscriptSearch::search)
//! takes a `&TranscriptCache` and calls its disk-only reads
//! ([`has_cached_on_disk`](palmier_transcribe::TranscriptCache::has_cached_on_disk) /
//! [`transcript`](palmier_transcribe::TranscriptCache::transcript) with no range) which
//! **never transcribe** — an asset with no cached transcript on disk simply yields no
//! hits. Keyword search is therefore **always available** with no model download.
//!
//! ## `model_id` / `language` defaulting
//! The E10-S4 cache key folds in `model_id` + `language` (ruling #19), so a lookup
//! needs both. The query path here defaults to the bundled English model
//! ([`DEFAULT_MODEL_ID`] = `"ggml-small.en"`, [`DEFAULT_LANGUAGE`] = `"en"`). The
//! E11-S6 coordinator drives indexing and knows the real `(model, language)` an asset
//! was transcribed under; when it wires this in it should call
//! [`search_with`](TranscriptSearch::search_with) with those values. [`search`] is the
//! convenience default for the bundled-English common case.

use std::path::PathBuf;

use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

use palmier_transcribe::TranscriptCache;

/// Default whisper model id for the bundled English model — the `(model, language)`
/// pair the E10-S4 cache keys on. The E11-S6 coordinator passes the real values via
/// [`TranscriptSearch::search_with`]; this is the bundled-English fallback.
pub const DEFAULT_MODEL_ID: &str = "ggml-small.en";

/// Default language tag paired with [`DEFAULT_MODEL_ID`] (see above).
pub const DEFAULT_LANGUAGE: &str = "en";

/// Default result cap (reference `limit: Int = 20`).
pub const DEFAULT_LIMIT: usize = 20;

/// One transcript-segment hit, mirroring the reference `TranscriptSearch.Hit`.
///
/// Named [`TranscriptHit`] (not `Hit`) to coexist with the E11-S5 visual-search
/// [`crate::visual_search::Hit`] in the same crate.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptHit {
    /// Owning asset id (carried through from the input `(id, path)` pair).
    pub asset_id: String,
    /// Segment start time, source seconds (`TranscriptionSegment.start`).
    pub start: f64,
    /// Segment end time, source seconds (`TranscriptionSegment.end`).
    pub end: f64,
    /// The matched segment's text (model punctuation + casing preserved).
    pub text: String,
}

/// Exact keyword search over cached transcripts (reference `enum TranscriptSearch`).
pub struct TranscriptSearch;

impl TranscriptSearch {
    /// Search cached transcript segments for `query`, using the bundled-English
    /// defaults ([`DEFAULT_MODEL_ID`] / [`DEFAULT_LANGUAGE`]).
    ///
    /// `assets` is an ordered slice of `(asset_id, file_path)` pairs. Hits are returned
    /// in asset order then segment order, capped at `limit` (the reference default is
    /// [`DEFAULT_LIMIT`] = 20). Reads disk-only via `cache` — **no transcription is
    /// triggered**; an asset with no transcript cached on disk yields no hits.
    pub fn search(
        query: &str,
        assets: &[(String, PathBuf)],
        limit: usize,
        cache: &TranscriptCache,
    ) -> Vec<TranscriptHit> {
        Self::search_with(query, assets, limit, cache, DEFAULT_MODEL_ID, DEFAULT_LANGUAGE)
    }

    /// Like [`search`](Self::search) but with explicit `model_id` / `language` — the
    /// E11-S6 coordinator threads the real values an asset was transcribed under
    /// (the E10-S4 cache key folds them in, so a lookup needs both).
    pub fn search_with(
        query: &str,
        assets: &[(String, PathBuf)],
        limit: usize,
        cache: &TranscriptCache,
        model_id: &str,
        language: &str,
    ) -> Vec<TranscriptHit> {
        let terms = Self::terms(query);
        if terms.is_empty() {
            return Vec::new();
        }
        // Pre-fold each term once (NFD + strip combining marks + lowercase).
        let folded_terms: Vec<String> = terms.iter().map(|t| fold(t)).collect();

        let mut hits: Vec<TranscriptHit> = Vec::new();
        for (asset_id, file) in assets {
            // Disk-only guard then read — never transcribes (E10-S4 query path).
            if !cache.has_cached_on_disk(file, model_id, language) {
                continue;
            }
            let transcript = match cache.transcript(file, model_id, language, None) {
                Ok(Some(t)) => t,
                // A clean miss or an I/O/key-derivation error both yield no hits for
                // this asset (the reference `guard let … else { continue }`).
                Ok(None) | Err(_) => continue,
            };
            for segment in &transcript.segments {
                if matches(&segment.text, &folded_terms) {
                    hits.push(TranscriptHit {
                        asset_id: asset_id.clone(),
                        start: segment.start,
                        end: segment.end,
                        text: segment.text.clone(),
                    });
                    // Reference returns as soon as the cap is reached.
                    if hits.len() >= limit {
                        return hits;
                    }
                }
            }
        }
        hits
    }

    /// Split a query into search terms: split on whitespace, strip **edge**
    /// punctuation from each token, drop empties.
    ///
    /// Parity with the reference
    /// `split(whereSeparator: \.isWhitespace).map(trim(.punctuationCharacters)).filter(!isEmpty)`.
    /// Only leading/trailing punctuation is stripped — interior punctuation (e.g. an
    /// apostrophe in `don't`) is preserved, matching `trimmingCharacters(in:)`.
    pub fn terms(query: &str) -> Vec<String> {
        query
            .split_whitespace()
            .map(|tok| {
                tok.trim_matches(|c: char| c.is_ascii_punctuation() || is_unicode_punctuation(c))
            })
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .collect()
    }
}

/// `true` if `text` contains **all** `folded_terms` under the case- and
/// diacritic-insensitive fold (reference `terms.allSatisfy { text.range(of:) != nil }`).
fn matches(text: &str, folded_terms: &[String]) -> bool {
    let folded_text = fold(text);
    folded_terms.iter().all(|t| folded_text.contains(t.as_str()))
}

/// Case- and diacritic-insensitive fold of `s`, reproducing Swift's
/// `[.caseInsensitive, .diacriticInsensitive]` comparison:
///  - NFD-decompose so accented letters split into base char + combining marks,
///  - drop the combining marks (diacritic-insensitive: "café" → "cafe"),
///  - lowercase the survivors (case-insensitive: "US" → "us").
///
/// Substring containment on two folded strings then mirrors Swift's `range(of:)` under
/// those options. (Lowercasing after stripping marks is sufficient for the Latin/Greek/
/// Cyrillic transcripts this search targets; full Unicode case-folding is not required
/// for parity with the reference's options-based comparison.)
fn fold(s: &str) -> String {
    s.nfd()
        .filter(|&c| !is_combining_mark(c))
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Unicode punctuation predicate for edge-trimming, covering the non-ASCII punctuation
/// Swift's `.punctuationCharacters` set includes (e.g. `«` `»` `…` `—`). ASCII
/// punctuation is handled by [`char::is_ascii_punctuation`] at the call site.
fn is_unicode_punctuation(c: char) -> bool {
    matches!(
        c,
        '\u{00A1}' // ¡
        | '\u{00BF}' // ¿
        | '\u{2010}'..='\u{2027}' // hyphens, dashes, quotes, ellipsis …
        | '\u{2030}'..='\u{205E}' // misc punctuation
        | '\u{00AB}' // «
        | '\u{00BB}' // »
        | '\u{300C}'..='\u{300F}' // CJK brackets 「」『』
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_transcribe::{TranscriptionResult, TranscriptionSegment};
    use std::io::Write as _;

    fn seg(text: &str, start: f64, end: f64) -> TranscriptionSegment {
        TranscriptionSegment {
            text: text.to_string(),
            start,
            end,
        }
    }

    /// Build a transcript from `(text, start, end)` triples (no word-level times needed
    /// for keyword search — it matches on segment text only).
    fn transcript(segments: Vec<TranscriptionSegment>) -> TranscriptionResult {
        let text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        TranscriptionResult {
            text,
            language: Some("en".to_string()),
            words: Vec::new(),
            segments,
        }
    }

    /// Write `bytes` to a unique temp media file; return its path.
    fn temp_media(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "palmier-tsearch-media-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("media.bin");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        path
    }

    fn temp_cache_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "palmier-tsearch-cache-{}-{}",
            std::process::id(),
            name
        ))
    }

    /// A fresh cache pointed at a temp disk dir, with `result` stored under the
    /// bundled-English defaults for `file`.
    fn cache_with(name: &str, file: &PathBuf, result: &TranscriptionResult) -> TranscriptCache {
        let dir = temp_cache_dir(name);
        let _ = std::fs::remove_dir_all(&dir);
        let cache = TranscriptCache::with_directory(&dir);
        cache
            .store(file, DEFAULT_MODEL_ID, DEFAULT_LANGUAGE, result)
            .unwrap();
        cache
    }

    // ---- term splitting -----------------------------------------------------

    #[test]
    fn terms_strip_edge_punctuation_and_drop_empties() {
        // "budget," → "budget"; "..." → empty (dropped); interior apostrophe kept.
        let t = TranscriptSearch::terms("  the budget, ... don't  ");
        assert_eq!(t, vec!["the", "budget", "don't"]);
        // An all-punctuation / whitespace-only query yields no terms.
        assert!(TranscriptSearch::terms("  ,.!?  ").is_empty());
        assert!(TranscriptSearch::terms("").is_empty());
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let media = temp_media("emptyq", b"EMPTYQ");
        let cache = cache_with("emptyq", &media, &transcript(vec![seg("hello world", 0.0, 1.0)]));
        let assets = vec![("a".to_string(), media.clone())];
        assert!(TranscriptSearch::search("   ", &assets, DEFAULT_LIMIT, &cache).is_empty());
    }

    // ---- SM-12 spoken exit: 100% keyword recall + all-terms ------------------

    #[test]
    fn sm12_full_keyword_recall_all_terms() {
        // Transcript fixture: 6 segments. Query "budget meeting" must return EVERY
        // segment containing both "budget" AND "meeting" (case-insensitive), in order,
        // and ONLY those — 100% recall, no false positives.
        let segs = vec![
            seg("Welcome to the budget meeting", 0.0, 2.0), // budget + meeting ✓
            seg("The meeting starts now", 2.0, 4.0),        // meeting only ✗
            seg("Our budget is tight", 4.0, 6.0),           // budget only ✗
            seg("BUDGET MEETING agenda follows", 6.0, 8.0), // budget + meeting (caps) ✓
            seg("No relevant content here", 8.0, 10.0),     // neither ✗
            seg("A productive budget meeting indeed", 10.0, 12.0), // budget + meeting ✓
        ];
        let media = temp_media("recall", b"RECALL-FIXTURE");
        let cache = cache_with("recall", &media, &transcript(segs));
        let assets = vec![("asset1".to_string(), media.clone())];

        let hits = TranscriptSearch::search("budget meeting", &assets, DEFAULT_LIMIT, &cache);

        // Exactly the 3 segments containing BOTH terms, in segment order.
        assert_eq!(hits.len(), 3, "100% recall: every all-terms segment returned");
        assert_eq!(hits[0].text, "Welcome to the budget meeting");
        assert_eq!(hits[1].text, "BUDGET MEETING agenda follows");
        assert_eq!(hits[2].text, "A productive budget meeting indeed");
        // Times carried through from the segments.
        assert_eq!((hits[0].start, hits[0].end), (0.0, 2.0));
        assert_eq!(hits[0].asset_id, "asset1");
    }

    // ---- diacritic + case insensitivity (café/cafe, US/us) -------------------

    #[test]
    fn diacritic_insensitive_cafe() {
        let media = temp_media("cafe", b"CAFE-FIXTURE");
        let cache = cache_with(
            "cafe",
            &media,
            &transcript(vec![
                seg("We met at the café downtown", 0.0, 2.0),
                seg("No accent here either way", 2.0, 4.0),
            ]),
        );
        let assets = vec![("a".to_string(), media.clone())];

        // Query "cafe" (no accent) matches the segment with "café" (accented).
        let hits = TranscriptSearch::search("cafe", &assets, DEFAULT_LIMIT, &cache);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text, "We met at the café downtown");

        // And the reverse: query "café" (accented) against the same fold.
        let hits_rev = TranscriptSearch::search("café", &assets, DEFAULT_LIMIT, &cache);
        assert_eq!(hits_rev.len(), 1);
    }

    #[test]
    fn case_insensitive_us() {
        let media = temp_media("us", b"US-FIXTURE");
        let cache = cache_with(
            "us",
            &media,
            &transcript(vec![
                seg("The US economy grew", 0.0, 2.0),
                seg("between us friends", 2.0, 4.0),
            ]),
        );
        let assets = vec![("a".to_string(), media.clone())];

        // Query "us" (lowercase) matches both "US" (the country) and "us" (the pronoun).
        let hits = TranscriptSearch::search("us", &assets, DEFAULT_LIMIT, &cache);
        assert_eq!(hits.len(), 2);
    }

    // ---- asset/segment order + limit cap ------------------------------------

    #[test]
    fn results_in_asset_then_segment_order_capped_at_limit() {
        let m1 = temp_media("order1", b"ORDER-ASSET-1");
        let m2 = temp_media("order2", b"ORDER-ASSET-2");
        let t1 = transcript(vec![
            seg("alpha one", 0.0, 1.0),
            seg("alpha two", 1.0, 2.0),
            seg("alpha three", 2.0, 3.0),
        ]);
        let t2 = transcript(vec![
            seg("alpha four", 0.0, 1.0),
            seg("alpha five", 1.0, 2.0),
        ]);
        // Two assets share one disk cache dir; store both.
        let dir = temp_cache_dir("order");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = TranscriptCache::with_directory(&dir);
        cache.store(&m1, DEFAULT_MODEL_ID, DEFAULT_LANGUAGE, &t1).unwrap();
        cache.store(&m2, DEFAULT_MODEL_ID, DEFAULT_LANGUAGE, &t2).unwrap();

        let assets = vec![("a1".to_string(), m1.clone()), ("a2".to_string(), m2.clone())];

        // All five segments contain "alpha"; asset order (a1 then a2), segment order within.
        let all = TranscriptSearch::search("alpha", &assets, DEFAULT_LIMIT, &cache);
        assert_eq!(all.len(), 5);
        let order: Vec<&str> = all.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(
            order,
            vec!["alpha one", "alpha two", "alpha three", "alpha four", "alpha five"]
        );
        assert_eq!(all[0].asset_id, "a1");
        assert_eq!(all[3].asset_id, "a2");

        // limit=2 caps mid-first-asset, in order.
        let capped = TranscriptSearch::search("alpha", &assets, 2, &cache);
        assert_eq!(capped.len(), 2);
        assert_eq!(capped[0].text, "alpha one");
        assert_eq!(capped[1].text, "alpha two");
    }

    // ---- disk-only: uncached asset yields no hits, never transcribes ---------

    #[test]
    fn uncached_asset_yields_no_hits() {
        let cached_media = temp_media("diskon-cached", b"DISK-ONLY-CACHED");
        let uncached_media = temp_media("diskon-uncached", b"DISK-ONLY-UNCACHED");
        let cache = cache_with(
            "diskon",
            &cached_media,
            &transcript(vec![seg("budget meeting today", 0.0, 2.0)]),
        );

        // The cached asset hits; the uncached one (no disk transcript) yields nothing —
        // and the call never transcribes (the cache only reads disk).
        let assets = vec![
            ("uncached".to_string(), uncached_media.clone()),
            ("cached".to_string(), cached_media.clone()),
        ];
        let hits = TranscriptSearch::search("budget", &assets, DEFAULT_LIMIT, &cache);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].asset_id, "cached");
        assert!(!cache.has_cached_on_disk(&uncached_media, DEFAULT_MODEL_ID, DEFAULT_LANGUAGE));
    }
}
