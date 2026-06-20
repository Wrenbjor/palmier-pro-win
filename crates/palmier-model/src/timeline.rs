//! `Track` + `Timeline` — the timeline root shapes (story E2-S6).
//!
//! Ported 1:1 from the macOS reference `Sources/PalmierPro/Models/Timeline.swift`
//! (`struct Timeline`, `struct Track`). See docs/reference/timeline-model.md
//! "Data model" and the Port risk "non-serialized `displayHeight`".
//!
//! ## Lenient / defaulted decode is load-bearing
//!
//! The reference `Track.init(from:)` decodes every field with `(try? …) ?? default`
//! and `Timeline` is a plain struct with stored defaults, so old/partial projects
//! load instead of erroring. We mirror this with `#[serde(default)]` on every field
//! plus a `#[derive(Default)]`-backed `Default` impl carrying the reference defaults
//! (fps 30 / 1920×1080 / `sync_locked = true` / etc.). Unknown extra fields are
//! ignored (serde's default behavior — no `deny_unknown_fields`).
//!
//! ## `display_height` is NOT serialized
//!
//! The reference declares `displayHeight: CGFloat = 50` and **omits it from
//! `CodingKeys`** — it is a display-only field, reset to 50 on every open. We mark
//! it `#[serde(skip)]` so it never encodes and always decodes to its `Default`
//! (50.0), even if a stray `displayHeight`/`display_height` key is present in JSON
//! (Port risk: "non-serialized `displayHeight` (reset to 50 on open) must be
//! preserved or old projects … look wrong").
//!
//! ## fps frozen-after-first-clip
//!
//! fps / width / height are **frozen once the timeline holds its first clip** (the
//! enforcement lives in Epic 3's edit layer — the reference UI gates settings
//! changes). Here the fields are just stored; [`Timeline::has_any_clip`] exposes the
//! predicate the Epic 3 guard consumes, and [`Timeline::set_fps`] is a convenience
//! that honors the freeze so the invariant is testable at the model layer.

use serde::{Deserialize, Serialize};

use crate::clip::Clip;
use crate::clip_type::ClipType;
use uuid::Uuid;

/// Fresh UUID string — serde default for `Track::id` (reference
/// `Track.init(from:)`: `(try? …id) ?? UUID().uuidString`).
fn new_uuid_string() -> String {
    Uuid::new_v4().to_string()
}

fn default_fps() -> i32 {
    30
}
fn default_width() -> i32 {
    1920
}
fn default_height() -> i32 {
    1080
}
fn default_sync_locked() -> bool {
    true
}
/// Display-only track height; reset to this on every open (reference
/// `displayHeight: CGFloat = 50`).
fn default_display_height() -> f64 {
    50.0
}

/// A single timeline track: an ordered, non-overlapping list of clips of one
/// compatible-kind, plus per-track mute/hide/sync flags.
///
/// Wire keys mirror the reference `CodingKeys { id, type, muted, hidden,
/// syncLocked, clips }`. `display_height` is intentionally absent from the wire
/// (`#[serde(skip)]`). Every field is `#[serde(default)]` so a track JSON omitting
/// any of `id/muted/hidden/sync_locked/clips` decodes with the documented default
/// (`type` is the only field the reference decodes non-optionally).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    /// UUID **string** (regenerated if absent on decode).
    #[serde(default = "new_uuid_string")]
    pub id: String,
    /// The kind of media this track carries (required field).
    #[serde(rename = "type")]
    pub track_type: ClipType,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub hidden: bool,
    /// Defaults to **`true`** (reference `syncLocked: Bool = true`).
    #[serde(rename = "syncLocked", default = "default_sync_locked")]
    pub sync_locked: bool,
    #[serde(default)]
    pub clips: Vec<Clip>,

    /// Display-only height — **never serialized** (reference omits it from
    /// `CodingKeys`). Always decodes/resets to `50.0` on open.
    #[serde(skip, default = "default_display_height")]
    pub display_height: f64,
}

impl Track {
    /// A new empty track of `track_type` with reference defaults.
    pub fn new(track_type: ClipType) -> Self {
        Track {
            id: new_uuid_string(),
            track_type,
            muted: false,
            hidden: false,
            sync_locked: true,
            clips: Vec::new(),
            display_height: default_display_height(),
        }
    }

