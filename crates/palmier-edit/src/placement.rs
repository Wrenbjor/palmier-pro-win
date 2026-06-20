//! Minimal clip-placement views the pure engines operate over.
//!
//! ## Why a placement view (decoupling note)
//!
//! The full `palmier-model::Clip` (Epic 2, story E2-S5) is being built
//! **concurrently** and is **not** on this branch's base. The Epic 3 editing
//! engines, however, only ever need a small slice of a clip — its timeline span
//! (`start_frame` / `duration_frames`), its track, its `id`, and (for the
//! source↔timeline conversions) its `speed` + `trim_*` fields. So this crate
//! defines a lightweight, self-contained **placement view** instead of depending
//! on the full `Clip`.
//!
//! The orchestration layer (story E3-S6) adapts `Clip → ClipPlacement` /
//! `Clip → SplitClip` later — exactly mirroring how E5-S6's audio mixer used a
//! local `AudioClip` view rather than coupling to the full model.
//!
//! Two views are provided:
//!
//! - [`ClipPlacement`] — the span/track view consumed by **ripple**, **overwrite**
//!   and **snap**. Carries `speed` + `trim_start_frame` because the overwrite
//!   engine emits source-frame trim offsets (`trim + round(Δ·speed)`).
//! - [`SplitClip`] — a richer per-clip view consumed by **split/trim**, which must
//!   migrate keyframes and clamp fades, so it additionally carries
//!   `trim_end_frame`, `fade_*`, `volume_track`, and a `has_no_source_media` flag
//!   (image/text — ruling carry-forward: the source-material trim cap is removed).

/// The span/track slice of a clip the ripple / overwrite / snap engines read.
///
/// `id` is a **UUID string** (matches `Clip.id`'s `String` storage and
/// `ClipShift.clip_id`). All frame fields are timeline frames; `speed` and
/// `trim_start_frame` are source-domain values used only for the overwrite
/// engine's source-offset recomputation.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipPlacement {
    /// UUID-string clip id.
    pub id: String,
    /// Timeline start frame.
    pub start_frame: i32,
    /// Timeline duration in frames (`end_frame = start_frame + duration_frames`).
    pub duration_frames: i32,
    /// Index of the track this clip lives on (into the timeline's `tracks`).
    pub track_index: usize,
    /// Playback speed (source frames per timeline frame). `1.0` = realtime.
    pub speed: f64,
    /// Source frames trimmed off the head. Used by the overwrite engine when it
    /// recomputes a new `trim_start` after clearing a region.
    pub trim_start_frame: i32,
}

impl ClipPlacement {
    /// Construct a placement. `speed` defaults to realtime; use
    /// [`ClipPlacement::with_speed`] / field init for non-realtime clips.
    pub fn new(
        id: impl Into<String>,
        start_frame: i32,
        duration_frames: i32,
        track_index: usize,
    ) -> Self {
        ClipPlacement {
            id: id.into(),
            start_frame,
            duration_frames,
            track_index,
            speed: 1.0,
            trim_start_frame: 0,
        }
    }

    /// Builder-style override for `speed` (chaining from [`ClipPlacement::new`]).
    pub fn with_speed(mut self, speed: f64) -> Self {
        self.speed = speed;
        self
    }

    /// Builder-style override for `trim_start_frame`.
    pub fn with_trim_start(mut self, trim_start_frame: i32) -> Self {
        self.trim_start_frame = trim_start_frame;
        self
    }

    /// Timeline frame where the clip ends — reference `Clip.endFrame =
    /// startFrame + durationFrames` (half-open: the clip occupies
    /// `[start_frame, end_frame)`).
    pub fn end_frame(&self) -> i32 {
        self.start_frame + self.duration_frames
    }
}
