//! Speed-retime sample-count math + the resample plan to the 48 kHz project rate.
//!
//! Two distinct rate changes apply to a clip's source audio before it is summed:
//!
//! 1. **Resample** the source sample rate → the project rate (48 kHz). Pure rate
//!    conversion via `rubato` (FOUNDATION §6.5 "resample to project rate (48 kHz)").
//! 2. **Speed time-stretch** for `speed != 1.0` — *pitch-preserving* (signalsmith-style
//!    stretch). The reference relies on AVFoundation `scaleTimeRange`, which does NOT
//!    preserve pitch; FOUNDATION §6.5 explicitly improves on this. This module computes
//!    the sample-count plan; the stretch DSP itself is wired by the mixer/transport.
//!
//! ## Frame-count parity (carry-forward, reconciliation §"Frame rounding")
//!
//! Source↔timeline frame conversion is `round(durationFrames * speed)` **ties-AWAY-
//! from-zero**, mirroring the reference `insertClip`
//! (`sourceFrames = max(1, Int(Double(durationFrames) * speed))` where Swift `.rounded()`
//! is half-away). Rust `f64::round` is already ties-away — we MUST NOT use
//! `round_ties_even`. Getting this wrong drifts clip↔source alignment (SM-C1).

/// Project (output) audio sample rate in Hz. FOUNDATION §6.5.
pub const PROJECT_SAMPLE_RATE_HZ: u32 = 48_000;

/// Source frames the visible portion of a clip consumes, given timeline duration and
/// speed. Verbatim port of the reference `insertClip` rule:
/// `speed == 1 ? durationFrames : max(1, round(durationFrames * speed))`.
///
/// Uses `f64::round` (ties-away-from-zero) per the carry-forward note — never
/// `round_ties_even`.
#[inline]
pub fn source_frames(duration_frames: i32, speed: f64) -> i32 {
    if speed == 1.0 {
        duration_frames
    } else {
        ((duration_frames as f64 * speed).round() as i32).max(1)
    }
}

/// Number of output (48 kHz) audio samples a clip occupies on the timeline, given its
/// visible frame duration and the timeline fps. This is the *played* length — speed
/// has already been applied by the time-stretch, so it is driven purely by the
/// timeline duration, not the source length.
///
/// `round(duration_frames / fps * 48000)`, ties-away.
#[inline]
pub fn timeline_output_samples(duration_frames: i32, fps: u32) -> u64 {
    if duration_frames <= 0 || fps == 0 {
        return 0;
    }
    let seconds = duration_frames as f64 / fps as f64;
    (seconds * PROJECT_SAMPLE_RATE_HZ as f64).round() as u64
}

/// How many source samples (at the source rate) the clip's visible portion reads,
/// before resample/stretch. `round(source_frames / fps * source_rate)`, ties-away.
#[inline]
pub fn source_samples(duration_frames: i32, speed: f64, fps: u32, source_rate_hz: u32) -> u64 {
    let src_frames = source_frames(duration_frames, speed);
    if src_frames <= 0 || fps == 0 {
        return 0;
    }
    let seconds = src_frames as f64 / fps as f64;
    (seconds * source_rate_hz as f64).round() as u64
}

/// The resample ratio `output_rate / input_rate` rubato is configured with for pure
/// sample-rate conversion (speed handled separately by the pitch-preserving stretch).
#[inline]
pub fn resample_ratio(source_rate_hz: u32) -> f64 {
    PROJECT_SAMPLE_RATE_HZ as f64 / source_rate_hz as f64
}

/// A resolved plan for one clip's audio: how many source samples to read, the
/// resample ratio, and the final output-sample count to place on the mix bus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResamplePlan {
    /// Source samples to decode (at `source_rate_hz`).
    pub source_samples: u64,
    /// Source → 48 kHz resample ratio.
    pub resample_ratio: f64,
    /// Pitch-preserving stretch factor applied for speed (`1.0 / speed`); a clip played
    /// at 2× speed reads twice the source but outputs the timeline length, so the
    /// stretch compresses by `1/speed`.
    pub stretch_factor: f64,
    /// Final 48 kHz output sample count the clip contributes to the bus.
    pub output_samples: u64,
}

/// Resolve the [`ResamplePlan`] for a clip. The output length is governed by the
/// timeline duration (what the user sees), while the source read length is governed by
/// `source_frames` (duration × speed). The stretch bridges the two while preserving
/// pitch.
pub fn plan(duration_frames: i32, speed: f64, fps: u32, source_rate_hz: u32) -> ResamplePlan {
    ResamplePlan {
        source_samples: source_samples(duration_frames, speed, fps, source_rate_hz),
        resample_ratio: resample_ratio(source_rate_hz),
        stretch_factor: if speed != 0.0 { 1.0 / speed } else { 1.0 },
        output_samples: timeline_output_samples(duration_frames, fps),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_one_is_identity() {
        assert_eq!(source_frames(100, 1.0), 100);
    }

    #[test]
    fn source_frames_rounds_ties_away_not_even() {
        // 5 * 0.5 = 2.5 → ties-away rounds to 3 (round_ties_even would give 2).
        assert_eq!(source_frames(5, 0.5), 3);
        // 3 * 0.5 = 1.5 → 2 (round_ties_even would also give 2 here, so use 2.5 case above
        // as the discriminating one). 1 * 0.5 = 0.5 → max(1, round(0.5)=1) wait 0.5→1.
        assert_eq!(source_frames(1, 0.5), 1);
        // 2x speed reads twice the source.
        assert_eq!(source_frames(100, 2.0), 200);
    }

    #[test]
    fn source_frames_floor_is_one() {
        // Even an extreme slow-down keeps ≥1 source frame.
        assert_eq!(source_frames(1, 0.01), 1);
        assert_eq!(source_frames(0, 0.5), 1);
    }

    #[test]
    fn timeline_output_samples_at_30fps() {
        // 30 frames @ 30 fps = 1.0 s = 48000 samples.
        assert_eq!(timeline_output_samples(30, 30), 48_000);
        // 15 frames @ 30 fps = 0.5 s = 24000.
        assert_eq!(timeline_output_samples(15, 30), 24_000);
        assert_eq!(timeline_output_samples(0, 30), 0);
    }

    #[test]
    fn resample_ratio_upsamples_44k() {
        let r = resample_ratio(44_100);
        assert!((r - (48_000.0 / 44_100.0)).abs() < 1e-12);
        // 48k source → identity.
        assert_eq!(resample_ratio(48_000), 1.0);
    }

    #[test]
    fn plan_double_speed_reads_twice_outputs_timeline_length() {
        // 60 frames @ 30 fps, 2x speed, 48k source.
        let p = plan(60, 2.0, 30, 48_000);
        // Output = 2 s timeline = 96000 samples.
        assert_eq!(p.output_samples, 96_000);
        // Source read = 120 source frames = 4 s of source = 192000 samples.
        assert_eq!(p.source_samples, 192_000);
        // Stretch compresses 2x source down to 1x output (pitch preserved).
        assert!((p.stretch_factor - 0.5).abs() < 1e-12);
        assert_eq!(p.resample_ratio, 1.0);
    }
}
