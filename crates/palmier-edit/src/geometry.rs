//! E3-S5 — Timeline layout geometry (pure).
//!
//! Frame↔pixel mapping, track hit-testing, and drop-target resolution. Ported
//! 1:1 from `Sources/PalmierPro/Timeline/TimelineGeometry.swift`. See
//! docs/reference/timeline-model.md §"Geometry" (lines 113-119) and
//! docs/reference/edit-engines.md lines 36, 49-50, 210-211, and story E3-S5.
//!
//! ## Constants (`Constants.swift`, edit-engines.md lines 49-50)
//! - `ruler_height = 24`, `drop_zone_height = 60`, `track_height = 50`
//! - `insert_threshold = 10`

/// Ruler band height in px (reference `Layout.rulerHeight`).
pub const RULER_HEIGHT: f64 = 24.0;
/// Top drop-zone height in px (reference `Layout.dropZoneHeight`).
pub const DROP_ZONE_HEIGHT: f64 = 60.0;
/// Default per-track height in px (reference `Layout.trackHeight`).
pub const TRACK_HEIGHT: f64 = 50.0;
/// Between-track insertion catch height in px (reference `Layout.insertThreshold`).
pub const INSERT_THRESHOLD: f64 = 10.0;

/// Where a drop lands vertically.
///
/// Reference `TrackDropTarget`. `NewTrackAt(i)` means "insert a new track before
/// index `i`".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackDropTarget {
    /// Drop onto the existing track at this index.
    ExistingTrack(usize),
    /// Insert a new track before this index.
    NewTrackAt(usize),
}

/// A pure axis-aligned rectangle (replaces `NSRect` — edit-engines.md line 192).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left edge x.
    pub x: f64,
    /// Top edge y.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// Pure timeline layout math, parameterized by zoom + the per-track heights.
///
/// Mirrors `TimelineGeometry`: the track stack starts at
/// `ruler_height + drop_zone_height` and accumulates each track's height. All
/// methods are pure and total (out-of-range indices clamp like the reference).
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineGeometry {
    pixels_per_frame: f64,
    header_width: f64,
    track_heights: Vec<f64>,
    /// Cumulative Y of each track's top (precomputed, like the reference).
    cumulative_y: Vec<f64>,
}

impl TimelineGeometry {
    /// Build geometry for `track_heights` at `pixels_per_frame` zoom, with a
    /// left `header_width` gutter.
    pub fn new(pixels_per_frame: f64, header_width: f64, track_heights: Vec<f64>) -> Self {
        let mut cumulative_y = Vec::with_capacity(track_heights.len());
        let mut y = RULER_HEIGHT + DROP_ZONE_HEIGHT;
        for &h in &track_heights {
            cumulative_y.push(y);
            y += h;
        }
        TimelineGeometry {
            pixels_per_frame,
            header_width,
            track_heights,
            cumulative_y,
        }
    }

    /// Number of tracks.
    pub fn track_count(&self) -> usize {
        self.track_heights.len()
    }

    /// Pixel x → timeline frame: `max(0, floor((x - header_width) / ppf))`
    /// (reference `frameAt`). Clamps to frame 0; never negative.
    pub fn frame_at(&self, x: f64) -> i32 {
        let raw = (x - self.header_width) / self.pixels_per_frame;
        // Swift `Int(_:)` truncates toward zero; combined with `max(0, …)` this is
        // a floor on the non-negative side.
        (raw as i32).max(0)
    }

    /// Timeline frame → pixel x: `header_width + frame · ppf` (reference `xForFrame`).
    pub fn x_for_frame(&self, frame: i32) -> f64 {
        self.header_width + frame as f64 * self.pixels_per_frame
    }

    /// Top Y of the track at `index` (clamps to the ruler band if out of range).
    pub fn track_y(&self, index: usize) -> f64 {
        self.cumulative_y.get(index).copied().unwrap_or(RULER_HEIGHT)
    }

    /// Height of the track at `index` (defaults to `TRACK_HEIGHT`).
    pub fn track_height(&self, index: usize) -> f64 {
        self.track_heights.get(index).copied().unwrap_or(TRACK_HEIGHT)
    }

    /// Pixel y → track index (reference `trackAt`): the first track whose bottom
    /// is below `y`, else the last track.
    pub fn track_at(&self, y: f64) -> usize {
        for i in 0..self.track_heights.len() {
            if y < self.cumulative_y[i] + self.track_heights[i] {
                return i;
            }
        }
        self.track_count().saturating_sub(1)
    }

    /// Clip body rect for a clip spanning `[start_frame, start_frame+duration)`
    /// on `track_index` (reference `clipRect`): `x = header + start·ppf`,
    /// `y = track_y + 2`, `w = duration·ppf`, `h = track_height - 4`.
    pub fn clip_rect(&self, start_frame: i32, duration_frames: i32, track_index: usize) -> Rect {
        Rect {
            x: self.header_width + start_frame as f64 * self.pixels_per_frame,
            y: self.track_y(track_index) + 2.0,
            width: duration_frames as f64 * self.pixels_per_frame,
            height: self.track_height(track_index) - 4.0,
        }
    }

