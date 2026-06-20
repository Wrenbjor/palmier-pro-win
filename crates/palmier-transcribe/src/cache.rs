//! Disk + memory transcript cache (E10-S4).
//!
//! Clean-room parity port of the macOS reference
//! `Sources/PalmierPro/Transcription/TranscriptCache.swift` (an `actor`). Here it is
//! a `tokio`-friendly singleton: a process-wide [`TranscriptCache::shared`]
//! (`OnceCell`) whose memory map is guarded by a `Mutex`, plus a JSON disk tier.
//!
//! Two behaviors are ported **verbatim** because the rest of the pipeline depends on
//! them:
//!  - the windowed [`filter`](TranscriptCache::filter) overlap predicates
//!    (`end > lo && start < hi`; words need *both* timestamps present), and
//!  - the memory cap: **4 entries, clear-all on overflow** (NOT LRU — reference
//!    `if memory.count >= memoryMax { memory.removeAll() }`).
//!
//! ## Cache-key DEVIATION from the reference (ruling #19 / FOUNDATION §6.9)
//! The reference keys on **file identity** — `prefix32(sha256("<path>|<mtime>|<size>"))`
//! — and does NOT fold in model or language. We deliberately adopt FOUNDATION's
//! **content** key instead: `sha256(file_content) + model_id + language`. Rationale:
//!  - `mtime` produces false cache hits (touch-without-edit) and false misses (copy);
//!    a content hash is exact.
//!  - a transcript is only valid for the `(model, language)` it was produced under, so
//!    both MUST invalidate the key — the reference's omission is a latent bug across
//!    model swaps.
//!
//! ## First-N-MB hashing cutoff
//! Hashing the *entire* file content of a 25-min recording on every cache lookup is
//! wasteful (hundreds of MB → GBs for video). We hash only the **first
//! [`HASH_PREFIX_BYTES`] (16 MiB)** of the file, mixed with the byte length, the
//! `model_id`, and the `language`. 16 MiB comfortably spans container headers, moov
//! atoms, and the leading media payload — enough that two genuinely different sources
//! diverge, while keeping lookups O(16 MiB) regardless of asset size. This is a
//! documented decision (full-content vs first-N): we chose **first-N = 16 MiB** plus
//! the exact byte length (so same-prefix/different-length files still differ).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use sha2::{Digest, Sha256};
use tokio::sync::OnceCell;

use crate::model::{TranscriptionResult, TranscriptionSegment, TranscriptionWord};

/// Memory tier cap. On the 5th distinct insert the whole map is cleared (reference
/// `memoryMax`; clear-all-on-overflow, **not** LRU eviction — parity-critical).
const MEMORY_MAX: usize = 4;

/// First-N-MB hashing cutoff (see module docs): hash only the leading 16 MiB of file
/// content into the cache key, not the whole asset. Mixed with the file's byte length
/// so two files sharing a 16 MiB prefix but differing in length still key apart.
pub const HASH_PREFIX_BYTES: u64 = 16 * 1024 * 1024;

/// A `tokio`-friendly disk + memory cache for **full-file** transcripts.
///
/// Windowed requests are served by [`TranscriptCache::filter`]ing a cached full
/// transcript — a windowed request never re-transcribes when a full transcript exists
/// (re-transcribing diverges timestamps and the dominant-track word logic).
///
/// Reference: `TranscriptCache` actor. The process-wide instance is
/// [`TranscriptCache::shared`]; tests use [`TranscriptCache::with_directory`] to point
/// the disk tier at a temp dir.
#[derive(Debug)]
pub struct TranscriptCache {
    /// Memory tier: `key -> result`. Cleared wholesale when it would exceed
    /// [`MEMORY_MAX`]. Guarded by a `Mutex` so `shared` can be a `&'static` singleton
    /// usable from async callers without holding a lock across `.await`.
    memory: Mutex<HashMap<String, TranscriptionResult>>,
    /// Disk tier directory: `<caches>/<subsystem>/Transcripts/`.
    directory: PathBuf,
}

static SHARED: OnceCell<TranscriptCache> = OnceCell::const_new();

