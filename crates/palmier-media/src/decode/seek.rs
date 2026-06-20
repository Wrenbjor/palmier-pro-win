//! Seek-mode selection + the reference's interactive-scrub tolerance/throttle
//! math, ported verbatim from `VideoEngine.swift` (story E5-S2 / preview-engine
//! risk #6).
//!
//! The transport loop (E5-S7) drives playback; this module owns the *decode
//! side* of the seek contract the transport speaks to: given a requested frame
//! and a [`SeekMode`], what tolerance window applies and is a dispatch allowed
//! right now under the 1/30 s throttle. Keeping the constants here (rather than
//! re-deriving them in the engine) means the one-decode-owner can serve a
//! tolerance-correct "nearest available" frame for `InteractiveScrub` and queue
//! the precise decode itself.
//!
//! ## Reference constants (`VideoEngine.swift`)
//! * `interactiveSeekInterval: TimeInterval = 1.0 / 30.0` — one dispatch per
//!   1/30 s, coalescing pending seeks.
//! * tolerance = `min(0.75, 0.15 * activeLayerCount)` s at timescale 600;
//!   `.exact` uses tolerance 0 and cancels pending seeks.

use std::time::Duration;

/// How a frame request should be satisfied.
///
/// Mirrors `PreviewSeekMode` in the reference (`VideoEngine.swift`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekMode {
    /// Precise seek/reset: tolerance 0, land on the **exact** frame, cancel any
    /// pending interactive seek first. Used for playback start and frame
    /// stepping.
    Exact,
    /// Interactive scrub: serve the nearest available frame immediately and
    /// queue a precise decode in the background, throttled to one dispatch per
    /// [`SCRUB_THROTTLE`]. Tolerance scales with the active layer count, capped
    /// at [`SCRUB_TOLERANCE_CAP_SECS`].
    InteractiveScrub,
}

impl SeekMode {
    /// Whether this mode tolerates serving a nearby cached frame while a precise
    /// decode is queued (`InteractiveScrub`), versus requiring the exact frame
    /// (`Exact`).
    pub fn allows_nearest(self) -> bool {
        matches!(self, SeekMode::InteractiveScrub)
    }
}

/// Throttle interval for `InteractiveScrub` dispatches: one per 1/30 s
/// (`VideoEngine.interactiveSeekInterval`). Exposed as a [`Duration`] for the
/// transport's coalescing pending-seek timer.
pub const SCRUB_THROTTLE: Duration = Duration::from_nanos(1_000_000_000 / 30);

/// Upper bound on the interactive-scrub tolerance, in seconds
/// (`min(0.75, ...)`). Above ~5 active layers the per-layer term saturates here.
pub const SCRUB_TOLERANCE_CAP_SECS: f64 = 0.75;

/// Per-active-layer tolerance term, in seconds (`0.15 * activeLayerCount`).
pub const SCRUB_TOLERANCE_PER_LAYER_SECS: f64 = 0.15;

/// The reference timescale the tolerance `CMTime` is built at
/// (`preferredTimescale: 600`). We round the tolerance to this grid so the
/// frame-tolerance math matches AVFoundation's exactly.
pub const SCRUB_TOLERANCE_TIMESCALE: i64 = 600;

/// Interactive-scrub tolerance in seconds for `activeLayerCount` active video
/// layers, **verbatim** to `VideoEngine.interactiveTolerance`:
/// `min(0.75, 0.15 * max(1, activeLayerCount))`.
///
/// The reference clamps the layer count to ≥ 1 (`max(1, activeLayerCount)`), so
/// a zero/empty timeline still gets the single-layer tolerance.
pub fn interactive_tolerance_secs(active_layer_count: u32) -> f64 {
    let layers = active_layer_count.max(1) as f64;
    f64::min(
        SCRUB_TOLERANCE_CAP_SECS,
        SCRUB_TOLERANCE_PER_LAYER_SECS * layers,
    )
}

/// Interactive-scrub tolerance expressed in **whole source frames** at `fps`,
/// rounding to the nearest frame. This is the window within which a cached
/// nearest frame is acceptable for `InteractiveScrub`: a cached frame within
/// `±tolerance_frames` of the target may be served while the precise decode is
/// queued.
///
/// Uses `f64::round` (ties away from zero) to match the carry-forward rounding
/// rule used across the source↔timeline mapping.
pub fn interactive_tolerance_frames(active_layer_count: u32, fps: f64) -> u64 {
    if fps <= 0.0 {
        return 0;
    }
    let secs = interactive_tolerance_secs(active_layer_count);
    // Quantize to the 600 timescale first (parity with the CMTime the reference
    // builds), then convert to frames.
    let ts_units = (secs * SCRUB_TOLERANCE_TIMESCALE as f64).round();
    let quantized_secs = ts_units / SCRUB_TOLERANCE_TIMESCALE as f64;
    (quantized_secs * fps).round() as u64
}

