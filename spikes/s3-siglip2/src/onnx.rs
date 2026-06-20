//! The real ONNX encode path (feature `ort`) — the SigLIP2 image/text encoders.
//!
//! Replaces the reference's `VisualEmbedder` (two CoreML `MLModel`s). Loads
//! `vision_model.onnx` + `text_model.onnx` from `onnx-community/siglip2-base-patch16-
//! 256-ONNX`, runs the preprocessed `pixel_values` / padded `input_ids`, reads the
//! `pooler_output`, and L2-normalizes (the ONNX graph does NOT normalize — see
//! `embed.rs`). DirectML (Windows GPU, DX12 — works on this AMD box) is registered
//! first with automatic CPU fallback, mirroring the reference's `.computeUnits=.all`
//! (ANE/GPU/CPU) intent.
//!
//! This module only compiles under `--features ort`, so the default build/test does
//! not need the ONNX Runtime libs or any weights.

use std::path::Path;

use anyhow::{Context, Result};
use ndarray::{Array, IxDyn};
use ort::execution_providers::{CPUExecutionProvider, DirectMLExecutionProvider};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::TensorRef;

use crate::embed;
use crate::preprocess;
use crate::spec::{CONTEXT_LENGTH, EMBEDDING_DIM, IMAGE_SIZE};
use crate::tokenize::SiglipTokenizer;

/// Loaded SigLIP2 ONNX encoders + tokenizer.
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
    /// `tokenizer.json` (the onnx-community repo layout, flattened).
    pub fn from_dir(dir: &Path) -> Result<Self> {
        let image_encoder = build_session(&dir.join("vision_model.onnx"))?;
        let text_encoder = build_session(&dir.join("text_model.onnx"))?;
        let tokenizer = SiglipTokenizer::from_file(&dir.join("tokenizer.json"))?;
        Ok(Self { image_encoder, text_encoder, tokenizer })
    }

    /// Encode an image file -> unit 768-vector.
    pub fn encode_image_path(&mut self, path: &Path) -> Result<Vec<f32>> {
        let pv = preprocess::pixel_values_from_path(path)?;
        self.encode_pixel_values(&pv)
    }

    /// Encode already-preprocessed `pixel_values` (`3*256*256` f32, CHW, [-1,1]).
    pub fn encode_pixel_values(&mut self, pixel_values: &[f32]) -> Result<Vec<f32>> {
        let input: Array<f32, IxDyn> =
            Array::from_shape_vec(IxDyn(&[1, 3, IMAGE_SIZE, IMAGE_SIZE]), pixel_values.to_vec())?;
        let outputs = self
            .image_encoder
            .run(ort::inputs!["pixel_values" => TensorRef::from_array_view(&input)?])?;
        let (_, data) = outputs["pooler_output"].try_extract_tensor::<f32>()?;
        embed::finalize(data.to_vec())
    }

    /// Encode a text query -> unit 768-vector.
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
        embed::finalize(data.to_vec())
    }
}
