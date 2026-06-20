//! Disk cache-key derivation for the media visual cache (E4-S2).
//!
//! Port of `MediaVisualCache.diskCacheKey(for:)` (Swift / CryptoKit) to Rust /
//! `sha2`. The key is keyed on `path | size | mtime` so a source edit (changed
//! size or mtime) yields a new key and misses the stale entry.
//!
//! Formula (ruling #16 / `docs/reference/media-panel.md` §"Disk cache key"):
//!
//! ```text
//! seed   = "<path>|<size>|<mtime_epoch_seconds>"
//! digest = SHA256(seed)
//! key    = hex(digest[0..16])   // first 16 BYTES → 32 hex chars
//! ```
//!
//! ## R-7 watch (Windows coarse-FS mtime)
//! On FAT/exFAT, mtime resolution is ~2 s, so two rapid edits could share an
//! mtime and false-hit the cache. We keep the key formula unchanged for parity
//! (per the story: "do not change the key"), but expose a feature-flagged
//! content-prefix fallback hook ([`cache_key_with_content_prefix`]) behind the
//! `content-prefix-fallback` cargo feature so it can be wired later without
//! touching the parity path.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// Number of leading SHA256 **bytes** kept (`.prefix(16)` in the Swift
/// reference). 16 bytes → 32 lowercase hex characters.
pub const KEY_PREFIX_BYTES: usize = 16;

/// The file-system facts the cache key is derived from. Pulling this into its
/// own struct keeps the hashing logic testable without touching the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceStat {
    /// File size in bytes.
    pub size: u64,
    /// Modification time as whole seconds since the Unix epoch (matches the
    /// Swift `Date.timeIntervalSince1970` integer-second seed used by the
    /// reference key — see note below).
    pub mtime_epoch_secs: i64,
}

impl SourceStat {
    /// Read size + mtime from the filesystem for `path`.
    ///
    /// Returns `None` if the file is missing or its metadata/mtime can't be
    /// read — mirroring the Swift guard that yields `nil` (no cache) in that
    /// case rather than hashing a bogus seed.
    pub fn from_path(path: &Path) -> Option<SourceStat> {
        let meta = std::fs::metadata(path).ok()?;
        let mtime = meta.modified().ok()?;
        let mtime_epoch_secs = system_time_to_epoch_secs(mtime)?;
        Some(SourceStat {
            size: meta.len(),
            mtime_epoch_secs,
        })
    }
}

/// Convert a [`SystemTime`] to whole seconds since the Unix epoch, handling
/// pre-1970 times (negative) the way `timeIntervalSince1970` would.
fn system_time_to_epoch_secs(t: SystemTime) -> Option<i64> {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).ok(),
        Err(e) => i64::try_from(e.duration().as_secs()).ok().map(|s| -s),
    }
}

/// Build the `path|size|mtime` seed string the digest is taken over.
///
/// `path` is rendered with [`Path::display`] so the same lossless string the
/// rest of the app uses is hashed. Keep this stable: changing the seed shape
/// invalidates every previously-written cache entry.
fn seed(path: &Path, stat: &SourceStat) -> String {
    format!(
        "{}|{}|{}",
        path.display(),
        stat.size,
        stat.mtime_epoch_secs
    )
}

