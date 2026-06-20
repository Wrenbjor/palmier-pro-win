//! `VisualIndexer` — turn one asset into a persisted `AssetIndex`, idempotently per
//! `(file identity, model, modelVersion, samplerVersion)` (story E11-S4). Port of
//! `Sources/PalmierPro/Search/Indexing/VisualIndexer.swift`.
//!
//! Drives the merged pieces of Epic 11 end-to-end:
//! [`FrameSampler`](crate::FrameSampler) (E11-S3, shot-aware frames) →
//! a [`FrameEmbedder`] (E11-S1 `VisualEmbedder`, 768-d unit vectors) →
//! [`EmbeddingStore`](crate::EmbeddingStore) (E11-S2, `.embed` PALMEMB1 disk format).
//!
//! ## Idempotency
//! [`VisualIndexer::needs_index`] = `!EmbeddingStore::is_current(key, model,
//! modelVersion, samplerVersion)`. The cache key folds `(path | mtime | size)`
//! ([`store::cache_key_for_file`](crate::store::cache_key_for_file)), so any file edit
//! (or a model/sampler version bump) makes the prior index stale and re-indexes;
//! otherwise [`index_video`](VisualIndexer::index_video) /
//! [`index_image`](VisualIndexer::index_image) are no-ops.
//!
//! ## Shot rows (video)
//! Mirrors the reference exactly. For each emitted frame, on `is_new_shot` push a
//! shot-start (**the first shot starts at 0**, every later shot at `frame.time`);
//! `vectors += encode(image)`; record `time` + `shot_index`. A row's `shot_start` is
//! its shot's start; `shot_end` is the **next** shot's start, or `duration` for the
//! last shot. `count` = number of kept frames.
//!
//! ## Stills
//! A still **skips the sampler**: decode a ≤512px thumbnail (the ImageIO analogue),
//! produce ONE embedding, and write a single row `(time: 0, shot_start: 0,
//! shot_end: 0)`. The `shot_start: 0` is load-bearing — E11-S5's best-per-shot
//! dedupe buckets on `shot_start`, so a still must collapse to the same bucket the
//! reference uses.
//!
//! ## Feature-gating (why a trait)
//! The real [`VisualEmbedder`](crate::embedder::VisualEmbedder) only compiles under
//! `--features ort` (it owns ONNX Runtime sessions). The indexing **logic** above is
//! pure orchestration and must build + test by DEFAULT (no ort, no weights). So the
//! indexer is generic over a [`FrameEmbedder`] trait: the default build/tests drive
//! it with a [mock](#tests) embedder (a fixed unit vector), and under `--features
//! ort` a blanket impl adapts the real `VisualEmbedder` to the trait. The byte-exact
//! row/shot/save behavior is therefore exercised without any model.
//!
//! ## Export yield (E11-S6 hook)
//! The reference calls `SearchIndexCoordinator.waitWhileExportActive()` **before each
//! frame embed and once per asset** so indexing pauses while an export contends for
//! the CPU/GPU. The real refcounted pause lands in E11-S6; here it is an **injected
//! no-op hook** ([`ExportYield`]) the indexer calls at exactly those points. E11-S6
//! supplies a real implementation (blocking 2s-loop on the process-global
//! `ExportPauseCounter`) without touching this logic.

use std::path::Path;

use anyhow::{Context, Result};

use crate::sampler::{Frame, FrameSampler, Options};
use crate::spec;
use crate::store::{cache_key_for_file, EmbeddingStore, Header, Row};

/// The embed step the indexer depends on — abstracted so the indexing logic builds
/// and tests by DEFAULT (a mock embedder) while the real ONNX
/// [`VisualEmbedder`](crate::embedder::VisualEmbedder) stays behind `--features ort`.
///
/// `encode_image` takes `&mut self` because the real embedder runs `ort::Session`s
/// (which require `&mut`). It returns a 768-d **L2-normalized** vector (E11-S1
/// guarantees normalization so the [`crate::visual_search`] raw dot == cosine).
pub trait FrameEmbedder {
    /// Encode one RGB frame → unit `EMBEDDING_DIM`-vector.
    fn encode_image(&mut self, img: &image::RgbImage) -> Result<Vec<f32>>;
}

