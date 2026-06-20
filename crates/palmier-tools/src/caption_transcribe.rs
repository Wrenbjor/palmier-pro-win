//! Transcription plumbing shared by the two E10-S7 caption tools — the
//! **cache-vs-bypass** decision, the **blocking-thread** engine call, the on-device
//! transcript-cache accessor, the model/language identity, and the media-ref → file
//! resolution + [`AssetInfo`] view over the library.
//!
//! ## Why this lives in the tool layer
//! `palmier-edit::generate_captions` is deliberately whisper/tokio-free: it takes a
//! `transcribe` closure and lets the **caller** decide cache vs bypass and how to run
//! the (synchronous, blocking) whisper call. This module is that caller's machinery,
//! so the exact rule is in one place for both `add_captions` (which feeds the closure
//! to `generate_captions`) and `get_transcript` (which reads the same cache directly).
//!
//! ## The cache-vs-bypass rule (parity, reference `generateCaptions` / `inspect_media`)
//! When `censor_profanity || locale.is_some()` the transcript is **request-specific**
//! (the bundled `.en` model's language tag or the masked text differs from the plain
//! cached artifact), so we **bypass** the cache: call the engine directly and never
//! store. Otherwise (the plain case) we go **through** [`TranscriptCache`]: read on a
//! hit; on a miss call the engine for the **full file** and `store(...)` it, so future
//! windowed reads are served by filtering. `get_transcript` is always the plain,
//! read-only case (it never transcribes — UJ-1: empty → tell the agent to transcribe).
//!
//! ## Blocking + async, from a synchronous tool body
//! `TranscriptCache::shared()` is async (tokio `OnceCell`) and `transcribe(...)` BLOCKS
//! (`whisper full()`), while the tool body is a plain `fn -> ToolResult`. We build one
//! small multi-thread tokio runtime per call and:
//! - `block_on` the async cache accessor, and
//! - run every engine transcription through [`tokio::task::spawn_blocking`] so the
//!   blocking whisper work never stalls a runtime worker (the story's spawn_blocking
//!   requirement; reference runs transcription off the main actor).

use std::path::PathBuf;

use palmier_edit::AssetInfo;
use palmier_model::{ClipType, MediaLibrary, MediaSource};
use palmier_transcribe::{
    english_only_supported, match_locale, resolve_locale_en, LocaleTag, TranscriptCache,
    TranscriptionError, TranscriptionResult,
};

/// The bundled whisper model id folded into the transcript-cache content key
/// (`ggml-small.en.bin` → `small.en`; ruling #19 key is `sha256(content)+model+language`).
/// One constant so `add_captions` (write) and `get_transcript` (read) key identically.
pub const WHISPER_MODEL_ID: &str = "small.en";

/// Env override for the transcript-cache directory (tests seed a JSON transcript here
/// and assert both tools read it). Absent ⇒ the process-wide
/// [`TranscriptCache::shared`] singleton (`%LOCALAPPDATA%\PalmierProWin\Transcripts`).
pub const CACHE_DIR_ENV: &str = "PALMIER_TRANSCRIPT_CACHE_DIR";

/// The resolved BCP-47 language tag a transcript is cached under for the bundled `.en`
/// model. Bypass requests (a `language` override) resolve their own tag; the plain
/// cached path always keys under this so the read/write tags agree.
///
/// `locale` is the optional user override (e.g. `"en-GB"`); for the English-only model
/// it resolves to an English tag, else falls back to `en-US` (the engine forces English
/// regardless — only the reported tag changes). Never errors here; unsupported locales
/// are rejected earlier in `add_captions`.
pub fn resolve_cache_language(locale: Option<&str>) -> String {
    resolve_locale_en(locale)
        .map(|t| t.to_bcp47())
        .unwrap_or_else(|_| "en-US".to_string())
}

/// Validate a user `language` (BCP-47) against the on-device model's supported set,
/// returning the resolved tag string or an `Err(message)` matching the reference
/// rejection (`add_captions: on-device transcription does not support language '…'.`).
///
/// Reference `addCaptions`: `matchLocale(candidates:[Locale(lang)], supported:
/// supportedLocales())` — `nil` ⇒ throw. Our supported universe for the bundled `.en`
/// model is English-only ([`english_only_supported`]).
pub fn validate_language(language: &str) -> Result<String, String> {
    let Some(candidate) = LocaleTag::parse(language) else {
        return Err(format!(
            "add_captions: on-device transcription does not support language '{language}'."
        ));
    };
    match match_locale(std::slice::from_ref(&candidate), &english_only_supported()) {
        Some(tag) => Ok(tag.to_bcp47()),
        None => Err(format!(
            "add_captions: on-device transcription does not support language '{language}'."
        )),
    }
}

