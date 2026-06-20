//! The `.embed` (PALMEMB1) binary format + on-disk `EmbeddingStore` — byte-exact
//! with the macOS reference `EmbeddingStore.swift`.
//!
//! Ported from Spike S-3 (`spikes/s3-siglip2/src/store.rs`), which proved the layout
//! round-trips against a macOS-written header. This module adds the disk layer the
//! reference carries (cache-dir resolution, atomic write, `is_current`, `clear_all`).
//!
//! Layout (reference `EmbeddingStore.save`/`load`):
//! ```text
//!   magic  : 8 bytes  = "PALMEMB1"
//!   jsonLen: u32 LE
//!   header : <jsonLen> bytes of JSON (Header, see below)
//!   rows   : count * rowBytes, where rowBytes = 3*8 + dim*2
//!     each row: time:f64 LE, shotStart:f64 LE, shotEnd:f64 LE, then dim x f16 LE
//! ```
//! Total file = 8 + 4 + jsonLen + count*(24 + dim*2). Written atomically.
//! Apple is little-endian; `Float16` is IEEE 754 binary16 == `half::f16` bit-for-bit,
//! and `Double`/`Float64` LE == Rust `f64::to_le_bytes`. So a file written here is
//! byte-identical to one the macOS app wrote, given the SAME header JSON bytes.
//!
//! HEADER JSON PARITY CAVEAT: Swift's `JSONEncoder` emits keys in a fixed but
//! implementation-defined order and no whitespace. `serde_json` emits struct fields
//! in declaration order, no whitespace. For BYTE-EXACT cross-build reuse the header
//! JSON must match exactly (key order + spacing). We declare the fields in the
//! reference's struct order and use `camelCase` to match; the reader is tolerant of
//! key order regardless. (The port re-indexes anyway — `modelVersion` is bumped to 2
//! per the S-3 ruling — so cross-build *vector* reuse is intentionally avoided; the
//! byte-exact format is preserved so a macOS header is still cheaply readable.)

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{bail, Context, Result};
use half::f16;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"PALMEMB1";

/// `.embed` JSON header — mirrors `EmbeddingStore.Header` (camelCase keys, reference
/// field order: `{model, modelVersion, samplerVersion, dim, count}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub model: String,
    #[serde(rename = "modelVersion")]
    pub model_version: i64,
    #[serde(rename = "samplerVersion")]
    pub sampler_version: i64,
    pub dim: usize,
    pub count: usize,
}

/// One row's scalar metadata (the embedding vector is stored separately, flattened).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    pub time: f64,
    pub shot_start: f64,
    pub shot_end: f64,
}

/// A loaded asset index: header + per-row metadata + flat `count*dim` f32 vectors
/// (widened from the on-disk f16, exactly like the reference widens to Float32 for BLAS).
#[derive(Debug, Clone, PartialEq)]
pub struct AssetIndex {
    pub header: Header,
    pub rows: Vec<Row>,
    pub vectors: Vec<f32>,
}

/// Serialize an asset index to the PALMEMB1 byte stream.
///
/// `vectors` is flat `count*dim` f32 (expected unit-normalized at index time); each
/// value is narrowed to f16 on disk (reference `Float16`).
pub fn encode(header: &Header, rows: &[Row], vectors: &[f32]) -> Result<Vec<u8>> {
    if rows.len() != header.count || vectors.len() != header.count * header.dim {
        bail!(
            "shape mismatch: rows={} count={} vectors={} expected={}",
            rows.len(),
            header.count,
            vectors.len(),
            header.count * header.dim
        );
    }
    let json = serde_json::to_vec(header)?;
    let mut out = Vec::with_capacity(8 + 4 + json.len() + header.count * (24 + header.dim * 2));
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    for (i, row) in rows.iter().enumerate() {
        out.extend_from_slice(&row.time.to_le_bytes());
        out.extend_from_slice(&row.shot_start.to_le_bytes());
        out.extend_from_slice(&row.shot_end.to_le_bytes());
        let base = i * header.dim;
        for d in 0..header.dim {
            let h = f16::from_f32(vectors[base + d]);
            out.extend_from_slice(&h.to_le_bytes());
        }
    }
    Ok(out)
}

