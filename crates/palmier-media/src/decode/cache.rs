//! The decoded-frame [`FrameCache`] — story E5-S2.
//!
//! An in-RAM cache of [`DecodedFrame`]s keyed by `(media_ref, source_frame)`.
//! Eviction is **by distance from the current playhead** (FOUNDATION §6.5): when
//! the cache is over its system-RAM ceiling it drops the frames *farthest* from
//! the playhead first, so frames near where we're scrubbing/playing stay hot.
//! This is the reference's implicit behavior (AVFoundation keeps a window around
//! the play head) made explicit for the per-frame model.
//!
//! ## Ceiling (FOUNDATION §6.5)
//! The decode pipeline has two ceilings: **1.5 GB VRAM for textures** and
//! **512 MB system RAM for decoded YUV planes**. Texture/VRAM accounting belongs
//! to the engine (E5-S8 owns wgpu); *this* cache enforces the **512 MB system-RAM
//! ceiling** for the decoded CPU planes it holds. The ceiling is **global**
//! (across all assets), per the FOUNDATION §6.5 reading — not per-asset.
//!
//! ## Why not a plain LRU
//! A recency-only LRU evicts the frame you scrubbed *away from* even when it's
//! one frame from the playhead and about to be needed again. Distance-from-
//! playhead eviction is the correct policy for a scrubbing editor: it keeps a
//! symmetric window around the playhead regardless of visit order.

use std::collections::HashMap;

use super::frame::DecodedFrame;

/// Default system-RAM ceiling for decoded planes: **512 MB** (FOUNDATION §6.5).
pub const DEFAULT_RAM_CEILING_BYTES: usize = 512 * 1024 * 1024;

/// Cache key: a decoded frame is addressed by its source asset and the source
/// frame index within that asset. `media_ref` is the model's `String`
/// media-reference id (Glossary one-decode-owner key).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FrameKey {
    /// Source-asset media reference (the model `clip.media_ref` id).
    pub media_ref: String,
    /// Source frame index within that asset.
    pub source_frame: u64,
}

impl FrameKey {
    /// Construct a key from a media reference and source-frame index.
    pub fn new(media_ref: impl Into<String>, source_frame: u64) -> Self {
        FrameKey {
            media_ref: media_ref.into(),
            source_frame,
        }
    }
}

/// Snapshot of cache occupancy for tests / the engine's cache-stats API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheStats {
    /// Number of cached frames.
    pub frame_count: usize,
    /// Total system-RAM bytes held by cached planes.
    pub ram_bytes: usize,
    /// The configured RAM ceiling.
    pub ram_ceiling_bytes: usize,
    /// Cumulative cache hits since construction.
    pub hits: u64,
    /// Cumulative cache misses since construction.
    pub misses: u64,
}

/// An entry plus the playhead-relative bookkeeping used for eviction.
struct Entry {
    frame: DecodedFrame,
    bytes: usize,
}

/// In-RAM decoded-frame cache with **distance-from-playhead** eviction under a
/// global system-RAM ceiling.
///
/// Single-asset semantics live in the key (`media_ref`): the cache is global,
/// holding frames from any number of assets. Eviction distance is computed
/// against a per-asset playhead so frames of the asset currently being scrubbed
/// are protected; frames of other assets (no playhead set) are treated as
/// maximally distant and evicted first.
pub struct FrameCache {
    map: HashMap<FrameKey, Entry>,
    /// Current playhead (source frame) per asset, set by the transport/engine.
    /// Eviction distance for a key is `|source_frame - playhead[media_ref]|`.
    playheads: HashMap<String, u64>,
    ram_bytes: usize,
    ram_ceiling_bytes: usize,
    hits: u64,
    misses: u64,
}

impl FrameCache {
    /// New cache with the default 512 MB system-RAM ceiling.
    pub fn new() -> Self {
        Self::with_ceiling(DEFAULT_RAM_CEILING_BYTES)
    }

    /// New cache with an explicit RAM ceiling (used in tests to force eviction
    /// at small sizes).
    pub fn with_ceiling(ram_ceiling_bytes: usize) -> Self {
        FrameCache {
            map: HashMap::new(),
            playheads: HashMap::new(),
            ram_bytes: 0,
            ram_ceiling_bytes,
            hits: 0,
            misses: 0,
        }
    }

    /// Set the playhead (source frame) for an asset. Eviction keeps frames near
    /// this position and drops the farthest first. The transport calls this on
    /// every seek/tick so the hot window tracks the play head.
    pub fn set_playhead(&mut self, media_ref: &str, source_frame: u64) {
        self.playheads.insert(media_ref.to_string(), source_frame);
    }

