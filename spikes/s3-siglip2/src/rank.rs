//! Visual-search ranking — reproduce `VisualSearch.search` exactly.
//!
//! Reference (`Query/VisualSearch.swift`):
//!   - per asset: `scores = vectors(count x dim) . query` (raw dot, sgemv);
//!   - **best-per-shot dedupe:** keep only the highest-scoring frame per `shotStart`;
//!   - sort all hits desc by score;
//!   - drop `< minScore` (the 0.05 cosine floor, when supplied);
//!   - require top > 0;
//!   - take `prefix(limit)` then keep `score >= top * relativeCutoff (0.85)`.
//!
//! We use a plain Rust dot product instead of BLAS sgemv; with unit vectors the math
//! is identical (FINDINGS notes BLAS/`ndarray`/wgpu as the production options).

use crate::store::AssetIndex;

/// One ranked frame, mirroring `VisualSearch.Hit`.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub asset_id: String,
    pub time: f64,
    pub shot_start: f64,
    pub shot_end: f64,
    pub score: f32,
}

/// Rank `query` (a unit 768-vector) across the given per-asset indexes.
///
/// `min_score` is the absolute cosine floor (reference passes 0.05). `relative_cutoff`
/// is the keep-within-top fraction (reference 0.85). Matches the reference ordering
/// and cutoff sequence byte-for-byte in behavior.
pub fn search(
    query: &[f32],
    indexes: &[(String, AssetIndex)],
    limit: usize,
    relative_cutoff: f32,
    min_score: Option<f32>,
) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();

    for (asset_id, index) in indexes {
        let dim = index.header.dim;
        if dim != query.len() || index.header.count == 0 {
            continue;
        }
        // scores[i] = dot(row_i, query)
        let mut scores = vec![0.0f32; index.header.count];
        for i in 0..index.header.count {
            let base = i * dim;
            let row = &index.vectors[base..base + dim];
            scores[i] = row.iter().zip(query).map(|(a, b)| a * b).sum();
        }

        // best-per-shot: highest score per shotStart bucket. Reference keeps the
        // FIRST-seen row on a tie (`existing.score >= score { continue }`), so we
        // replicate that strict-greater replacement.
        use std::collections::HashMap;
        // key by shotStart bits so f64 is a valid map key (reference keys by Double).
        let mut best: HashMap<u64, (usize, f32)> = HashMap::new();
        for (i, &score) in scores.iter().enumerate() {
            let shot_bits = index.rows[i].shot_start.to_bits();
            match best.get(&shot_bits) {
                Some(&(_, existing)) if existing >= score => {}
                _ => {
                    best.insert(shot_bits, (i, score));
                }
            }
        }
        for (_, (row_idx, score)) in best {
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

    // sort desc by score (reference: `hits.sort { $0.score > $1.score }`)
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    if let Some(min) = min_score {
        hits.retain(|h| h.score >= min);
    }

    let top = match hits.first() {
        Some(h) if h.score > 0.0 => h.score,
        _ => return Vec::new(),
    };
    let floor = top * relative_cutoff;
    hits.into_iter().take(limit).filter(|h| h.score >= floor).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Header, Row};
    use crate::spec;

    fn idx(rows: Vec<(f64, f64, f64)>, vecs: Vec<Vec<f32>>) -> AssetIndex {
        let dim = vecs[0].len();
        let mut flat = Vec::new();
        for v in &vecs {
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

    #[test]
    fn ranks_and_applies_relative_cutoff() {
        // query = [1,0]; rows align/oppose. Use unit vectors.
        let index = idx(
            vec![(0.0, 0.0, 1.0), (1.0, 1.0, 2.0), (2.0, 2.0, 3.0)],
            vec![vec![1.0, 0.0], vec![0.9, 0.43589], vec![0.0, 1.0]],
        );
        let hits = search(&[1.0, 0.0], &[("a".into(), index)], 20, 0.85, Some(spec::COSINE_FLOOR));
        // top=1.0; 0.9 >= 0.85 kept; 0.0 dropped by both floor and cutoff.
        assert_eq!(hits.len(), 2);
        assert!((hits[0].score - 1.0).abs() < 1e-5);
        assert!((hits[1].score - 0.9).abs() < 1e-4);
    }

    #[test]
    fn best_per_shot_dedupes() {
        // two frames in the SAME shot (shotStart 0); only the better survives.
        let index = idx(
            vec![(0.0, 0.0, 5.0), (1.0, 0.0, 5.0)],
            vec![vec![0.6, 0.0], vec![1.0, 0.0]],
        );
        let hits = search(&[1.0, 0.0], &[("a".into(), index)], 20, 0.85, Some(spec::COSINE_FLOOR));
        assert_eq!(hits.len(), 1);
        assert!((hits[0].score - 1.0).abs() < 1e-5);
        assert_eq!(hits[0].time, 1.0);
    }

    #[test]
    fn cosine_floor_drops_weak_hits() {
        // single weak hit below 0.05 -> empty.
        let index = idx(vec![(0.0, 0.0, 1.0)], vec![vec![0.02, 0.9998]]);
        let hits = search(&[1.0, 0.0], &[("a".into(), index)], 20, 0.85, Some(spec::COSINE_FLOOR));
        assert!(hits.is_empty());
    }
}
