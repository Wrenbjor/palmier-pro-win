//! GPU texture LRU cache with a 1.5 GB VRAM ceiling — E5-S8.
//!
//! `palmier-media` owns the **system-RAM** decoded-frame cache (512 MB, evicting by
//! playhead distance). The compositor owns the **VRAM** side: uploaded
//! `wgpu::Texture`s keyed by `(media_ref, source_frame)` — the same
//! [`FrameRef`](crate::FrameRef) identity. A held frame uploads once and is reused
//! across redraws (refresh_visuals re-samples transform/opacity but keeps the
//! `FrameRef`, so its texture stays hot). The ceiling is the FOUNDATION §6.5
//! **1.5 GB VRAM texture cache** budget; we evict **least-recently-used** when an
//! insert would exceed it.
//!
//! This type is generic over the texture handle `T` so the eviction policy is
//! **pure and unit-testable** without a GPU (the wgpu compositor instantiates it
//! with `T = wgpu::Texture`). `T: Drop` releases the GPU allocation on eviction.

use std::collections::HashMap;

/// The FOUNDATION §6.5 VRAM texture-cache ceiling: 1.5 GB.
pub const DEFAULT_VRAM_CEILING_BYTES: u64 = 1_500 * 1024 * 1024;

/// Cache key: the `(media_ref, source_frame)` identity of a decoded frame — the
/// same addressing [`FrameRef`](crate::FrameRef) uses, so a layer's texture is
/// found by its frame ref.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TexKey {
    pub media_ref: String,
    pub source_frame: u64,
}

impl TexKey {
    pub fn new(media_ref: impl Into<String>, source_frame: u64) -> Self {
        TexKey { media_ref: media_ref.into(), source_frame }
    }
}

struct Entry<T> {
    texture: T,
    /// VRAM footprint in bytes (`width * height * 4` for RGBA8).
    bytes: u64,
    /// Monotonic tick of last access — LRU orders by the smallest.
    last_used: u64,
}

/// Cache occupancy snapshot (the engine's GPU-side `cache-stats`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TexCacheStats {
    /// Number of resident textures.
    pub texture_count: usize,
    /// Total VRAM bytes held.
    pub vram_bytes: u64,
    /// The ceiling enforced.
    pub ceiling_bytes: u64,
}

/// An LRU `(media_ref, source_frame) → texture` cache under a VRAM ceiling.
///
/// `get` marks an entry most-recently-used; `insert` evicts the least-recently-used
/// entries until the new texture fits under the ceiling. A single texture larger
/// than the whole ceiling is rejected (returns it back to the caller via the
/// `Err`), never evicting everything for an impossible fit.
pub struct TextureCache<T> {
    map: HashMap<TexKey, Entry<T>>,
    vram_bytes: u64,
    ceiling_bytes: u64,
    clock: u64,
}

impl<T> TextureCache<T> {
    /// New cache at the default 1.5 GB ceiling.
    pub fn new() -> Self {
        Self::with_ceiling(DEFAULT_VRAM_CEILING_BYTES)
    }

    /// New cache with an explicit ceiling (tests / tuning).
    pub fn with_ceiling(ceiling_bytes: u64) -> Self {
        TextureCache {
            map: HashMap::new(),
            vram_bytes: 0,
            ceiling_bytes: ceiling_bytes.max(1),
            clock: 0,
        }
    }

    fn tick(&mut self) -> u64 {
        self.clock = self.clock.wrapping_add(1);
        self.clock
    }

    /// Borrow the texture for `key`, marking it most-recently-used. `None` on miss.
    pub fn get(&mut self, key: &TexKey) -> Option<&T> {
        let t = self.tick();
        let entry = self.map.get_mut(key)?;
        entry.last_used = t;
        Some(&entry.texture)
    }

    /// Whether `key` is resident (does not touch LRU order).
    pub fn contains(&self, key: &TexKey) -> bool {
        self.map.contains_key(key)
    }

    /// Insert `texture` (size `bytes`) under `key`, evicting LRU entries to stay
    /// under the ceiling. Replaces any existing entry for `key`.
    ///
    /// Returns `Err(texture)` handing the texture back if it alone exceeds the
    /// ceiling (the caller should draw it once without caching rather than thrash).
    pub fn insert(&mut self, key: TexKey, texture: T, bytes: u64) -> Result<(), T> {
        if bytes > self.ceiling_bytes {
            return Err(texture);
        }
        // Replace existing: drop its bytes first (the old `T` is dropped on overwrite).
        if let Some(old) = self.map.remove(&key) {
            self.vram_bytes -= old.bytes;
            // old.texture dropped here.
        }
        // Evict LRU until the newcomer fits.
        while self.vram_bytes + bytes > self.ceiling_bytes {
            if !self.evict_one() {
                break; // nothing left to evict (shouldn't happen given the guard).
            }
        }
        let t = self.tick();
        self.map.insert(key, Entry { texture, bytes, last_used: t });
        self.vram_bytes += bytes;
        Ok(())
    }

    /// Evict the single least-recently-used entry. Returns false if empty.
    fn evict_one(&mut self) -> bool {
        let Some(victim) = self
            .map
            .iter()
            .min_by_key(|(_, e)| e.last_used)
            .map(|(k, _)| k.clone())
        else {
            return false;
        };
        if let Some(e) = self.map.remove(&victim) {
            self.vram_bytes -= e.bytes;
            // e.texture dropped → GPU allocation released.
        }
        true
    }