/// Resolve a `media_ref` to its on-disk file path through the library's asset catalog.
/// `MediaSource::External` gives an absolute path directly; `Project` is project-bundle
/// relative — without a bundle root in the tool layer we use the relative path as-is
/// (the host resolves real bundle paths; tests use `External`). `None` ⇒ no such asset.
pub fn asset_path(library: &MediaLibrary, media_ref: &str) -> Option<PathBuf> {
    let asset = library.assets.iter().find(|a| a.id == media_ref)?;
    Some(match &asset.source {
        MediaSource::External { absolute_path } => PathBuf::from(absolute_path),
        MediaSource::Project { relative_path } => PathBuf::from(relative_path),
    })
}

/// An **owned** snapshot of the library's asset catalog answering the two questions
/// `generate_captions` asks per `media_ref` (has-audio / is-video) plus the resolved
/// file path. Owned (not borrowing the library) so it can be captured into the
/// `agent_edit` closure, which already holds `&mut library.timeline` — borrowing the
/// library there too would conflict. The reference's `mediaAssets` lookup, snapshotted.
#[derive(Debug, Default, Clone)]
pub struct LibraryAssets {
    entries: std::collections::HashMap<String, AssetEntry>,
}

#[derive(Debug, Clone)]
struct AssetEntry {
    has_audio: bool,
    is_video: bool,
    file: PathBuf,
}

impl LibraryAssets {
    /// Snapshot the catalog: id → (has_audio, is_video, resolved file path).
    pub fn snapshot(library: &MediaLibrary) -> Self {
        let mut entries = std::collections::HashMap::new();
        for a in &library.assets {
            let file = match &a.source {
                MediaSource::External { absolute_path } => PathBuf::from(absolute_path),
                MediaSource::Project { relative_path } => PathBuf::from(relative_path),
            };
            entries.insert(
                a.id.clone(),
                AssetEntry {
                    has_audio: a.has_audio,
                    is_video: a.asset_type == ClipType::Video,
                    file,
                },
            );
        }
        LibraryAssets { entries }
    }

    /// The resolved on-disk file for `media_ref`, if known.
    pub fn file(&self, media_ref: &str) -> Option<&PathBuf> {
        self.entries.get(media_ref).map(|e| &e.file)
    }
}

impl AssetInfo for LibraryAssets {
    fn has_audio(&self, media_ref: &str) -> Option<bool> {
        self.entries.get(media_ref).map(|e| e.has_audio)
    }

    fn asset_is_video(&self, media_ref: &str) -> Option<bool> {
        self.entries.get(media_ref).map(|e| e.is_video)
    }
}

/// Acquire the transcript cache: the [`CACHE_DIR_ENV`]-pointed directory if set
/// (tests / non-default roots), else the process-wide [`TranscriptCache::shared`]
/// singleton. Must be called inside a tokio runtime (the singleton init is async).
pub async fn acquire_cache() -> CacheHandle {
    match std::env::var(CACHE_DIR_ENV) {
        Ok(dir) if !dir.is_empty() => CacheHandle::Owned(TranscriptCache::with_directory(dir)),
        _ => CacheHandle::Shared(TranscriptCache::shared().await),
    }
}

/// A cache reference that is either the borrowed `'static` shared singleton or an
/// owned per-call instance over an override directory. Derefs to [`TranscriptCache`].
pub enum CacheHandle {
    /// The process-wide singleton (`'static`).
    Shared(&'static TranscriptCache),
    /// An owned cache over [`CACHE_DIR_ENV`] (tests).
    Owned(TranscriptCache),
}

impl std::ops::Deref for CacheHandle {
    type Target = TranscriptCache;
    fn deref(&self) -> &TranscriptCache {
        match self {
            CacheHandle::Shared(c) => c,
            CacheHandle::Owned(c) => c,
        }
    }
}

/// Run the synchronous, blocking whisper engine call on a blocking thread (the story's
/// spawn_blocking requirement). `is_video` selects video-audio extraction
/// (`transcribe_video_audio`) vs plain audio (`transcribe`). `range` is the optional
/// source-seconds window. Returns the engine's `TranscriptionResult` (offset back into
/// source time by the engine).
pub async fn transcribe_blocking(
    file: PathBuf,
    censor_profanity: bool,
    locale: Option<String>,
    range: Option<std::ops::RangeInclusive<f64>>,
    is_video: bool,
) -> Result<TranscriptionResult, TranscriptionError> {
    tokio::task::spawn_blocking(move || {
        let loc = locale.as_deref();
        if is_video {
            palmier_transcribe::transcribe_video_audio(&file, censor_profanity, loc, range.as_ref())
        } else {
            palmier_transcribe::transcribe(&file, censor_profanity, loc, range.as_ref())
        }
    })
    .await
    .map_err(|join_err| {
        // A panicked/aborted blocking task surfaces as an analysis failure.
        TranscriptionError::AnalysisFailed(format!("transcription task failed: {join_err}"))
    })?
}

/// Build a small multi-thread tokio runtime for one tool call (cache singleton init +
/// `spawn_blocking` need a runtime; the sync tool body owns it for the call's duration).
pub fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
}