    /// Frame where the last clip ends (reference `Track.endFrame`):
    /// `max(clip.end_frame)`, or `0` when the track is empty.
    pub fn end_frame(&self) -> i32 {
        self.clips.iter().map(Clip::end_frame).max().unwrap_or(0)
    }

    /// Re-sort clips by `start_frame` ascending (reference keeps clips ordered for
    /// contiguity/chain logic). Stable so equal starts keep insertion order.
    pub fn sort_clips(&mut self) {
        self.clips.sort_by_key(|c| c.start_frame);
    }

    /// Whether any two clips overlap on `[start, end)`. Used by edit-layer
    /// invariants (E2-S6 acceptance: "clips sorted no-overlap"). Pure check —
    /// it does not mutate; the edit layer (Epic 3) owns conflict resolution.
    pub fn has_overlap(&self) -> bool {
        let mut sorted: Vec<&Clip> = self.clips.iter().collect();
        sorted.sort_by_key(|c| c.start_frame);
        sorted
            .windows(2)
            .any(|w| w[1].start_frame < w[0].end_frame())
    }
}

/// The timeline root: project frame rate + canvas resolution + the ordered track
/// stack. Persisted as `project.json`.
///
/// Every field is `#[serde(default)]`; a `{}` JSON decodes to all reference
/// defaults (fps 30 / 1920×1080 / `settings_configured = false` / no tracks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    /// Project frame rate. Frozen after the first clip (Epic 3 enforces).
    #[serde(default = "default_fps")]
    pub fps: i32,
    /// Canvas width in pixels. Frozen after the first clip.
    #[serde(default = "default_width")]
    pub width: i32,
    /// Canvas height in pixels. Frozen after the first clip.
    #[serde(default = "default_height")]
    pub height: i32,
    /// Whether the user has confirmed project settings (reference
    /// `settingsConfigured`).
    #[serde(rename = "settingsConfigured", default)]
    pub settings_configured: bool,
    #[serde(default)]
    pub tracks: Vec<Track>,
}

impl Timeline {
    /// A new empty timeline with reference defaults.
    pub fn new() -> Self {
        Timeline::default()
    }

    /// Total timeline length in frames (reference `totalFrames`):
    /// `max(track.end_frame)` over all tracks, or `0` when empty.
    pub fn total_frames(&self) -> i32 {
        self.tracks.iter().map(Track::end_frame).max().unwrap_or(0)
    }

    /// Whether the timeline holds at least one clip. The Epic 3 freeze guard reads
    /// this before allowing an fps/resolution change (fps is frozen-after-first-clip).
    pub fn has_any_clip(&self) -> bool {
        self.tracks.iter().any(|t| !t.clips.is_empty())
    }

    /// Set the project fps, honoring the **frozen-after-first-clip** rule:
    /// a no-op (returns `false`) once any clip exists; otherwise applies it
    /// (returns `true`). The authoritative enforcement is Epic 3's; this keeps the
    /// invariant testable at the model layer.
    pub fn set_fps(&mut self, fps: i32) -> bool {
        if self.has_any_clip() {
            return false;
        }
        self.fps = fps;
        true
    }
}

