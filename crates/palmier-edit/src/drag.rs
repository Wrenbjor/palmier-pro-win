//! E3-S7 — Drag-state machine + clamping (pure transitions).
//!
//! The `DragState` machine and its clamping as **pure** transitions driven by
//! pointer events, so move / trim / duplicate / marquee / range behavior is
//! testable without the canvas. Ported from `Timeline/DragState.swift` and
//! `Timeline/TimelineInputController.swift` (mouseDown/Dragged/Up hit-testing +
//! clamping). See docs/reference/edit-engines.md §"Selection & tool modes" (lines
//! 149-174) and §"drag mod" (lines 211-212), and story E3-S7.
//!
//! This module is pure layout/clamp math: it consumes [`crate::geometry`] (the
//! canvas hit regions already live there) and [`crate::snap`] (the snap finder),
//! and the trim clamps from [`crate::split`]. It returns *what* should happen
//! (a resolved move/trim/split); the orchestration layer ([`crate::orchestration`])
//! applies it to the real timeline.
//!
//! ## Modifier mapping (edit-engines.md line 190)
//! macOS Cmd → **Ctrl**, Option → **Alt** on Win/Linux. The pure machine takes a
//! [`Modifiers`] struct the UI fills from JS pointer-event flags.

use crate::snap::{find_snap, SnapResult, SnapState, SnapTarget};
use crate::split::{trim_clamp, SplitClip, TrimClamp, TrimEdge};

/// Trim-handle hit width in px (reference `Trim.handleWidth`). A grab within this
/// many px of an edge is a trim, not a move.
pub const TRIM_HANDLE_PX: f64 = 4.0;
/// Marquee drag threshold in px (reference `Layout.dragThreshold`): a marquee
/// cancels a gap selection once the rect grows past this.
pub const DRAG_THRESHOLD: f64 = 3.0;

/// Pointer modifier flags (Win/Linux mapping of the macOS event flags).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// `shiftKey` — additive selection / range drag on the ruler.
    pub shift: bool,
    /// `ctrlKey` (or `metaKey`) — macOS Cmd. Toggle selection; add volume kf.
    pub ctrl: bool,
    /// `altKey` — macOS Option. Duplicate on body grab; suppress link expansion.
    pub alt: bool,
}

/// The drag-interaction the machine is in. Mirrors the reference `enum DragState`
/// (edit-engines.md lines 33-34). Each non-`Idle` case carries the payload the
/// commit needs.
#[derive(Debug, Clone, PartialEq)]
pub enum DragState {
    /// No active drag.
    Idle,
    /// Scrubbing the playhead from the ruler.
    ScrubPlayhead {
        /// Frame under the cursor.
        frame: i32,
    },
    /// Moving (or duplicating) one or more clips.
    MoveClip(MovePayload),
    /// Trimming a clip's left (head) edge.
    TrimLeft(TrimPayload),
    /// Trimming a clip's right (tail) edge.
    TrimRight(TrimPayload),
    /// Dragging an audio volume keyframe (canvas interaction — payload opaque here).
    AudioVolumeKf {
        /// Id of the clip whose volume kf is being dragged.
        clip_id: String,
    },
    /// Dragging a fade knee.
    FadeKnee {
        /// Id of the clip whose fade is being dragged.
        clip_id: String,
        /// Which edge's fade (`Left` = fade-in knee, `Right` = fade-out knee).
        edge: TrimEdge,
    },
    /// Rubber-band marquee selection.
    Marquee(MarqueePayload),
    /// Shift-drag time-range selection on the ruler.
    TimelineRange {
        /// Anchor frame where the range drag began.
        anchor_frame: i32,
        /// Current cursor frame.
        cursor_frame: i32,
    },
}

/// Payload for a [`DragState::MoveClip`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovePayload {
    /// The lead clip's id (the one under the cursor on mouse-down).
    pub lead_id: String,
    /// The lead clip's original start frame (the reference frame for delta math).
    pub lead_original_frame: i32,
    /// `true` when Alt was held on a body grab → drop duplicates instead of moving.
    pub is_duplicate: bool,
}

/// Payload for a [`DragState::TrimLeft`] / [`DragState::TrimRight`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrimPayload {
    /// The clip being trimmed.
    pub clip_id: String,
    /// Which edge.
    pub edge: TrimEdge,
}

/// Payload for a [`DragState::Marquee`].
#[derive(Debug, Clone, PartialEq)]
pub struct MarqueePayload {
    /// Pixel origin of the marquee (mouse-down point).
    pub origin: (f64, f64),
    /// Current pixel cursor.
    pub cursor: (f64, f64),
    /// The selection that existed before the marquee began (the base it adds to).
    pub base_selection: Vec<String>,
}

