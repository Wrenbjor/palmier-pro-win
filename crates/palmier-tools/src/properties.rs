//! EDIT tool bodies — properties / keyframes / ripple (E7-S7; reference
//! `ToolExecutor+Clips.swift` + `+Timeline.swift`).
//!
//! `set_clip_properties`, `set_keyframes`, `ripple_delete_ranges`. All mutating →
//! one named agent-undo step via [`agent_edit`](crate::undo::agent_edit).
//!
//! ## Rulings honoured
//! - **#7 center Transform** — `set_clip_properties`' `transform` patch is
//!   center-based (`centerX`/`centerY`/`width`/`height`), partial-merged over the
//!   clip's current transform.
//! - **#8 smooth keyframe default** — `set_keyframes` rows omit `interp` ⇒ smooth.
//! - **`f64::round` ties-away** — all source↔timeline frame math uses
//!   [`round_ties_away`](palmier_edit::round_ties_away).
//! - **`trim_*` are SOURCE offsets**; setting `volume`/`opacity` clears that
//!   property's keyframe track; text-only fields on a non-text clip → reject.
//! - **ripple units / `trackIndex` XOR `clipId`** are contract text — enforced both
//!   in validation (E7-S3) and here (the body re-derives the resolved track + maps
//!   source-seconds ranges through trim/speed/position).

use serde_json::{json, Value};

use palmier_edit::{ripple_delete_ranges_on_track, round_ties_away};
use palmier_model::{
    AnimPair, ClipType, Crop, FrameRange, Interpolation, Keyframe, KeyframeTrack, Timeline,
};

use crate::editor::EditorState;
use crate::result::ToolResult;
use crate::undo::agent_edit;
use crate::validate::parse_rgba;

// ─────────────────────────────────────────────────────────────────────────────
// set_clip_properties
// ─────────────────────────────────────────────────────────────────────────────

/// The text-only property keys (reference `textOnlyKeys`). Rejected on non-text clips.
const TEXT_ONLY_KEYS: [&str; 5] = ["content", "fontName", "fontSize", "color", "alignment"];

/// `set_clip_properties` (`clipIds[]` + any of the property fields): apply to ALL
/// listed clips. `trim_*` are SOURCE offsets; setting `volume`/`opacity` clears that
/// keyframe track; transform is center-based (ruling #7); text-only fields on a
/// non-text clip → reject. Reference `setClipProperties`.
pub fn set_clip_properties(state: &mut EditorState, args: &Value) -> ToolResult {
    let obj = args.as_object().expect("validated object");
    let clip_ids: Vec<String> = match obj.get("clipIds").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => return ToolResult::error("Missing or empty 'clipIds' array"),
    };

    // At least one property must be present.
    let property_keys = [
        "durationFrames", "trimStartFrame", "trimEndFrame", "speed", "volume", "opacity",
        "transform", "content", "fontName", "fontSize", "color", "alignment",
    ];
    if !property_keys.iter().any(|k| obj.contains_key(*k)) {
        return ToolResult::error("set_clip_properties needs at least one property to apply");
    }
    if let Some(df) = obj.get("durationFrames").and_then(Value::as_i64)
        && df < 1
    {
        return ToolResult::error(format!("durationFrames must be >= 1 (got {df})"));
    }
    // Validate color up front (reject bad hex before mutating).
    if let Some(c) = obj.get("color").and_then(Value::as_str)
        && let Err(e) = parse_rgba(c)
    {
        return ToolResult::error(e.message);
    }

    // Resolve clips + collect types; reject text-only fields on non-text clips.
    let mut clip_types: Vec<(String, ClipType)> = Vec::new();
    for id in &clip_ids {
        let ty = state
            .library
            .timeline
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .find(|c| &c.id == id)
            .map(|c| c.media_type);
        match ty {
            Some(t) => clip_types.push((id.clone(), t)),
            None => return ToolResult::error(format!("Clip not found: {id}")),
        }
    }
    let text_only_used: Vec<&str> = TEXT_ONLY_KEYS.iter().copied().filter(|k| obj.contains_key(*k)).collect();
    if !text_only_used.is_empty() {
        let non_text: Vec<String> = clip_types
            .iter()
            .filter(|(_, t)| *t != ClipType::Text)
            .map(|(id, _)| id.clone())
            .collect();
        if !non_text.is_empty() {
            return ToolResult::error(format!(
                "text-only fields '{}' rejected on non-text clips: {}",
                text_only_used.join("', '"),
                non_text.join(", ")
            ));
        }
    }

    // Capture the patch as owned values for the swap closure.
    let patch = PropertyPatch::from_args(obj);
    let action_name = if clip_ids.len() == 1 { "Set Clip Property (Agent)" } else { "Set Clip Properties (Agent)" };
    let count = clip_ids.len();
    agent_edit(state, action_name, move |timeline, _hist| {
        let mut summaries = Vec::new();
        for (id, ty) in &clip_types {
            let is_text = *ty == ClipType::Text;
            let changed = apply_property_changes(timeline, id, &patch, is_text);
            summaries.push(if changed.is_empty() {
                format!("{id} (no-op)")
            } else {
                format!("{id}: {}", changed.join(", "))
            });
        }
        Ok(ToolResult::ok(format!("Updated {count} clip(s): {}", summaries.join("; "))))
    })
}

