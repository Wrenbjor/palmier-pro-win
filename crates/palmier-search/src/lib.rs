//! # palmier-search
//!
//! Visual frame index (SigLIP2/CLIP embeddings) + transcript full-text search
//! (FOUNDATION §4, §6.10). Embeds frames/queries via SigLIP2 ONNX (`ort`).
//!
//! ## E11-S1 — SigLIP2 embedder ([`embedder`], [`preprocess`], [`tokenize`],
//! [`model_loader`], [`manifest`])
//!
//! The production port of Spike S-3 (`spikes/s3-siglip2/`): the [`preprocess`]
//! 256×256 squash / black-fill / sRGB / [-1,1] CHW path, the [`tokenize`] Gemma
//! pad-to-64-id-0-no-mask tokenizer, the [`embedder`] explicit **L2-normalize** (so
//! the [`visual_search`] raw dot == cosine), the [`model_loader`] state machine
//! (`unknown → notInstalled | preparing → ready | downloading | failed`), and the
//! [`manifest`] ONNX download manifest. The real ONNX encode
//! ([`embedder::VisualEmbedder`]) is **feature-gated behind `ort`** (ONNX Runtime 2.x,
//! DirectML + CPU fallback) so the default build needs no `onnxruntime.dll`; the live
//! image/text encode + real cosine is gated on downloading the ~750 MB weights.
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
//!
//! ## E11-S3 — `FrameSampler` (shot detection + keep cadence) ([`sampler`])
//!
//! The [`sampler`] module ports `FrameSampler.swift`: a shot-aware frame stream
//! over a video ([`FrameSampler::frames`]) that decodes candidate times via Epic
//! 4's shared FFmpeg path ([`palmier_media::extract_frame_timed`] — no second
//! decode path), fingerprints each frame with an 8×8 BT.601 [`LumaGrid`], starts
//! a new shot on a luma scene change (`meanDiff > promoteDiff`), and keeps frames
//! on either a new shot or the 8 s coverage floor. The cadence/shot logic lives in
//! the pure [`SamplerState`] / [`candidate_times`] core (`samplerVersion = 1`,
//! matching [`spec::SAMPLER_VERSION`]) so it tests without a decoder.

pub mod embedder;
pub mod indexer;
pub mod manifest;
pub mod model_loader;
pub mod preprocess;
pub mod sampler;
pub mod store;
pub mod tokenize;
pub mod transcript_search;
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

pub use indexer::{
    ExportYield, FrameEmbedder, Indexed, NoExportYield, VisualIndexer,
};
pub use sampler::{
    candidate_times, effective_interval, Frame, FrameSampler, LumaGrid, Options, SampleError,
    SamplerState, SAMPLER_VERSION,
};
pub use store::{AssetIndex, EmbeddingStore, Header, Row};
pub use transcript_search::{
    TranscriptHit, TranscriptSearch, DEFAULT_LANGUAGE, DEFAULT_LIMIT, DEFAULT_MODEL_ID,
};
pub use visual_search::{search as visual_search, Hit};

// E11-S1 — SigLIP2 embedder + preprocessing + tokenizer + model-loader state machine,
// ported from Spike S-3. The real ONNX encode (`VisualEmbedder`) is behind the `ort`
// feature; preprocess/tokenize/normalize/manifest/model-loader-state compile by default.
pub use embedder::{cosine, finalize, l2_normalize};
#[cfg(feature = "ort")]
pub use embedder::VisualEmbedder;
pub use manifest::{ManifestFile, OnnxFiles, OnnxManifest};
pub use model_loader::{InstalledModel, ModelState, VisualModelLoader};
pub use preprocess::{pixel_values_from_path, pixel_values_from_rgb, to_pixel_values};
pub use tokenize::SiglipTokenizer;
