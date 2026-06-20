//! Playback clock + active-layer counting — E5-S7 support.
//!
//! The reference drives playback with `AVPlayer` + a periodic time observer at
//! `1/fps`. We can't reuse AVFoundation's clock, and we must keep the transport
//! **testable under a fake clock** (the acceptance criterion: "current_frame advance
//! under a fake clock"). So the transport owns a [`Clock`] it ticks; the production
//! clock reads wall time, the test clock is advanced by hand.

use std::time::{Duration, Instant};

use palmier_model::{ClipType, Timeline};

/// A monotonic time source the transport samples to advance the playhead.
///
/// `now()` returns elapsed time since some fixed origin as a [`Duration`]. The
/// production impl ([`WallClock`]) reads [`Instant`]; tests use [`ManualClock`] and
/// advance it explicitly so playback timing is deterministic.
pub trait Clock {
    /// Monotonic elapsed time since the clock's origin.
    fn now(&self) -> Duration;
}

/// Wall-clock backed by [`Instant`] — the production playback clock.
#[derive(Debug, Clone, Copy)]
pub struct WallClock {
    origin: Instant,
}

impl WallClock {
    /// A wall clock whose origin is the moment of construction.
    pub fn new() -> Self {
        WallClock {
            origin: Instant::now(),
        }
    }
}

impl Default for WallClock {
    fn default() -> Self {
        WallClock::new()
    }
}

impl Clock for WallClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// A hand-advanced clock for deterministic transport tests.
///
/// Starts at zero; [`ManualClock::advance`] moves it forward. Used to verify
/// `current_frame` advances exactly one frame per `1/fps` of elapsed time without
/// real timing.
#[derive(Debug, Clone, Default)]
pub struct ManualClock {
    elapsed: Duration,
}

impl ManualClock {
    /// A clock at time zero.
    pub fn new() -> Self {
        ManualClock::default()
    }

    /// Advance the clock by `delta`.
    pub fn advance(&mut self, delta: Duration) {
        self.elapsed += delta;
    }

    /// Advance by `frames` worth of time at `fps` (a convenience for frame-paced
    /// tests).
    pub fn advance_frames(&mut self, frames: u32, fps: u32) {
        if fps > 0 {
            self.elapsed += Duration::from_secs_f64(frames as f64 / fps as f64);
        }
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        self.elapsed
    }
}

/// Active **video** layer count at `frame`, ported verbatim from the reference
/// `VideoEngine.activeVideoLayerCount(at:editor:)`. This scales the
/// interactive-scrub tolerance (`min(0.75, 0.15 * count)` — FR-19): more visible
/// layers ⇒ a looser scrub tolerance.
///
/// The reference counts non-hidden **video** tracks holding a `.video` **or**
/// `.image` clip active at `frame`. For an asset tab (not the timeline) the
/// reference short-circuits to `1`; the transport passes `is_timeline` so we match
/// that. `.lottie` and `.text` are excluded (the reference checks `.video || .image`
/// only).
pub fn active_video_layer_count(timeline: &Timeline, frame: i32, is_timeline: bool) -> u32 {
    if !is_timeline {
        return 1;
    }
    timeline
        .tracks
        .iter()
        .filter(|track| track.track_type == ClipType::Video && !track.hidden)
        .filter(|track| {
            track.clips.iter().any(|clip| {
                matches!(clip.media_type, ClipType::Video | ClipType::Image)
                    && frame >= clip.start_frame
                    && frame < clip.end_frame()
            })
        })
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use palmier_model::{Clip, Track};

    #[test]
    fn manual_clock_advances() {
        let mut c = ManualClock::new();
        assert_eq!(c.now(), Duration::ZERO);
        c.advance(Duration::from_millis(33));
        assert_eq!(c.now(), Duration::from_millis(33));
        c.advance_frames(2, 30); // 2/30 s ≈ 66.67 ms
        assert!((c.now().as_secs_f64() - (0.033 + 2.0 / 30.0)).abs() < 1e-3);
    }

    fn timeline_with(tracks: Vec<Track>) -> Timeline {
        let mut tl = Timeline::new();
        tl.fps = 30;
        tl.tracks = tracks;
        tl
    }

    #[test]
    fn active_layer_count_counts_visible_video_and_image_tracks() {
        let mut t0 = Track::new(ClipType::Video);
        t0.clips.push(Clip::new("v", 0, 30));
        let mut t1 = Track::new(ClipType::Video);
        let mut img = Clip::new("i", 0, 30);
        img.media_type = ClipType::Image;
        t1.clips.push(img);
        // Hidden track does not count.
        let mut hidden = Track::new(ClipType::Video);
        hidden.hidden = true;
        hidden.clips.push(Clip::new("h", 0, 30));
        // Audio track does not count.
        let mut audio = Track::new(ClipType::Audio);
        let mut ac = Clip::new("a", 0, 30);
        ac.media_type = ClipType::Audio;
        audio.clips.push(ac);

        let tl = timeline_with(vec![t0, t1, hidden, audio]);
        assert_eq!(active_video_layer_count(&tl, 10, true), 2);
        // Outside the active range → 0.
        assert_eq!(active_video_layer_count(&tl, 100, true), 0);
        // Asset tab short-circuits to 1.
        assert_eq!(active_video_layer_count(&tl, 10, false), 1);
    }

    #[test]
    fn lottie_and_text_excluded_from_count() {
        let mut lot = Track::new(ClipType::Video);
        let mut lc = Clip::new("l", 0, 30);
        lc.media_type = ClipType::Lottie;
        lot.clips.push(lc);
        let tl = timeline_with(vec![lot]);
        // Reference counts only .video || .image.
        assert_eq!(active_video_layer_count(&tl, 10, true), 0);
    }
}
