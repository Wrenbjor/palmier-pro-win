//! The SigLIP2 visual embedder — image/text → unit 768-vector.
//!
//! Ported from Spike S-3 (`spikes/s3-siglip2/src/embed.rs` + `onnx.rs`), the
//! production port of the macOS reference `VisualEmbedder` (two CoreML `MLModel`s).
//!
//! Two layers:
//!
//! 1. **Normalization (always compiled).** The load-bearing parity detail
//!    (search.md "Vectors assumed pre-normalized"): the ranking path
//!    ([`crate::visual_search`]) does a **raw dot product** with no normalization, so
//!    the stored/query vectors MUST be L2-normalized — then dot == cosine. The
//!    reference's CoreML model outputs an **already L2-normalized** `embedding`; the
//!    ONNX `vision_model.onnx`/`text_model.onnx` output `pooler_output`, which is the
//!    SigLIP pooled hidden state and is **NOT** L2-normalized by the graph. So the
//!    port MUST normalize it here, at index time AND query time, or the 0.05 floor
//!    and 0.85 relative cutoff break. (Confirmed against `khasinski/siglip2-rb`,
//!    which runs the same ONNX repo and L2-normalizes `pooler_output` before
//!    similarity.) [`l2_normalize`] / [`finalize`] / [`cosine`].
//!
//! 2. **Real ONNX encode ([`VisualEmbedder`], feature `ort`).** Loads
//!    `vision_model.onnx` + `text_model.onnx` from
//!    `onnx-community/siglip2-base-patch16-256-ONNX`, runs the preprocessed
//!    `pixel_values` / padded `input_ids`, reads the `pooler_output`, and
//!    L2-normalizes via [`finalize`]. DirectML (Windows GPU, DX12 — works on this
//!    AMD box) is registered first with automatic CPU fallback, mirroring the
//!    reference's `.computeUnits=.all` (ANE/GPU/CPU) intent. This struct only
//!    compiles under `--features ort`, so the default build/test does not need the
//!    ONNX Runtime libs or any weights.

use crate::spec::EMBEDDING_DIM;

/// L2-normalize a vector in place to unit length. No-op for a zero vector (returns
/// it unchanged rather than dividing by zero — a degenerate but safe fallback).
pub fn l2_normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Validate + normalize a pooled output into a unit embedding of the expected dim.
pub fn finalize(pooled: Vec<f32>) -> anyhow::Result<Vec<f32>> {
    anyhow::ensure!(
        pooled.len() == EMBEDDING_DIM,
        "expected {EMBEDDING_DIM}-dim pooled output, got {}",
        pooled.len()
    );
    let mut v = pooled;
    l2_normalize(&mut v);
    Ok(v)
}

/// Cosine similarity assuming inputs are already unit vectors (== dot product).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(feature = "ort")]
pub use ort_backend::VisualEmbedder;

#[cfg(feature = "ort")]
mod ort_backend {
    use std::path::Path;

    use anyhow::{Context, Result};
    use ndarray::{Array, IxDyn};
    use ort::execution_providers::{CPUExecutionProvider, DirectMLExecutionProvider};
    use ort::session::{builder::GraphOptimizationLevel, Session};
    use ort::value::TensorRef;

    use super::finalize;
    use crate::preprocess;
    use crate::spec::{CONTEXT_LENGTH, EMBEDDING_DIM, IMAGE_SIZE};
    use crate::tokenize::SiglipTokenizer;

    /// Loaded SigLIP2 ONNX encoders + tokenizer — the production `VisualEmbedder`.
    ///
    /// Consumed by E11-S4 (indexer: `encode_image` per sampled frame) and E11-S6
    /// (coordinator: `encode_text` per query). Both outputs are unit 768-vectors so
    /// the [`crate::visual_search`] raw-dot ranking == cosine.
    pub struct VisualEmbedder {
        image_encoder: Session,
        text_encoder: Session,
        tokenizer: SiglipTokenizer,
    }

    fn build_session(path: &Path) -> Result<Session> {
        // Register DirectML (GPU) first, then CPU as fallback. ort registers EPs in
        // order and silently falls through to the next if one can't initialize, so a
        // box without a DX12 GPU still runs on CPU. (Set ORT_DYLIB_PATH to the
        // onnxruntime.dll for the `load-dynamic` feature.)
        Session::builder()?
            .with_execution_providers([
                DirectMLExecutionProvider::default().build(),
                CPUExecutionProvider::default().build(),
            ])?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .commit_from_file(path)
            .with_context(|| format!("load ONNX model: {}", path.display()))
    }

