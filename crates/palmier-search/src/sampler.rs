//! `FrameSampler` — a shot-aware frame stream over a video for visual indexing
//! (story E11-S3). Port of `Sources/PalmierPro/Search/Indexing/FrameSampler.swift`.
//!
//! Streams visually **distinct** frames: a luma scene-change starts a new shot
//! (`isNewShot`), and a coverage floor keeps long static shots represented so a
//! single scene neither floods the index with near-duplicates nor disappears.
//!
//! ## Cadence (parity contract — `samplerVersion = 1`)
//! * Default [`Options`]: `candidateInterval = 2.0s`, `coverageFloor = 8.0s`,
//!   `promoteDiff = 12`, `maxSize = 512×512`, `highResEdge = 3000`.
//! * If the video's **larger edge ≥ `highResEdge` (3000 px)**, the interval is
//!   **doubled** (→ 4s) so 4K+ footage isn't over-sampled.
//! * Candidate times: `stride(from: interval/2, to: duration, by: interval)`; if
//!   that is empty (very short clip), use `[duration / 2]`.
//! * Each candidate is decoded via FFmpeg seek+decode+scale
//!   ([`palmier_media::extract_frame_timed`]) with tolerance `max(interval/2, 1.0)s`
//!   (the decoder snaps to the nearest sync frame — exact frame times not required),
//!   `maxSize 512×512`, preferred-track-transform applied. Frames whose
//!   **actualTime ≤ the previous actualTime are skipped** (two nearby candidates
//!   can snap to the same keyframe).
//!
//! ## Shot detection
//! Each kept frame is downsampled to an **8×8 luma grid** ([`LumaGrid`], BT.601
//! weights `0.299 / 0.587 / 0.114` on premultiplied-RGBA pixels). `meanDiff` is
//! the mean absolute per-cell delta vs the previous **kept** grid;
//! `isNewShot = meanDiff > promoteDiff (12)`, and the **first** frame is always a
//! new shot.
//!
//! ## Keep rule
//! A frame is emitted iff `isNewShot || (t - lastKeptTime) >= coverageFloor (8s)`.
//!
//! ## Structure
//! Like Epic 4's thumbnail pipeline (`times.rs` pure / `video.rs` decode), the
//! decision logic is split from the decoder: [`SamplerState`] is the pure
//! frame-by-frame core (testable on synthetic frames — the 3 cadence/shot unit
//! tests drive it directly), [`candidate_times`] is the pure cadence formula, and
//! [`FrameSampler::frames`] wires them to [`palmier_media::extract_frame_timed`].

use image::RgbImage;

/// Sampler format version — written into the `.embed` header so a cadence change
/// forces a clean re-index. **1** in the reference (and [`crate::spec::SAMPLER_VERSION`]).
pub const SAMPLER_VERSION: i64 = 1;

/// 8×8 grid ⇒ 64 luma cells per frame.
const GRID: u32 = 8;

/// Sampling knobs (reference `FrameSampler.Options`). Defaults are the parity
/// contract; downstream code constructs `Options::default()`.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Seconds between candidate sample times before the high-res doubling.
    pub candidate_interval: f64,
    /// Max gap (s) a kept frame may have from the previous kept frame — keeps a
    /// long static shot represented even with no scene change.
    pub coverage_floor: f64,
    /// `meanDiff` threshold above which a frame starts a new shot.
    pub promote_diff: f32,
    /// Max decoded frame size (width, height) — frames scale to fit this box.
    pub max_size: (u32, u32),
    /// If the video's larger edge ≥ this many px, the interval is doubled.
    pub high_res_edge: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            candidate_interval: 2.0,
            coverage_floor: 8.0,
            promote_diff: 12.0,
            max_size: (512, 512),
            high_res_edge: 3000,
        }
    }
}

