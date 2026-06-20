//! Edit value types shared by the editing engines (`palmier-edit`) and the
//! orchestration layer (Epic 3).
//!
//! Ported 1:1 from the macOS reference `Editor/RippleEngine.swift`
//! (`FrameRange`, `ClipShift`, `GapSelection`) and
//! `Timeline/TimelineRangeSelection.swift` (`TimelineRangeSelection`). See
//! docs/reference/edit-engines.md §"Key types" and docs/reference/timeline-model.md
//! §"Range selection".
//!
//! Story **E3-S1** — these types give every engine (Ripple/Overwrite/Snap/Split)
//! and the orchestration layer a shared vocabulary. They are pure value types: no
//! fs, no async, `Copy` where the reference value type is trivially copyable.
//!
//! ## Conventions
//!
//! - **Frame ranges are half-open `[start, end)`** (docs/reference/edit-engines.md
//!   line 26): `length = end − start`, and `contains(frame)` is true for
//!   `frame == start` but false for `frame == end`.
//! - **IDs are UUID strings, not typed `Uuid`** (docs/reference/timeline-model.md
//!   line 58 / ruling carry-forward): `ClipShift.clip_id` is a plain `String`,
//!   matching `Clip.id`'s `String` storage.

use serde::{Deserialize, Serialize};

/// A half-open `[start, end)` frame interval on a single track.
///
/// Used to describe the gaps a ripple edit needs to close (reference
/// `RippleEngine.FrameRange`). `start`/`end` are timeline frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRange {
    /// Inclusive start frame.
    pub start: i32,
    /// Exclusive end frame.
    pub end: i32,
}

impl FrameRange {
    /// Construct a half-open range `[start, end)`.
    pub fn new(start: i32, end: i32) -> Self {
        FrameRange { start, end }
    }

    /// Number of frames spanned — reference `FrameRange.length = end − start`.
    /// May be zero (empty) or negative (reversed); callers normalize as needed.
    pub fn length(self) -> i32 {
        self.end - self.start
    }

    /// Half-open membership — `frame >= start && frame < end`. `start` is in the
    /// range; `end` is NOT (boundary semantics the engines rely on).
    pub fn contains(self, frame: i32) -> bool {
        frame >= self.start && frame < self.end
    }
}

/// A proposed new start frame for a single clip, produced by the ripple engine
/// and applied by the caller (reference `RippleEngine.ClipShift`).
///
/// `clip_id` is a **UUID string** (matches `Clip.id`'s `String` storage), not a
/// typed `Uuid`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClipShift {
    /// The clip to move, by its UUID-string id.
    pub clip_id: String,
    /// The clip's new `start_frame` after the shift.
    pub new_start_frame: i32,
}

impl ClipShift {
    /// Construct a shift for `clip_id` to `new_start_frame`.
    pub fn new(clip_id: impl Into<String>, new_start_frame: i32) -> Self {
        ClipShift {
            clip_id: clip_id.into(),
            new_start_frame,
        }
    }
}

/// A user-selected empty gap on a single track (reference
/// `RippleEngine.GapSelection`). `track_index` is a position into the timeline's
/// `tracks` vec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GapSelection {
    /// Index of the track holding the gap.
    pub track_index: usize,
    /// The gap's half-open frame range.
    pub range: FrameRange,
}

impl GapSelection {
    /// Construct a gap selection on `track_index` over `range`.
    pub fn new(track_index: usize, range: FrameRange) -> Self {
        GapSelection { track_index, range }
    }
}

/// A shift-drag time range over the timeline (reference
/// `TimelineRangeSelection`). Stored as raw `start_frame`/`end_frame` which may be
/// reversed during a drag; use [`normalized`](Self::normalized) to order them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimelineRangeSelection {
    /// Drag-anchor frame (may be greater than `end_frame` mid-drag).
    pub start_frame: i32,
    /// Drag-cursor frame.
    pub end_frame: i32,
}

impl TimelineRangeSelection {
    /// Construct a range selection (frames may be in either order).
    pub fn new(start_frame: i32, end_frame: i32) -> Self {
        TimelineRangeSelection {
            start_frame,
            end_frame,
        }
    }

    /// Ordered copy with `start_frame <= end_frame` (reference `normalized`):
    /// swaps the two frames if the drag ran backward.
    pub fn normalized(self) -> Self {
        if self.start_frame <= self.end_frame {
            self
        } else {
            TimelineRangeSelection {
                start_frame: self.end_frame,
                end_frame: self.start_frame,
            }
        }
    }

    /// Whether the (normalized) range spans at least one frame (reference
    /// `isValid = end > start`).
    pub fn is_valid(self) -> bool {
        let r = self.normalized();
        r.end_frame > r.start_frame
    }

    /// Half-open membership on the normalized range — `[start, end)` (reference
    /// `contains(frame:)`).
    pub fn contains(self, frame: i32) -> bool {
        let r = self.normalized();
        frame >= r.start_frame && frame < r.end_frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_range_length_and_half_open_contains() {
        let r = FrameRange::new(10, 20);
        assert_eq!(r.length(), 10);
        // Half-open: start is in, end is out.
        assert!(r.contains(10), "start frame is inside");
        assert!(r.contains(19), "frame just before end is inside");
        assert!(!r.contains(20), "end frame is OUTSIDE (half-open)");
        assert!(!r.contains(9), "before start is outside");
    }

    #[test]
    fn frame_range_empty_and_round_trips() {
        let empty = FrameRange::new(5, 5);
        assert_eq!(empty.length(), 0);
        assert!(!empty.contains(5), "empty half-open range contains nothing");

        let json = serde_json::to_string(&FrameRange::new(3, 7)).unwrap();
        let back: FrameRange = serde_json::from_str(&json).unwrap();
        assert_eq!(back, FrameRange::new(3, 7));
    }

    #[test]
    fn clip_shift_holds_uuid_string_id() {
        let s = ClipShift::new("550e8400-e29b-41d4-a716-446655440000", 120);
        assert_eq!(s.clip_id, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(s.new_start_frame, 120);

        let json = serde_json::to_string(&s).unwrap();
        let back: ClipShift = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn gap_selection_wraps_track_and_range() {
        let g = GapSelection::new(2, FrameRange::new(30, 45));
        assert_eq!(g.track_index, 2);
        assert_eq!(g.range.length(), 15);
    }

    #[test]
    fn range_selection_normalizes_reversed_drag() {
        let reversed = TimelineRangeSelection::new(100, 40);
        let n = reversed.normalized();
        assert_eq!(n.start_frame, 40);
        assert_eq!(n.end_frame, 100);

        // Forward drag is unchanged.
        let forward = TimelineRangeSelection::new(40, 100);
        assert_eq!(forward.normalized(), forward);
    }

    #[test]
    fn range_selection_is_valid_requires_span() {
        assert!(TimelineRangeSelection::new(10, 20).is_valid());
        assert!(TimelineRangeSelection::new(20, 10).is_valid(), "reversed but spans");
        assert!(!TimelineRangeSelection::new(15, 15).is_valid(), "zero-width invalid");
    }

    #[test]
    fn range_selection_half_open_contains_boundary() {
        let sel = TimelineRangeSelection::new(10, 20);
        assert!(sel.contains(10), "start inside");
        assert!(sel.contains(19), "before end inside");
        assert!(!sel.contains(20), "end OUTSIDE (half-open)");
        assert!(!sel.contains(9), "before start outside");

        // Works the same on a reversed (un-normalized) selection.
        let rev = TimelineRangeSelection::new(20, 10);
        assert!(rev.contains(10));
        assert!(!rev.contains(20));
    }
}
