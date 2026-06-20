//! # palmier-edit
//!
//! Pure editing engines for the timeline editor (FOUNDATION §4, §6.4): ripple,
//! overwrite, snap + geometry, and split/trim. Every function here is a **pure
//! function** over lightweight clip-placement views — no UI, no IO, no async,
//! no side effects (the reference's one haptic side effect in `findSnap` is
//! dropped — see [`snap`]).
//!
//! ## Decoupling from the full `Clip` (Epic 3 wave note)
//!
//! The full `palmier-model::Clip` (story E2-S5) is built **concurrently** and is
//! not on this crate's branch base, so these engines operate over minimal
//! placement views defined here ([`placement::ClipPlacement`],
//! [`split::SplitClip`], [`snap::SnapClip`]) rather than the full `Clip`. The
//! orchestration layer (story E3-S6) adapts `Clip → placement` later — mirroring
//! how E5-S6's audio mixer used a local `AudioClip` view. The shared edit value
//! types ([`palmier_model::FrameRange`], [`palmier_model::ClipShift`]) and the
//! `Interpolation` / `lerp` / `smoothstep` sampling helpers **are** on main and
//! are used directly.
//!
//! ## Modules
//! - [`ripple`] — E3-S2: gap-close shifts, insert push, range merge, validate.
//! - [`overwrite`] — E3-S3: clear-region delete/trim/split actions.
//! - [`split`] — E3-S4: split math, keyframe migration, trim source↔timeline math.
//! - [`snap`] — E3-S5: side-effect-free snap-target collection + sticky finder.
//! - [`geometry`] — E3-S5: frame↔pixel mapping, track + drop-target hit-testing.
//! - [`adapter`] — E3-S6: the `Clip → placement-view` seam (real model → engines).
//! - [`orchestration`] — E3-S6: ripple/overwrite/split fan-out over a real
//!   `Timeline` + `palmier-history`, with sync-lock/linked fan-out, atomic
//!   refuse-the-whole-edit, and one-undo-step-per-edit grouping.
//! - [`drag`] — E3-S7: the pure `DragState` machine + move/trim clamping feeding snap.
//! - [`rounding`] — the shared `f64::round` ties-away convention (no `round_ties_even`).
//!
//! Slip and Slide are intentionally **absent** (reconciliation ruling #11 — no
//! reference implementation exists).

pub mod adapter;
pub mod captions;
pub mod drag;
pub mod geometry;
pub mod orchestration;
pub mod overwrite;
pub mod placement;
pub mod ripple;
pub mod rounding;
pub mod snap;
pub mod split;

// Re-export the most-used surface so the orchestration layer (E3-S6) and the
// Tauri command layer can `use palmier_edit::{...}` without deep paths.
pub use adapter::{
    clip_to_placement, clip_to_snap_clip, clip_to_split_clip, has_no_source_media,
    track_to_placements, tracks_to_snap_clips,
};
pub use captions::{
    generate_captions, place_text_clips, specs, AssetInfo, CaptionCase, CaptionError,
    CaptionRequest, GenerateCaptionsResult, NoAssetInfo, TextClipSpec, GENERATE_CAPTIONS_UNDO_NAME,
};
pub use drag::{
    begin_clip_drag, clamp_move_frame_delta, clamp_trim_delta, clamped_track_delta, clip_sub_mode,
    is_pinned, marquee_exceeds_threshold, move_snap_probes, pinned_companions, resolve_move_delta,
    trim_drag_clamp, ClipSubMode, DragState, Modifiers, MoveClip, MovePayload, MarqueePayload,
    TrackKind, TrimPayload,
};
pub use geometry::{Rect, TimelineGeometry, TrackDropTarget};
pub use orchestration::{
    expand_to_link_group, move_clips, ripple_delete_gap, ripple_delete_ranges_on_track,
    ripple_delete_selected_clips, ripple_insert_clips, split_at, MoveSpec, RippleRangeReport,
};
pub use overwrite::{compute_overwrite, compute_overwrite_with, OverwriteAction};
pub use placement::ClipPlacement;
pub use ripple::{
    compute_ripple_push, compute_ripple_shifts, compute_ripple_shifts_for_ranges, merge_ranges,
    validate_shifts, RefuseReason,
};
pub use rounding::round_ties_away;
pub use snap::{collect_targets, find_snap, SnapClip, SnapKind, SnapResult, SnapState, SnapTarget};
pub use split::{
    apply_trim_internal, migrate_volume_split, sample_volume, split_clip, trim_clamp, trim_values,
    SplitClip, TrimClamp, TrimEdge, TrimResult, VolumeKeyframe,
};