/// One emitted frame: its source time, the decoded RGB image, and whether it
/// starts a new shot (reference `FrameSampler.Frame`).
#[derive(Debug, Clone)]
pub struct Frame {
    /// Actual presentation time of the frame, in source seconds.
    pub time: f64,
    /// The decoded, downscaled frame (≤ `Options::max_size`).
    pub image: RgbImage,
    /// True iff this frame starts a new shot (a luma scene change, or the very
    /// first kept frame).
    pub is_new_shot: bool,
}

/// Mean luma per cell of an 8×8 downsample — a cheap visual-change fingerprint
/// (reference `LumaGrid`). 64 `f32` cells, BT.601-weighted on premultiplied RGBA.
#[derive(Debug, Clone, PartialEq)]
pub struct LumaGrid {
    cells: [f32; (GRID * GRID) as usize],
}

impl LumaGrid {
    /// Downsample `image` to an 8×8 grid and compute per-cell BT.601 luma.
    ///
    /// The reference draws the `CGImage` into an 8×8 **premultiplied-last RGBA**
    /// context at high interpolation, then weights each cell
    /// `R*0.299 + G*0.587 + B*0.114`. Decoded video frames are opaque (alpha =
    /// 255), so premultiplied == straight RGB; we resize the RGB frame to 8×8 with
    /// a high-quality (Lanczos3) filter — the analogue of the reference's
    /// high-interpolation draw — and apply the same BT.601 weights.
    pub fn compute(image: &RgbImage) -> LumaGrid {
        // Resize to 8×8 (high quality, matching the reference's `.high`
        // interpolation). `image::imageops::resize` needs a non-zero source; a
        // degenerate frame yields an all-zero grid (treated as "no change").
        let small = if image.width() == 0 || image.height() == 0 {
            RgbImage::new(GRID, GRID)
        } else {
            image::imageops::resize(image, GRID, GRID, image::imageops::FilterType::Lanczos3)
        };
        let mut cells = [0.0_f32; (GRID * GRID) as usize];
        for (i, px) in small.pixels().enumerate() {
            let [r, g, b] = px.0;
            cells[i] = r as f32 * 0.299 + g as f32 * 0.587 + b as f32 * 0.114;
        }
        LumaGrid { cells }
    }

    /// Mean absolute per-cell delta between two grids (reference
    /// `LumaGrid.meanDiff`).
    pub fn mean_diff(&self, other: &LumaGrid) -> f32 {
        let mut diff = 0.0_f32;
        for i in 0..self.cells.len() {
            diff += (self.cells[i] - other.cells[i]).abs();
        }
        diff / self.cells.len() as f32
    }
}

/// Pure frame-by-frame decision core (reference `FrameSampler.sample`'s loop
/// body). Decoupled from the decoder so the cadence/shot rules test on synthetic
/// frames. Feed decoded `(actual_time, image)` pairs **in candidate order** via
/// [`SamplerState::offer`]; it returns the [`Frame`] to emit, or `None` to skip.
#[derive(Debug)]
pub struct SamplerState {
    promote_diff: f32,
    coverage_floor: f64,
    last_grid: Option<LumaGrid>,
    last_kept_time: f64,
    last_time: f64,
}

impl SamplerState {
    /// New state for the given [`Options`].
    pub fn new(options: &Options) -> Self {
        SamplerState {
            promote_diff: options.promote_diff,
            coverage_floor: options.coverage_floor,
            last_grid: None,
            last_kept_time: f64::NEG_INFINITY,
            last_time: f64::NEG_INFINITY,
        }
    }

