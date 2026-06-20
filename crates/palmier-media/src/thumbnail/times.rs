//! Video thumbnail **sampling times** (story E4-S3) — the pure formula split out
//! so it tests without a decoder.
//!
//! Port of `MediaVisualCache.videoThumbnailTimes(duration:)`
//! (`Sources/PalmierPro/Timeline/MediaVisualCache.swift`):
//!
//! ```text
//! interval = duration < 10 ? 1.0 : 2.0
//! times    = 0, interval, 2*interval, …  while time < duration
//! ```
//!
//! Non-finite / `duration <= 0` ⇒ empty. See `docs/reference/media-panel.md`
//! §"Video thumbnail strip".

/// The two sampling target sizes (max width × height) the reference clamps frames
/// to: `CGSize(width: 120, height: 68)`. The extractor scales each frame to fit
/// inside this box, preserving aspect ratio.
pub const THUMB_MAX_WIDTH: u32 = 120;
/// See [`THUMB_MAX_WIDTH`].
pub const THUMB_MAX_HEIGHT: u32 = 68;

/// Source-seconds timestamps to extract a thumbnail at, for a clip of `duration`
/// seconds.
///
/// `interval = 1.0` if `duration < 10`, else `2.0`; samples `0..duration`
/// exclusive of `duration` (`while time < duration`). Returns an empty `Vec` for
/// a non-finite or non-positive duration (parity with the Swift `guard`).
pub fn video_thumbnail_times(duration: f64) -> Vec<f64> {
    if !duration.is_finite() || duration <= 0.0 {
        return Vec::new();
    }
    let interval = if duration < 10.0 { 1.0 } else { 2.0 };
    let mut times = Vec::new();
    let mut time = 0.0_f64;
    while time < duration {
        times.push(time);
        time += interval;
    }
    times
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_5_uses_1s_interval() {
        // < 10 ⇒ 1.0s interval: 0,1,2,3,4 (5 is not < 5).
        assert_eq!(video_thumbnail_times(5.0), vec![0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn duration_10_uses_2s_interval() {
        // == 10 ⇒ NOT < 10 ⇒ 2.0s interval: 0,2,4,6,8.
        assert_eq!(video_thumbnail_times(10.0), vec![0.0, 2.0, 4.0, 6.0, 8.0]);
    }

    #[test]
    fn duration_30_uses_2s_interval() {
        let t = video_thumbnail_times(30.0);
        assert_eq!(t.first(), Some(&0.0));
        assert_eq!(t.last(), Some(&28.0));
        assert_eq!(t.len(), 15); // 0,2,…,28
        // All strictly < duration and spaced by 2.0.
        assert!(t.iter().all(|&x| x < 30.0));
        for w in t.windows(2) {
            assert!((w[1] - w[0] - 2.0).abs() < 1e-9);
        }
    }

    #[test]
    fn just_under_10_still_1s() {
        // 9.5 < 10 ⇒ 1s interval.
        let t = video_thumbnail_times(9.5);
        assert_eq!(t, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    }

    #[test]
    fn non_positive_or_non_finite_is_empty() {
        assert!(video_thumbnail_times(0.0).is_empty());
        assert!(video_thumbnail_times(-1.0).is_empty());
        assert!(video_thumbnail_times(f64::NAN).is_empty());
        assert!(video_thumbnail_times(f64::INFINITY).is_empty());
    }
}
