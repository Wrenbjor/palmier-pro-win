//! # palmier-search
//!
//! Visual frame index (SigLIP2/CLIP embeddings) + transcript full-text search
//! (FOUNDATION §4, §6.10). Embeds frames/queries via `candle` or `ort`; those
//! heavy deps are added per-story, not in this skeleton.
//!
//! ## E11-S2 — `.embed` binary store + cache-key identity ([`store`])
//!
//! The [`store`] module implements the `.embed` (PALMEMB1) binary format
//! byte-exactly with the macOS reference `EmbeddingStore.swift`, plus the disk
//! [`EmbeddingStore`](store::EmbeddingStore) (cache-dir resolution, atomic write,
//! `is_current`, `clear_all`) and the file-identity cache key
//! [`cache_key`](store::cache_key) `SHA256(path|mtime|size)[:32]`.
//!
//! Ported from Spike S-3 (`spikes/s3-siglip2/`), which proved the layout round-trips
//! against a macOS-written header. Per the S-3 ruling (`docs/phase0-reconciliation.md`),
//! the format/magic is preserved but [`spec::MODEL_VERSION`] is **2** to force a clean
//! re-index — ONNX (ort) vectors are not bit-equivalent to the reference CoreML vectors,
//! so a macOS-built index is treated as stale on the port.

pub mod store;
pub mod visual_search;

/// The reference model spec (`SearchIndexConfig.manifest` in the macOS reference).
/// These constants are the parity contract; changing any of them re-indexes.
pub mod spec {
    /// Model identifier written into the `.embed` JSON header (`Header.model`).
    pub const MODEL: &str = "siglip2-base-patch16-256";
    /// `Header.modelVersion`. **2** on the port (S-3 ruling): keep the PALMEMB1 byte
    /// format but bump the version so any macOS-authored index (CoreML vectors, not
    /// bit-equivalent to the port's ONNX vectors) is treated as stale and rebuilt.
    pub const MODEL_VERSION: i64 = 2;
    /// `Header.samplerVersion` (FrameSampler.samplerVersion). 1 in the reference.
    pub const SAMPLER_VERSION: i64 = 1;
    /// Embedding dimensionality (`Header.dim`). SigLIP2 base = 768.
    pub const EMBEDDING_DIM: usize = 768;
    /// Square image input edge in px (squash-resize target).
    pub const IMAGE_SIZE: usize = 256;
    /// Text context length: pad/truncate to this many token ids.
    pub const CONTEXT_LENGTH: usize = 64;

    /// Ranking: drop hits below this absolute cosine (`visualMatchCosineFloor`).
    pub const COSINE_FLOOR: f32 = 0.05;
    /// Ranking: keep hits within this fraction of the top score (`relativeCutoff`).
    pub const RELATIVE_CUTOFF: f32 = 0.85;
}

pub use store::{AssetIndex, EmbeddingStore, Header, Row};
pub use visual_search::{search as visual_search, Hit};
