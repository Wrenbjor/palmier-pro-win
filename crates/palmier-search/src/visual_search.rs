//! # Visual-search ranking — `VisualSearch::search` (E11-S5)
//!
//! Ranks an asset's frame embeddings against a query vector and returns deduped
//! top-K hits, so one scene can't flood results and only confident matches surface.
//! Verbatim behavioral port of the macOS reference
//! `Sources/PalmierPro/Search/Query/VisualSearch.swift`, itself proven in Spike S-3
//! (`spikes/s3-siglip2/src/rank.rs`).
//!
//! ## Algorithm (parity-critical — do not reorder)
//! Per asset:
//!   - `scores = vectors(count×dim) · query` — a **raw dot product**. The reference
//!     uses `cblas_sgemv(CblasRowMajor, CblasNoTrans)`; we use a plain per-row dot
//!     loop (see "Dot-product backend" below). Vectors are assumed **pre-L2-normalized**
//!     at index time (E11-S1 / S-3 ruling, `docs/phase0-reconciliation.md`,
//!     `MODEL_VERSION = 2`), so dot ≡ cosine. We do **not** normalize here.
//!   - **Best-per-shot dedupe:** keep only the highest-scoring frame per `shot_start`.
//!     The bucket key is the `f64` `shot_start` *bits* (`to_bits()`) so it is a valid
//!     `HashMap` key while matching the reference `[Double: …]` keying exactly — stills
//!     bucket at `shot_start 0.0` distinctly. On a tie the **first-seen** row wins
//!     (reference: `existing.score >= score { continue }`).
//!
//! Then across all surviving hits, in this exact order:
//!   1. sort **descending** by score (`hits.sort { $0.score > $1.score }`);
//!   2. drop `< min_score` (the 0.05 cosine floor) when supplied;
//!   3. require `top > 0`, else return empty;
//!   4. `prefix(limit)`;
//!   5. keep `score >= top * relative_cutoff` (the 0.85 relative cutoff).
//!
//! ## Dot-product backend
//! A plain Rust per-row dot loop — **not** BLAS. This box has no guaranteed BLAS, and
//! with pre-normalized unit vectors the math is identical to `sgemv` (the reference's
//! `Accelerate` call). It is portable (no native lib, no extra crate), correctness-first,
//! and the encoder — not this dot — is the cost (S-3 FINDINGS §9.7). A `matrixmultiply`
//! / `ndarray`+BLAS / wgpu backend can swap in behind this same signature if profiling
//! ever shows the dot to matter.

use crate::store::AssetIndex;

/// One ranked frame, mirroring the reference `VisualSearch.Hit`.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    /// Owning asset id (carried through from the `(asset_id, index)` input pair).
    pub asset_id: String,
    /// Frame presentation time (seconds) — `Row.time`.
    pub time: f64,
    /// Shot start (seconds) the frame belongs to — `Row.shot_start` (dedupe bucket).
    pub shot_start: f64,
    /// Shot end (seconds) — `Row.shot_end`.
    pub shot_end: f64,
    /// Raw dot product with the query == cosine for pre-normalized vectors.
    pub score: f32,
}