    /// Offer one decoded frame at `actual_time`. Applies, in reference order:
    /// 1. **Monotonic skip** — drop frames whose `actual_time` ≤ the previous
    ///    frame's (two candidates snapped to the same keyframe).
    /// 2. **Shot detection** — `isNewShot = meanDiff(grid, lastKept) > promoteDiff`;
    ///    the first frame is always a new shot. `last_grid` advances on **every**
    ///    non-skipped frame (the reference updates `lastGrid` before the keep test).
    /// 3. **Keep rule** — emit iff `isNewShot || (t - lastKeptTime) >= coverageFloor`.
    ///
    /// Returns the [`Frame`] to emit, or `None` if the frame is skipped/collapsed.
    pub fn offer(&mut self, actual_time: f64, image: RgbImage) -> Option<Frame> {
        let t = actual_time;
        if !(t > self.last_time) {
            return None;
        }
        self.last_time = t;

        let grid = LumaGrid::compute(&image);
        let is_new_shot = match &self.last_grid {
            Some(last) => grid.mean_diff(last) > self.promote_diff,
            None => true,
        };
        self.last_grid = Some(grid);

        if !(is_new_shot || t - self.last_kept_time >= self.coverage_floor) {
            return None;
        }
        self.last_kept_time = t;
        Some(Frame {
            time: t,
            image,
            is_new_shot,
        })
    }
}

/// Candidate sample times for a clip of `duration` seconds at the given
/// `interval` (reference: `stride(from: interval/2, to: duration, by: interval)`,
/// falling back to `[duration/2]` when that stride is empty). Returns empty for a
/// non-positive / non-finite duration or interval.
pub fn candidate_times(duration: f64, interval: f64) -> Vec<f64> {
    if !duration.is_finite() || duration <= 0.0 || !interval.is_finite() || interval <= 0.0 {
        return Vec::new();
    }
    let mut times = Vec::new();
    let mut t = interval / 2.0;
    while t < duration {
        times.push(t);
        t += interval;
    }
    if times.is_empty() {
        times.push(duration / 2.0);
    }
    times
}

/// Resolve the sampling interval for a video whose larger natural edge is
/// `larger_edge` px: the base `candidate_interval`, doubled if
/// `larger_edge >= high_res_edge` (reference's 4K-doubling guard).
pub fn effective_interval(options: &Options, larger_edge: u32) -> f64 {
    if larger_edge >= options.high_res_edge {
        options.candidate_interval * 2.0
    } else {
        options.candidate_interval
    }
}

/// Errors the sampler can surface (decode failures bubble up from palmier-media).
#[derive(Debug)]
pub enum SampleError {
    /// FFmpeg open/decode/scale failure, or no decodable video stream.
    Decode(String),
}

impl std::fmt::Display for SampleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SampleError::Decode(m) => write!(f, "decode: {m}"),
        }
    }
}

impl std::error::Error for SampleError {}

/// A shot-aware frame stream over a video (reference `FrameSampler`). Stateless;
/// `frames` does all the work per call.
pub struct FrameSampler;

impl FrameSampler {
    /// Sampler format version (re-exported for convenience).
    pub const VERSION: i64 = SAMPLER_VERSION;