/// The drag sub-mode hit-test result for a mouse-down **over a clip body**
/// (reference `mouseDown`/`localX` switch, edit-engines.md lines 155-158).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipSubMode {
    /// `local_x <= 4` → trimming the left edge.
    TrimLeft,
    /// `local_x >= width - 4` → trimming the right edge.
    TrimRight,
    /// Anywhere in the body → moving (or duplicating with Alt).
    Move,
}

/// Decide the drag sub-mode from the cursor's x **relative to the clip's left edge**
/// (`local_x = point_x - clip_rect.min_x`) and the clip's pixel `width`.
///
/// Reference `localX` switch (edit-engines.md lines 155-157): `local_x <= 4` →
/// trim-left; `local_x >= width - 4` → trim-right; else → move. Fade-knee and
/// audio-volume-kf hits are resolved *before* this by the canvas (they need the
/// clip's fade/keyframe geometry) and are not part of this width-based decision.
pub fn clip_sub_mode(local_x: f64, width: f64) -> ClipSubMode {
    if local_x <= TRIM_HANDLE_PX {
        ClipSubMode::TrimLeft
    } else if local_x >= width - TRIM_HANDLE_PX {
        ClipSubMode::TrimRight
    } else {
        ClipSubMode::Move
    }
}

/// Begin a clip drag from a body/edge grab, producing the initial [`DragState`].
///
/// `local_x`/`width` pick the sub-mode; `alt` on a body grab sets `is_duplicate`.
/// `lead_original_frame` is the grabbed clip's `start_frame`.
pub fn begin_clip_drag(
    clip_id: &str,
    local_x: f64,
    width: f64,
    lead_original_frame: i32,
    mods: Modifiers,
) -> DragState {
    match clip_sub_mode(local_x, width) {
        ClipSubMode::TrimLeft => DragState::TrimLeft(TrimPayload {
            clip_id: clip_id.to_string(),
            edge: TrimEdge::Left,
        }),
        ClipSubMode::TrimRight => DragState::TrimRight(TrimPayload {
            clip_id: clip_id.to_string(),
            edge: TrimEdge::Right,
        }),
        ClipSubMode::Move => DragState::MoveClip(MovePayload {
            lead_id: clip_id.to_string(),
            lead_original_frame,
            is_duplicate: mods.alt,
        }),
    }
}

/// A clip participating in a move drag: its id, original frame, original track, and
/// type — enough to compute probes, clamps, and pinned companions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveClip {
    /// Clip id.
    pub id: String,
    /// Original timeline start frame.
    pub original_frame: i32,
    /// Original duration in frames (used to push the second snap probe).
    pub duration_frames: i32,
    /// Original track index.
    pub original_track: usize,
    /// Whether this clip is a linked partner of the lead (pins it to its own row).
    pub linked_to_lead: bool,
    /// `true` when this clip's type is **incompatible** with the lead's destination
    /// track type (also pins it — A/V pairs stay split).
    pub incompatible_with_dest: bool,
}

/// The two snap probe offsets for a moving clip, **relative to the lead's original
/// frame**: its start and its end (`start + duration`).
///
/// Reference (edit-engines.md lines 145-147): for every dragged clip the controller
/// pushes TWO probe offsets so any edge of any selected clip can snap. The offset
/// is `clip.original_frame - lead.original_frame` (+`duration` for the end edge).
pub fn move_snap_probes(movers: &[MoveClip], lead_original_frame: i32) -> Vec<i32> {
    let mut probes = Vec::with_capacity(movers.len() * 2);
    for m in movers {
        let base = m.original_frame - lead_original_frame;
        probes.push(base);
        probes.push(base + m.duration_frames);
    }
    probes
}

/// Run the move snap and resolve the raw timeline-frame delta.
///
/// `position` is the lead's *candidate* frame (lead original + the raw drag delta).
/// Returns the snapped `delta_frames = (snap.frame - snap.probe_offset) -
/// lead_original_frame` when a snap is found, else the raw `position -
/// lead_original_frame` (edit-engines.md line 147).
pub fn resolve_move_delta(
    position: i32,
    lead_original_frame: i32,
    probe_offsets: &[i32],
    targets: &[SnapTarget],
    state: &mut SnapState,
    base_threshold_px: f64,
    pixels_per_frame: f64,
) -> (i32, Option<SnapResult>) {
    match find_snap(
        position,
        probe_offsets,
        targets,
        state,
        base_threshold_px,
        pixels_per_frame,
    ) {
        Some(snap) => {
            let delta = (snap.frame - snap.probe_offset) - lead_original_frame;
            (delta, Some(snap))
        }
        None => (position - lead_original_frame, None),
    }
}