/// Top hits across assets, best-per-shot.
///
/// Ranks `query` (a unit `EMBEDDING_DIM`-vector) against each `(asset_id, index)`
/// pair and returns the deduped, cut-off top-K. Mirrors the macOS reference
/// `VisualSearch.search` byte-for-byte in behavior.
///
/// - `limit` — keep at most this many hits (`prefix(limit)`); the reference default is 20.
/// - `relative_cutoff` — keep hits within this fraction of the top score; reference 0.85.
/// - `min_score` — absolute cosine floor; the reference passes `Some(0.05)`
///   ([`crate::spec::COSINE_FLOOR`]). `None` skips the absolute floor (reference default).
///
/// Returns `Vec<Hit>` sorted descending by score. Empty if nothing clears the floors
/// or the top score is not `> 0`.
pub fn search(
    query: &[f32],
    indexes: &[(String, AssetIndex)],
    limit: usize,
    relative_cutoff: f32,
    min_score: Option<f32>,
) -> Vec<Hit> {
    use std::collections::HashMap;

    let mut hits: Vec<Hit> = Vec::new();

    for (asset_id, index) in indexes {
        let dim = index.header.dim;
        // Reference guard: `dim == query.count, index.header.count > 0`.
        if dim != query.len() || index.header.count == 0 {
            continue;
        }

        // scores[i] = dot(row_i, query)  — sgemv(RowMajor, NoTrans) analogue.
        // Vectors are flat `count*dim` f32, already widened f16→f32 by the store.
        let mut scores = vec![0.0f32; index.header.count];
        for (i, score) in scores.iter_mut().enumerate() {
            let base = i * dim;
            let row = &index.vectors[base..base + dim];
            *score = row.iter().zip(query).map(|(a, b)| a * b).sum();
        }

        // Best-per-shot dedupe. Key by `shot_start` BITS so the f64 is a valid map key
        // while keying exactly like the reference `[Double: …]` (stills' 0.0 bucket is
        // distinct). First-seen wins on a tie (`existing >= score` => skip).
        let mut best_per_shot: HashMap<u64, (usize, f32)> = HashMap::new();
        for (i, &score) in scores.iter().enumerate() {
            let shot_bits = index.rows[i].shot_start.to_bits();
            match best_per_shot.get(&shot_bits) {
                Some(&(_, existing)) if existing >= score => {}
                _ => {
                    best_per_shot.insert(shot_bits, (i, score));
                }
            }
        }
        for (_, (row_idx, score)) in best_per_shot {
            let row = &index.rows[row_idx];
            hits.push(Hit {
                asset_id: asset_id.clone(),
                time: row.time,
                shot_start: row.shot_start,
                shot_end: row.shot_end,
                score,
            });
        }
    }

    // 1. sort descending by score (reference: `hits.sort { $0.score > $1.score }`).
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    // 2. drop < min_score when supplied (the 0.05 absolute cosine floor).
    if let Some(min) = min_score {
        hits.retain(|h| h.score >= min);
    }

    // 3. require top > 0, else empty.
    let top = match hits.first() {
        Some(h) if h.score > 0.0 => h.score,
        _ => return Vec::new(),
    };

    // 4. prefix(limit) then 5. keep score >= top * relative_cutoff.
    let floor = top * relative_cutoff;
    hits.into_iter()
        .take(limit)
        .filter(|h| h.score >= floor)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec;
    use crate::store::{Header, Row};

    /// Build an `AssetIndex` from `(time, shot_start, shot_end)` rows + matching vectors.
    fn idx(rows: Vec<(f64, f64, f64)>, vecs: Vec<Vec<f32>>) -> AssetIndex {
        let dim = vecs[0].len();
        let mut flat = Vec::new();
        for v in &vecs {
            assert_eq!(v.len(), dim, "ragged test vectors");
            flat.extend_from_slice(v);
        }
        AssetIndex {
            header: Header {
                model: spec::MODEL.into(),
                model_version: spec::MODEL_VERSION,
                sampler_version: spec::SAMPLER_VERSION,
                dim,
                count: rows.len(),
            },
            rows: rows
                .into_iter()
                .map(|(t, s, e)| Row { time: t, shot_start: s, shot_end: e })
                .collect(),
            vectors: flat,
        }
    }

    /// Ranks descending and applies the 0.85 relative cutoff; the absolute floor drops
    /// the orthogonal frame.
    #[test]
    fn ranks_and_applies_relative_cutoff() {
        // query = [1,0]; unit rows aligning / partly-aligning / orthogonal.
        let index = idx(
            vec![(0.0, 0.0, 1.0), (1.0, 1.0, 2.0), (2.0, 2.0, 3.0)],
            vec![vec![1.0, 0.0], vec![0.9, 0.43589], vec![0.0, 1.0]],
        );
        let hits = search(&[1.0, 0.0], &[("a".into(), index)], 20, 0.85, Some(spec::COSINE_FLOOR));
        // top = 1.0; 0.9 >= 0.85 kept; 0.0 dropped (both the floor and the cutoff).
        assert_eq!(hits.len(), 2);
        assert!((hits[0].score - 1.0).abs() < 1e-5);
        assert!((hits[1].score - 0.9).abs() < 1e-4);
        // sorted descending.
        assert!(hits[0].score >= hits[1].score);
    }

    /// Planted-frame parity (feeds SM-12 visual): a synthetic B-roll frame that is
    /// closest to the query lands in the returned top-K, carrying its shot metadata.
    #[test]
    fn planted_broll_frame_is_in_top_k() {
        // Three distinct shots; the planted frame (shot 5.0) is the closest to the query.
        let q = [0.6f32, 0.8]; // unit query.
        let index = idx(
            vec![
                (0.0, 0.0, 2.0),  // shot 0 — weakly aligned
                (3.0, 3.0, 5.0),  // shot 3 — orthogonal-ish
                (6.0, 5.0, 8.0),  // shot 5 — PLANTED: identical to the query direction
            ],
            vec![
                vec![1.0, 0.0],   // dot = 0.6
                vec![0.0, 1.0],   // dot = 0.8
                vec![0.6, 0.8],   // dot = 1.0  (planted)
            ],
        );
        let hits = search(&q, &[("broll".into(), index)], 20, 0.85, Some(spec::COSINE_FLOOR));
        // Planted shot must be the top hit, carrying its shot_start / shot_end / time.
        assert_eq!(hits[0].shot_start, 5.0);
        assert_eq!(hits[0].shot_end, 8.0);
        assert_eq!(hits[0].time, 6.0);
        assert_eq!(hits[0].asset_id, "broll");
        assert!((hits[0].score - 1.0).abs() < 1e-5);
        // The planted shot is present in the returned set.
        assert!(hits.iter().any(|h| h.shot_start == 5.0));
    }

    /// Best-per-shot dedupe: two frames in the SAME shot — only the higher-scoring one
    /// survives (one Hit per shot).
    #[test]
    fn best_per_shot_keeps_only_higher_frame() {
        let index = idx(
            vec![(0.0, 0.0, 5.0), (1.0, 0.0, 5.0)], // both shot_start 0.0
            vec![vec![0.6, 0.0], vec![1.0, 0.0]],   // second frame scores higher
        );
        let hits = search(&[1.0, 0.0], &[("a".into(), index)], 20, 0.85, Some(spec::COSINE_FLOOR));
        assert_eq!(hits.len(), 1, "only one hit per shot survives");
        assert!((hits[0].score - 1.0).abs() < 1e-5);
        assert_eq!(hits[0].time, 1.0, "the higher-scoring frame's metadata is kept");
    }

    /// The absolute `min_score` (0.05) floor drops a weak hit, leaving nothing.
    #[test]
    fn min_score_drops_weak_hits() {
        // single hit below 0.05 → empty (dropped before the top>0 guard).
        let index = idx(vec![(0.0, 0.0, 1.0)], vec![vec![0.02, 0.9998]]);
        let hits = search(&[1.0, 0.0], &[("a".into(), index)], 20, 0.85, Some(spec::COSINE_FLOOR));
        assert!(hits.is_empty());
    }

    /// The relative cutoff (0.85) filters frames below `top * 0.85` even when they clear
    /// the absolute floor.
    #[test]
    fn relative_cutoff_filters_below_fraction() {
        // top = 1.0; 0.80 clears the 0.05 floor but is below 0.85 → filtered out.
        let index = idx(
            vec![(0.0, 0.0, 1.0), (1.0, 1.0, 2.0)],
            vec![vec![1.0, 0.0], vec![0.8, 0.6]], // dots: 1.0, 0.8
        );
        let hits = search(&[1.0, 0.0], &[("a".into(), index)], 20, 0.85, Some(spec::COSINE_FLOOR));
        assert_eq!(hits.len(), 1, "0.8 < top*0.85 = 0.85 is filtered");
        assert!((hits[0].score - 1.0).abs() < 1e-5);
    }

    /// `limit` is applied (prefix) BEFORE the relative-cutoff filter, matching the
    /// reference `prefix(limit).filter(...)` order.
    #[test]
    fn limit_prefixes_before_relative_filter() {
        // Three equal-top hits across distinct shots; limit=2 keeps two.
        let index = idx(
            vec![(0.0, 0.0, 1.0), (1.0, 1.0, 2.0), (2.0, 2.0, 3.0)],
            vec![vec![1.0, 0.0], vec![1.0, 0.0], vec![1.0, 0.0]],
        );
        let hits = search(&[1.0, 0.0], &[("a".into(), index)], 2, 0.85, Some(spec::COSINE_FLOOR));
        assert_eq!(hits.len(), 2);
    }

    /// Mismatched query dim and empty-count indexes are skipped (reference guard).
    #[test]
    fn skips_dim_mismatch_and_empty_index() {
        let good = idx(vec![(0.0, 0.0, 1.0)], vec![vec![1.0, 0.0]]);
        let mut empty = idx(vec![(0.0, 0.0, 1.0)], vec![vec![1.0, 0.0]]);
        empty.header.count = 0;
        empty.rows.clear();
        empty.vectors.clear();
        // query dim 3 mismatches the good index (dim 2) → that asset skipped; empty skipped.
        let hits = search(
            &[1.0, 0.0, 0.0],
            &[("good".into(), good), ("empty".into(), empty)],
            20,
            0.85,
            Some(spec::COSINE_FLOOR),
        );
        assert!(hits.is_empty());
    }

    /// `None` min_score skips the absolute floor (reference default); the top>0 guard
    /// and relative cutoff still apply.
    #[test]
    fn none_min_score_skips_absolute_floor() {
        // A 0.02 hit would be dropped by the 0.05 floor; with None it can surface as top.
        let index = idx(vec![(0.0, 0.0, 1.0)], vec![vec![0.02, 0.9998]]);
        let hits = search(&[1.0, 0.0], &[("a".into(), index)], 20, 0.85, None);
        assert_eq!(hits.len(), 1);
        assert!((hits[0].score - 0.02).abs() < 1e-3);
    }
}