/// Under `--features ort`, the real [`VisualEmbedder`] satisfies [`FrameEmbedder`]
/// directly (same signature), so the E11-S6 coordinator can drive the indexer with
/// the live encoder. This adapter is the only `ort`-gated code in this module.
#[cfg(feature = "ort")]
impl FrameEmbedder for crate::embedder::VisualEmbedder {
    fn encode_image(&mut self, img: &image::RgbImage) -> Result<Vec<f32>> {
        crate::embedder::VisualEmbedder::encode_image(self, img)
    }
}

/// The export-pause hook (E11-S6). Called **before each frame embed** and **once per
/// asset**; a real impl blocks while an export is in flight (the reference's 2s-loop
/// on the process-global `ExportPauseCounter`).
///
/// Stubbed here as an injected trait so E11-S4 carries no export coupling: the
/// default [`NoExportYield`] returns immediately. E11-S6 supplies the refcounted
/// implementation without changing the indexer.
pub trait ExportYield {
    /// Block until no export is active. Default/stub: returns immediately.
    fn wait_while_export_active(&self) -> Result<()> {
        Ok(())
    }
}

/// The default no-op [`ExportYield`] (E11-S6 replaces it with the refcounted pause).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoExportYield;

impl ExportYield for NoExportYield {}

/// Outcome of an index call — distinguishes a real (re)index from an idempotent skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Indexed {
    /// A `.embed` file was written with this many rows (kept frames; 1 for a still).
    Wrote { rows: usize },
    /// Already current for `(model, modelVersion, samplerVersion)` — nothing written.
    Skipped,
    /// No cache key could be computed (unreadable file) — nothing written. Mirrors
    /// the reference's `guard let key … else { return }`.
    NoKey,
}

/// Indexes one asset: sampled frames → embeddings → [`EmbeddingStore`]. Idempotent
/// per `(file identity, model, modelVersion, samplerVersion)`.
///
/// Stateless; each call does all the work. Generic over the [`FrameEmbedder`] (so the
/// default build tests with a mock) and the [`ExportYield`] hook (E11-S6).
pub struct VisualIndexer<'a> {
    store: &'a EmbeddingStore,
}

impl<'a> VisualIndexer<'a> {
    /// New indexer writing into `store`.
    pub fn new(store: &'a EmbeddingStore) -> Self {
        Self { store }
    }

    /// `needsIndex` — true iff `path` has no current index for
    /// `(spec::MODEL, spec::MODEL_VERSION, spec::SAMPLER_VERSION)`. False if the file
    /// is unreadable (no key) — matches the reference's `guard let key else false`.
    pub fn needs_index(&self, path: &Path) -> bool {
        match cache_key_for_file(path) {
            Some(key) => !self.store.is_current(
                &key,
                spec::MODEL,
                spec::MODEL_VERSION,
                spec::SAMPLER_VERSION,
            ),
            None => false,
        }
    }