/// The owned property patch, parsed from the args object once before the swap.
struct PropertyPatch {
    duration_frames: Option<i32>,
    trim_start_frame: Option<i32>,
    trim_end_frame: Option<i32>,
    speed: Option<f64>,
    volume: Option<f64>,
    opacity: Option<f64>,
    transform: Option<TransformPatch>,
    content: Option<String>,
    font_name: Option<String>,
    font_size: Option<f64>,
    color: Option<(u8, u8, u8, u8)>,
    alignment: Option<String>,
}

struct TransformPatch {
    center_x: Option<f64>,
    center_y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    flip_h: Option<bool>,
    flip_v: Option<bool>,
}

impl PropertyPatch {
    fn from_args(obj: &serde_json::Map<String, Value>) -> PropertyPatch {
        let transform = obj.get("transform").and_then(Value::as_object).map(|t| TransformPatch {
            center_x: t.get("centerX").and_then(Value::as_f64),
            center_y: t.get("centerY").and_then(Value::as_f64),
            width: t.get("width").and_then(Value::as_f64),
            height: t.get("height").and_then(Value::as_f64),
            flip_h: t.get("flipHorizontal").and_then(Value::as_bool),
            flip_v: t.get("flipVertical").and_then(Value::as_bool),
        });
        PropertyPatch {
            duration_frames: obj.get("durationFrames").and_then(Value::as_i64).map(|v| v as i32),
            trim_start_frame: obj.get("trimStartFrame").and_then(Value::as_i64).map(|v| v as i32),
            trim_end_frame: obj.get("trimEndFrame").and_then(Value::as_i64).map(|v| v as i32),
            speed: obj.get("speed").and_then(Value::as_f64),
            volume: obj.get("volume").and_then(Value::as_f64),
            opacity: obj.get("opacity").and_then(Value::as_f64),
            transform,
            content: obj.get("content").and_then(Value::as_str).map(str::to_string),
            font_name: obj.get("fontName").and_then(Value::as_str).map(str::to_string),
            font_size: obj.get("fontSize").and_then(Value::as_f64),
            color: obj.get("color").and_then(Value::as_str).and_then(|c| parse_rgba(c).ok()),
            alignment: obj.get("alignment").and_then(Value::as_str).map(str::to_string),
        }
    }
}