    impl VisualEmbedder {
        /// Load from a directory containing `vision_model.onnx`, `text_model.onnx`,
        /// `tokenizer.json` (the onnx-community repo layout, flattened — the layout
        /// E11-S6's `ModelDownloader` installs under the models dir).
        pub fn from_dir(dir: &Path) -> Result<Self> {
            let image_encoder = build_session(&dir.join("vision_model.onnx"))?;
            let text_encoder = build_session(&dir.join("text_model.onnx"))?;
            let tokenizer = SiglipTokenizer::from_file(&dir.join("tokenizer.json"))?;
            Ok(Self { image_encoder, text_encoder, tokenizer })
        }

        /// Construct from already-loaded encoder sessions + tokenizer (used by the
        /// model loader, which resolves the installed model paths itself).
        pub fn new(image_encoder: Session, text_encoder: Session, tokenizer: SiglipTokenizer) -> Self {
            Self { image_encoder, text_encoder, tokenizer }
        }

        /// Encode an in-memory RGB frame (the `palmier_media`/sampler pixel type) →
        /// unit 768-vector. The primary E11-S4 indexer entry point.
        pub fn encode_image(&mut self, img: &image::RgbImage) -> Result<Vec<f32>> {
            let pv = preprocess::pixel_values_from_rgb(img);
            self.encode_pixel_values(&pv)
        }

        /// Encode an image file (jpeg/png) → unit 768-vector.
        pub fn encode_image_path(&mut self, path: &Path) -> Result<Vec<f32>> {
            let pv = preprocess::pixel_values_from_path(path)?;
            self.encode_pixel_values(&pv)
        }

        /// Encode already-preprocessed `pixel_values` (`3*256*256` f32, CHW, [-1,1]).
        pub fn encode_pixel_values(&mut self, pixel_values: &[f32]) -> Result<Vec<f32>> {
            let input: Array<f32, IxDyn> = Array::from_shape_vec(
                IxDyn(&[1, 3, IMAGE_SIZE, IMAGE_SIZE]),
                pixel_values.to_vec(),
            )?;
            let outputs = self
                .image_encoder
                .run(ort::inputs!["pixel_values" => TensorRef::from_array_view(&input)?])?;
            let (_, data) = outputs["pooler_output"].try_extract_tensor::<f32>()?;
            finalize(data.to_vec())
        }

        /// Encode a text query → unit 768-vector. The E11-S6 coordinator query path.
        pub fn encode_text(&mut self, text: &str) -> Result<Vec<f32>> {
            let ids = self.tokenizer.encode(text)?; // Vec<i64>, len 64
            debug_assert_eq!(ids.len(), CONTEXT_LENGTH);
            let input: Array<i64, IxDyn> = Array::from_shape_vec(IxDyn(&[1, CONTEXT_LENGTH]), ids)?;
            let outputs = self
                .text_encoder
                .run(ort::inputs!["input_ids" => TensorRef::from_array_view(&input)?])?;
            let (_, data) = outputs["pooler_output"].try_extract_tensor::<f32>()?;
            anyhow::ensure!(
                data.len() == EMBEDDING_DIM,
                "text pooler_output dim {} != {EMBEDDING_DIM}",
                data.len()
            );
            finalize(data.to_vec())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_to_unit_length() {
        let mut v = vec![3.0, 4.0]; // |v| = 5
        l2_normalize(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6);
        assert!((v[1] - 0.8).abs() < 1e-6);
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-6);
    }

    #[test]
    fn zero_vector_is_safe() {
        let mut v = vec![0.0, 0.0, 0.0];
        l2_normalize(&mut v); // must not produce NaN
        assert!(v.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn dot_of_unit_vectors_is_cosine() {
        let mut a = vec![1.0, 1.0, 0.0];
        let mut b = vec![1.0, 0.0, 0.0];
        l2_normalize(&mut a);
        l2_normalize(&mut b);
        // angle 45° → cos = 1/sqrt(2)
        assert!((cosine(&a, &b) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn finalize_outputs_unit_768() {
        let pooled = vec![2.0f32; EMBEDDING_DIM];
        let v = finalize(pooled).unwrap();
        assert_eq!(v.len(), EMBEDDING_DIM);
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5, "finalize must L2-normalize, |v|={n}");
    }

    #[test]
    fn finalize_rejects_wrong_dim() {
        assert!(finalize(vec![1.0, 2.0, 3.0]).is_err());
    }
}