    /// Look up a cached frame, counting a hit/miss. Returns a cheap `Arc`-backed
    /// clone on hit.
    pub fn get(&mut self, key: &FrameKey) -> Option<DecodedFrame> {
        match self.map.get(key) {
            Some(entry) => {
                self.hits += 1;
                Some(entry.frame.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Whether a frame is cached (no hit/miss accounting — used by the nearest
    /// lookup and tests).
    pub fn contains(&self, key: &FrameKey) -> bool {
        self.map.contains_key(key)
    }

    /// Find the nearest cached frame to `target` for `media_ref` within
    /// `±tolerance_frames`, returning it and its absolute frame distance. Used to
    /// serve `InteractiveScrub` requests immediately while the precise decode is
    /// queued. Ties prefer the closer-or-equal lower frame, then the closest.
    pub fn nearest_within(
        &self,
        media_ref: &str,
        target: u64,
        tolerance_frames: u64,
    ) -> Option<(DecodedFrame, u64)> {
        let mut best: Option<(u64, &Entry)> = None; // (distance, entry)
        for (key, entry) in &self.map {
            if key.media_ref != media_ref {
                continue;
            }
            let dist = key.source_frame.abs_diff(target);
            if dist > tolerance_frames {
                continue;
            }
            match best {
                Some((best_dist, _)) if dist >= best_dist => {}
                _ => best = Some((dist, entry)),
            }
        }
        best.map(|(dist, entry)| (entry.frame.clone(), dist))
    }

    /// Insert a decoded frame under `media_ref`, then evict the farthest-from-
    /// playhead frames until back under the RAM ceiling.
    pub fn insert_with(&mut self, media_ref: impl Into<String>, frame: DecodedFrame) {
        let media_ref = media_ref.into();
        let key = FrameKey::new(media_ref, frame.source_frame);
        let bytes = frame.ram_bytes();

        if let Some(old) = self.map.remove(&key) {
            self.ram_bytes = self.ram_bytes.saturating_sub(old.bytes);
        }
        self.ram_bytes += bytes;
        self.map.insert(key.clone(), Entry { frame, bytes });

        self.evict_to_ceiling(Some(&key));
    }

    /// Evict farthest-from-playhead frames until at/under the ceiling. `protect`
    /// (the just-inserted key) is never evicted even if it is, momentarily, the
    /// farthest — without this a single frame larger than the ceiling would
    /// evict itself and the cache would thrash.
    fn evict_to_ceiling(&mut self, protect: Option<&FrameKey>) {
        while self.ram_bytes > self.ram_ceiling_bytes && self.map.len() > 1 {
            // Find the key with the greatest distance from its asset's playhead.
            let victim = self
                .map
                .keys()
                .filter(|k| Some(*k) != protect)
                .max_by_key(|k| self.distance(k))
                .cloned();
            let Some(victim) = victim else { break };
            if let Some(entry) = self.map.remove(&victim) {
                self.ram_bytes = self.ram_bytes.saturating_sub(entry.bytes);
            }
        }
    }

    /// Eviction distance for a key: absolute frame distance from its asset's
    /// playhead. Assets with no playhead set are treated as maximally distant
    /// (evict first) so stale single-frame assets don't linger.
    fn distance(&self, key: &FrameKey) -> u64 {
        match self.playheads.get(&key.media_ref) {
            Some(&playhead) => key.source_frame.abs_diff(playhead),
            None => u64::MAX,
        }
    }

    /// Drop every cached frame for an asset (e.g. when its tab closes or the
    /// source edits). Returns the freed byte count.
    pub fn evict_asset(&mut self, media_ref: &str) -> usize {
        let keys: Vec<FrameKey> = self
            .map
            .keys()
            .filter(|k| k.media_ref == media_ref)
            .cloned()
            .collect();
        let mut freed = 0;
        for k in keys {
            if let Some(entry) = self.map.remove(&k) {
                freed += entry.bytes;
                self.ram_bytes = self.ram_bytes.saturating_sub(entry.bytes);
            }
        }
        self.playheads.remove(media_ref);
        freed
    }

    /// Clear every cached frame and playhead.
    pub fn clear(&mut self) {
        self.map.clear();
        self.playheads.clear();
        self.ram_bytes = 0;
    }

    /// Current occupancy snapshot.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            frame_count: self.map.len(),
            ram_bytes: self.ram_bytes,
            ram_ceiling_bytes: self.ram_ceiling_bytes,
            hits: self.hits,
            misses: self.misses,
        }
    }
}

impl Default for FrameCache {
    fn default() -> Self {
        FrameCache::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::frame::{Plane, PixelLayout};
    use std::sync::Arc;

    /// A frame of `bytes` system-RAM footprint at `source_frame` (single RGBA
    /// plane sized so `ram_bytes() == bytes`).
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
    fn hit_and_miss_accounting() {
        let mut cache = FrameCache::with_ceiling(1_000_000);
        let key = FrameKey::new("a", 10);
        assert!(cache.get(&key).is_none());
        cache.insert_with("a", frame_of(10, 100));
        assert!(cache.get(&key).is_some());
        let s = cache.stats();
        assert_eq!(s.hits, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.frame_count, 1);
        assert_eq!(s.ram_bytes, 100);
    }

    #[test]
    fn evicts_farthest_from_playhead_first() {
        // Ceiling holds 3 frames of 100 bytes. Playhead at frame 100.
        let mut cache = FrameCache::with_ceiling(300);
        cache.set_playhead("a", 100);
        // Insert frames at increasing distance from the playhead.
        cache.insert_with("a", frame_of(100, 100)); // dist 0
        cache.insert_with("a", frame_of(101, 100)); // dist 1
        cache.insert_with("a", frame_of(140, 100)); // dist 40
        assert_eq!(cache.stats().frame_count, 3);

        // Insert a 4th near the playhead; the farthest (frame 140, dist 40) must
        // be evicted — NOT the just-inserted one and NOT the near ones.
        cache.insert_with("a", frame_of(99, 100)); // dist 1
        assert_eq!(cache.stats().frame_count, 3);
        assert!(cache.contains(&FrameKey::new("a", 100)));
        assert!(cache.contains(&FrameKey::new("a", 101)));
        assert!(cache.contains(&FrameKey::new("a", 99)));
        assert!(
            !cache.contains(&FrameKey::new("a", 140)),
            "frame farthest from playhead is evicted first"
        );
    }

    #[test]
    fn moving_playhead_changes_eviction_victim() {
        let mut cache = FrameCache::with_ceiling(300);
        cache.set_playhead("a", 0);
        cache.insert_with("a", frame_of(0, 100));
        cache.insert_with("a", frame_of(50, 100));
        cache.insert_with("a", frame_of(100, 100));
        // Move the playhead to 100; now frame 0 is farthest.
        cache.set_playhead("a", 100);
        cache.insert_with("a", frame_of(101, 100));
        assert!(!cache.contains(&FrameKey::new("a", 0)), "now-farthest evicted");
        assert!(cache.contains(&FrameKey::new("a", 100)));
        assert!(cache.contains(&FrameKey::new("a", 101)));
    }

    #[test]
    fn frames_of_asset_with_no_playhead_evict_before_tracked_asset() {
        let mut cache = FrameCache::with_ceiling(200);
        cache.set_playhead("tracked", 0);
        // "other" has no playhead → distance u64::MAX → evicts first.
        cache.insert_with("other", frame_of(5, 100));
        cache.insert_with("tracked", frame_of(0, 100));
        cache.insert_with("tracked", frame_of(1, 100)); // forces one eviction
        assert!(!cache.contains(&FrameKey::new("other", 5)));
        assert!(cache.contains(&FrameKey::new("tracked", 0)));
    }

    #[test]
    fn nearest_within_serves_closest_in_tolerance() {
        let mut cache = FrameCache::with_ceiling(10_000);
        cache.insert_with("a", frame_of(10, 50));
        cache.insert_with("a", frame_of(13, 50));
        cache.insert_with("a", frame_of(20, 50));
        // target 12, tolerance 2 → frames 10 (dist 2) and 13 (dist 1); pick 13.
        let (f, dist) = cache.nearest_within("a", 12, 2).expect("a near frame");
        assert_eq!(f.source_frame, 13);
        assert_eq!(dist, 1);
        // target 16, tolerance 2 → nothing within range (13 is dist 3, 20 dist 4).
        assert!(cache.nearest_within("a", 16, 2).is_none());
        // wrong asset → none.
        assert!(cache.nearest_within("b", 13, 5).is_none());
    }

    #[test]
    fn evict_asset_frees_only_that_asset() {
        let mut cache = FrameCache::with_ceiling(10_000);
        cache.insert_with("a", frame_of(0, 100));
        cache.insert_with("a", frame_of(1, 100));
        cache.insert_with("b", frame_of(0, 100));
        let freed = cache.evict_asset("a");
        assert_eq!(freed, 200);
        assert_eq!(cache.stats().frame_count, 1);
        assert!(cache.contains(&FrameKey::new("b", 0)));
    }

    #[test]
    fn replacing_a_key_updates_ram_accounting() {
        let mut cache = FrameCache::with_ceiling(10_000);
        cache.insert_with("a", frame_of(5, 100));
        cache.insert_with("a", frame_of(5, 250)); // same key, bigger
        assert_eq!(cache.stats().frame_count, 1);
        assert_eq!(cache.stats().ram_bytes, 250);
    }

    #[test]
    fn single_oversized_frame_is_kept_not_self_evicted() {
        // A frame bigger than the ceiling stays (map.len()==1 guard); we never
        // thrash to an empty cache.
        let mut cache = FrameCache::with_ceiling(50);
        cache.insert_with("a", frame_of(0, 500));
        assert_eq!(cache.stats().frame_count, 1);
    }
}
