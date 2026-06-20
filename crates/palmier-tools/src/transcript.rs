//! `span_frames` — map a word's source-second span to project-timeline frames
//! through a clip's trim/speed/position (reference `ToolExecutor+Timeline.swift`
//! `spanFrames`).
//!
//! Used by `get_transcript` (E7-S5) and `inspect_media`-with-`clipId` (E7-S9) to
//! turn a transcribed word's `[start, end]` source-seconds into the clip's
//! project-frame range. The actual transcription words come from Epic 10's store
//! (M3); this mapping is implemented now so Epic 10 only supplies the words.
//!
//! ## The math (1:1 with the reference)
//! Given a clip and a word `[start, end]` in **source seconds**:
//! 1. To source frames: `start * fps`, `end * fps`.
//! 2. Clamp into the clip's visible source window
//!    `[trim_start, trim_start + duration * max(speed, 1e-4)]`.
//! 3. If the clamped span is empty (`e <= s`) → `None` (word not visible).
//! 4. Map a source frame to a timeline frame:
//!    `start_frame + (source_frame − trim_start) / max(speed, 1e-4)`, then round
//!    **ties-away-from-zero** (`f64::round`, matching Swift `Double.rounded()`).
//! 5. `end` is `max(start, mapped_end)` so the range is never inverted.

use palmier_model::Clip;

/// Map a word's `[start_seconds, end_seconds]` to the clip's project-frame range,
/// or `None` if the word falls outside the clip's visible source window. Reference
/// `spanFrames(start:end:clip:fps:)`.
pub fn span_frames(start_seconds: f64, end_seconds: f64, clip: &Clip, fps: i32) -> Option<(i32, i32)> {
    let fps_d = fps as f64;
    let speed = clip.speed.max(0.0001);
    let vis_start = clip.trim_start_frame as f64;
    let vis_end = vis_start + clip.duration_frames as f64 * speed;

    // Clamp the source-frame span into the visible source window.
    let s = (start_seconds * fps_d).max(vis_start);
    let e = (end_seconds * fps_d).min(vis_end);
    if e <= s {
        return None;
    }

    // Source frame → timeline frame; ties-away rounding (Swift `.rounded()`).
    let to_timeline = |source_frame: f64| -> i32 {
        (clip.start_frame as f64 + (source_frame - vis_start) / speed).round() as i32
    };
    let a = to_timeline(s);
    let b = to_timeline(e).max(a);
    Some((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_untrimmed_unscaled_clip() {
        // Clip at start 0, duration 90, speed 1, no trim, fps 30.
        let c = Clip::new("m", 0, 90);
        // word 1.0s..2.0s → source frames 30..60 → timeline 30..60.
        assert_eq!(span_frames(1.0, 2.0, &c, 30), Some((30, 60)));
    }

    #[test]
    fn trim_shifts_source_origin() {
        // trim_start 30 → the visible source window starts 1s in.
        let mut c = Clip::new("m", 0, 60);
        c.trim_start_frame = 30;
        // word at source 1.0s (=frame 30, the trim point) → timeline frame 0.
        assert_eq!(span_frames(1.0, 1.5, &c, 30), Some((0, 15)));
        // word entirely before the trim window → None.
        assert_eq!(span_frames(0.0, 0.5, &c, 30), None);
    }

    #[test]
    fn speed_compresses_timeline_span() {
        // speed 2.0 → 2 source frames consumed per timeline frame.
        let mut c = Clip::new("m", 0, 30);
        c.speed = 2.0;
        // visible source window = [0, 60). word source 1.0s..2.0s = frames 30..60.
        // timeline: 0 + (30-0)/2 = 15 .. 0 + (60-0)/2 = 30 (clamped to vis_end).
        let span = span_frames(1.0, 2.0, &c, 30).unwrap();
        assert_eq!(span.0, 15);
        assert!(span.1 >= span.0);
    }

    #[test]
    fn rounding_is_ties_away() {
        // Construct a fractional source frame that lands on x.5 to prove ties-away.
        let mut c = Clip::new("m", 0, 100);
        c.speed = 1.0;
        // 0.5s * 30fps = 15.0 source frame → timeline 15. Use 0.51666.. for a .5.
        // frame = 0 + (15.5 - 0)/1 = 15.5 → ties-away → 16.
        let span = span_frames(15.5 / 30.0, 20.0 / 30.0, &c, 30).unwrap();
        assert_eq!(span.0, 16, "15.5 rounds away to 16");
    }
}