/// Apply the patch to one clip in place. Returns the list of changed-field labels.
/// Reference `applyPropertyChanges`.
fn apply_property_changes(timeline: &mut Timeline, clip_id: &str, p: &PropertyPatch, is_text: bool) -> Vec<String> {
    let mut changed = Vec::new();
    let Some(clip) = timeline.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()).find(|c| c.id == clip_id) else {
        return changed;
    };
    if let Some(v) = p.duration_frames {
        clip.set_duration(v);
        changed.push("durationFrames".to_string());
    }
    if let Some(v) = p.trim_start_frame {
        clip.trim_start_frame = v;
        changed.push("trimStartFrame".to_string());
    }
    if let Some(v) = p.trim_end_frame {
        clip.trim_end_frame = v;
        changed.push("trimEndFrame".to_string());
    }
    if let Some(v) = p.speed {
        // Recompute duration to preserve the consumed source span (reference: when
        // durationFrames not also set and speed > 0). Uses ties-away rounding.
        if p.duration_frames.is_none() && v > 0.0 {
            let source_consumed = clip.duration_frames as f64 * clip.speed;
            let new_dur = (round_ties_away(source_consumed / v)).max(1);
            clip.set_duration(new_dur);
            changed.push("durationFrames".to_string());
        }
        clip.speed = v;
        changed.push("speed".to_string());
    }
    // Setting a scalar clears that property's keyframe track.
    if let Some(v) = p.volume {
        clip.volume = v;
        clip.volume_track = None;
        changed.push("volume".to_string());
    }
    if let Some(v) = p.opacity {
        clip.opacity = v;
        clip.opacity_track = None;
        changed.push("opacity".to_string());
    }
    if let Some(t) = &p.transform {
        // Center-based partial merge over the current transform (ruling #7).
        if let Some(cx) = t.center_x { clip.transform.center_x = cx; }
        if let Some(cy) = t.center_y { clip.transform.center_y = cy; }
        if let Some(w) = t.width { clip.transform.width = w; }
        if let Some(h) = t.height { clip.transform.height = h; }
        if let Some(fh) = t.flip_h { clip.transform.flip_horizontal = fh; }
        if let Some(fv) = t.flip_v { clip.transform.flip_vertical = fv; }
        changed.push("transform".to_string());
    }
    if is_text {
        if let Some(c) = &p.content {
            clip.text_content = Some(c.clone());
            changed.push("content".to_string());
        }
        if p.font_name.is_some() || p.font_size.is_some() || p.color.is_some() || p.alignment.is_some() {
            let mut style = clip.text_style.clone().unwrap_or_default();
            if let Some(f) = &p.font_name {
                style.font_name = palmier_model::FontName::from_str(f.as_str());
                changed.push("fontName".to_string());
            }
            if let Some(s) = p.font_size {
                style.font_size = s;
                changed.push("fontSize".to_string());
            }
            if let Some((r, g, b, a)) = p.color {
                style.color = palmier_model::Rgba::new(
                    r as f64 / 255.0,
                    g as f64 / 255.0,
                    b as f64 / 255.0,
                    a as f64 / 255.0,
                );
                changed.push("color".to_string());
            }
            if let Some(al) = &p.alignment {
                style.alignment = match al.as_str() {
                    "left" => palmier_model::TextAlignment::Left,
                    "right" => palmier_model::TextAlignment::Right,
                    _ => palmier_model::TextAlignment::Center,
                };
                changed.push("alignment".to_string());
            }
            clip.text_style = Some(style);
        }
    }
    changed
}

// ─────────────────────────────────────────────────────────────────────────────
// set_keyframes
// ─────────────────────────────────────────────────────────────────────────────

/// The animatable property names accepted by `set_keyframes` (reference
/// `keyframePropertyNames`).
const KEYFRAME_PROPERTIES: [&str; 6] = ["volume", "opacity", "rotation", "position", "scale", "crop"];

/// `set_keyframes` (`clipId`, `property`, `keyframes[[frame, …values, interp?]]`):
/// REPLACES the property's track (empty array clears). Frames are CLIP-RELATIVE;
/// `interp` default smooth (ruling #8). Reference `setKeyframes`.
pub fn set_keyframes(state: &mut EditorState, args: &Value) -> ToolResult {
    let clip_id = match args.get("clipId").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return ToolResult::error("Missing required field 'clipId'"),
    };
    let property = match args.get("property").and_then(Value::as_str) {
        Some(s) => s.to_string(),
        None => return ToolResult::error("Missing required field 'property'"),
    };
    let rows = match args.get("keyframes").and_then(Value::as_array) {
        Some(r) => r.clone(),
        None => return ToolResult::error("Missing required field 'keyframes' (must be an array)"),
    };
    if !KEYFRAME_PROPERTIES.contains(&property.as_str()) {
        return ToolResult::error(format!(
            "Unknown property '{property}'. Expected one of: {}",
            KEYFRAME_PROPERTIES.join(", ")
        ));
    }
    let exists = state
        .library
        .timeline
        .tracks
        .iter()
        .any(|t| t.clips.iter().any(|c| c.id == clip_id));
    if !exists {
        return ToolResult::error(format!("Clip not found: {clip_id}"));
    }
    // Parse the rows up front so a malformed row errors before mutating.
    let parsed = match parse_keyframe_rows(&property, &rows) {
        Ok(p) => p,
        Err(msg) => return ToolResult::error(msg),
    };

    let row_count = rows.len();
    agent_edit(state, "Set Keyframes (Agent)", move |timeline, _hist| {
        let Some(clip) = timeline.tracks.iter_mut().flat_map(|t| t.clips.iter_mut()).find(|c| c.id == clip_id) else {
            return Err(format!("Clip not found: {clip_id}"));
        };
        parsed.apply(clip);
        let action = if row_count == 0 { "cleared".to_string() } else { format!("set {row_count}") };
        Ok(ToolResult::ok(format!("{action} keyframes on {property} for {clip_id}")))
    })
}