    /// Drop every texture for `media_ref` (asset removed / source edited) — mirrors
    /// `FrameSource::evict_asset` on the RAM side.
    pub fn evict_asset(&mut self, media_ref: &str) {
        let keys: Vec<TexKey> = self
            .map
            .keys()
            .filter(|k| k.media_ref == media_ref)
            .cloned()
            .collect();
        for k in keys {
            if let Some(e) = self.map.remove(&k) {
                self.vram_bytes -= e.bytes;
            }
        }
    }

    /// Occupancy snapshot.
    pub fn stats(&self) -> TexCacheStats {
        TexCacheStats {
            texture_count: self.map.len(),
            vram_bytes: self.vram_bytes,
            ceiling_bytes: self.ceiling_bytes,
        }
    }
}

impl<T> Default for TextureCache<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A fake texture that records its size; lets us assert eviction without a GPU.
    #[derive(Debug, PartialEq)]
    struct FakeTex(u32);

    #[test]
    fn insert_and_get_hits() {
        let mut c: TextureCache<FakeTex> = TextureCache::with_ceiling(1000);
        c.insert(TexKey::new("a", 0), FakeTex(0), 100).unwrap();
        assert!(c.contains(&TexKey::new("a", 0)));
        assert_eq!(c.get(&TexKey::new("a", 0)), Some(&FakeTex(0)));
        assert!(c.get(&TexKey::new("a", 1)).is_none());
        assert_eq!(c.stats().vram_bytes, 100);
        assert_eq!(c.stats().texture_count, 1);
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let mut c: TextureCache<FakeTex> = TextureCache::with_ceiling(250);
        c.insert(TexKey::new("a", 0), FakeTex(0), 100).unwrap();
        c.insert(TexKey::new("a", 1), FakeTex(1), 100).unwrap();
        // Touch frame 0 so frame 1 becomes the LRU victim.
        let _ = c.get(&TexKey::new("a", 0));
        // Insert a third 100-byte texture → 300 > 250 ceiling → evict one LRU (frame 1).
        c.insert(TexKey::new("a", 2), FakeTex(2), 100).unwrap();
        assert!(c.contains(&TexKey::new("a", 0)), "recently-used frame 0 kept");
        assert!(!c.contains(&TexKey::new("a", 1)), "LRU frame 1 evicted");
        assert!(c.contains(&TexKey::new("a", 2)));
        assert_eq!(c.stats().vram_bytes, 200);
    }

    #[test]
    fn insert_evicts_multiple_to_fit() {
        let mut c: TextureCache<FakeTex> = TextureCache::with_ceiling(300);
        for f in 0..3 {
            c.insert(TexKey::new("a", f), FakeTex(f as u32), 100).unwrap();
        }
        assert_eq!(c.stats().texture_count, 3); // 300 bytes, exactly at ceiling.
        // A 150-byte insert needs vram ≤ 150 → evict the two LRUs (frames 0,1);
        // frame 2 (most-recent) survives (150 + 100 ≤ 300).
        c.insert(TexKey::new("a", 9), FakeTex(9), 150).unwrap();
        assert!(c.contains(&TexKey::new("a", 9)));
        assert!(c.contains(&TexKey::new("a", 2)), "most-recent kept");
        assert!(!c.contains(&TexKey::new("a", 0)), "LRU evicted");
        assert!(!c.contains(&TexKey::new("a", 1)), "2nd LRU evicted");
        assert_eq!(c.stats().vram_bytes, 250);
        assert!(c.stats().vram_bytes <= 300);
    }

    #[test]
    fn oversize_texture_rejected_without_thrashing() {
        let mut c: TextureCache<FakeTex> = TextureCache::with_ceiling(150);
        c.insert(TexKey::new("a", 0), FakeTex(0), 100).unwrap();
        // A texture bigger than the whole ceiling is handed back; the existing entry stays.
        let err = c.insert(TexKey::new("a", 1), FakeTex(1), 200);
        assert_eq!(err, Err(FakeTex(1)));
        assert!(c.contains(&TexKey::new("a", 0)), "existing entry not evicted for an impossible fit");
        assert_eq!(c.stats().vram_bytes, 100);
    }

    #[test]
    fn replace_same_key_updates_size() {
        let mut c: TextureCache<FakeTex> = TextureCache::with_ceiling(1000);
        c.insert(TexKey::new("a", 0), FakeTex(0), 100).unwrap();
        c.insert(TexKey::new("a", 0), FakeTex(99), 250).unwrap();
        assert_eq!(c.stats().texture_count, 1);
        assert_eq!(c.stats().vram_bytes, 250);
        assert_eq!(c.get(&TexKey::new("a", 0)), Some(&FakeTex(99)));
    }

    #[test]
    fn evict_asset_drops_all_its_textures() {
        let mut c: TextureCache<FakeTex> = TextureCache::with_ceiling(1000);
        c.insert(TexKey::new("a", 0), FakeTex(0), 100).unwrap();
        c.insert(TexKey::new("a", 1), FakeTex(1), 100).unwrap();
        c.insert(TexKey::new("b", 0), FakeTex(2), 100).unwrap();
        c.evict_asset("a");
        assert!(!c.contains(&TexKey::new("a", 0)));
        assert!(!c.contains(&TexKey::new("a", 1)));
        assert!(c.contains(&TexKey::new("b", 0)));
        assert_eq!(c.stats().vram_bytes, 100);
    }

    #[test]
    fn default_ceiling_is_one_and_a_half_gb() {
        let c: TextureCache<FakeTex> = TextureCache::new();
        assert_eq!(c.stats().ceiling_bytes, 1_500 * 1024 * 1024);
    }
}