    /// Index a **video**: sample shot-aware frames, embed each, build shot-boundaried
    /// rows, and save. Idempotent — returns [`Indexed::Skipped`] if already current.
    ///
    /// `duration` is the clip length (s); `larger_edge` is the video's larger natural
    /// edge (px) for the sampler's high-res interval doubling (pass `0` if unknown).
    /// `export` is called before each frame embed and once for the asset (E11-S6).
    pub fn index_video<E, Y>(
        &self,
        path: &Path,
        duration: f64,
        larger_edge: u32,
        embedder: &mut E,
        options: &Options,
        export: &Y,
    ) -> Result<Indexed>
    where
        E: FrameEmbedder,
        Y: ExportYield,
    {
        let Some(key) = cache_key_for_file(path) else {
            return Ok(Indexed::NoKey);
        };
        if self.store.is_current(&key, spec::MODEL, spec::MODEL_VERSION, spec::SAMPLER_VERSION) {
            return Ok(Indexed::Skipped);
        }

        // Per-asset yield (reference: once before the sampler loop in the still path;
        // here we also yield per asset for the video path before sampling/embedding).
        export.wait_while_export_active().context("export yield (per asset)")?;

        let frames = FrameSampler::frames(path, duration, larger_edge, options)
            .map_err(|e| anyhow::anyhow!("frame sampling failed: {e}"))?;

        let (rows, vectors) = self.build_video_rows(&frames, duration, embedder, export)?;
        self.save(&key, &rows, &vectors)?;
        Ok(Indexed::Wrote { rows: rows.len() })
    }

    /// Build `(rows, flat vectors)` from emitted frames — the pure row/shot core
    /// (testable without a decoder). On `is_new_shot` push a shot-start (**first shot
    /// = 0**, else `frame.time`); embed each frame; `shot_index` = current shot.
    /// Then `shot_start` = that shot's start, `shot_end` = next shot's start (or
    /// `duration` for the last). Calls the export hook before **each** embed.
    fn build_video_rows<E, Y>(
        &self,
        frames: &[Frame],
        duration: f64,
        embedder: &mut E,
        export: &Y,
    ) -> Result<(Vec<Row>, Vec<f32>)>
    where
        E: FrameEmbedder,
        Y: ExportYield,
    {
        let mut times: Vec<f64> = Vec::with_capacity(frames.len());
        let mut shot_indices: Vec<usize> = Vec::with_capacity(frames.len());
        let mut shot_starts: Vec<f64> = Vec::new();
        let mut vectors: Vec<f32> = Vec::with_capacity(frames.len() * spec::EMBEDDING_DIM);

        for frame in frames {
            // Reference yields before EACH frame embed.
            export.wait_while_export_active().context("export yield (per frame)")?;
            if frame.is_new_shot {
                // First shot starts at 0; every later shot at the frame's own time.
                shot_starts.push(if shot_starts.is_empty() { 0.0 } else { frame.time });
            }
            let vec = embedder.encode_image(&frame.image)?;
            anyhow::ensure!(
                vec.len() == spec::EMBEDDING_DIM,
                "embedder returned {}-d vector, expected {}",
                vec.len(),
                spec::EMBEDDING_DIM
            );
            vectors.extend_from_slice(&vec);
            times.push(frame.time);
            // shot_starts is non-empty here: the first frame is always a new shot
            // (FrameSampler guarantees it), so the first push above ran.
            shot_indices.push(shot_starts.len() - 1);
        }

        let rows = times
            .iter()
            .zip(&shot_indices)
            .map(|(&time, &shot)| Row {
                time,
                shot_start: shot_starts[shot],
                shot_end: if shot + 1 < shot_starts.len() {
                    shot_starts[shot + 1]
                } else {
                    duration
                },
            })
            .collect();
        Ok((rows, vectors))
    }

    /// Index a **still image**: skip the sampler, decode a ≤512px thumbnail, produce
    /// ONE embedding, write a single row `(time: 0, shot_start: 0, shot_end: 0)`.
    /// Idempotent — returns [`Indexed::Skipped`] if already current.
    ///
    /// The `shot_start: 0` is load-bearing for E11-S5's best-per-shot dedupe.
    pub fn index_image<E, Y>(
        &self,
        path: &Path,
        embedder: &mut E,
        export: &Y,
    ) -> Result<Indexed>
    where
        E: FrameEmbedder,
        Y: ExportYield,
    {
        let Some(key) = cache_key_for_file(path) else {
            return Ok(Indexed::NoKey);
        };
        if self.store.is_current(&key, spec::MODEL, spec::MODEL_VERSION, spec::SAMPLER_VERSION) {
            return Ok(Indexed::Skipped);
        }

        // Per-asset yield (reference yields once before embedding the still).
        export.wait_while_export_active().context("export yield (still asset)")?;

        let image = decode_still_thumbnail(path)
            .with_context(|| format!("decode still {}", path.display()))?;
        let vec = embedder.encode_image(&image)?;
        anyhow::ensure!(
            vec.len() == spec::EMBEDDING_DIM,
            "embedder returned {}-d vector, expected {}",
            vec.len(),
            spec::EMBEDDING_DIM
        );

        let rows = vec![Row { time: 0.0, shot_start: 0.0, shot_end: 0.0 }];
        self.save(&key, &rows, &vec)?;
        Ok(Indexed::Wrote { rows: 1 })
    }

