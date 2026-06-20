//! End-to-end parity pipeline (no model): preprocess -> (synthetic embed) -> store
//! round-trip -> rank with the reference cutoffs. Proves the non-ONNX surface wires
//! together exactly as the reference's index+query path does.

use s3_siglip2::{embed, preprocess, rank, spec, store};

use image::{DynamicImage, RgbImage};

/// A deterministic stand-in for the ONNX encoder: hash the pixel_values into a
/// pseudo-embedding, then L2-normalize (exactly what the real path does to
/// pooler_output). This lets us exercise the WHOLE pipeline with no weights.
fn fake_encode(pixel_values: &[f32]) -> Vec<f32> {
    let mut v = vec![0.0f32; spec::EMBEDDING_DIM];
    for (i, p) in pixel_values.iter().enumerate() {
        v[i % spec::EMBEDDING_DIM] += *p;
    }
    embed::l2_normalize(&mut v);
    v
}

#[test]
fn full_index_then_query_round_trips_and_ranks() {
    // 1. preprocess two distinct stills.
    let red = DynamicImage::ImageRgb8(RgbImage::from_pixel(320, 240, image::Rgb([220, 30, 30])));
    let blue = DynamicImage::ImageRgb8(RgbImage::from_pixel(320, 240, image::Rgb([30, 30, 220])));
    let red_pv = preprocess::to_pixel_values(&red);
    let blue_pv = preprocess::to_pixel_values(&blue);

    // 2. encode (synthetic) -> unit vectors.
    let red_vec = fake_encode(&red_pv);
    let blue_vec = fake_encode(&blue_pv);
    assert_eq!(red_vec.len(), spec::EMBEDDING_DIM);

    // 3. build an .embed for an asset with two frames (two shots), persist to bytes,
    //    reload — proving the PALMEMB1 format round-trips the real-dim vectors.
    let header = store::Header {
        model: spec::MODEL.into(),
        model_version: spec::MODEL_VERSION,
        sampler_version: spec::SAMPLER_VERSION,
        dim: spec::EMBEDDING_DIM,
        count: 2,
    };
    let rows = vec![
        store::Row { time: 0.0, shot_start: 0.0, shot_end: 2.0 },
        store::Row { time: 2.0, shot_start: 2.0, shot_end: 4.0 },
    ];
    let mut flat = red_vec.clone();
    flat.extend_from_slice(&blue_vec);
    let bytes = store::encode(&header, &rows, &flat).unwrap();
    let index = store::decode(&bytes).unwrap();
    assert_eq!(index.header.count, 2);
    assert_eq!(index.vectors.len(), 2 * spec::EMBEDDING_DIM);

    // 4. query with the red embedding -> the red frame must win and survive the
    //    0.05 floor + 0.85 relative cutoff. (Self-similarity == 1.0 after f16.)
    let hits = rank::search(
        &red_vec,
        &[("asset-1".into(), index)],
        20,
        spec::RELATIVE_CUTOFF,
        Some(spec::COSINE_FLOOR),
    );
    assert!(!hits.is_empty(), "query should produce at least the self-match");
    // top hit is the red frame (shotStart 0.0, time 0.0)
    assert_eq!(hits[0].shot_start, 0.0);
    assert!(hits[0].score > 0.99, "self-cosine should be ~1.0, got {}", hits[0].score);
}
