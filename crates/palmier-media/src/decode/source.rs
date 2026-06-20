//! The engine-facing decode API — story E5-S2.
//!
//! [`FrameSource`] is the handle `palmier-engine` consumes. It owns:
//! * a [`DecoderPool`] — **exactly one [`Decoder`] per distinct source URL**
//!   (the Glossary one-decode-owner contract; the engine never opens a format
//!   context), and
//! * a [`FrameCache`] — the LRU-by-playhead-distance decoded-frame cache.
//!
//! The engine calls [`FrameSource::request_frame`] with `(media_ref,
//! source_frame, mode)`:
//! * **`Exact`** → return the precise frame (from cache, or decode-on-miss).
//! * **`InteractiveScrub`** → return the nearest cached frame within tolerance
//!   *immediately* (with a `pending` flag), and queue the precise decode in the
//!   background, throttled to one dispatch per 1/30 s. If nothing is cached
//!   within tolerance it decodes synchronously so the first scrub still shows a
//!   frame.
//!
//! ## Where the URL comes from
//! The cache and the API key on `media_ref` (the model id). The source URL for a
//! `media_ref` is resolved by a caller-supplied resolver (the engine's
//! `MediaResolver` lives above this crate); the [`FrameSource`] is constructed
//! with a resolver closure so it can open the right decoder on a miss without
//! depending on the model's asset table.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::cache::{CacheStats, FrameCache, FrameKey};
use super::decoder::{DecodeError, Decoder, HwDecodeStatus};
use super::frame::DecodedFrame;
use super::seek::{interactive_tolerance_frames, ScrubThrottle, SeekMode};

/// Result of a frame request.
#[derive(Debug, Clone)]
pub struct FrameResult {
    /// The frame to display now. For `Exact` this is the precise frame; for
    /// `InteractiveScrub` it may be a nearby cached frame (see `pending`).
    pub frame: DecodedFrame,
    /// True when this is a *nearest* frame served for an interactive scrub and a
    /// precise decode of the requested frame is still pending in the background.
    /// `Exact` results always have `pending == false`.
    pub pending: bool,
}

/// One decoder per distinct source URL. A [`Mutex`] guards each decoder (a
/// decoder is single-threaded), and the map itself is guarded so the pool can be
/// shared. Opening is deduped: the first request for a URL opens it; subsequent
/// requests reuse the same decoder.
#[derive(Default)]
pub struct DecoderPool {
    decoders: Mutex<HashMap<PathBuf, Arc<Mutex<Decoder>>>>,
}

impl DecoderPool {
    /// New empty pool.
    pub fn new() -> Self {
        DecoderPool::default()
    }

    /// Get (opening on first use) the single decoder for `url`. Returns the same
    /// `Arc` for repeated calls — **one `AVFormatContext` per URL**.
    pub fn get_or_open(&self, url: &PathBuf) -> Result<Arc<Mutex<Decoder>>, DecodeError> {
        // Fast path: already open.
        if let Some(dec) = self.decoders.lock().unwrap().get(url).cloned() {
            return Ok(dec);
        }
        // Open outside the map lock (decode open can be slow), then dedup-insert:
        // if another thread opened it meanwhile, keep theirs and drop ours.
        let opened = Arc::new(Mutex::new(Decoder::open(url)?));
        let mut map = self.decoders.lock().unwrap();
        let entry = map.entry(url.clone()).or_insert(opened);
        Ok(entry.clone())
    }

    /// Number of open decoders (== number of distinct URLs opened).
    pub fn open_count(&self) -> usize {
        self.decoders.lock().unwrap().len()
    }

    /// Whether a decoder is open for `url`.
    pub fn is_open(&self, url: &PathBuf) -> bool {
        self.decoders.lock().unwrap().contains_key(url)
    }

    /// Close the decoder for `url` (e.g. asset removed). No-op if not open.
    pub fn close(&self, url: &PathBuf) {
        self.decoders.lock().unwrap().remove(url);
    }
}

/// Resolver from a `media_ref` id to its source URL. Supplied by the engine
/// (which owns the model's asset table); `None` means the ref is unknown/offline.
pub type UrlResolver = Arc<dyn Fn(&str) -> Option<PathBuf> + Send + Sync>;

/// The engine-facing decode handle: pool + cache + URL resolver.
///
/// Clone is cheap (shared `Arc`s) so the transport and prefetcher can hold
/// copies. All state is interior-mutable behind locks.
#[derive(Clone)]
pub struct FrameSource {
    pool: Arc<DecoderPool>,
    cache: Arc<Mutex<FrameCache>>,
    throttle: Arc<Mutex<ScrubThrottle>>,
    resolver: UrlResolver,
}

impl FrameSource {
    /// New frame source with the default 512 MB cache ceiling.
    pub fn new(resolver: UrlResolver) -> Self {
        FrameSource {
            pool: Arc::new(DecoderPool::new()),
            cache: Arc::new(Mutex::new(FrameCache::new())),
            throttle: Arc::new(Mutex::new(ScrubThrottle::default())),
            resolver,
        }
    }