/// Parse a PALMEMB1 byte stream into an `AssetIndex` (f16 widened to f32).
pub fn decode(data: &[u8]) -> Result<AssetIndex> {
    if data.len() < 12 || &data[..8] != MAGIC {
        bail!("not a PALMEMB1 file (bad magic or too short)");
    }
    let json_len = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
    let header_end = 12 + json_len;
    if data.len() < header_end {
        bail!("truncated header");
    }
    let header: Header =
        serde_json::from_slice(&data[12..header_end]).context("decode header json")?;

    let row_bytes = 24 + header.dim * 2;
    let expected = header_end + header.count * row_bytes;
    if data.len() != expected {
        bail!("size mismatch: file={} expected={}", data.len(), expected);
    }

    let mut rows = Vec::with_capacity(header.count);
    let mut vectors = vec![0.0f32; header.count * header.dim];
    let mut off = header_end;
    for i in 0..header.count {
        let time = f64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        let shot_start = f64::from_le_bytes(data[off + 8..off + 16].try_into().unwrap());
        let shot_end = f64::from_le_bytes(data[off + 16..off + 24].try_into().unwrap());
        rows.push(Row { time, shot_start, shot_end });
        let mut p = off + 24;
        for d in 0..header.dim {
            let h = f16::from_le_bytes(data[p..p + 2].try_into().unwrap());
            vectors[i * header.dim + d] = h.to_f32();
            p += 2;
        }
        off += row_bytes;
    }
    Ok(AssetIndex { header, rows, vectors })
}

/// Read the JSON header cheaply (magic + len + json prefix) from raw bytes — mirrors
/// the reference's `header(key:)` FileHandle prefix read; no row body parsed.
fn header_from_prefix<R: Read>(r: &mut R) -> Result<Header> {
    let mut prefix = [0u8; 12];
    r.read_exact(&mut prefix)?;
    if &prefix[..8] != MAGIC {
        bail!("bad magic");
    }
    let json_len = u32::from_le_bytes(prefix[8..12].try_into().unwrap()) as usize;
    let mut json = vec![0u8; json_len];
    r.read_exact(&mut json)?;
    Ok(serde_json::from_slice(&json)?)
}

/// Read the JSON header cheaply from a file path (seek prefix, no full-file load).
pub fn read_header(path: &Path) -> Result<Header> {
    let mut f = std::fs::File::open(path)?;
    header_from_prefix(&mut f)
}

