//! The `.embed` (PALMEMB1) binary format — byte-exact with `EmbeddingStore.swift`.
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
//! key order regardless. See FINDINGS "the `.embed` format decision".

use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};
use half::f16;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAGIC: &[u8; 8] = b"PALMEMB1";

/// `.embed` JSON header — mirrors `EmbeddingStore.Header` (camelCase keys).
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
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub time: f64,
    pub shot_start: f64,
    pub shot_end: f64,
}

/// A loaded asset index: header + per-row metadata + flat `count*dim` f32 vectors
/// (widened from the on-disk f16, exactly like the reference widens to Float32).
#[derive(Debug, Clone, PartialEq)]
pub struct AssetIndex {
    pub header: Header,
    pub rows: Vec<Row>,
    pub vectors: Vec<f32>,
}

/// Serialize an asset index to the PALMEMB1 byte stream.
///
/// `vectors` is flat `count*dim` f32 (unit-normalized); each value is stored as f16.
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
    let header: Header = serde_json::from_slice(&data[12..header_end]).context("decode header json")?;

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

/// Read the JSON header cheaply (magic + len + json prefix) without the row body —
/// mirrors the reference's `header(key:)` FileHandle prefix read.
pub fn read_header(path: &Path) -> Result<Header> {
    let mut f = std::fs::File::open(path)?;
    let mut prefix = [0u8; 12];
    f.read_exact(&mut prefix)?;
    if &prefix[..8] != MAGIC {
        bail!("bad magic");
    }
    let json_len = u32::from_le_bytes(prefix[8..12].try_into().unwrap()) as usize;
    let mut json = vec![0u8; json_len];
    f.read_exact(&mut json)?;
    Ok(serde_json::from_slice(&json)?)
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
    fn header_json_field_order_is_reference_order() {
        // The reference Header is {model, modelVersion, samplerVersion, dim, count}.
        // serde emits in declaration order; assert the camelCase keys appear in order.
        let (h, _, _) = sample();
        let s = String::from_utf8(serde_json::to_vec(&h).unwrap()).unwrap();
        let order = ["model", "modelVersion", "samplerVersion", "dim", "count"];
        let mut last = 0usize;
        for k in order {
            let pos = s.find(&format!("\"{k}\"")).unwrap_or_else(|| panic!("missing key {k} in {s}"));
            assert!(pos >= last, "key {k} out of order in {s}");
            last = pos;
        }
    }

    #[test]
    fn read_header_prefix_only() {
        let (h, rows, vecs) = sample();
        let bytes = encode(&h, &rows, &vecs).unwrap();
        let tmp = std::env::temp_dir().join("s3-siglip2-test.embed");
        std::fs::write(&tmp, &bytes).unwrap();
        let read = read_header(&tmp).unwrap();
        assert_eq!(read, h);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn cache_key_is_32_hex_chars_and_stable() {
        let k = cache_key("C:/media/clip.mp4", 1_700_000_000.0, 1234);
        assert_eq!(k.len(), 32);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
        // changing any component changes the key
        assert_ne!(k, cache_key("C:/media/clip.mp4", 1_700_000_001.0, 1234));
        assert_ne!(k, cache_key("C:/media/clip.mp4", 1_700_000_000.0, 1235));
    }
}