/// A parsed keyframe track ready to write onto a clip's matching property.
enum ParsedKeyframes {
    Scalar(KeyframeTrack<f64>, ScalarTarget),
    Pair(KeyframeTrack<AnimPair>, PairTarget),
    Crop(KeyframeTrack<Crop>),
}

enum ScalarTarget { Volume, Opacity, Rotation }
enum PairTarget { Position, Scale }

impl ParsedKeyframes {
    fn apply(self, clip: &mut palmier_model::Clip) {
        match self {
            ParsedKeyframes::Scalar(track, target) => {
                let value = if track.keyframes.is_empty() { None } else { Some(track) };
                match target {
                    ScalarTarget::Volume => clip.volume_track = value,
                    ScalarTarget::Opacity => clip.opacity_track = value,
                    ScalarTarget::Rotation => clip.rotation_track = value,
                }
            }
            ParsedKeyframes::Pair(track, target) => {
                let value = if track.keyframes.is_empty() { None } else { Some(track) };
                match target {
                    PairTarget::Position => clip.position_track = value,
                    PairTarget::Scale => clip.scale_track = value,
                }
            }
            ParsedKeyframes::Crop(track) => {
                clip.crop_track = if track.keyframes.is_empty() { None } else { Some(track) };
            }
        }
    }
}

/// Parse `[[frame, …values, interp?], …]` for `property` into a typed track.
fn parse_keyframe_rows(property: &str, rows: &[Value]) -> Result<ParsedKeyframes, String> {
    match property {
        "volume" => Ok(ParsedKeyframes::Scalar(parse_scalar_rows(rows)?, ScalarTarget::Volume)),
        "opacity" => Ok(ParsedKeyframes::Scalar(parse_scalar_rows(rows)?, ScalarTarget::Opacity)),
        "rotation" => Ok(ParsedKeyframes::Scalar(parse_scalar_rows(rows)?, ScalarTarget::Rotation)),
        "position" => Ok(ParsedKeyframes::Pair(parse_pair_rows(rows)?, PairTarget::Position)),
        "scale" => Ok(ParsedKeyframes::Pair(parse_pair_rows(rows)?, PairTarget::Scale)),
        "crop" => Ok(ParsedKeyframes::Crop(parse_crop_rows(rows)?)),
        _ => Err(format!("Unknown property '{property}'")),
    }
}

/// Pull `frame` (int) + `arity` float values + optional `interp` from a row.
fn row_parts(row: &Value, arity: usize, labels: &str, idx: usize) -> Result<(i32, Vec<f64>, Interpolation), String> {
    let arr = row.as_array().ok_or_else(|| format!("keyframes[{idx}]: expected array [frame, {labels}, interp?]"))?;
    let min_len = arity + 1;
    let max_len = arity + 2;
    if arr.len() != min_len && arr.len() != max_len {
        return Err(format!(
            "keyframes[{idx}]: expected [frame, {labels}] or [frame, {labels}, interp] (got {} elements)",
            arr.len()
        ));
    }
    let frame = arr[0].as_i64().ok_or_else(|| format!("keyframes[{idx}][0] (frame): expected integer"))? as i32;
    let mut values = Vec::with_capacity(arity);
    for k in 0..arity {
        let v = arr[k + 1].as_f64().ok_or_else(|| format!("keyframes[{idx}][{}]: expected number", k + 1))?;
        if !v.is_finite() {
            return Err(format!("keyframes[{idx}][{}]: value must be finite", k + 1));
        }
        values.push(v);
    }
    let interp = if arr.len() == max_len {
        match arr[min_len].as_str() {
            Some("linear") => Interpolation::Linear,
            Some("hold") => Interpolation::Hold,
            Some("smooth") => Interpolation::Smooth,
            other => return Err(format!(
                "keyframes[{idx}][{min_len}] (interp): expected one of 'linear', 'hold', 'smooth' (got {other:?})"
            )),
        }
    } else {
        // Default smooth (ruling #8).
        Interpolation::Smooth
    };
    Ok((frame, values, interp))
}