impl TranscriptCache {
    /// Process-wide singleton (reference `TranscriptCache.shared`), disk tier at
    /// [`TranscriptCache::default_directory`]. Falls back to a temp-dir-rooted cache if
    /// the OS exposes no cache dir, so the singleton is always constructible.
    pub async fn shared() -> &'static TranscriptCache {
        SHARED
            .get_or_init(|| async {
                let directory = Self::default_directory()
                    .unwrap_or_else(|| std::env::temp_dir().join("PalmierProWin/Transcripts"));
                Self::with_directory(directory)
            })
            .await
    }

    /// Default disk cache directory, resolved per FOUNDATION logging paths:
    /// `%LOCALAPPDATA%\PalmierProWin\Transcripts\` on Windows (`dirs::cache_dir()`),
    /// `~/.cache/PalmierProWin/Transcripts/` on Linux. Returns `None` only if the OS
    /// exposes no cache dir.
    ///
    /// (Reference uses `.cachesDirectory/<subsystem>/Transcripts`; `<subsystem>` =
    /// `PalmierProWin`. `dirs::cache_dir()` maps to `%LOCALAPPDATA%` on Windows and
    /// `~/.cache` on Linux, matching the story's FOUNDATION-paths requirement.)
    pub fn default_directory() -> Option<PathBuf> {
        Some(
            dirs::cache_dir()?
                .join("PalmierProWin")
                .join("Transcripts"),
        )
    }

    /// Build a cache whose disk tier reads/writes under `directory` (tests / non-default
    /// roots). The memory tier starts empty.
    pub fn with_directory(directory: impl Into<PathBuf>) -> Self {
        Self {
            memory: Mutex::new(HashMap::new()),
            directory: directory.into(),
        }
    }

    /// The disk directory this cache reads/writes.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// On-disk path for a key: `<directory>/<key>.json` (reference `diskURL`).
    pub fn disk_path(&self, key: &str) -> PathBuf {
        self.directory.join(format!("{key}.json"))
    }

    /// Look up a **full** transcript for `(file, model_id, language)`, optionally
    /// filtered to a `range` (source seconds, inclusive).
    ///
    /// Parity with the reference `transcript(for:isVideo:range:)`, restructured around
    /// ruling #19's content key:
    ///  - If a full transcript is cached (memory then disk), return it — filtered to
    ///    `range` via [`TranscriptCache::filter`] when a range is given, else whole.
    ///    **A windowed hit NEVER re-transcribes.**
    ///  - On a miss this returns `None`; the caller (E10-S2 engine) transcribes the
    ///    full file and calls [`TranscriptCache::store`] so future windowed calls are
    ///    served by filtering. (Only full-file transcripts are cached — a windowed miss
    ///    does not produce a cacheable artifact.)
    ///
    /// `language` is the resolved BCP-47 tag the transcript was produced under (folded
    /// into the key so a language switch invalidates).
    ///
    /// Returns `Err` only on an I/O/key-derivation failure reading the source file for
    /// hashing; a clean miss is `Ok(None)`.
    pub fn transcript(
        &self,
        file: &Path,
        model_id: &str,
        language: &str,
        range: Option<&std::ops::RangeInclusive<f64>>,
    ) -> std::io::Result<Option<TranscriptionResult>> {
        let key = Self::key(file, model_id, language)?;
        match self.cached(&key) {
            Some(full) => Ok(Some(match range {
                Some(r) => Self::filter(&full, r),
                None => full,
            })),
            None => Ok(None),
        }
    }

    /// Cache a freshly produced **full** transcript under `(file, model_id, language)`.
    /// Writes both tiers (memory + JSON disk). Mirrors the reference `store`.
    ///
    /// Only ever call this with a *full-file* transcript (never a windowed one) — the
    /// invariant the windowed-filter path relies on.
    pub fn store(
        &self,
        file: &Path,
        model_id: &str,
        language: &str,
        result: &TranscriptionResult,
    ) -> std::io::Result<()> {
        let key = Self::key(file, model_id, language)?;
        self.remember(&key, result.clone());
        std::fs::create_dir_all(&self.directory)?;
        let json = serde_json::to_vec(result)?;
        std::fs::write(self.disk_path(&key), json)?;
        Ok(())
    }

    /// Filter a cached **full** transcript down to a source-seconds `range`.
    ///
    /// **Ported verbatim** from the reference `filter(_:to:)` (overlap predicates are
    /// parity-critical):
    ///  - keep segments where `end > lo && start < hi`,
    ///  - keep words that have **both** timestamps present **and** `end > lo && start < hi`
    ///    (a word missing either timestamp is dropped),
    ///  - `text = segments.map(text).joined(" ")` (rebuilt from the kept segments, not
    ///    the original flattened text).
    ///
    /// `range` is `lo..=hi` inclusive; the predicates use strict `>`/`<` exactly as the
    /// reference does (`$0.end > range.lowerBound && $0.start < range.upperBound`).
    #[must_use]
    pub fn filter(
        r: &TranscriptionResult,
        range: &std::ops::RangeInclusive<f64>,
    ) -> TranscriptionResult {
        let lo = *range.start();
        let hi = *range.end();

        let segments: Vec<TranscriptionSegment> = r
            .segments
            .iter()
            .filter(|s| s.end > lo && s.start < hi)
            .cloned()
            .collect();

        let words: Vec<TranscriptionWord> = r
            .words
            .iter()
            .filter(|w| match (w.start, w.end) {
                (Some(s), Some(e)) => e > lo && s < hi,
                // A word missing either timestamp is dropped (reference `guard let`).
                _ => false,
            })
            .cloned()
            .collect();

        let text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ");

        TranscriptionResult {
            text,
            language: r.language.clone(),
            words,
            segments,
        }
    }

    /// Memory-then-disk read. A disk hit is promoted into memory (reference `cached`).
    fn cached(&self, key: &str) -> Option<TranscriptionResult> {
        if let Some(r) = self.memory.lock().expect("cache mutex").get(key).cloned() {
            return Some(r);
        }
        let data = std::fs::read(self.disk_path(key)).ok()?;
        let r: TranscriptionResult = serde_json::from_slice(&data).ok()?;
        self.remember(key, r.clone());
        Some(r)
    }

    /// Insert into the memory tier, clearing the whole map first if it is already at
    /// [`MEMORY_MAX`] (reference: `if memory.count >= memoryMax { memory.removeAll() }`
    /// — clear-all, NOT LRU).
    fn remember(&self, key: &str, result: TranscriptionResult) {
        let mut mem = self.memory.lock().expect("cache mutex");
        if mem.len() >= MEMORY_MAX && !mem.contains_key(key) {
            mem.clear();
        }
        mem.insert(key.to_string(), result);
    }

    /// `true` if a full transcript for `(file, model_id, language)` exists on disk.
    pub fn has_cached_on_disk(&self, file: &Path, model_id: &str, language: &str) -> bool {
        match Self::key(file, model_id, language) {
            Ok(key) => self.disk_path(&key).exists(),
            Err(_) => false,
        }
    }

    /// Derive the cache key: `sha256( first-N-MB(file_content) || len || model_id || language )`
    /// rendered as lowercase hex (ruling #19 / FOUNDATION §6.9 content key — **not** the
    /// reference `path|mtime|size`). See module docs for the N=16 MiB cutoff rationale.
    ///
    /// The byte length is folded in alongside the prefix so two files sharing a 16 MiB
    /// prefix but differing in total length still key apart. `model_id` and `language`
    /// are appended with explicit separators so a model or language switch invalidates.
    fn key(file: &Path, model_id: &str, language: &str) -> std::io::Result<String> {
        let f = std::fs::File::open(file)?;
        let len = f.metadata()?.len();

        let mut hasher = Sha256::new();
        // Hash up to the first HASH_PREFIX_BYTES of content.
        let mut limited = f.take(HASH_PREFIX_BYTES);
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = limited.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        // Mix in length + model + language with separators that can't appear in the
        // hashed content boundary (length is a fixed-width domain separator already).
        hasher.update(b"|len|");
        hasher.update(len.to_le_bytes());
        hasher.update(b"|model|");
        hasher.update(model_id.as_bytes());
        hasher.update(b"|lang|");
        hasher.update(language.as_bytes());

        Ok(hex_lower(&hasher.finalize()))
    }
}