impl Default for Timeline {
    fn default() -> Self {
        Timeline {
            fps: default_fps(),
            width: default_width(),
            height: default_height(),
            settings_configured: false,
            tracks: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_timeline_json_decodes_to_defaults() {
        // A minimal `{}` timeline → all reference defaults.
        let t: Timeline = serde_json::from_str("{}").unwrap();
        assert_eq!(t.fps, 30);
        assert_eq!(t.width, 1920);
        assert_eq!(t.height, 1080);
        assert!(!t.settings_configured);
        assert!(t.tracks.is_empty());
        assert_eq!(t.total_frames(), 0);
    }

    #[test]
    fn timeline_ignores_unknown_fields() {
        // Unknown extra keys are ignored (no deny_unknown_fields) — lenient decode.
        let json = r#"{"fps":24,"somethingNew":42,"tracks":[]}"#;
        let t: Timeline = serde_json::from_str(json).unwrap();
        assert_eq!(t.fps, 24);
    }

    #[test]
    fn track_omitting_flags_decodes_to_reference_defaults() {
        // muted/hidden/sync_locked absent → false / false / TRUE (reference default).
        let json = r#"{"type":"video"}"#;
        let tr: Track = serde_json::from_str(json).unwrap();
        assert!(!tr.muted);
        assert!(!tr.hidden);
        assert!(tr.sync_locked, "sync_locked default must be true");
        assert!(tr.clips.is_empty());
        // id regenerated as a UUID string.
        assert!(Uuid::parse_str(&tr.id).is_ok());
    }

    #[test]
    fn display_height_is_not_serialized_and_resets_to_50() {
        // Encode never emits display_height.
        let mut tr = Track::new(ClipType::Video);
        tr.display_height = 123.0;
        let json = serde_json::to_string(&tr).unwrap();
        assert!(
            !json.contains("display"),
            "display_height must not be serialized: {json}"
        );
        // Decode resets to 50 even if the input JSON carried a different value
        // (the key is skipped, so it can never override the default).
        let with_height = r#"{"type":"video","displayHeight":200,"display_height":200}"#;
        let back: Track = serde_json::from_str(with_height).unwrap();
        assert_eq!(back.display_height, 50.0);
    }

    #[test]
    fn track_round_trips_byte_stable() {
        let mut tr = Track::new(ClipType::Audio);
        tr.id = "track-1".to_string();
        tr.muted = true;
        tr.clips.push(Clip::new("asset-1", 0, 30));
        let json = serde_json::to_string(&tr).unwrap();
        let back: Track = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, back);
    }

    #[test]
    fn end_frame_and_total_frames_over_multi_clip_fixture() {
        let mut v = Track::new(ClipType::Video);
        v.clips.push(Clip::new("a", 0, 30)); // ends at 30
        v.clips.push(Clip::new("b", 30, 60)); // ends at 90
        assert_eq!(v.end_frame(), 90);

        let mut a = Track::new(ClipType::Audio);
        a.clips.push(Clip::new("c", 0, 120)); // ends at 120
        assert_eq!(a.end_frame(), 120);

        let mut tl = Timeline::new();
        tl.tracks.push(v);
        tl.tracks.push(a);
        // total_frames = max over all tracks' end_frame.
        assert_eq!(tl.total_frames(), 120);
    }

    #[test]
    fn timeline_round_trips_with_tracks() {
        let mut tl = Timeline::new();
        tl.fps = 24;
        tl.width = 1280;
        tl.height = 720;
        tl.settings_configured = true;
        let mut v = Track::new(ClipType::Video);
        v.id = "t1".into();
        v.clips.push(Clip::new("a", 0, 30));
        tl.tracks.push(v);

        let json = serde_json::to_string(&tl).unwrap();
        let back: Timeline = serde_json::from_str(&json).unwrap();
        assert_eq!(tl, back);
        // settingsConfigured is the wire key.
        assert!(json.contains("settingsConfigured"));
    }

    #[test]
    fn fps_freeze_after_first_clip() {
        let mut tl = Timeline::new();
        // No clips yet → fps is settable.
        assert!(tl.set_fps(60));
        assert_eq!(tl.fps, 60);

        // Add a clip → fps is now frozen.
        let mut v = Track::new(ClipType::Video);
        v.clips.push(Clip::new("a", 0, 30));
        tl.tracks.push(v);
        assert!(tl.has_any_clip());
        assert!(!tl.set_fps(24), "fps must be frozen after the first clip");
        assert_eq!(tl.fps, 60, "fps unchanged after freeze");
    }

    #[test]
    fn has_overlap_detects_overlapping_clips() {
        let mut tr = Track::new(ClipType::Video);
        tr.clips.push(Clip::new("a", 0, 30)); // [0, 30)
        tr.clips.push(Clip::new("b", 30, 30)); // [30, 60) — adjacent, no overlap
        assert!(!tr.has_overlap());
        tr.clips.push(Clip::new("c", 50, 30)); // [50, 80) overlaps b
        assert!(tr.has_overlap());
    }
}
