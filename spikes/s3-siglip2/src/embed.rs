//! Embedding normalization.
//!
//! THE load-bearing parity detail (search.md "Vectors assumed pre-normalized"):
//! the ranking path does a **raw dot product** (`cblas_sgemv`, no normalization),
//! so the stored/query vectors MUST be L2-normalized — then dot == cosine.
//!
//! The reference's CoreML model outputs an **already L2-normalized** `embedding`.
//! The ONNX `vision_model.onnx` / `text_model.onnx` output `pooler_output`, which is
//! the SigLIP pooled hidden state and is **NOT** L2-normalized by the graph. So the
//! port MUST normalize it here, at index time AND query time, or the 0.05 floor and
//! 0.85 relative cutoff break. (Confirmed against `khasinski/siglip2-rb`, which runs
//! the same ONNX repo and L2-normalizes `pooler_output` in code before similarity.)

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
        // angle 45deg -> cos = 1/sqrt(2)
        assert!((cosine(&a, &b) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }
}
