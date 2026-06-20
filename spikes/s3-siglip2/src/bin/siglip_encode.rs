//! Live SigLIP2 encode + cosine-similarity sanity check (feature `ort` only).
//!
//! Usage (from inside spikes/s3-siglip2):
//!   set SIGLIP_MODEL_DIR=C:\path\with\vision_model.onnx,text_model.onnx,tokenizer.json
//!   set ORT_DYLIB_PATH=C:\path\to\onnxruntime.dll      (load-dynamic feature)
//!   pwsh -File ../../scripts/with-msvc.ps1 cargo run --features ort --bin siglip_encode -- IMAGE.jpg "a cat on a sofa" "a city street at night"
//!
//! Prints the cosine of the image embedding against each text query. A correct
//! model gives a clearly higher score for the matching caption. This is the
//! "real encode" parity confirmation the spike documents as the remaining live step.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use s3_siglip2::embed::cosine;
use s3_siglip2::onnx::VisualEmbedder;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let image = args.next().context("usage: siglip_encode <image> <text...>")?;
    let texts: Vec<String> = args.collect();
    anyhow::ensure!(!texts.is_empty(), "provide at least one text query");

    let model_dir: PathBuf = std::env::var("SIGLIP_MODEL_DIR")
        .context("set SIGLIP_MODEL_DIR to the dir with vision_model.onnx/text_model.onnx/tokenizer.json")?
        .into();

    eprintln!("loading SigLIP2 ONNX encoders from {}", model_dir.display());
    let mut embedder = VisualEmbedder::from_dir(&model_dir)?;

    let img_vec = embedder.encode_image_path(Path::new(&image))?;
    eprintln!("encoded image {image} -> {}-dim unit vector", img_vec.len());

    println!("cosine(image, text):");
    for t in &texts {
        let tv = embedder.encode_text(t)?;
        let score = cosine(&img_vec, &tv);
        println!("  {score:+.4}  \"{t}\"");
    }
    Ok(())
}