/// Clamp a move's frame delta so no moving clip is pushed before frame 0.
///
/// Reference `frameDelta = max(-minOrigFrame, deltaFrames)` (edit-engines.md line
/// 164): `min_orig_frame` is the smallest original start frame among the movers.
pub fn clamp_move_frame_delta(delta_frames: i32, movers: &[MoveClip]) -> i32 {
    let min_orig = movers.iter().map(|m| m.original_frame).min().unwrap_or(0);
    delta_frames.max(-min_orig)
}

/// A track and its clip type — the slice the track-delta clamp needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackKind {
    /// The track's clip type.
    pub clip_type: palmier_model::ClipType,
}

/// Step a vertical track delta toward 0 until **every non-pinned mover** lands on a
/// type-compatible track.
///
/// Reference `clampedTrackDelta` (edit-engines.md lines 166-167): the requested
/// `track_delta` is reduced toward zero until each mover's destination
/// (`original_track + delta`) exists and is compatible with the mover's type;
/// pinned companions are excluded (they keep their own row). Returns the largest
/// in-magnitude delta (same sign) for which all movers fit, or 0 if none does.
pub fn clamped_track_delta(
    track_delta: isize,
    movers: &[MoveClip],
    mover_types: &[palmier_model::ClipType],
    tracks: &[TrackKind],
) -> isize {
    debug_assert_eq!(movers.len(), mover_types.len());
    let fits = |delta: isize| -> bool {
        for (m, &ty) in movers.iter().zip(mover_types) {
            if is_pinned(m) {
                continue;
            }
            let dest = m.original_track as isize + delta;
            if dest < 0 || dest as usize >= tracks.len() {
                return false;
            }
            if !ty.is_compatible(tracks[dest as usize].clip_type) {
                return false;
            }
        }
        true
    };
    let step: isize = if track_delta >= 0 { -1 } else { 1 };
    let mut d = track_delta;
    loop {
        if fits(d) {
            return d;
        }
        if d == 0 {
            return 0;
        }
        d += step;
    }
}

/// Whether a mover is a **pinned companion**: a linked partner of the lead, OR a
/// co-selected clip whose type is incompatible with the lead's destination type.
///
/// Pinned companions keep their own row during a cross-track move (edit-engines.md
/// lines 167-168, 230-231) — this is what keeps A/V pairs correct.
pub fn is_pinned(m: &MoveClip) -> bool {
    m.linked_to_lead || m.incompatible_with_dest
}

/// The ids of the pinned companions among `movers`.
pub fn pinned_companions(movers: &[MoveClip]) -> Vec<String> {
    movers
        .iter()
        .filter(|m| is_pinned(m))
        .map(|m| m.id.clone())
        .collect()
}

/// The trim clamp for a trim drag (delegates to [`crate::split::trim_clamp`]).
///
/// Re-exported through the drag module because the input controller asks the drag
/// machine, not the split engine, for clamps (edit-engines.md lines 113-118).
pub fn trim_drag_clamp(clip: &SplitClip, edge: TrimEdge) -> TrimClamp {
    trim_clamp(clip, edge)
}

/// Clamp a raw trim drag delta to the clip's [`TrimClamp`] bounds.
pub fn clamp_trim_delta(delta_frames: i32, clamp: TrimClamp) -> i32 {
    delta_frames.clamp(clamp.min_delta, clamp.max_delta)
}

/// Whether a marquee rect has grown past the drag threshold (cancels a gap
/// selection). Reference: a marquee cancels gap selection once the rect exceeds
/// `drag_threshold = 3` in either dimension (edit-engines.md lines 162-163).
pub fn marquee_exceeds_threshold(origin: (f64, f64), cursor: (f64, f64)) -> bool {
    (cursor.0 - origin.0).abs() > DRAG_THRESHOLD || (cursor.1 - origin.1).abs() > DRAG_THRESHOLD
}

/// The pixel rect (x, y, w, h) of a marquee from origin↔cursor (min/max).
pub fn marquee_rect(origin: (f64, f64), cursor: (f64, f64)) -> crate::geometry::Rect {
    let x = origin.0.min(cursor.0);
    let y = origin.1.min(cursor.1);
    let w = (cursor.0 - origin.0).abs();
    let h = (cursor.1 - origin.1).abs();
    crate::geometry::Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests;