/// Hex-encode the first [`KEY_PREFIX_BYTES`] of `SHA256(seed)`.
fn hash_seed(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let mut out = String::with_capacity(KEY_PREFIX_BYTES * 2);
    for byte in &digest[..KEY_PREFIX_BYTES] {
        // {:02x} → exactly two lowercase hex chars per byte, matching the
        // Swift `String(format: "%02x", $0)`.
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Compute the disk cache key for already-collected source stats.
///
/// `key = hex(SHA256("<path>|<size>|<mtime>")[0..16])` — a 32-char lowercase
/// hex string. Deterministic for identical inputs; changes if size or mtime
/// changes. Use [`cache_key`] when you want it read from the filesystem.
pub fn cache_key_for_stat(path: &Path, stat: &SourceStat) -> String {
    hash_seed(&seed(path, stat))
}

/// Compute the disk cache key for `path`, reading its size + mtime from disk.
///
/// Returns `None` if the file's metadata can't be read (parity with the Swift
/// `diskCacheKey` `nil` branch → caller skips the disk cache).
pub fn cache_key(path: &Path) -> Option<String> {
    let stat = SourceStat::from_path(path)?;
    Some(cache_key_for_stat(path, &stat))
}

/// R-7 fallback hook (feature-gated, OFF by default): mix a short content
/// prefix into the seed so coarse-mtime collisions on FAT/exFAT don't false-hit.
///
/// This is intentionally **not** the parity key — enabling the
/// `content-prefix-fallback` feature changes the key shape and so invalidates
/// existing entries. It exists so the R-7 mitigation can be turned on later
/// without reworking call sites. `prefix` is caller-supplied (e.g. the first N
/// bytes of the file) to keep this module free of read policy.
#[cfg(feature = "content-prefix-fallback")]
pub fn cache_key_with_content_prefix(path: &Path, stat: &SourceStat, prefix: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seed(path, stat).as_bytes());
    hasher.update(b"|");
    hasher.update(prefix);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(KEY_PREFIX_BYTES * 2);
    for byte in &digest[..KEY_PREFIX_BYTES] {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn stat(size: u64, mtime: i64) -> SourceStat {
        SourceStat {
            size,
            mtime_epoch_secs: mtime,
        }
    }

    #[test]
    fn key_is_16_byte_prefix_32_hex_chars() {
        let p = PathBuf::from("/clips/a.mp4");
        let k = cache_key_for_stat(&p, &stat(1234, 1_700_000_000));
        assert_eq!(k.len(), KEY_PREFIX_BYTES * 2, "16 bytes = 32 hex chars");
        assert_eq!(k.len(), 32);
        assert!(
            k.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "key must be lowercase hex, got {k}"
        );
    }

    #[test]
    fn key_is_deterministic_for_identical_inputs() {
        let p = PathBuf::from("/clips/a.mp4");
        let s = stat(1234, 1_700_000_000);
        let a = cache_key_for_stat(&p, &s);
        let b = cache_key_for_stat(&p, &s);
        assert_eq!(a, b, "identical inputs must yield an identical key");
    }

    #[test]
    fn key_changes_when_size_changes() {
        let p = PathBuf::from("/clips/a.mp4");
        let a = cache_key_for_stat(&p, &stat(1234, 1_700_000_000));
        let b = cache_key_for_stat(&p, &stat(1235, 1_700_000_000));
        assert_ne!(a, b, "a changed size must invalidate the key");
    }

    #[test]
    fn key_changes_when_mtime_changes() {
        let p = PathBuf::from("/clips/a.mp4");
        let a = cache_key_for_stat(&p, &stat(1234, 1_700_000_000));
        let b = cache_key_for_stat(&p, &stat(1234, 1_700_000_001));
        assert_ne!(a, b, "a changed mtime must invalidate the key");
    }

    #[test]
    fn key_changes_when_path_changes() {
        let s = stat(1234, 1_700_000_000);
        let a = cache_key_for_stat(&PathBuf::from("/clips/a.mp4"), &s);
        let b = cache_key_for_stat(&PathBuf::from("/clips/b.mp4"), &s);
        assert_ne!(a, b, "a different path must yield a different key");
    }

    #[test]
    fn seed_shape_is_pipe_joined() {
        // Lock the seed format so we don't silently invalidate the cache.
        let p = PathBuf::from("/clips/a.mp4");
        let s = seed(&p, &stat(42, 7));
        assert_eq!(s, format!("{}|42|7", p.display()));
    }

    #[test]
    fn cache_key_reads_real_file_and_invalidates_on_edit() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.bin");

        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"hello").unwrap();
        f.sync_all().unwrap();
        drop(f);
        let k1 = cache_key(&path).expect("key for existing file");
        assert_eq!(k1.len(), 32);

        // Grow the file → size changes → key must change even if mtime is coarse.
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b" world, extra bytes").unwrap();
        f.sync_all().unwrap();
        drop(f);
        let k2 = cache_key(&path).expect("key after edit");
        assert_ne!(k1, k2, "editing the source must miss the stale entry");
    }

    #[test]
    fn cache_key_missing_file_is_none() {
        assert!(cache_key(&PathBuf::from("/no/such/file/at/all.xyz")).is_none());
    }
}