/// The reference cache-key identity: `SHA256("<path>|<mtime epoch>|<size>")[:32]`
/// hex. `EmbeddingStore.key`. Any mtime/size change yields a new key (re-index).
///
/// NOTE: `mtime` is seconds-since-epoch as an f64 to match Swift's
/// `Date.timeIntervalSince1970`. On Windows the FS mtime resolution is coarser than
/// macOS, so this may false-HIT (not miss) — see search.md ruling #16 / FINDINGS.
pub fn cache_key(path: &str, mtime_epoch_secs: f64, size: u64) -> String {
    let identity = format!("{path}|{mtime_epoch_secs}|{size}");
    let digest = Sha256::digest(identity.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex[..32].to_string()
}

/// Compute the cache key for an on-disk media file by reading its mtime + size.
///
/// Mirrors `EmbeddingStore.key(for:)` (returns `None` if the file is unreadable).
/// `mtime` is rendered as `timeIntervalSince1970` seconds (f64) for Swift parity.
pub fn cache_key_for_file(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    let size = meta.len();
    let mtime = meta.modified().ok()?;
    let secs = mtime
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    Some(cache_key(&path.to_string_lossy(), secs, size))
}

/// Disk cache for per-asset frame embeddings, keyed by file identity.
///
/// Port of `EmbeddingStore` (a Swift `struct` of statics). Here it is a thin handle
/// carrying the embeddings directory so tests can target a temp dir; the production
/// directory is [`EmbeddingStore::default_directory`]
/// (`%APPDATA%\PalmierProWin\Cache\embeddings\` on Windows).
#[derive(Debug, Clone)]
pub struct EmbeddingStore {
    directory: PathBuf,
}

impl EmbeddingStore {
    /// The default embeddings cache directory:
    /// `<config_dir>/PalmierProWin/Cache/embeddings/` on Windows (`%APPDATA%`),
    /// `~/.config/palmier-pro/Cache/embeddings/` on Linux. Same `dirs` convention as
    /// the project registry. Returns `None` only if the OS exposes no config dir.
    ///
    /// (The reference uses `.cachesDirectory/<subsystem>/Embeddings`; the port folds
    /// the cache under the existing `PalmierProWin` app dir — directory is a port
    /// choice, the byte format/magic is the load-bearing contract. search.md note.)
    pub fn default_directory() -> Option<PathBuf> {
        let base = dirs::config_dir()?;
        #[cfg(windows)]
        let dir = base.join("PalmierProWin");
        #[cfg(not(windows))]
        let dir = base.join("palmier-pro");
        Some(dir.join("Cache").join("embeddings"))
    }

    /// Open the store at the default cache directory. Errors only if no OS config dir.
    pub fn open() -> Result<Self> {
        let directory = Self::default_directory()
            .context("no OS config directory for the embeddings cache")?;
        Ok(Self { directory })
    }

    /// Open the store at an explicit directory (tests / non-default cache roots).
    pub fn with_directory(directory: impl Into<PathBuf>) -> Self {
        Self { directory: directory.into() }
    }

    /// The directory this store reads/writes.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// On-disk path for a key: `<directory>/<key>.embed` (reference `diskURL`).
    pub fn disk_path(&self, key: &str) -> PathBuf {
        self.directory.join(format!("{key}.embed"))
    }

    /// Read a key's JSON header cheaply (no row body). `None` if missing/corrupt —
    /// reference `header(key:)` returns `nil` on any failure.
    pub fn header(&self, key: &str) -> Option<Header> {
        read_header(&self.disk_path(key)).ok()
    }

    /// True iff a stored index for `key` matches `(model, model_version, sampler_version)`.
    /// Reference `isCurrent` — gates whether an asset needs re-indexing.
    pub fn is_current(
        &self,
        key: &str,
        model: &str,
        model_version: i64,
        sampler_version: i64,
    ) -> bool {
        match self.header(key) {
            Some(h) => {
                h.model == model
                    && h.model_version == model_version
                    && h.sampler_version == sampler_version
            }
            None => false,
        }
    }

    /// Load a full `AssetIndex` from disk by key.
    pub fn load(&self, key: &str) -> Result<AssetIndex> {
        let path = self.disk_path(key);
        let data = std::fs::read(&path)
            .with_context(|| format!("read embed cache {}", path.display()))?;
        decode(&data)
    }

    /// Save an `AssetIndex` to `<directory>/<key>.embed` **atomically** (write to a
    /// sibling temp file, then rename). Reference `save(... options: .atomic)`.
    pub fn save(&self, key: &str, header: &Header, rows: &[Row], vectors: &[f32]) -> Result<()> {
        let bytes = encode(header, rows, vectors)?;
        std::fs::create_dir_all(&self.directory)
            .with_context(|| format!("create cache dir {}", self.directory.display()))?;
        let final_path = self.disk_path(key);
        // Sibling temp on the same volume so the rename is atomic (no cross-device move).
        let tmp_path = self.directory.join(format!("{key}.embed.tmp"));
        std::fs::write(&tmp_path, &bytes)
            .with_context(|| format!("write temp {}", tmp_path.display()))?;
        // On Windows `rename` over an existing file fails; remove first. The whole
        // store is keyed by file identity, so a racing reader either sees the old
        // complete file or the new complete file — never a torn write.
        if final_path.exists() {
            let _ = std::fs::remove_file(&final_path);
        }
        std::fs::rename(&tmp_path, &final_path)
            .with_context(|| format!("atomic rename into {}", final_path.display()))?;
        Ok(())
    }

    /// Wipe the entire embeddings directory. Reference `clearAll()` (best-effort).
    pub fn clear_all(&self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec;

    fn sample() -> (Header, Vec<Row>, Vec<f32>) {
        let dim = 4;
        let header = Header {
            model: spec::MODEL.into(),
            model_version: spec::MODEL_VERSION,
            sampler_version: spec::SAMPLER_VERSION,
            dim,
            count: 2,
        };
        let rows = vec![
            Row { time: 0.0, shot_start: 0.0, shot_end: 3.0 },
            Row { time: 3.0, shot_start: 3.0, shot_end: 6.0 },
        ];
        let vectors = vec![0.5, -0.5, 0.5, -0.5, 0.1, 0.2, 0.3, 0.92736];
        (header, rows, vectors)
    }

    #[test]
    fn round_trips_through_bytes() {
        let (h, rows, vecs) = sample();
        let bytes = encode(&h, &rows, &vecs).unwrap();
        let back = decode(&bytes).unwrap();
        assert_eq!(back.header, h);
        assert_eq!(back.rows, rows);
        // f16 round-trip introduces ~1e-3 error (reference note) — assert within it.
        for (a, b) in back.vectors.iter().zip(&vecs) {
            assert!((a - b).abs() < 2e-3, "f16 drift too large: {a} vs {b}");
        }
    }

    #[test]
    fn f16_round_trip_within_reference_tolerance() {
        // Reference accepts ~1e-3 from the Float16 storage. Sweep a spread of values
        // (incl. tricky fractions) and assert each widened value is within tolerance
        // of the nearest representable f16 (the only loss the format introduces).
        let dim = 8;
        let vectors: Vec<f32> =
            vec![0.0, 1.0, -1.0, 0.5, -0.5, 0.123_456, -0.987_654, 0.030_303];
        let header = Header {
            model: spec::MODEL.into(),
            model_version: spec::MODEL_VERSION,
            sampler_version: spec::SAMPLER_VERSION,
            dim,
            count: 1,
        };
        let rows = vec![Row { time: 1.5, shot_start: 0.0, shot_end: 1.5 }];
        let bytes = encode(&header, &rows, &vectors).unwrap();
        let back = decode(&bytes).unwrap();
        for (orig, got) in vectors.iter().zip(&back.vectors) {
            // Compare against the exact nearest-f16 (what the format must preserve)…
            let nearest = f16::from_f32(*orig).to_f32();
            assert_eq!(*got, nearest, "decode must equal the stored f16 exactly");
            // …and confirm the absolute error stays within the reference ~1e-3 band.
            assert!(
                (orig - got).abs() < 1e-3,
                "f16 error {} exceeds 1e-3 for {orig}",
                (orig - got).abs()
            );
        }
    }

    #[test]
    fn byte_layout_matches_reference_formula() {
        // total = 8 + 4 + jsonLen + count*(24 + dim*2)
        let (h, rows, vecs) = sample();
        let bytes = encode(&h, &rows, &vecs).unwrap();
        let json_len = serde_json::to_vec(&h).unwrap().len();
        let expected = 8 + 4 + json_len + h.count * (24 + h.dim * 2);
        assert_eq!(bytes.len(), expected);
        assert_eq!(&bytes[..8], MAGIC);
        // jsonLen is LE u32 right after the magic.
        let read_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        assert_eq!(read_len, json_len);
    }

    #[test]
    fn exact_golden_byte_buffer_for_known_count_dim() {
        // Lock the EXACT byte layout for a known (count=1, dim=2) header, so any drift
        // in field order, endianness, or row packing is caught. The header JSON is the
        // serde-emitted bytes (reference field order, camelCase) — we assert the full
        // buffer equals a hand-built golden.
        let header = Header {
            model: "m".into(),
            model_version: 2,
            sampler_version: 1,
            dim: 2,
            count: 1,
        };
        let rows = vec![Row { time: 1.0, shot_start: 0.0, shot_end: 2.0 }];
        // f16(0.5) = 0x3800, f16(-1.0) = 0xBC00 (LE bytes below).
        let vectors = vec![0.5f32, -1.0f32];
        let bytes = encode(&header, &rows, &vectors).unwrap();

        let json = serde_json::to_vec(&header).unwrap();
        let mut golden = Vec::new();
        golden.extend_from_slice(b"PALMEMB1");
        golden.extend_from_slice(&(json.len() as u32).to_le_bytes());
        golden.extend_from_slice(&json);
        golden.extend_from_slice(&1.0f64.to_le_bytes()); // time
        golden.extend_from_slice(&0.0f64.to_le_bytes()); // shotStart
        golden.extend_from_slice(&2.0f64.to_le_bytes()); // shotEnd
        golden.extend_from_slice(&[0x00, 0x38]); // f16(0.5) LE
        golden.extend_from_slice(&[0x00, 0xBC]); // f16(-1.0) LE
        assert_eq!(bytes, golden, "PALMEMB1 byte layout drifted from golden");

        // Sanity: the explicit f16 byte expectations are correct.
        assert_eq!(f16::from_f32(0.5).to_le_bytes(), [0x00, 0x38]);
        assert_eq!(f16::from_f32(-1.0).to_le_bytes(), [0x00, 0xBC]);
    }

    #[test]
    fn header_json_field_order_is_reference_order() {
        // The reference Header is {model, modelVersion, samplerVersion, dim, count}.
        // serde emits in declaration order; assert the camelCase keys appear in order.
        let (h, _, _) = sample();
        let s = String::from_utf8(serde_json::to_vec(&h).unwrap()).unwrap();
        let order = ["model", "modelVersion", "samplerVersion", "dim", "count"];
        let mut last = 0usize;
        for k in order {
            let pos = s
                .find(&format!("\"{k}\""))
                .unwrap_or_else(|| panic!("missing key {k} in {s}"));
            assert!(pos >= last, "key {k} out of order in {s}");
            last = pos;
        }
    }

    #[test]
    fn cache_key_is_32_hex_chars_and_stable() {
        let k = cache_key("C:/media/clip.mp4", 1_700_000_000.0, 1234);
        assert_eq!(k.len(), 32);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
        // changing any component changes the key (mtime/size ⇒ natural re-index)
        assert_ne!(k, cache_key("C:/media/clip.mp4", 1_700_000_001.0, 1234));
        assert_ne!(k, cache_key("C:/media/clip.mp4", 1_700_000_000.0, 1235));
        assert_ne!(k, cache_key("C:/media/other.mp4", 1_700_000_000.0, 1234));
    }

    #[test]
    fn cache_key_matches_known_sha256_prefix() {
        // Lock the identity formula: SHA256("path|mtime|size") hex, first 32 chars.
        // Computed independently (Swift `Date.timeIntervalSince1970` renders an
        // integral epoch as "1700000000" with no decimal, matching Rust's f64 fmt).
        use sha2::{Digest, Sha256};
        let identity = "C:/media/clip.mp4|1700000000|1234";
        let full: String = Sha256::digest(identity.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(cache_key("C:/media/clip.mp4", 1_700_000_000.0, 1234), &full[..32]);
    }

    // --- disk layer (EmbeddingStore) ---

    #[test]
    fn save_load_round_trip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = EmbeddingStore::with_directory(dir.path());
        let (h, rows, vecs) = sample();
        let key = "deadbeefdeadbeefdeadbeefdeadbeef";
        store.save(key, &h, &rows, &vecs).unwrap();
        assert!(store.disk_path(key).exists());
        let back = store.load(key).unwrap();
        assert_eq!(back.header, h);
        assert_eq!(back.rows, rows);
    }

    #[test]
    fn is_current_reflects_header_match() {
        let dir = tempfile::tempdir().unwrap();
        let store = EmbeddingStore::with_directory(dir.path());
        let (h, rows, vecs) = sample();
        let key = "k0";
        // No file yet ⇒ not current.
        assert!(!store.is_current(key, &h.model, h.model_version, h.sampler_version));
        store.save(key, &h, &rows, &vecs).unwrap();
        // Matching header ⇒ current.
        assert!(store.is_current(key, &h.model, h.model_version, h.sampler_version));
        // Any header mismatch ⇒ stale (re-index).
        assert!(!store.is_current(key, "other-model", h.model_version, h.sampler_version));
        assert!(!store.is_current(key, &h.model, h.model_version + 1, h.sampler_version));
        assert!(!store.is_current(key, &h.model, h.model_version, h.sampler_version + 1));
    }

    #[test]
    fn header_reads_prefix_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = EmbeddingStore::with_directory(dir.path());
        let (h, rows, vecs) = sample();
        let key = "kp";
        store.save(key, &h, &rows, &vecs).unwrap();
        assert_eq!(store.header(key).unwrap(), h);
        // Missing key ⇒ None.
        assert!(store.header("nope").is_none());
    }

    #[test]
    fn save_is_atomic_via_temp_rename() {
        let dir = tempfile::tempdir().unwrap();
        let store = EmbeddingStore::with_directory(dir.path());
        let (h, rows, vecs) = sample();
        let key = "atomic";
        store.save(key, &h, &rows, &vecs).unwrap();
        // Overwrite (re-index) must succeed and leave no stray .tmp.
        store.save(key, &h, &rows, &vecs).unwrap();
        let tmp = dir.path().join(format!("{key}.embed.tmp"));
        assert!(!tmp.exists(), "temp file should be renamed away");
        assert!(store.disk_path(key).exists());
    }

    #[test]
    fn clear_all_wipes_directory() {
        let dir = tempfile::tempdir().unwrap();
        let store = EmbeddingStore::with_directory(dir.path().join("embeddings"));
        let (h, rows, vecs) = sample();
        store.save("a", &h, &rows, &vecs).unwrap();
        store.save("b", &h, &rows, &vecs).unwrap();
        assert!(store.directory().exists());
        store.clear_all();
        assert!(!store.directory().exists());
        // Re-save after clear re-creates the dir (clear is not terminal).
        store.save("c", &h, &rows, &vecs).unwrap();
        assert!(store.disk_path("c").exists());
    }

    #[test]
    fn cache_key_for_file_reads_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("clip.mp4");
        std::fs::write(&p, b"hello").unwrap();
        let k = cache_key_for_file(&p).unwrap();
        assert_eq!(k.len(), 32);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
        // Missing file ⇒ None (reference `key(for:)` returns nil).
        assert!(cache_key_for_file(&dir.path().join("missing.mp4")).is_none());
    }
}