    /// Sample shot-aware frames from the video at `path`.
    ///
    /// `duration` is the clip length in seconds (read upstream from metadata);
    /// `larger_edge` is the video's larger natural-size edge in px (drives the
    /// high-res interval doubling — pass `0` if unknown to keep the base interval).
    ///
    /// Each candidate time is decoded via [`palmier_media::extract_frame_timed`]
    /// (the shared Epic 4 FFmpeg seek+decode+scale path) scaled into
    /// `options.max_size`. A candidate that fails to decode is **skipped** (parity
    /// with the reference, which ignores non-`.success` generator results) rather
    /// than aborting the stream. Returns the kept frames in time order.
    ///
    /// This is synchronous and CPU/IO-bound; callers run it on a blocking pool.
    pub fn frames(
        path: &std::path::Path,
        duration: f64,
        larger_edge: u32,
        options: &Options,
    ) -> Result<Vec<Frame>, SampleError> {
        let interval = effective_interval(options, larger_edge);
        let times = candidate_times(duration, interval);
        let (max_w, max_h) = options.max_size;

        let mut state = SamplerState::new(options);
        let mut out = Vec::new();
        for t in times {
            // Tolerance is implicit in ffmpeg's keyframe seek (reference uses
            // `max(interval/2, 1.0)s` before/after to snap to the nearest sync
            // frame); extract_frame_timed seeks to ≤ t and decodes the nearest
            // frame, then reports its actual PTS.
            match palmier_media::extract_frame_timed(path, t, max_w, max_h) {
                Ok(frame) => {
                    if let Some(emit) = state.offer(frame.time, frame.image) {
                        out.push(emit);
                    }
                }
                // A bad GOP / seek past EOF skips this candidate, like the
                // reference's non-`.success` results.
                Err(palmier_media::thumbnail::VideoThumbnailError::NoVideoStream) => {
                    return Err(SampleError::Decode("no video stream".into()));
                }
                Err(_) => continue,
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    /// Solid-color frame of the given luma-ish fill.
    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(w, h, Rgb(rgb))
    }

    // ---- pure cadence formula -------------------------------------------------

    #[test]
    fn candidate_times_strides_from_half_interval() {
        // duration 10, interval 2 ⇒ 1,3,5,7,9.
        assert_eq!(candidate_times(10.0, 2.0), vec![1.0, 3.0, 5.0, 7.0, 9.0]);
    }

    #[test]
    fn candidate_times_falls_back_to_midpoint_when_empty() {
        // Very short clip: interval/2 already ≥ duration ⇒ [duration/2].
        // duration 1.5, interval 4 ⇒ start 2.0 ≥ 1.5 ⇒ [0.75].
        assert_eq!(candidate_times(1.5, 4.0), vec![0.75]);
    }

    #[test]
    fn candidate_times_empty_for_bad_inputs() {
        assert!(candidate_times(0.0, 2.0).is_empty());
        assert!(candidate_times(-1.0, 2.0).is_empty());
        assert!(candidate_times(f64::NAN, 2.0).is_empty());
        assert!(candidate_times(10.0, 0.0).is_empty());
    }

    // ---- AC: high-res edge doubles the interval -------------------------------

    #[test]
    fn high_res_edge_doubles_interval() {
        let opt = Options::default();
        // Below the 3000px edge ⇒ base 2.0s.
        assert_eq!(effective_interval(&opt, 1920), 2.0);
        assert_eq!(effective_interval(&opt, 2999), 2.0);
        // At/above the edge ⇒ doubled to 4.0s.
        assert_eq!(effective_interval(&opt, 3000), 4.0);
        assert_eq!(effective_interval(&opt, 3840), 4.0);
        // And the doubled interval changes the candidate cadence (4s vs 2s).
        let base = candidate_times(20.0, effective_interval(&opt, 1920));
        let hires = candidate_times(20.0, effective_interval(&opt, 3840));
        assert_eq!(base, vec![1.0, 3.0, 5.0, 7.0, 9.0, 11.0, 13.0, 15.0, 17.0, 19.0]);
        assert_eq!(hires, vec![2.0, 6.0, 10.0, 14.0, 18.0]);
        assert!(hires.len() < base.len());
    }

    // ---- AC: scene cut is flagged isNewShot -----------------------------------

    #[test]
    fn scene_cut_is_flagged_new_shot() {
        // Synthetic "clip": three near-identical dark frames, then a hard cut to
        // bright frames. meanDiff across the cut must exceed promoteDiff (12).
        let opt = Options::default();
        let mut state = SamplerState::new(&opt);
        let dark = || solid(64, 64, [10, 10, 10]);
        let bright = || solid(64, 64, [240, 240, 240]);

        // t=0: first frame ⇒ always a new shot, always kept.
        let f0 = state.offer(0.0, dark()).expect("first frame kept");
        assert!(f0.is_new_shot, "first frame is always a new shot");

        // t=2: identical dark frame ⇒ NOT a new shot (meanDiff ~0 < 12). Within
        // the coverage floor (8s) so it is collapsed (not emitted).
        assert!(
            state.offer(2.0, dark()).is_none(),
            "near-duplicate within coverage floor is collapsed"
        );

        // t=4: the scene cut to bright ⇒ meanDiff huge ⇒ new shot ⇒ emitted.
        let cut = state.offer(4.0, bright()).expect("cut frame emitted");
        assert!(cut.is_new_shot, "hard cut must be flagged isNewShot");
        assert_eq!(cut.time, 4.0);

        // t=6: identical bright frame ⇒ not a new shot, within floor ⇒ collapsed.
        assert!(state.offer(6.0, bright()).is_none());
    }

    #[test]
    fn luma_grid_detects_change_and_ignores_identical() {
        let a = LumaGrid::compute(&solid(32, 32, [0, 0, 0]));
        let b = LumaGrid::compute(&solid(32, 32, [0, 0, 0]));
        let c = LumaGrid::compute(&solid(32, 32, [255, 255, 255]));
        assert!(a.mean_diff(&b) <= f32::EPSILON, "identical frames ⇒ ~0 diff");
        // Black→white is the maximum luma swing (255), far above promoteDiff 12.
        assert!(a.mean_diff(&c) > 12.0, "black→white exceeds promoteDiff");
    }

    // ---- AC: long static clip keeps via the coverage floor --------------------

    #[test]
    fn long_static_clip_keeps_via_coverage_floor() {
        // A long static shot: no scene change ever, but the coverage floor (8s)
        // must still keep frames so the shot is represented in the index.
        let opt = Options::default(); // coverage_floor = 8.0
        let mut state = SamplerState::new(&opt);
        let frame = || solid(48, 48, [120, 120, 120]);

        // Candidate cadence for a 24s clip at 2s interval: 1,3,5,...,23.
        let times = candidate_times(24.0, opt.candidate_interval);
        let mut kept = Vec::new();
        for t in times {
            if let Some(f) = state.offer(t, frame()) {
                kept.push((f.time, f.is_new_shot));
            }
        }

        // First frame (t=1) is the new-shot start; then every frame is identical
        // (no new shot), so only the coverage floor keeps frames: emit when
        // t - lastKept >= 8. From t=1: next kept at 9 (9-1=8), then 17 (17-9=8).
        assert_eq!(kept[0], (1.0, true), "first kept frame starts a shot");
        let kept_times: Vec<f64> = kept.iter().map(|(t, _)| *t).collect();
        assert_eq!(kept_times, vec![1.0, 9.0, 17.0]);
        // None after the first are new shots — they survive purely on the floor.
        assert!(kept[1..].iter().all(|(_, new)| !new));
    }

    // ---- monotonic-skip rule --------------------------------------------------

    #[test]
    fn non_monotonic_actual_times_are_skipped() {
        let opt = Options::default();
        let mut state = SamplerState::new(&opt);
        let a = solid(16, 16, [10, 10, 10]);
        let b = solid(16, 16, [240, 240, 240]);
        // First kept at t=4.
        assert!(state.offer(4.0, a).is_some());
        // A later candidate that snapped back to the SAME (or earlier) keyframe
        // time must be skipped even though it's a big visual change.
        assert!(
            state.offer(4.0, b.clone()).is_none(),
            "actualTime == previous ⇒ skipped"
        );
        assert!(
            state.offer(3.5, b).is_none(),
            "actualTime < previous ⇒ skipped"
        );
    }

    // ---- real-ffmpeg integration (needs a committed fixture) ------------------

    #[test]
    #[ignore = "needs a real video fixture via PALMIER_TEST_VIDEO"]
    fn frames_from_real_video() {
        let Ok(path) = std::env::var("PALMIER_TEST_VIDEO") else {
            return;
        };
        let opt = Options::default();
        let frames =
            FrameSampler::frames(std::path::Path::new(&path), 12.0, 1920, &opt).expect("sample");
        assert!(!frames.is_empty(), "real video yields at least one frame");
        assert!(frames[0].is_new_shot, "first frame is a new shot");
        // Monotonic non-decreasing emitted times.
        for w in frames.windows(2) {
            assert!(w[1].time > w[0].time);
        }
        // Frames fit the 512 box.
        assert!(frames.iter().all(|f| f.image.width() <= 512 && f.image.height() <= 512));
    }
}