    /// Pixel y → drop target (reference `dropTargetAt`):
    /// - above the first track's top → `NewTrackAt(0)` (top drop zone);
    /// - within `INSERT_THRESHOLD` of a between-track boundary → `NewTrackAt(i+1)`;
    /// - past the last track's bottom → `NewTrackAt(track_count)`;
    /// - otherwise → `ExistingTrack(i)`.
    pub fn drop_target_at(&self, y: f64) -> TrackDropTarget {
        let n = self.track_count();
        if n == 0 {
            return TrackDropTarget::NewTrackAt(0);
        }
        // Top drop zone.
        if y < self.cumulative_y[0] {
            return TrackDropTarget::NewTrackAt(0);
        }
        // Between-track boundaries.
        for i in 0..n - 1 {
            let bottom_of_track = self.cumulative_y[i] + self.track_heights[i];
            let top_of_next = self.cumulative_y[i + 1];
            if y >= bottom_of_track - INSERT_THRESHOLD && y <= top_of_next + INSERT_THRESHOLD {
                return TrackDropTarget::NewTrackAt(i + 1);
            }
        }
        // Bottom drop zone.
        let last_bottom = self.cumulative_y[n - 1] + self.track_heights[n - 1];
        if y >= last_bottom {
            return TrackDropTarget::NewTrackAt(n);
        }
        // On an existing track.
        for i in 0..n {
            if y < self.cumulative_y[i] + self.track_heights[i] {
                return TrackDropTarget::ExistingTrack(i);
            }
        }
        TrackDropTarget::ExistingTrack(n.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two tracks of default height, default zoom, no header gutter.
    fn geom() -> TimelineGeometry {
        TimelineGeometry::new(4.0, 0.0, vec![TRACK_HEIGHT, TRACK_HEIGHT])
    }

    #[test]
    fn frame_at_floors_and_clamps_to_zero() {
        let g = geom(); // ppf = 4
        assert_eq!(g.frame_at(0.0), 0);
        assert_eq!(g.frame_at(7.9), 1, "floor toward zero: 7.9/4 = 1.97 → 1");
        assert_eq!(g.frame_at(8.0), 2);
        assert_eq!(g.frame_at(-100.0), 0, "clamped at frame 0");
    }

    #[test]
    fn x_for_frame_round_trips_with_header() {
        let g = TimelineGeometry::new(4.0, 30.0, vec![TRACK_HEIGHT]);
        assert_eq!(g.x_for_frame(10), 30.0 + 40.0);
        assert_eq!(g.frame_at(g.x_for_frame(10)), 10);
    }

    #[test]
    fn clip_rect_matches_reference_insets() {
        let g = geom();
        // Track 0 top = ruler(24) + dropzone(60) = 84; clip y = 86, h = 46.
        let r = g.clip_rect(5, 10, 0);
        assert_eq!(r.x, 20.0); // 5 * 4
        assert_eq!(r.y, 86.0);
        assert_eq!(r.width, 40.0); // 10 * 4
        assert_eq!(r.height, 46.0); // 50 - 4
    }

    #[test]
    fn track_at_returns_index_then_clamps() {
        let g = geom(); // track0 [84,134), track1 [134,184)
        assert_eq!(g.track_at(90.0), 0);
        assert_eq!(g.track_at(140.0), 1);
        assert_eq!(g.track_at(9999.0), 1, "below all tracks → last");
    }

    // ---- drop_target_at boundary regions ---------------------------------

    #[test]
    fn drop_target_top_zone_is_new_track_zero() {
        let g = geom(); // first track top = 84
        assert_eq!(g.drop_target_at(0.0), TrackDropTarget::NewTrackAt(0));
        assert_eq!(g.drop_target_at(83.0), TrackDropTarget::NewTrackAt(0));
    }

    #[test]
    fn drop_target_between_tracks_within_insert_threshold() {
        let g = geom();
        // Boundary between track0 (bottom 134) and track1 (top 134). Within 10px
        // either side → NewTrackAt(1).
        assert_eq!(g.drop_target_at(130.0), TrackDropTarget::NewTrackAt(1));
        assert_eq!(g.drop_target_at(134.0), TrackDropTarget::NewTrackAt(1));
        assert_eq!(g.drop_target_at(143.0), TrackDropTarget::NewTrackAt(1));
    }

    #[test]
    fn drop_target_on_track_body_outside_threshold() {
        let g = geom();
        // 100 is inside track0 and >10px from the 134 boundary → ExistingTrack(0).
        assert_eq!(g.drop_target_at(100.0), TrackDropTarget::ExistingTrack(0));
    }

    #[test]
    fn drop_target_past_last_track_is_new_track_at_count() {
        let g = geom(); // last bottom = 184
        assert_eq!(g.drop_target_at(184.0), TrackDropTarget::NewTrackAt(2));
        assert_eq!(g.drop_target_at(500.0), TrackDropTarget::NewTrackAt(2));
    }

    #[test]
    fn drop_target_empty_timeline_is_new_track_zero() {
        let g = TimelineGeometry::new(4.0, 0.0, vec![]);
        assert_eq!(g.drop_target_at(50.0), TrackDropTarget::NewTrackAt(0));
    }
}