/// Lowercase-hex encode a byte slice (reference `String(format: "%02x", …)`).
fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn word(text: &str, start: Option<f64>, end: Option<f64>) -> TranscriptionWord {
        TranscriptionWord {
            text: text.to_string(),
            start,
            end,
        }
    }

    fn seg(text: &str, start: f64, end: f64) -> TranscriptionSegment {
        TranscriptionSegment {
            text: text.to_string(),
            start,
            end,
        }
    }

    /// A full transcript spanning 0..6 s with three segments and word-level times.
    fn full() -> TranscriptionResult {
        TranscriptionResult {
            text: "one two three".to_string(),
            language: Some("en-US".to_string()),
            words: vec![
                word("one", Some(0.0), Some(1.0)),
                word("two", Some(2.0), Some(3.0)),
                // A word with no aligned time range — must be dropped by filter.
                word("untimed", None, None),
                word("three", Some(4.0), Some(5.0)),
            ],
            segments: vec![
                seg("one", 0.0, 1.5),
                seg("two", 2.0, 3.5),
                seg("three", 4.0, 5.5),
            ],
        }
    }

    /// Write `bytes` to a unique temp file and return its path (caller is the test;
    /// the OS temp dir is fine for these — no cleanup needed for the assertions).
    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "palmier-cache-test-{}-{}",
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
            "palmier-cache-disk-{}-{}",
            std::process::id(),
            name
        ))
    }

    // ---- filter (windowed) -------------------------------------------------

    #[test]
    fn filter_keeps_overlapping_segments_and_timed_words() {
        let r = full();
        // Window 2.0..=4.0: overlaps seg "two" (2.0..3.5) and seg "three" (4.0..5.5)?
        //   seg "one"  : end 1.5 > 2.0? no  -> dropped
        //   seg "two"  : end 3.5 > 2.0 && start 2.0 < 4.0 -> kept
        //   seg "three": end 5.5 > 2.0 && start 4.0 < 4.0? start<hi is 4.0<4.0=false -> dropped
        let out = TranscriptCache::filter(&r, &(2.0..=4.0));
        assert_eq!(out.segments.len(), 1);
        assert_eq!(out.segments[0].text, "two");
        // text is rebuilt from kept segments joined by a single space.
        assert_eq!(out.text, "two");

        // words: "one"(0..1) dropped, "two"(2..3) kept, "untimed"(None) dropped,
        //        "three"(4..5) -> start 4.0 < 4.0 false -> dropped.
        assert_eq!(out.words.len(), 1);
        assert_eq!(out.words[0].text, "two");
        // language carried through.
        assert_eq!(out.language.as_deref(), Some("en-US"));
    }

    #[test]
    fn filter_drops_words_missing_either_timestamp() {
        let r = full();
        // A wide window keeps every timed segment/word but still drops the untimed word.
        let out = TranscriptCache::filter(&r, &(0.0..=10.0));
        assert_eq!(out.segments.len(), 3);
        assert!(out.words.iter().all(|w| w.text != "untimed"));
        assert_eq!(out.words.len(), 3);
        assert_eq!(out.text, "one two three");
    }

    // ---- windowed call serves from a cached full transcript, no re-transcribe ----

    #[test]
    fn windowed_request_filters_cached_full_no_retranscribe() {
        // Distinct content so the key is unique to this test.
        let media = temp_file("windowed", b"WINDOWED-REQUEST-MEDIA-PAYLOAD");
        let cache = TranscriptCache::with_directory(temp_cache_dir("windowed"));
        let _ = std::fs::remove_dir_all(cache.directory());

        // No transcript yet -> miss (Ok(None)). The engine would transcribe here; we do
        // NOT, proving the cache never invents a transcript on a miss.
        let miss = cache
            .transcript(&media, "small.en", "en-US", Some(&(2.0..=4.0)))
            .unwrap();
        assert!(miss.is_none());

        // Store the FULL transcript (what the engine does after a full-file run).
        cache.store(&media, "small.en", "en-US", &full()).unwrap();

        // A windowed request now returns the FILTERED subset, served from cache.
        let hit = cache
            .transcript(&media, "small.en", "en-US", Some(&(2.0..=4.0)))
            .unwrap()
            .expect("windowed hit served from cached full transcript");
        // Exactly the filtered subset (== filter(full, range)), proving no re-transcribe.
        assert_eq!(hit, TranscriptCache::filter(&full(), &(2.0..=4.0)));
        assert_eq!(hit.segments.len(), 1);
        assert_eq!(hit.segments[0].text, "two");

        // A full (no-range) request returns the whole cached transcript unchanged.
        let whole = cache
            .transcript(&media, "small.en", "en-US", None)
            .unwrap()
            .expect("full hit");
        assert_eq!(whole, full());
    }

    // ---- disk round-trip ----------------------------------------------------

    #[test]
    fn disk_round_trip_survives_a_fresh_cache_instance() {
        let media = temp_file("diskrt", b"DISK-ROUNDTRIP-MEDIA");
        let dir = temp_cache_dir("diskrt");
        let _ = std::fs::remove_dir_all(&dir);

        // Store via one cache instance (writes JSON to disk).
        {
            let cache = TranscriptCache::with_directory(&dir);
            cache.store(&media, "small.en", "en-US", &full()).unwrap();
            assert!(cache.has_cached_on_disk(&media, "small.en", "en-US"));
        }

        // A brand-new instance (empty memory tier) must read it back from disk byte-equal.
        let fresh = TranscriptCache::with_directory(&dir);
        let back = fresh
            .transcript(&media, "small.en", "en-US", None)
            .unwrap()
            .expect("disk hit on a fresh cache instance");
        assert_eq!(back, full());
    }

    // ---- key invalidation: model + language fold in ------------------------

    #[test]
    fn key_changes_with_model_and_language() {
        let media = temp_file("keyinval", b"KEY-INVALIDATION-MEDIA");
        let dir = temp_cache_dir("keyinval");
        let _ = std::fs::remove_dir_all(&dir);
        let cache = TranscriptCache::with_directory(&dir);

        cache.store(&media, "small.en", "en-US", &full()).unwrap();

        // Same file, DIFFERENT model -> miss (model folds into the key).
        assert!(cache
            .transcript(&media, "large-v3", "en-US", None)
            .unwrap()
            .is_none());
        // Same file/model, DIFFERENT language -> miss (language folds into the key).
        assert!(cache
            .transcript(&media, "small.en", "fr-FR", None)
            .unwrap()
            .is_none());
        // Original tuple still hits.
        assert!(cache
            .transcript(&media, "small.en", "en-US", None)
            .unwrap()
            .is_some());
    }

    #[test]
    fn key_changes_with_content() {
        let a = temp_file("content-a", b"CONTENT-A-PAYLOAD");
        let b = temp_file("content-b", b"CONTENT-B-PAYLOAD-DIFFERENT");
        assert_ne!(
            TranscriptCache::key(&a, "small.en", "en-US").unwrap(),
            TranscriptCache::key(&b, "small.en", "en-US").unwrap(),
        );
    }

    // ---- memory tier: clear-all on overflow (NOT LRU) ----------------------

    #[test]
    fn memory_clears_all_on_overflow() {
        let cache = TranscriptCache::with_directory(temp_cache_dir("memcap"));
        // Insert MEMORY_MAX (4) distinct keys directly into the memory tier.
        for i in 0..MEMORY_MAX {
            cache.remember(&format!("k{i}"), full());
        }
        assert_eq!(cache.memory.lock().unwrap().len(), MEMORY_MAX);

        // The 5th distinct insert clears the whole map first, then inserts -> len 1.
        cache.remember("k4", full());
        let mem = cache.memory.lock().unwrap();
        assert_eq!(mem.len(), 1, "overflow clears all, not LRU-evicts one");
        assert!(mem.contains_key("k4"));
        // None of the original 4 survive (clear-all, not single eviction).
        assert!(!mem.contains_key("k0"));
    }
}