    /// Write the `.embed` header + rows + vectors for `key` (reference `save`). The
    /// header `count` is `rows.len()`; `dim` is [`spec::EMBEDDING_DIM`].
    fn save(&self, key: &str, rows: &[Row], vectors: &[f32]) -> Result<()> {
        let header = Header {
            model: spec::MODEL.into(),
            model_version: spec::MODEL_VERSION,
            sampler_version: spec::SAMPLER_VERSION,
            dim: spec::EMBEDDING_DIM,
            count: rows.len(),
        };
        self.store.save(key, &header, rows, vectors)
    }
}

/// Decode a still image to a ≤512px (max edge) [`RgbImage`] — the Windows analogue of
/// the reference's ImageIO `CGImageSourceCreateThumbnailAtIndex` with
/// `kCGImageSourceThumbnailMaxPixelSize: 512`. `image::thumbnail` fits within the box
/// preserving aspect (never upscaling), matching ImageIO's maxPixelSize semantics.
///
/// The downstream embedder squash-resizes to 256 regardless, so the 512 cap only
/// bounds decode cost / memory — but we keep it for parity with the reference.
fn decode_still_thumbnail(path: &Path) -> Result<image::RgbImage> {
    const STILL_THUMB_MAX: u32 = 512;
    let img = image::open(path).with_context(|| format!("open image {}", path.display()))?;
    Ok(img.thumbnail(STILL_THUMB_MAX, STILL_THUMB_MAX).to_rgb8())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// A deterministic mock embedder: returns a fixed L2-normalized 768-vector,
    /// regardless of input. Exercises the indexing LOGIC (needs_index, shot rows,
    /// still row, save) with NO ort/weights. Counts calls so tests can assert the
    /// per-frame embed count.
    struct MockEmbedder {
        calls: usize,
    }

    impl MockEmbedder {
        fn new() -> Self {
            Self { calls: 0 }
        }
        /// A fixed unit vector: e_0 = 1.0, rest 0 (already L2-normalized).
        fn fixed_unit() -> Vec<f32> {
            let mut v = vec![0.0f32; spec::EMBEDDING_DIM];
            v[0] = 1.0;
            v
        }
    }

    impl FrameEmbedder for MockEmbedder {
        fn encode_image(&mut self, _img: &RgbImage) -> Result<Vec<f32>> {
            self.calls += 1;
            Ok(Self::fixed_unit())
        }
    }

    /// Counts export-yield calls so tests assert the "before each frame embed AND per
    /// asset" contract.
    #[derive(Default)]
    struct CountingYield {
        count: std::cell::Cell<usize>,
    }

    impl ExportYield for CountingYield {
        fn wait_while_export_active(&self) -> Result<()> {
            self.count.set(self.count.get() + 1);
            Ok(())
        }
    }

    fn solid(rgb: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(32, 32, Rgb(rgb))
    }

    /// Build the row set directly from synthetic emitted frames (no decoder), so the
    /// shot_start/shot_end boundary math is what's under test.
    fn rows_from_frames(frames: &[Frame], duration: f64) -> (Vec<Row>, usize, usize) {
        let store = EmbeddingStore::with_directory(std::env::temp_dir());
        let indexer = VisualIndexer::new(&store);
        let mut embedder = MockEmbedder::new();
        let yield_hook = CountingYield::default();
        let (rows, vectors) = indexer
            .build_video_rows(frames, duration, &mut embedder, &yield_hook)
            .unwrap();
        // Vectors must be one EMBEDDING_DIM block per kept frame.
        assert_eq!(vectors.len(), rows.len() * spec::EMBEDDING_DIM);
        (rows, embedder.calls, yield_hook.count.get())
    }

    fn frame(time: f64, is_new_shot: bool) -> Frame {
        Frame { time, image: solid([time as u8, 0, 0]), is_new_shot }
    }

    // ---- AC: 2-shot video → correct shot_start/shot_end boundaries + count --------

    #[test]
    fn two_shot_video_rows_have_correct_shot_boundaries() {
        // Shot A: first frame (new shot) at t=1, plus a coverage-floor frame at t=9
        // (same shot). Shot B: new shot at t=12, plus a same-shot frame at t=20.
        // duration = 24.
        let duration = 24.0;
        let frames = vec![
            frame(1.0, true),   // shot 0 start → shot_start 0 (FIRST shot starts at 0)
            frame(9.0, false),  // shot 0
            frame(12.0, true),  // shot 1 start → shot_start 12
            frame(20.0, false), // shot 1
        ];
        let (rows, embed_calls, _) = rows_from_frames(&frames, duration);

        assert_eq!(rows.len(), 4, "count == kept frames");
        assert_eq!(embed_calls, 4, "one embed per kept frame");

        // Shot 0: starts at 0 (first shot), ends at next shot's start (12).
        assert_eq!(rows[0], Row { time: 1.0, shot_start: 0.0, shot_end: 12.0 });
        assert_eq!(rows[1], Row { time: 9.0, shot_start: 0.0, shot_end: 12.0 });
        // Shot 1: starts at its own frame.time (12), ends at duration (last shot).
        assert_eq!(rows[2], Row { time: 12.0, shot_start: 12.0, shot_end: 24.0 });
        assert_eq!(rows[3], Row { time: 20.0, shot_start: 12.0, shot_end: 24.0 });
    }

    #[test]
    fn first_shot_always_starts_at_zero_even_if_frame_time_nonzero() {
        // Parity: the FIRST shot's start is 0, NOT the first frame's time (which can
        // be ~interval/2 from the sampler cadence).
        let frames = vec![frame(1.0, true)];
        let (rows, _, _) = rows_from_frames(&frames, 10.0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], Row { time: 1.0, shot_start: 0.0, shot_end: 10.0 });
    }

    #[test]
    fn single_shot_last_shot_end_is_duration() {
        let frames = vec![frame(1.0, true), frame(9.0, false), frame(17.0, false)];
        let (rows, _, _) = rows_from_frames(&frames, 24.0);
        // One shot only ⇒ every row spans [0, duration].
        for r in &rows {
            assert_eq!(r.shot_start, 0.0);
            assert_eq!(r.shot_end, 24.0);
        }
    }

    #[test]
    fn three_shots_chain_shot_end_to_next_shot_start() {
        let frames = vec![
            frame(1.0, true),  // shot 0 → start 0
            frame(5.0, true),  // shot 1 → start 5
            frame(9.0, true),  // shot 2 → start 9
        ];
        let (rows, _, _) = rows_from_frames(&frames, 12.0);
        assert_eq!(rows[0], Row { time: 1.0, shot_start: 0.0, shot_end: 5.0 });
        assert_eq!(rows[1], Row { time: 5.0, shot_start: 5.0, shot_end: 9.0 });
        assert_eq!(rows[2], Row { time: 9.0, shot_start: 9.0, shot_end: 12.0 });
    }

    #[test]
    fn export_yield_called_before_each_frame_embed() {
        let frames = vec![frame(1.0, true), frame(9.0, false), frame(12.0, true)];
        let (_, _, yields) = rows_from_frames(&frames, 24.0);
        // build_video_rows yields once per frame (the per-asset yield lives in
        // index_video, exercised separately).
        assert_eq!(yields, 3, "one export yield before each frame embed");
    }

    // ---- AC: still → exactly one row (0,0,0) -------------------------------------

    #[test]
    fn still_produces_single_zero_row() {
        use std::io::Cursor;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("still.png");
        // Write a real PNG so decode_still_thumbnail can open it.
        let img = RgbImage::from_pixel(64, 48, Rgb([10, 120, 200]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&path, &buf).unwrap();

        let store = EmbeddingStore::with_directory(dir.path().join("embeddings"));
        let indexer = VisualIndexer::new(&store);
        let mut embedder = MockEmbedder::new();
        let yield_hook = CountingYield::default();

        let result = indexer
            .index_image(&path, &mut embedder, &yield_hook)
            .unwrap();
        assert_eq!(result, Indexed::Wrote { rows: 1 });
        assert_eq!(embedder.calls, 1, "exactly one embedding for a still");
        assert!(yield_hook.count.get() >= 1, "per-asset export yield ran");

        // Reload and assert the single (0,0,0) row.
        let key = cache_key_for_file(&path).unwrap();
        let index = store.load(&key).unwrap();
        assert_eq!(index.header.count, 1);
        assert_eq!(index.rows.len(), 1);
        assert_eq!(index.rows[0], Row { time: 0.0, shot_start: 0.0, shot_end: 0.0 });
        assert_eq!(index.vectors.len(), spec::EMBEDDING_DIM);
    }

    // ---- idempotency -------------------------------------------------------------

    #[test]
    fn index_image_is_idempotent() {
        use std::io::Cursor;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        let img = RgbImage::from_pixel(20, 20, Rgb([1, 2, 3]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(&path, &buf).unwrap();

        let store = EmbeddingStore::with_directory(dir.path().join("embeddings"));
        let indexer = VisualIndexer::new(&store);
        let yield_hook = CountingYield::default();

        // needs_index before any write.
        assert!(indexer.needs_index(&path));

        let mut e1 = MockEmbedder::new();
        let r1 = indexer.index_image(&path, &mut e1, &yield_hook).unwrap();
        assert!(matches!(r1, Indexed::Wrote { rows: 1 }));
        assert_eq!(e1.calls, 1);

        // Now current ⇒ needs_index false ⇒ second call skips (no embed).
        assert!(!indexer.needs_index(&path));
        let mut e2 = MockEmbedder::new();
        let r2 = indexer.index_image(&path, &mut e2, &yield_hook).unwrap();
        assert_eq!(r2, Indexed::Skipped);
        assert_eq!(e2.calls, 0, "skip must not embed");
    }

    #[test]
    fn needs_index_false_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = EmbeddingStore::with_directory(dir.path());
        let indexer = VisualIndexer::new(&store);
        // Unreadable file ⇒ no key ⇒ needs_index false (reference guard).
        assert!(!indexer.needs_index(&dir.path().join("missing.mp4")));
    }

    #[test]
    fn index_video_no_key_for_missing_file() {
        // index_video on a nonexistent path returns NoKey (no panic, no write).
        let dir = tempfile::tempdir().unwrap();
        let store = EmbeddingStore::with_directory(dir.path());
        let indexer = VisualIndexer::new(&store);
        let mut embedder = MockEmbedder::new();
        let yield_hook = CountingYield::default();
        let r = indexer
            .index_video(
                &dir.path().join("nope.mp4"),
                10.0,
                1920,
                &mut embedder,
                &Options::default(),
                &yield_hook,
            )
            .unwrap();
        assert_eq!(r, Indexed::NoKey);
        assert_eq!(embedder.calls, 0);
    }

    #[test]
    fn no_export_yield_is_a_noop() {
        // The default stub returns Ok immediately (E11-S6 replaces it).
        let y = NoExportYield;
        assert!(y.wait_while_export_active().is_ok());
    }
}