fn sort_dedupe<V>(mut kfs: Vec<Keyframe<V>>) -> Vec<Keyframe<V>> {
    kfs.sort_by_key(|k| k.frame);
    let mut out: Vec<Keyframe<V>> = Vec::with_capacity(kfs.len());
    for kf in kfs {
        if out.last().map(|l| l.frame) == Some(kf.frame) {
            *out.last_mut().unwrap() = kf;
        } else {
            out.push(kf);
        }
    }
    out
}

fn parse_scalar_rows(rows: &[Value]) -> Result<KeyframeTrack<f64>, String> {
    let mut kfs = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let (frame, vals, interp) = row_parts(row, 1, "value", i)?;
        kfs.push(Keyframe::with_interpolation(frame, vals[0], interp));
    }
    Ok(KeyframeTrack { keyframes: sort_dedupe(kfs) })
}

fn parse_pair_rows(rows: &[Value]) -> Result<KeyframeTrack<AnimPair>, String> {
    let mut kfs = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let (frame, vals, interp) = row_parts(row, 2, "a, b", i)?;
        kfs.push(Keyframe::with_interpolation(frame, AnimPair::new(vals[0], vals[1]), interp));
    }
    Ok(KeyframeTrack { keyframes: sort_dedupe(kfs) })
}

fn parse_crop_rows(rows: &[Value]) -> Result<KeyframeTrack<Crop>, String> {
    let mut kfs = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        // Row layout: top, right, bottom, left (reference parseCropKeyframes).
        let (frame, vals, interp) = row_parts(row, 4, "top, right, bottom, left", i)?;
        let crop = Crop { top: vals[0], right: vals[1], bottom: vals[2], left: vals[3] };
        kfs.push(Keyframe::with_interpolation(frame, crop, interp));
    }
    Ok(KeyframeTrack { keyframes: sort_dedupe(kfs) })
}

// ─────────────────────────────────────────────────────────────────────────────
// ripple_delete_ranges
// ─────────────────────────────────────────────────────────────────────────────

