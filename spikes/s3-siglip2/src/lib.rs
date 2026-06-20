//! Spike S-3 — SigLIP2 visual-search encoder parity logic (de-risks M4 Epic 11).
//!
//! This crate reproduces, in pure Rust, the load-bearing *parity surface* of the
//! macOS reference's visual search (`../palmier-pro/Sources/PalmierPro/Search/`):
//!
//!   1. [`preprocess`]  — image -> 256x256 squash (no crop) + black fill + sRGB,
//!      pixels scaled to [-1, 1], channel-first `[1,3,256,256]` f32 (ONNX layout).
//!   2. [`tokenize`]    — SigLIP/Gemma tokenizer, pad-to-64 with id 0, no mask.
//!   3. [`embed`]       — L2-normalize a pooled output to a unit 768-vector.
//!   4. [`rank`]        — raw dot-product ranking + best-per-shot dedupe + the
//!      reference cutoffs (cosine floor 0.05, relative cutoff 0.85).
//!   5. [`store`]       — the `.embed` (PALMEMB1) binary format reader/writer,
//!      byte-exact with `EmbeddingStore.swift` so the macOS-built cache is reusable.
//!
//! The actual ONNX inference (the only part that needs the ~hundreds-of-MB weights
//! and the ONNX Runtime libs) lives in [`onnx`], behind `--features ort`. The
//! default build + `cargo test` exercise everything *except* the live model — that
//! is the spike's "proven without a download" surface.
//!
//! Reference contract (confirmed from the reference Swift + the model cards):
//! - Model: SigLIP2 base patch16-256, 768-dim, image 256, context 64.
//! - ONNX source: `onnx-community/siglip2-base-patch16-256-ONNX`
//!     - `vision_model.onnx`: in `pixel_values [1,3,256,256] f32` -> out `pooler_output [1,768] f32`
//!     - `text_model.onnx`:   in `input_ids [1,64] i64`           -> out `pooler_output [1,768] f32`
//!   The `pooler_output` is NOT L2-normalized by the ONNX graph (unlike the
//!   reference CoreML `embedding` output) — so we normalize explicitly (step 3).
//! - Preprocessing (preprocessor_config.json): resize 256x256 bilinear, rescale
//!   1/255, normalize mean/std 0.5 -> exactly [-1, 1]. Reference squash-resizes a
//!   CGImage into a black-filled BGRA square; geometry is identical, only the tensor
//!   layout differs (CoreML ImageType vs ONNX CHW float).
//! - Tokenizer: GemmaTokenizer, pad `<pad>`=0, eos `<eos>`=1, add_eos_token=true,
//!   do_lower_case=true; reference pads/truncates to 64 with id 0, no attention mask.

pub mod preprocess;
pub mod tokenize;
pub mod embed;
pub mod rank;
pub mod store;
pub mod manifest;

#[cfg(feature = "ort")]
pub mod onnx;

/// The reference model spec (SearchIndexConfig.manifest in the macOS reference).
/// These constants are the parity contract; changing any of them re-indexes.
pub mod spec {
    /// Model identifier written into the `.embed` JSON header (`Header.model`).
    pub const MODEL: &str = "siglip2-base-patch16-256";
    /// `Header.modelVersion`. Bump if the embedding semantics change.
    pub const MODEL_VERSION: i64 = 1;
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
