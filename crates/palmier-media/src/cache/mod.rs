//! Media visual-cache **infrastructure** (story E4-S2).
//!
//! This module ports the shared plumbing of the Swift `MediaVisualCache`
//! (`Sources/PalmierPro/Timeline/MediaVisualCache.swift`) — *not* the
//! thumbnail/waveform pipelines themselves, which land in E4-S3/S4/S5 once the
//! ffmpeg/symphonia deps are introduced. What's here is the three pieces every
//! one of those pipelines depends on:
//!
//! * [`key`] — the SHA256 disk cache key `sha256("<path>|<size>|<mtime>")`,
//!   first 16 bytes hex (ruling #16). Source edits invalidate the entry.
//! * [`dir`] — cache-directory resolution (`%LOCALAPPDATA%\PalmierProWin\Cache`
//!   on Windows, XDG cache dir on Linux) via the `dirs` crate.
//! * [`gates`] — the concurrency gates (waveform 2, image-thumb 4, video-thumb
//!   ungated) and in-flight dedup so concurrent same-key requests share one job.
//!
//! See `docs/reference/media-panel.md` §"Thumbnails / waveforms" and
//! `_bmad-output/implementation-artifacts/epic-04-media-panel.md` (E4-S2).
//!
//! ## R-7 carry-forward (QA note)
//! The cache key includes `mtime`. On Windows FAT/exFAT, mtime resolution is
//! ~2 s, so two rapid edits within that window could share an mtime and
//! *false-hit* the cache. Per the story we keep the key formula unchanged for
//! parity; a feature-flagged content-prefix fallback hook lives in
//! [`key::cache_key_with_content_prefix`] (behind `content-prefix-fallback`).

pub mod dir;
pub mod gates;
pub mod key;

pub use dir::{
    ensure_media_visual_cache_dir, media_visual_cache_dir, platform_cache_root,
    APP_CACHE_NAMESPACE, MEDIA_VISUAL_CACHE_NAME,
};
pub use gates::{CacheGates, CacheKind};
pub use key::{cache_key, cache_key_for_stat, SourceStat, KEY_PREFIX_BYTES};
