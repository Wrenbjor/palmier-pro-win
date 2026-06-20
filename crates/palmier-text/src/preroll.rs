//! The 30-frame text preroll (FOUNDATION §6.6, reference
//! `TextLayerController.reconcile`).
//!
//! The reference materializes a text clip's `CATextLayer` slightly **before** its
//! start so playback never hitches on typesetting, but keeps it transparent until
//! the clip is actually active:
//!
//! ```swift
//! guard currentFrame >= clip.startFrame - prerollFrames,
//!       currentFrame < clip.endFrame else { continue }   // materialize window
//! let visible = currentFrame >= clip.startFrame          // opacity gate
//! let target = visible ? clip.opacityAt(currentFrame) : 0
//! ```
//!
//! We mirror both halves: [`preroll_window`] decides whether a clip is *laid out*
//! at all this frame (the cache/atlas warm-up), and the **visible** flag (frame
//! ≥ start) decides whether its sampled opacity applies or it draws at 0.

/// Frames to materialize a text clip ahead of its start (reference
/// `TextLayerController.prerollFrames = 30`).
pub const PREROLL_FRAMES: i32 = 30;

/// Whether a text clip with `[start_frame, end_frame)` should be **materialized**
/// (laid out / atlas-warmed) at `current_frame`: the preroll window
/// `start_frame - 30 <= current_frame < end_frame`. Mirrors the reference `guard`.
///
/// `end_frame` is exclusive (the reference uses `< clip.endFrame`). A clip with
/// `end_frame <= start_frame` is degenerate and never materializes.
pub fn preroll_window(current_frame: i32, start_frame: i32, end_frame: i32) -> bool {
    if end_frame <= start_frame {
        return false;
    }
    current_frame >= start_frame - PREROLL_FRAMES && current_frame < end_frame
}

/// Whether a materialized clip is **visible** (its sampled opacity applies) at
/// `current_frame` — the reference `visible = currentFrame >= clip.startFrame`.
/// During the 30-frame preroll lead-in this is `false` (laid out but drawn at 0).
pub fn is_visible(current_frame: i32, start_frame: i32) -> bool {
    current_frame >= start_frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_opens_30_frames_early() {
        // Clip [100, 200). Before frame 70 → not materialized; at 70 → materialized
        // (preroll lead-in); through 199 → materialized; at 200 → gone.
        assert!(!preroll_window(69, 100, 200));
        assert!(preroll_window(70, 100, 200), "preroll opens at start-30");
        assert!(preroll_window(100, 100, 200));
        assert!(preroll_window(199, 100, 200));
        assert!(!preroll_window(200, 100, 200), "end is exclusive");
    }

    #[test]
    fn visible_only_from_start() {
        // In the preroll lead-in [70, 100) the clip is laid out but NOT visible.
        assert!(!is_visible(70, 100));
        assert!(!is_visible(99, 100));
        assert!(is_visible(100, 100));
        assert!(is_visible(150, 100));
    }

    #[test]
    fn degenerate_clip_never_materializes() {
        assert!(!preroll_window(100, 100, 100));
        assert!(!preroll_window(100, 100, 50));
    }
}