    /// New frame source with an explicit cache ceiling (tests / tuning).
    pub fn with_ceiling(resolver: UrlResolver, ram_ceiling_bytes: usize) -> Self {
        FrameSource {
            pool: Arc::new(DecoderPool::new()),
            cache: Arc::new(Mutex::new(FrameCache::with_ceiling(ram_ceiling_bytes))),
            throttle: Arc::new(Mutex::new(ScrubThrottle::default())),
            resolver,
        }
    }

    /// The decoder pool (for `open_count` assertions / asset teardown).
    pub fn pool(&self) -> &Arc<DecoderPool> {
        &self.pool
    }

    /// Set the playhead for an asset so the cache keeps frames near it.
    pub fn set_playhead(&self, media_ref: &str, source_frame: u64) {
        self.cache.lock().unwrap().set_playhead(media_ref, source_frame);
    }

    /// Cache occupancy snapshot (the engine's `cache-stats`).
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.lock().unwrap().stats()
    }

    /// The frames-per-second of `media_ref`'s source (needed to turn the
    /// tolerance seconds into frames). Opens the decoder on first use.
    pub fn fps_of(&self, media_ref: &str) -> Option<f64> {
        let url = (self.resolver)(media_ref)?;
        let dec = self.pool.get_or_open(&url).ok()?;
        let fps = dec.lock().unwrap().fps();
        Some(fps)
    }

    /// HW/CPU decode status for `media_ref`'s decoder (opens on first use).
    pub fn hw_status_of(&self, media_ref: &str) -> Option<HwDecodeStatus> {
        let url = (self.resolver)(media_ref)?;
        let dec = self.pool.get_or_open(&url).ok()?;
        let status = dec.lock().unwrap().hw_status();
        Some(status)
    }

    /// Request the frame at `(media_ref, source_frame)` under `mode`. The core
    /// engine-facing entry point.
    ///
    /// * `Exact` → cache hit, else decode the precise frame (and cache it).
    /// * `InteractiveScrub` → serve the nearest cached frame within tolerance
    ///   immediately (`pending = true`) and queue the precise decode; if nothing
    ///   is cached within tolerance, decode synchronously so a frame still shows.
    pub fn request_frame(
        &self,
        media_ref: &str,
        source_frame: u64,
        mode: SeekMode,
        active_layer_count: u32,
    ) -> Result<FrameResult, DecodeError> {
        let key = FrameKey::new(media_ref, source_frame);

        // Exact cache hit always wins, regardless of mode.
        if let Some(frame) = self.cache.lock().unwrap().get(&key) {
            return Ok(FrameResult {
                frame,
                pending: false,
            });
        }

        match mode {
            SeekMode::Exact => {
                let frame = self.decode_and_cache(media_ref, source_frame)?;
                Ok(FrameResult {
                    frame,
                    pending: false,
                })
            }
            SeekMode::InteractiveScrub => {
                // Try to serve a nearest cached frame within tolerance.
                let fps = self.fps_of(media_ref).unwrap_or(30.0);
                let tol = interactive_tolerance_frames(active_layer_count, fps);
                let nearest = self
                    .cache
                    .lock()
                    .unwrap()
                    .nearest_within(media_ref, source_frame, tol);

                if let Some((frame, _dist)) = nearest {
                    // Serve nearest now; queue the precise decode (throttled).
                    self.maybe_queue_precise(media_ref, source_frame);
                    Ok(FrameResult {
                        frame,
                        pending: true,
                    })
                } else {
                    // Nothing close enough — decode synchronously so the first
                    // scrub frame still appears (the reference's seek still lands
                    // a frame; we don't show black).
                    let frame = self.decode_and_cache(media_ref, source_frame)?;
                    Ok(FrameResult {
                        frame,
                        pending: false,
                    })
                }
            }
        }
    }

    /// Decode `source_frame` and insert it into the cache, returning it. Opens
    /// the decoder for `media_ref`'s URL on first use (one per URL).
    fn decode_and_cache(
        &self,
        media_ref: &str,
        source_frame: u64,
    ) -> Result<DecodedFrame, DecodeError> {
        let url = (self.resolver)(media_ref).ok_or(DecodeError::NoVideoStream)?;
        let dec = self.pool.get_or_open(&url)?;
        let frame = dec.lock().unwrap().decode_frame(source_frame)?;
        self.cache
            .lock()
            .unwrap()
            .insert_with(media_ref.to_string(), frame.clone());
        Ok(frame)
    }

    /// Queue a precise decode for an interactive scrub if the throttle permits a
    /// dispatch right now (coalescing). When permitted, decodes synchronously on
    /// the calling thread and records the dispatch; when throttled, the request
    /// is dropped (a later scrub will re-request). This mirrors the reference's
    /// coalescing pending-seek: at most one precise decode per 1/30 s.
    ///
    /// (Background-thread dispatch is the transport's job in E5-S7; here we keep
    /// the *throttle contract* the engine relies on. The transport can replace
    /// the synchronous decode with a spawn while keeping this gate.)
    fn maybe_queue_precise(&self, media_ref: &str, source_frame: u64) {
        let now = std::time::Instant::now();
        let mut throttle = self.throttle.lock().unwrap();
        if !throttle.can_dispatch(now) {
            return;
        }
        throttle.record_dispatch(now);
        drop(throttle);
        // Best-effort precise decode; ignore errors (a later scrub retries).
        let _ = self.decode_and_cache(media_ref, source_frame);
    }

    /// Prefetch `source_frame` for `media_ref` into the cache without returning
    /// it (warm the window ahead of the playhead). No-op if already cached.
    pub fn prefetch(&self, media_ref: &str, source_frame: u64) -> Result<(), DecodeError> {
        let key = FrameKey::new(media_ref, source_frame);
        if self.cache.lock().unwrap().contains(&key) {
            return Ok(());
        }
        self.decode_and_cache(media_ref, source_frame).map(|_| ())
    }

    /// Drop all cached frames for `media_ref` and close its decoder (asset
    /// removed / source edited).
    pub fn evict_asset(&self, media_ref: &str) {
        self.cache.lock().unwrap().evict_asset(media_ref);
        if let Some(url) = (self.resolver)(media_ref) {
            self.pool.close(&url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::frame::{PixelLayout, Plane};
    use std::sync::Arc;

    fn resolver_for(map: Vec<(&'static str, &'static str)>) -> UrlResolver {
        let m: HashMap<String, PathBuf> = map
            .into_iter()
            .map(|(k, v)| (k.to_string(), PathBuf::from(v)))
            .collect();
        Arc::new(move |r: &str| m.get(r).cloned())
    }

    fn frame_of(source_frame: u64, bytes: usize) -> DecodedFrame {
        DecodedFrame {
            layout: PixelLayout::Rgba8,
            width: 1,
            height: 1,
            has_alpha: true,
            planes: Arc::new(vec![Plane {
                bytes: vec![0u8; bytes],
                stride: bytes,
                width: 1,
                height: 1,
            }]),
            source_frame,
        }
    }

    #[test]
    fn exact_cache_hit_returns_without_pending() {
        let src = FrameSource::new(resolver_for(vec![("a", "/nonexistent.mp4")]));
        // Pre-seed the cache so we don't need a real decoder.
        src.cache
            .lock()
            .unwrap()
            .insert_with("a".to_string(), frame_of(42, 100));
        let res = src
            .request_frame("a", 42, SeekMode::Exact, 1)
            .expect("cache hit");
        assert!(!res.pending);
        assert_eq!(res.frame.source_frame, 42);
    }

    #[test]
    fn scrub_serves_nearest_cached_with_pending_flag() {
        // Ceiling large; seed two frames; scrub to a target within tolerance of
        // a cached frame. fps default 30, 1 layer → tolerance ~5 frames.
        let src = FrameSource::new(resolver_for(vec![("a", "/nonexistent.mp4")]));
        {
            let mut c = src.cache.lock().unwrap();
            c.insert_with("a".to_string(), frame_of(100, 100));
        }
        // Request frame 102 (dist 2 from cached 100) in scrub mode. No decoder is
        // openable (path doesn't exist) so fps_of falls back to 30; tolerance for
        // 1 layer ≈ 5 frames ⇒ 100 is within range and served as nearest.
        let res = src
            .request_frame("a", 102, SeekMode::InteractiveScrub, 1)
            .expect("nearest served");
        assert!(res.pending, "nearest frame served while precise decode queued");
        assert_eq!(res.frame.source_frame, 100);
    }

    #[test]
    fn one_decoder_per_url_dedup() {
        let pool = DecoderPool::new();
        // Opening a nonexistent file errors; the pool must not insert a half-open
        // entry, and open_count stays 0.
        let url = PathBuf::from("/definitely/not/here.mp4");
        assert!(pool.get_or_open(&url).is_err());
        assert_eq!(pool.open_count(), 0);
        assert!(!pool.is_open(&url));
    }

    #[test]
    fn evict_asset_drops_cache_and_closes_decoder() {
        let src = FrameSource::new(resolver_for(vec![("a", "/nonexistent.mp4")]));
        src.cache
            .lock()
            .unwrap()
            .insert_with("a".to_string(), frame_of(0, 100));
        assert_eq!(src.cache_stats().frame_count, 1);
        src.evict_asset("a");
        assert_eq!(src.cache_stats().frame_count, 0);
    }

    #[test]
    fn unknown_media_ref_errors() {
        let src = FrameSource::new(resolver_for(vec![]));
        let err = src.request_frame("missing", 0, SeekMode::Exact, 1);
        assert!(err.is_err());
    }
}