/// Coalescing throttle for interactive-scrub dispatch, mirroring
/// `VideoEngine.enqueueInteractiveSeek`/`flushPendingInteractiveSeek`.
///
/// The transport calls [`ScrubThrottle::can_dispatch`] with the current
/// monotonic time; if the throttle window has elapsed since the last dispatch,
/// it returns the delay (0 ⇒ dispatch now) so the caller can either flush
/// immediately or schedule a single coalesced flush after the returned delay.
#[derive(Debug, Clone, Copy)]
pub struct ScrubThrottle {
    last_dispatch: Option<std::time::Instant>,
    interval: Duration,
}

impl Default for ScrubThrottle {
    fn default() -> Self {
        ScrubThrottle::new(SCRUB_THROTTLE)
    }
}

impl ScrubThrottle {
    /// New throttle with the given minimum interval between dispatches.
    pub fn new(interval: Duration) -> Self {
        ScrubThrottle {
            last_dispatch: None,
            interval,
        }
    }

    /// Delay before the next dispatch is permitted given `now`. `Duration::ZERO`
    /// means dispatch immediately (the reference's `delay <= 0` fast path);
    /// otherwise the caller should schedule a single coalesced flush after the
    /// returned delay (mirroring the one-shot `interactiveThrottleTask`).
    pub fn delay_until_next(&self, now: std::time::Instant) -> Duration {
        match self.last_dispatch {
            None => Duration::ZERO,
            Some(last) => {
                let elapsed = now.saturating_duration_since(last);
                self.interval.saturating_sub(elapsed)
            }
        }
    }

    /// Whether a dispatch is allowed right now (delay == 0).
    pub fn can_dispatch(&self, now: std::time::Instant) -> bool {
        self.delay_until_next(now).is_zero()
    }

    /// Record that a dispatch happened at `now` (the reference sets
    /// `lastInteractiveDispatchTime` on flush).
    pub fn record_dispatch(&mut self, now: std::time::Instant) {
        self.last_dispatch = Some(now);
    }

    /// Reset the throttle (the reference's `invalidateSeekState` zeroes the last
    /// dispatch time so the next scrub dispatches immediately).
    pub fn reset(&mut self) {
        self.last_dispatch = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn tolerance_secs_matches_reference_at_layer_counts() {
        // min(0.75, 0.15 * max(1, n)). Reference clamps n to >= 1.
        assert!((interactive_tolerance_secs(0) - 0.15).abs() < 1e-12); // max(1,0)=1
        assert!((interactive_tolerance_secs(1) - 0.15).abs() < 1e-12);
        assert!((interactive_tolerance_secs(3) - 0.45).abs() < 1e-12);
        // 0.15 * 6 = 0.90, capped to 0.75.
        assert!((interactive_tolerance_secs(6) - 0.75).abs() < 1e-12);
        // The cap kicks in at >= 5 layers (0.15*5 = 0.75).
        assert!((interactive_tolerance_secs(5) - 0.75).abs() < 1e-12);
    }

    #[test]
    fn tolerance_frames_at_required_layer_counts_30fps() {
        // FR-19 gate asserts tolerance at activeLayerCount ∈ {1, 3, 6}.
        // 1 layer: 0.15 s * 30 = 4.5 → 5 (ties away from zero via round).
        assert_eq!(interactive_tolerance_frames(1, 30.0), 5);
        // 3 layers: 0.45 s * 30 = 13.5 → 14.
        assert_eq!(interactive_tolerance_frames(3, 30.0), 14);
        // 6 layers: capped 0.75 s * 30 = 22.5 → 23.
        assert_eq!(interactive_tolerance_frames(6, 30.0), 23);
    }

    #[test]
    fn tolerance_frames_zero_fps_is_safe() {
        assert_eq!(interactive_tolerance_frames(3, 0.0), 0);
    }

    #[test]
    fn exact_mode_requires_exact_frame() {
        assert!(!SeekMode::Exact.allows_nearest());
        assert!(SeekMode::InteractiveScrub.allows_nearest());
    }

    #[test]
    fn throttle_allows_first_dispatch_then_gates_within_window() {
        let t0 = Instant::now();
        let mut throttle = ScrubThrottle::new(Duration::from_millis(33));
        // First dispatch (no prior) is immediate.
        assert!(throttle.can_dispatch(t0));
        throttle.record_dispatch(t0);
        // Within the window: gated, with a positive delay.
        let t_mid = t0 + Duration::from_millis(10);
        assert!(!throttle.can_dispatch(t_mid));
        let delay = throttle.delay_until_next(t_mid);
        assert_eq!(delay, Duration::from_millis(23));
        // After the window: dispatch allowed again.
        let t_after = t0 + Duration::from_millis(40);
        assert!(throttle.can_dispatch(t_after));
    }

    #[test]
    fn throttle_reset_clears_gate() {
        let t0 = Instant::now();
        let mut throttle = ScrubThrottle::default();
        throttle.record_dispatch(t0);
        assert!(!throttle.can_dispatch(t0 + Duration::from_millis(1)));
        throttle.reset();
        assert!(throttle.can_dispatch(t0 + Duration::from_millis(1)));
    }

    #[test]
    fn scrub_throttle_constant_is_one_thirtieth_second() {
        assert_eq!(SCRUB_THROTTLE, Duration::from_nanos(33_333_333));
    }
}