/// `ripple_delete_ranges` (`ranges[[start, end]]` + exactly one of `trackIndex`
/// (project frames, units 'frames') or `clipId` (clamped, units seconds|frames
/// default frames)): routes to palmier-edit's atomic `ripple_delete_ranges_on_track`.
/// Overlaps merge; linked partners cut on the same span; sync-locked tracks shift.
/// Reference `rippleDeleteRanges`.
///
/// **E10-S8 / FR-38 — the transcript-driven cut path (UJ-1 climax).** This is the
/// agent-facing edit `get_transcript` (E10-S7) hands off to: the agent reads a
/// clip's dead-air/filler words (already in project frames) and passes the word's
/// `clipId` + frames straight here, or a source-seconds range (e.g. from
/// `inspect_media`) with `units:"seconds"`. The `clipId` path below performs the
/// source-seconds → project-frame conversion through the clip's placement + trim +
/// speed — the same `Clip::timeline_frame` math as `span_frames`/E10-S6
/// (`f64::round` ties-away, speed floor `0.0001`, half-open `[start, end)`) — then
/// the merged Epic 3 engine cuts and closes the gap in **one atomic, undoable**
/// [`agent_edit`] step. No new editing engine: glue over get_transcript + ripple.
pub fn ripple_delete_ranges(state: &mut EditorState, args: &Value) -> ToolResult {
    let obj = args.as_object().expect("validated object");
    let ranges = match obj.get("ranges").and_then(Value::as_array) {
        Some(r) if !r.is_empty() => r,
        _ => return ToolResult::error("Missing or empty 'ranges' array"),
    };
    let units = obj.get("units").and_then(Value::as_str).unwrap_or("frames");
    let has_clip = obj.contains_key("clipId");
    let has_track = obj.contains_key("trackIndex");
    // (validation already enforced exactly-one + units rules in E7-S3, but re-guard.)
    if has_clip == has_track {
        return ToolResult::error(
            "Provide exactly one of 'clipId' or 'trackIndex'.".to_string(),
        );
    }

    // Validate each [start, end] pair shape.
    for (i, r) in ranges.iter().enumerate() {
        let arr = r.as_array();
        match arr {
            Some(a) if a.len() == 2 => {
                let s = a[0].as_f64().unwrap_or(f64::NAN);
                let e = a[1].as_f64().unwrap_or(f64::NAN);
                // Reject unless strictly increasing (also rejects NaN, which fails
                // every comparison — `e > s` is false for NaN).
                if !matches!(e.partial_cmp(&s), Some(std::cmp::Ordering::Greater)) {
                    return ToolResult::error(format!("ranges[{i}]: end ({e}) must be greater than start ({s})"));
                }
            }
            Some(a) => return ToolResult::error(format!("ranges[{i}]: expected [start, end] (got {} elements)", a.len())),
            None => return ToolResult::error(format!("ranges[{i}]: expected [start, end]")),
        }
    }

    let fps = state.library.timeline.fps;

    // Resolve the anchor track + the frame ranges (project frames).
    let (track_index, frame_ranges, dropped) = if has_clip {
        let clip_id = obj.get("clipId").and_then(Value::as_str).unwrap();
        let loc = state.library.timeline.tracks.iter().enumerate().find_map(|(ti, t)| {
            t.clips.iter().find(|c| c.id == clip_id).map(|c| (ti, c.clone()))
        });
        let Some((ti, clip)) = loc else {
            return ToolResult::error(format!("Clip not found: {clip_id}"));
        };
        let mut frs = Vec::new();
        let mut dropped = 0;
        for r in ranges {
            let a = r.as_array().unwrap();
            let to_frame = |v: f64| -> f64 {
                if units == "frames" {
                    v
                } else {
                    // source seconds → project frame through trim/speed/position.
                    clip.start_frame as f64
                        + (v * fps as f64 - clip.trim_start_frame as f64) / clip.speed.max(0.0001)
                }
            };
            let s = round_ties_away(to_frame(a[0].as_f64().unwrap()))
                .clamp(clip.start_frame, clip.end_frame());
            let e = round_ties_away(to_frame(a[1].as_f64().unwrap()))
                .clamp(clip.start_frame, clip.end_frame());
            if e > s {
                frs.push(FrameRange::new(s, e));
            } else {
                dropped += 1;
            }
        }
        if frs.is_empty() {
            return ToolResult::error(format!(
                "No ranges fall within clip {clip_id} (frames {}..{}). In '{units}' units, ranges must overlap the clip's visible span.",
                clip.start_frame, clip.end_frame()
            ));
        }
        (ti, frs, dropped)
    } else {
        let ti = obj.get("trackIndex").and_then(Value::as_i64).unwrap() as usize;
        if state.library.timeline.tracks.get(ti).is_none() {
            return ToolResult::error(format!("Track index out of range: {ti}"));
        }
        let mut frs = Vec::new();
        let mut dropped = 0;
        for r in ranges {
            let a = r.as_array().unwrap();
            let s = round_ties_away(a[0].as_f64().unwrap()).max(0);
            let e = round_ties_away(a[1].as_f64().unwrap());
            if e > s {
                frs.push(FrameRange::new(s, e));
            } else {
                dropped += 1;
            }
        }
        if frs.is_empty() {
            return ToolResult::error(format!("No valid project-frame ranges to delete on track {ti}."));
        }
        (ti, frs, dropped)
    };

    agent_edit(state, "Ripple Delete Range (Agent)", move |timeline, hist| {
        let mut mint = palmier_model::Track::new(ClipType::Video).id;
        let mut gen_fn = || {
            let id = mint.clone();
            // Re-mint each call so right-fragment ids are unique.
            mint = palmier_model::Track::new(ClipType::Video).id;
            id
        };
        match ripple_delete_ranges_on_track(timeline, hist, track_index, &frame_ranges, &mut gen_fn) {
            Ok(report) => {
                let mut payload = json!({
                    "removedFrames": report.removed_frames,
                    "clearedTracks": report.cleared_tracks,
                    "shiftedClips": report.shifted_clips,
                    "anchorTrackIndex": track_index,
                });
                if !report.removed_clip_ids.is_empty() {
                    payload["removedClipIds"] = json!(report.removed_clip_ids);
                }
                if dropped > 0 {
                    payload["rangesIgnored"] = json!(dropped);
                }
                Ok(ToolResult::ok(serde_json::to_string(&payload).unwrap_or_default()))
            }
            Err(reason) => Err(format!("{reason:?}")),
        }
    })
}
