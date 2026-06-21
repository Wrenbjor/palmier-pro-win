//! READ tool bodies — `get_timeline`, `get_media`, `get_transcript`,
//! `list_folders`, `list_models` (E7-S5; reference `ToolExecutor+Timeline.swift`
//! + `ToolExecutor+Folders.swift`).
//!
//! All five are **non-mutating** and **non-async**: they read [`EditorState`] and
//! return a text [`ToolResult`]. None touch the agent undo stack. Output strings
//! are passed through ShortId shortening by the executor on the way out (E7-S4).
//!
//! ## Parity authority
//! - `get_timeline` shaping (default-omission, captionGroup collapse, the 200-row
//!   cap, keyframe folding) — `ToolExecutor+Timeline.swift` `getTimeline` /
//!   `compactTrack` / `compactClip` / `captionGroup` / `strippingDefaults`.
//! - `get_transcript` — `ToolExecutor+Timeline.swift` `getTranscript` /
//!   `spanFrames` (source-seconds → timeline-frame, ties-away rounding). The real
//!   word data depends on Epic 10's transcription store; in M2 there is no store,
//!   so every clip yields zero words (the reference returns empty → the agent
//!   tells the user to transcribe; UJ-1 edge case).
//! - `get_media` — the JSON of the media manifest entries (asset list).
//! - `list_folders` / `list_models` — straightforward projections.
//!
//! Numbers in `get_timeline` / `get_media` / `list_models` are rounded to 3 places
//! ([`round_json_numbers`]); `get_transcript` is NOT rounded (integer frames).

use serde_json::{json, Map, Value};

use palmier_model::{Clip, GenerationStatus, MediaAsset, Timeline, Track};

use crate::caption_transcribe::{
    acquire_cache, asset_path, build_runtime, resolve_cache_language, WHISPER_MODEL_ID,
};
use crate::editor::EditorState;
use crate::json_round::{round_json_numbers, JSON_ROUND_PLACES};
use crate::result::ToolResult;
use crate::transcript::span_frames;

/// Per-group caption-row cap (reference `captionRowLimit = 200`).
pub const CAPTION_ROW_LIMIT: usize = 200;

/// The caption row column format (reference `captionRowFormat`).
const CAPTION_ROW_FORMAT: [&str; 4] = ["clipId", "startFrame", "durationFrames", "text"];

// ─────────────────────────────────────────────────────────────────────────────
// get_timeline
// ─────────────────────────────────────────────────────────────────────────────

/// `get_timeline` (`startFrame?`, `endFrame?`). Returns the timeline encoded with
/// the reference's exact shaping: default-valued clip/track fields omitted, caption
/// clips collapsed into per-track `captionGroups` (200-row cap, paged by the
/// window), plus `totalFrames` / `canGenerate` / `currentFrame` injected.
///
/// Reference `ToolExecutor+Timeline.swift:getTimeline`.
pub fn get_timeline(state: &EditorState, args: &Value) -> ToolResult {
    let timeline = state.timeline();

    // Parse the optional [startFrame, endFrame) window. A window is present if
    // either bound is set; missing start defaults to 0, missing end to "no upper".
    let window = match parse_window(args) {
        Ok(w) => w,
        Err(msg) => return ToolResult::error(msg),
    };

    // Base = the timeline's serde JSON (fps/width/height come straight from the
    // Timeline Codable), then mutated.
    let Ok(Value::Object(mut dict)) = serde_json::to_value(timeline).map(Some).map(|o| o.unwrap())
    else {
        return ToolResult::error("Failed to encode timeline");
    };

    // Replace `tracks` with the compacted, labelled tracks.
    let tracks_out: Vec<Value> = timeline
        .tracks
        .iter()
        .enumerate()
        .map(|(i, track)| compact_track(track, window, &track_display_label(timeline, i)))
        .collect();
    dict.insert("tracks".to_string(), Value::Array(tracks_out));

    // Injected computed fields.
    let total_frames = timeline.total_frames();
    dict.insert("totalFrames".to_string(), json!(total_frames));
    if let Some((s, e)) = window {
        let upper = e.min(total_frames);
        dict.insert("window".to_string(), json!([s, upper]));
    }
    dict.insert("canGenerate".to_string(), json!(state.can_generate));

    let rounded = round_json_numbers(Value::Object(dict), JSON_ROUND_PLACES);
    match serde_json::to_string(&rounded) {
        Ok(s) => ToolResult::ok(s),
        Err(_) => ToolResult::error("Failed to serialize timeline"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// full_timeline_json — UI-only full-fidelity serializer (NOT an MCP tool)
// ─────────────────────────────────────────────────────────────────────────────

/// Full-fidelity timeline JSON for the **editor UI** (`editor_get_timeline`),
/// NOT the compact MCP `get_timeline` summary.
///
/// Unlike [`get_timeline`] — which strips default-valued fields, collapses caption
/// clips into `captionGroups`, folds keyframe tracks into a terse `[frame, …vals]`
/// table, and windows the result for an LLM — this emits **every** field the
/// frontend `ClipView` / `TrackView` (`src-ui/src/editor/types.ts`) need, so the
/// canvas renders accurate data (real volume / trim / speed / opacity / fades /
/// keyframes) rather than reconstructing defaults.
///
/// The `Clip` / `Track` / `Timeline` serde encodings already carry the exact wire
/// keys and shapes the view types expect:
/// - clip: `id`, `mediaRef`, `mediaType`, `sourceClipType`, `startFrame`,
///   `durationFrames`, `trimStartFrame`, `trimEndFrame`, `speed`, `volume`,
///   `opacity`, `fadeInFrames` / `fadeOutFrames`, `fadeInInterpolation` /
///   `fadeOutInterpolation`, `linkGroupId`, `textContent`, plus the six keyframe
///   tracks (`volumeTrack` / `opacityTrack` / `positionTrack` / `scaleTrack` /
///   `cropTrack` / `rotationTrack`) as `{ keyframes: [{ frame, value,
///   interpolationOut }] }` — matching TS `KeyframeTrackView` / `KeyframeView`.
/// - track: `id`, `type`, `muted`, `hidden`, `syncLocked`, `clips`.
///
/// Two fixups over the raw serde value:
/// 1. **`displayHeight`** is injected per track — the model marks it `#[serde(skip)]`
///    (display-only, reset to 50 on open), so the wire omits it; the UI needs it.
/// 2. **`totalFrames`** / **`canGenerate`** are injected at the root (harmless
///    extras the adapter ignores; kept for parity with the compact summary).
///
/// Keyframe values stay verbatim: scalars (`volume` / `opacity` / `rotation` in dB
/// or degrees), `AnimPair` `{ a, b }` (position / scale), and `Crop`
/// `{ left, top, right, bottom }` — the adapter passes them through.
///
/// Numbers are rounded to [`JSON_ROUND_PLACES`] like the other read projections.
pub fn full_timeline_json(state: &EditorState) -> Value {
    let timeline = state.timeline();

    let Ok(Value::Object(mut dict)) = serde_json::to_value(timeline) else {
        // Encoding a Timeline never fails in practice; degrade to an empty shape
        // the adapter tolerates rather than panicking.
        return json!({ "fps": timeline.fps, "width": timeline.width, "height": timeline.height, "tracks": [] });
    };

    // Rebuild `tracks` so each track carries its (skipped) displayHeight.
    let tracks_out: Vec<Value> = timeline
        .tracks
        .iter()
        .map(|track| {
            let mut t = match serde_json::to_value(track) {
                Ok(Value::Object(m)) => m,
                _ => Map::new(),
            };
            // display_height is `#[serde(skip)]` on the model — inject it so the
            // lane renders at the right height.
            t.insert("displayHeight".to_string(), json!(track.display_height));
            Value::Object(t)
        })
        .collect();
    dict.insert("tracks".to_string(), Value::Array(tracks_out));

    // Computed extras (parity with the compact summary; the adapter ignores them).
    dict.insert("totalFrames".to_string(), json!(timeline.total_frames()));
    dict.insert("canGenerate".to_string(), json!(state.can_generate));

    round_json_numbers(Value::Object(dict), JSON_ROUND_PLACES)
}

/// Parse the `[startFrame, endFrame)` window. `None` when neither bound is set.
/// Errors (matching the reference) if `start >= end`.
fn parse_window(args: &Value) -> Result<Option<(i32, i32)>, String> {
    let start = args.get("startFrame").and_then(Value::as_i64);
    let end = args.get("endFrame").and_then(Value::as_i64);
    if start.is_none() && end.is_none() {
        return Ok(None);
    }
    let s = start.unwrap_or(0) as i32;
    let e = end.map(|v| v as i32).unwrap_or(i32::MAX);
    if s >= e {
        return Err(format!(
            "Invalid window [{s}, {e}): startFrame must be less than endFrame"
        ));
    }
    Ok(Some((s, e)))
}

/// A track's displayed label: `V1`/`A1`/`I1`-style (reference
/// `timelineTrackDisplayLabel(at:)`). The reference numbers audio top-down and
/// visual with mirrored numbering; here we produce a stable per-type prefix + a
/// 1-based ordinal among same-type tracks, which reads the same for the common
/// single-zone layouts. (Exact mirrored numbering is a UI nicety; the agent uses
/// `trackIndex`, not the label, for addressing.)
fn track_display_label(timeline: &Timeline, index: usize) -> String {
    use palmier_model::ClipType::*;
    let track = &timeline.tracks[index];
    let prefix = match track.track_type {
        Video => "V",
        Audio => "A",
        Image => "I",
        Text => "T",
        Lottie => "L",
    };
    let ordinal = timeline.tracks[..=index]
        .iter()
        .filter(|t| t.track_type == track.track_type)
        .count();
    format!("{prefix}{ordinal}")
}

/// Whether a clip at `[start, start+duration)` intersects the window
/// (reference `clipIntersects`).
fn clip_intersects(start: i32, duration: i32, window: Option<(i32, i32)>) -> bool {
    match window {
        None => true,
        Some((lo, hi)) => start < hi && start + duration > lo,
    }
}

/// Compact one track (reference `compactTrack`): strip track defaults, compact each
/// clip, collapse caption clips into `captionGroups`, window-filter loose clips,
/// add `totalClips` when the window hides some, set the display `label`.
fn compact_track(track: &Track, window: Option<(i32, i32)>, label: &str) -> Value {
    // The track's serde JSON, then strip defaults.
    let Ok(Value::Object(track_json)) = serde_json::to_value(track) else {
        return json!({});
    };
    let mut out = strip_track_defaults(track_json);
    out.insert("label".to_string(), json!(label));

    // Compact every clip first.
    let compacted: Vec<Map<String, Value>> = track
        .clips
        .iter()
        .map(compact_clip)
        .collect();

    // Partition into loose (no captionGroupId) and grouped (by captionGroupId,
    // first-seen order).
    let mut loose: Vec<Map<String, Value>> = Vec::new();
    let mut group_order: Vec<String> = Vec::new();
    let mut grouped: std::collections::HashMap<String, Vec<Map<String, Value>>> =
        std::collections::HashMap::new();
    for clip in compacted {
        match clip.get("captionGroupId").and_then(Value::as_str) {
            Some(gid) => {
                let gid = gid.to_string();
                if !grouped.contains_key(&gid) {
                    group_order.push(gid.clone());
                }
                grouped.entry(gid).or_default().push(clip);
            }
            None => loose.push(clip),
        }
    }

    // Build caption groups; deviant clips fall back into loose.
    let mut groups: Vec<Value> = Vec::new();
    for gid in &group_order {
        let members = grouped.remove(gid).unwrap_or_default();
        let (group, deviants) = caption_group(gid, members, window);
        groups.push(group);
        loose.extend(deviants);
    }

    // Sort loose clips by startFrame; window-filter.
    loose.sort_by_key(|c| c.get("startFrame").and_then(Value::as_i64).unwrap_or(0));
    let loose_count = loose.len();
    let visible: Vec<Value> = loose
        .into_iter()
        .filter(|c| {
            let start = c.get("startFrame").and_then(Value::as_i64).unwrap_or(0) as i32;
            let dur = c.get("durationFrames").and_then(Value::as_i64).unwrap_or(0) as i32;
            clip_intersects(start, dur, window)
        })
        .map(Value::Object)
        .collect();

    let visible_count = visible.len();
    out.insert("clips".to_string(), Value::Array(visible));
    if visible_count < loose_count {
        out.insert("totalClips".to_string(), json!(loose_count));
    }
    if !groups.is_empty() {
        out.insert("captionGroups".to_string(), Value::Array(groups));
    }

    Value::Object(out)
}

/// Strip track fields equal to their defaults (reference `trackDefaults`:
/// `muted=false`, `hidden=false`, `syncLocked=true`).
fn strip_track_defaults(mut track: Map<String, Value>) -> Map<String, Value> {
    if track.get("muted") == Some(&json!(false)) {
        track.remove("muted");
    }
    if track.get("hidden") == Some(&json!(false)) {
        track.remove("hidden");
    }
    if track.get("syncLocked") == Some(&json!(true)) {
        track.remove("syncLocked");
    }
    track
}

/// Compact one clip (reference `compactClip`): fold keyframe tracks, drop
/// `sourceClipType` when it equals `mediaType`, drop trims for text clips, strip
/// default-valued fields.
fn compact_clip(clip: &Clip) -> Map<String, Value> {
    let Ok(Value::Object(mut obj)) = serde_json::to_value(clip) else {
        return Map::new();
    };

    // Fold the six keyframe tracks into a compact `keyframes` map.
    fold_keyframes(&mut obj);

    // sourceClipType == mediaType → drop (both are strings on the wire).
    let media_type = obj.get("mediaType").cloned();
    if obj.get("sourceClipType") == media_type.as_ref() {
        obj.remove("sourceClipType");
    }

    // text clips never report trims.
    if obj.get("mediaType").and_then(Value::as_str) == Some("text") {
        obj.remove("trimStartFrame");
        obj.remove("trimEndFrame");
    }

    strip_clip_defaults(obj)
}

/// Fold the raw keyframe-track fields (`opacityTrack`/`positionTrack`/… ) into a
/// compact `keyframes` map keyed by property, with per-row `[frame, …values,
/// interp?]` (interp appended only when not `smooth`). Reference
/// `compactClipKeyframes` + `KeyframeValueShape.values`.
fn fold_keyframes(obj: &mut Map<String, Value>) {
    // (trackKey, propKey, value-shape) — order mirrors the reference table.
    const SCALAR: u8 = 0;
    const PAIR: u8 = 1;
    const CROP: u8 = 2;
    let specs: [(&str, &str, u8); 6] = [
        ("volumeTrack", "volume", SCALAR),
        ("opacityTrack", "opacity", SCALAR),
        ("rotationTrack", "rotation", SCALAR),
        ("positionTrack", "position", PAIR),
        ("scaleTrack", "scale", PAIR),
        ("cropTrack", "crop", CROP),
    ];

    let mut keyframes = Map::new();
    for (track_key, prop_key, shape) in specs {
        let track_val = obj.remove(track_key);
        let Some(track_val) = track_val else { continue };
        // The KeyframeTrack serde shape carries a `keyframes` array.
        let Some(rows) = track_val.get("keyframes").and_then(Value::as_array) else {
            continue;
        };
        if rows.is_empty() {
            continue;
        }
        let mut out_rows: Vec<Value> = Vec::with_capacity(rows.len());
        for kf in rows {
            let frame = kf.get("frame").cloned().unwrap_or(json!(0));
            let mut row: Vec<Value> = vec![frame];
            let value = kf.get("value");
            match shape {
                SCALAR => row.push(value.cloned().unwrap_or(json!(0))),
                PAIR => {
                    let a = value.and_then(|v| v.get("a")).cloned().unwrap_or(json!(0));
                    let b = value.and_then(|v| v.get("b")).cloned().unwrap_or(json!(0));
                    row.push(a);
                    row.push(b);
                }
                CROP => {
                    for side in ["top", "right", "bottom", "left"] {
                        row.push(value.and_then(|v| v.get(side)).cloned().unwrap_or(json!(0)));
                    }
                }
                _ => {}
            }
            // Append the interpolation only when it isn't the smooth default.
            // The keyframe serde shape carries `interpolationOut` (ruling #8 —
            // default smooth; absent → smooth).
            let interp = kf.get("interpolationOut").and_then(Value::as_str);
            if let Some(i) = interp {
                if i != "smooth" {
                    row.push(json!(i));
                }
            }
            out_rows.push(Value::Array(row));
        }
        keyframes.insert(prop_key.to_string(), Value::Array(out_rows));
    }

    if !keyframes.is_empty() {
        obj.insert("keyframes".to_string(), Value::Object(keyframes));
    }
}

/// Strip clip fields equal to a default `Clip`'s encoding (reference
/// `clipDefaults`), keeping `id/mediaRef/startFrame/durationFrames/sourceClipType`
/// always. Built by encoding a baseline clip and removing matching values.
fn strip_clip_defaults(mut clip: Map<String, Value>) -> Map<String, Value> {
    // The reference computes its default map from `Clip(mediaRef:"", startFrame:0,
    // durationFrames:0)` with a default TextStyle, then removes the always-keep
    // keys from that map so they never strip. We do the same: encode a baseline.
    let baseline = Clip::new("", 0, 0);
    let Ok(Value::Object(mut defaults)) = serde_json::to_value(&baseline) else {
        return clip;
    };
    // Fold the baseline's keyframes the same way so the comparison is apples to
    // apples (a default clip has none, so this is a no-op, but keeps parity).
    fold_keyframes(&mut defaults);

    // Always-keep keys are never strip candidates.
    for keep in ["id", "mediaRef", "startFrame", "durationFrames", "sourceClipType"] {
        defaults.remove(keep);
    }

    strip_matching_defaults(&mut clip, &defaults);
    clip
}

/// Recursively remove keys from `obj` whose value equals the same key in
/// `defaults` (reference `strippingDefaults`): nested objects recurse and are
/// dropped if they strip empty; scalars drop on equality. Keys absent from
/// `defaults` always survive.
fn strip_matching_defaults(obj: &mut Map<String, Value>, defaults: &Map<String, Value>) {
    let keys: Vec<String> = defaults.keys().cloned().collect();
    for key in keys {
        let Some(def) = defaults.get(&key) else { continue };
        let Some(val) = obj.get(&key) else { continue };
        match (val, def) {
            (Value::Object(_), Value::Object(def_map)) => {
                // Recurse into the nested object.
                if let Some(Value::Object(mut nested)) = obj.remove(&key) {
                    strip_matching_defaults(&mut nested, def_map);
                    if !nested.is_empty() {
                        obj.insert(key, Value::Object(nested));
                    }
                    // else: dropped (stripped empty).
                }
            }
            (v, d) if v == d => {
                obj.remove(&key);
            }
            _ => {}
        }
    }
}

/// Collapse the caption clips of one group (reference `captionGroup`): the modal
/// residual (shared props) is hoisted into `shared`; each modal-matching member
/// becomes a `[clipId, startFrame, durationFrames, text]` row (window-filtered,
/// sorted, capped at 200); members whose props deviate are returned as deviants to
/// rejoin `loose`. Returns `(group_value, deviant_clips)`.
fn caption_group(
    gid: &str,
    members: Vec<Map<String, Value>>,
    window: Option<(i32, i32)>,
) -> (Value, Vec<Map<String, Value>>) {
    // Row-carried keys never participate in the shared/residual comparison.
    let row_keys = ["id", "startFrame", "durationFrames", "textContent", "captionGroupId"];

    // residual(clip) = clip minus row keys, with transform.width/height removed.
    let residual_of = |clip: &Map<String, Value>| -> Map<String, Value> {
        let mut r: Map<String, Value> = clip
            .iter()
            .filter(|(k, _)| !row_keys.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let Some(Value::Object(mut t)) = r.remove("transform") {
            t.remove("width");
            t.remove("height");
            if !t.is_empty() {
                r.insert("transform".to_string(), Value::Object(t));
            }
        }
        r
    };

    // Tally residuals by canonical JSON; track the modal one.
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut residual_by_key: std::collections::HashMap<String, Map<String, Value>> =
        std::collections::HashMap::new();
    let mut modal_key: Option<String> = None;
    let mut modal_count = 0usize;
    let member_residuals: Vec<(Map<String, Value>, String)> = members
        .iter()
        .map(|m| {
            let res = residual_of(m);
            let key = canonical_json(&res);
            (res, key)
        })
        .collect();
    for (res, key) in &member_residuals {
        let c = counts.entry(key.clone()).or_insert(0);
        *c += 1;
        residual_by_key.entry(key.clone()).or_insert_with(|| res.clone());
        if *c > modal_count {
            modal_count = *c;
            modal_key = Some(key.clone());
        }
    }
    let modal_key = modal_key.unwrap_or_default();
    let shared = residual_by_key.get(&modal_key).cloned().unwrap_or_default();

    // Build rows for modal members; deviants go back to loose.
    let mut rows: Vec<(i32, i32, Value)> = Vec::new(); // (start, dur, row)
    let mut deviants: Vec<Map<String, Value>> = Vec::new();
    let mut frame_min = i32::MAX;
    let mut frame_max = i32::MIN;
    for (member, (_, key)) in members.into_iter().zip(member_residuals.into_iter()) {
        let start = member.get("startFrame").and_then(Value::as_i64).unwrap_or(0) as i32;
        let dur = member.get("durationFrames").and_then(Value::as_i64).unwrap_or(0) as i32;
        frame_min = frame_min.min(start);
        frame_max = frame_max.max(start + dur);
        if key == modal_key {
            let clip_id = member.get("id").cloned().unwrap_or(json!(""));
            let text = member.get("textContent").cloned().unwrap_or(json!(""));
            let row = json!([clip_id, start, dur, text]);
            rows.push((start, dur, row));
        } else {
            deviants.push(member);
        }
    }

    let total = rows.len();
    // Window-filter rows on [start, start+dur) intersection.
    rows.retain(|(start, dur, _)| clip_intersects(*start, *dur, window));
    rows.sort_by_key(|(start, _, _)| *start);
    let shown: Vec<Value> = rows.into_iter().take(CAPTION_ROW_LIMIT).map(|(_, _, r)| r).collect();
    let shown_count = shown.len();

    if frame_min == i32::MAX {
        frame_min = 0;
    }
    if frame_max == i32::MIN {
        frame_max = 0;
    }

    let mut group = Map::new();
    group.insert("captionGroupId".to_string(), json!(gid));
    group.insert("clipCount".to_string(), json!(total));
    group.insert("frameRange".to_string(), json!([frame_min, frame_max]));
    group.insert("clipFormat".to_string(), json!(CAPTION_ROW_FORMAT));
    group.insert("clips".to_string(), Value::Array(shown));
    if !shared.is_empty() {
        group.insert("shared".to_string(), Value::Object(shared));
    }
    if shown_count < total {
        group.insert(
            "clipsNote".to_string(),
            json!(format!(
                "Showing {shown_count} of {total} caption clips. Page with startFrame/endFrame."
            )),
        );
    }

    (Value::Object(group), deviants)
}

/// Canonical (sorted-keys) JSON string for an object — the residual-equality key
/// (reference `canonicalJSON` via `.sortedKeys`).
fn canonical_json(map: &Map<String, Value>) -> String {
    // serde_json sorts object keys when the `preserve_order` feature is off, which
    // is the default — but to be robust against build features, build a BTreeMap.
    let sorted: std::collections::BTreeMap<&String, &Value> = map.iter().collect();
    serde_json::to_string(&sorted).unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────
// get_media
// ─────────────────────────────────────────────────────────────────────────────

/// `get_media`: the asset library — `assets[{id, name, type, duration,
/// generationStatus, folderId}]` (reference `getMedia`, the encoded manifest /
/// asset catalog). Numbers rounded to 3 places.
pub fn get_media(state: &EditorState) -> ToolResult {
    let assets: Vec<Value> = state.library.assets.iter().map(asset_json).collect();
    let body = json!({ "assets": assets });
    let rounded = round_json_numbers(body, JSON_ROUND_PLACES);
    match serde_json::to_string(&rounded) {
        Ok(s) => ToolResult::ok(s),
        Err(_) => ToolResult::error("Failed to serialize media"),
    }
}

/// One asset's get_media projection.
fn asset_json(asset: &MediaAsset) -> Value {
    let mut obj = Map::new();
    obj.insert("id".to_string(), json!(asset.id));
    obj.insert("name".to_string(), json!(asset.name));
    obj.insert("type".to_string(), serde_json::to_value(asset.asset_type).unwrap_or(json!("video")));
    obj.insert("duration".to_string(), json!(asset.duration_seconds));
    obj.insert("generationStatus".to_string(), json!(generation_status_str(&asset.generation_status)));
    if let Some(folder) = &asset.folder_id {
        obj.insert("folderId".to_string(), json!(folder));
    }
    Value::Object(obj)
}

/// Map [`GenerationStatus`] to the get_media wire string
/// (`generating | downloading | failed | none`; `rendering` collapses to
/// `generating` for the manifest surface, `failed` carries no message here).
fn generation_status_str(status: &GenerationStatus) -> &'static str {
    match status {
        GenerationStatus::None => "none",
        GenerationStatus::Generating => "generating",
        GenerationStatus::Downloading => "downloading",
        GenerationStatus::Rendering => "generating",
        GenerationStatus::Failed(_) => "failed",
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_folders
// ─────────────────────────────────────────────────────────────────────────────

/// `list_folders`: `folders[{id, name, parentFolderId?}]` (reference
/// `listFolders`). `parentFolderId` omitted at top level. No rounding (no floats).
pub fn list_folders(state: &EditorState) -> ToolResult {
    let folders: Vec<Value> = state
        .library
        .manifest
        .folders
        .iter()
        .map(|f| {
            let mut obj = Map::new();
            obj.insert("id".to_string(), json!(f.id));
            obj.insert("name".to_string(), json!(f.name));
            if let Some(parent) = &f.parent_id {
                obj.insert("parentFolderId".to_string(), json!(parent));
            }
            Value::Object(obj)
        })
        .collect();
    let body = json!({ "folders": folders });
    match serde_json::to_string(&body) {
        Ok(s) => ToolResult::ok(s),
        Err(_) => ToolResult::error("Failed to serialize folders"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// list_models
// ─────────────────────────────────────────────────────────────────────────────

/// `list_models` (`type?`): `{ models, loaded }`. `loaded=false` ⇒ the catalog
/// hasn't synced (not signed in). The real catalog is Epic 9 (M3); in M2 the
/// catalog is empty and `loaded=false` (reference `ModelCatalog.isLoaded`).
pub fn list_models(_state: &EditorState, _args: &Value) -> ToolResult {
    let body = json!({ "models": [], "loaded": false });
    match serde_json::to_string(&body) {
        Ok(s) => ToolResult::ok(s),
        Err(_) => ToolResult::error("Failed to serialize models"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// get_transcript
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum words returned by `get_transcript` (reference `inspectMaxWords`).
pub const TRANSCRIPT_WORD_CAP: usize = 10000;

/// The transcript **segment** cap (400) — the OTHER cap class, applied by
/// `inspect_media`'s per-source transcript-segment output (mcp-tools.md). `get_transcript`
/// emits per-clip **words**, not segments, so it is bounded by [`TRANSCRIPT_WORD_CAP`];
/// this constant documents the distinct 400-segment ceiling (it must NOT be conflated
/// with the `maxFrames ≤ 12` image-frame cap, a third class).
pub const TRANSCRIPT_SEGMENT_CAP: usize = 400;

/// `get_transcript` (`startFrame?`, `endFrame?`, `clipId?`): the spoken transcript of
/// the CURRENT timeline in project frames — clips in timeline order, words `[text,
/// startFrame, endFrame]` capped at 10000 ([`TRANSCRIPT_WORD_CAP`]), paged via
/// `nextStartFrame`. Reference `ToolExecutor+Timeline.swift:getTranscript`.
///
/// Each audio/video clip's words come from the on-device transcript **cache**
/// (E10-S4): a word is attributed to the clip whose visible source window contains its
/// **midpoint**, then mapped to project frames via [`span_frames`]. The cache is **read
/// only** here — `get_transcript` NEVER transcribes. When nothing is cached, every clip
/// yields zero words and the result is empty: the contract description instructs the
/// agent to transcribe first (UJ-1 — the agent must not guess cut points).
pub fn get_transcript(state: &EditorState, args: &Value) -> ToolResult {
    let timeline = state.timeline();
    let fps = timeline.fps;

    // Window validation: if both bounds set, start < end (reference).
    let window_start = args.get("startFrame").and_then(Value::as_i64).map(|v| v as i32);
    let window_end = args.get("endFrame").and_then(Value::as_i64).map(|v| v as i32);
    if let (Some(s), Some(e)) = (window_start, window_end) {
        if s >= e {
            return ToolResult::error(format!(
                "startFrame ({s}) must be less than endFrame ({e})"
            ));
        }
    }
    let clip_filter = args.get("clipId").and_then(Value::as_str);

    // Walk every audio/video clip, sorted by start_frame (reference `frags`). Snapshot
    // each clip so the borrow ends before we read the cache.
    let mut frags: Vec<(usize, Clip)> = Vec::new();
    for (track_index, track) in timeline.tracks.iter().enumerate() {
        for clip in &track.clips {
            use palmier_model::ClipType::*;
            if !matches!(clip.media_type, Video | Audio) {
                continue;
            }
            if let Some(filter) = clip_filter {
                if clip.id != filter {
                    continue;
                }
            }
            frags.push((track_index, clip.clone()));
        }
    }
    frags.sort_by_key(|(_, c)| c.start_frame);

    if clip_filter.is_some() && frags.is_empty() {
        return ToolResult::error(format!(
            "Clip {} not found, or it has no audio/video to transcribe.",
            clip_filter.unwrap()
        ));
    }

    // Transcribe each UNIQUE source once via the cache (read-only; never transcribes).
    // Per-asset cache/IO errors are skipped (reference records them in `skipped`),
    // never failing the whole call.
    let cache_language = resolve_cache_language(None);
    let runtime = build_runtime();
    let mut transcripts: std::collections::HashMap<String, palmier_transcribe::TranscriptionResult> =
        std::collections::HashMap::new();
    let mut skipped: Vec<Value> = Vec::new();
    if let Ok(runtime) = &runtime {
        let cache = runtime.block_on(acquire_cache());
        let mut seen: Vec<String> = Vec::new();
        for (_, clip) in &frags {
            if seen.contains(&clip.media_ref) {
                continue;
            }
            seen.push(clip.media_ref.clone());
            let Some(file) = asset_path(&state.library, &clip.media_ref) else {
                continue;
            };
            match cache.transcript(&file, WHISPER_MODEL_ID, &cache_language, None) {
                Ok(Some(result)) => {
                    transcripts.insert(clip.media_ref.clone(), result);
                }
                // Clean miss → no entry (clip yields no words; UJ-1 empty path).
                Ok(None) => {}
                Err(e) => skipped.push(json!({
                    "file": file.file_name().map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| clip.media_ref.clone()),
                    "reason": e.to_string(),
                })),
            }
        }
    }

    // Build the per-clip word rows. Words attributed to the clip whose visible source
    // window contains their midpoint; mapped to project frames via span_frames; window
    // filtered; globally capped at TRANSCRIPT_WORD_CAP with nextStartFrame paging.
    let fps_f = fps as f64;
    let mut clips_out: Vec<Value> = Vec::new();
    let mut total_words = 0usize;
    let mut remaining = TRANSCRIPT_WORD_CAP;
    let mut last_end: Option<i32> = None;

    for (track_index, clip) in &frags {
        let Some(transcript) = transcripts.get(&clip.media_ref) else {
            // No cached transcript for this source → still emit the clip with empty
            // words so the shape is stable (reference only emits clips that have rows;
            // we keep an empty entry so the agent sees the clip exists but is untranscribed).
            clips_out.push(json!({
                "clipId": clip.id,
                "trackIndex": track_index,
                "startFrame": clip.start_frame,
                "endFrame": clip.end_frame(),
                "words": [],
            }));
            continue;
        };

        let vis_start = clip.trim_start_frame as f64;
        let vis_end = vis_start + clip.duration_frames as f64 * clip.speed.max(0.0001);

        // Collect (start, end, row) for words whose midpoint is in the visible window.
        let mut rows: Vec<(i32, i32, Value)> = Vec::new();
        for w in &transcript.words {
            let (Some(s), Some(e)) = (w.start, w.end) else {
                continue;
            };
            let mid_frame = (s + e) / 2.0 * fps_f;
            if !(mid_frame >= vis_start && mid_frame < vis_end) {
                continue;
            }
            let Some((fs, fe)) = span_frames(s, e, clip, fps) else {
                continue;
            };
            // Window filter: drop words ending at/before startFrame or starting at/after endFrame.
            if let Some(ws) = window_start {
                if fe <= ws {
                    continue;
                }
            }
            if let Some(we) = window_end {
                if fs >= we {
                    continue;
                }
            }
            rows.push((fs, fe, json!([w.text, fs, fe])));
        }
        rows.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        if rows.is_empty() {
            continue;
        }
        total_words += rows.len();
        if remaining == 0 {
            continue;
        }
        let take = rows.len().min(remaining);
        remaining -= take;
        if let Some((_, e, _)) = rows.get(take.saturating_sub(1)) {
            last_end = Some(*e);
        }
        let slice: Vec<Value> = rows.into_iter().take(take).map(|(_, _, r)| r).collect();
        clips_out.push(json!({
            "clipId": clip.id,
            "trackIndex": track_index,
            "startFrame": clip.start_frame,
            "endFrame": clip.end_frame(),
            "words": slice,
        }));
    }

    let mut body = Map::new();
    body.insert("fps".to_string(), json!(fps));
    body.insert("timing".to_string(), json!("projectFrames"));
    body.insert("wordFormat".to_string(), json!(["text", "start", "end"]));
    body.insert("clips".to_string(), Value::Array(clips_out));
    if total_words > TRANSCRIPT_WORD_CAP {
        body.insert("totalWords".to_string(), json!(total_words));
        if let Some(end) = last_end {
            body.insert("nextStartFrame".to_string(), json!(end));
            body.insert(
                "wordsNote".to_string(),
                json!(format!(
                    "First {TRANSCRIPT_WORD_CAP} of {total_words} words. Continue with startFrame = nextStartFrame."
                )),
            );
        }
    }
    if !skipped.is_empty() {
        body.insert("skipped".to_string(), Value::Array(skipped));
    }

    // get_transcript is NOT rounded (integer frames).
    match serde_json::to_string(&Value::Object(body)) {
        Ok(s) => ToolResult::ok(s),
        Err(_) => ToolResult::error("Failed to serialize transcript"),
    }
}

#[cfg(test)]
mod full_timeline_tests {
    use super::*;
    use palmier_model::{
        AnimPair, Clip, ClipType, Interpolation, Keyframe, KeyframeTrack, MediaLibrary, Track,
    };

    /// Build an `EditorState` whose library wraps `timeline`.
    fn state_with(timeline: Timeline) -> EditorState {
        let mut lib = MediaLibrary::new();
        lib.timeline = timeline;
        EditorState::with_library(lib)
    }

    #[test]
    fn full_timeline_empty_has_default_root() {
        let st = state_with(Timeline::new());
        let v = full_timeline_json(&st);
        assert_eq!(v.get("fps").and_then(Value::as_i64), Some(30));
        assert_eq!(v.get("width").and_then(Value::as_i64), Some(1920));
        assert_eq!(v.get("height").and_then(Value::as_i64), Some(1080));
        assert_eq!(v.get("tracks").and_then(Value::as_array).map(|a| a.len()), Some(0));
        assert_eq!(v.get("totalFrames").and_then(Value::as_i64), Some(0));
        assert_eq!(v.get("canGenerate").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn full_timeline_preserves_non_default_clip_and_track_fields() {
        // A clip carrying NON-default volume / trim / speed / opacity / fades.
        let mut clip = Clip::new("asset-1", 10, 90);
        clip.id = "clip-1".into();
        clip.media_type = ClipType::Audio;
        clip.source_clip_type = ClipType::Video;
        clip.trim_start_frame = 7;
        clip.trim_end_frame = 3;
        clip.speed = 2.0;
        clip.volume = 0.42;
        clip.opacity = 0.6;
        clip.fade_in_frames = 12;
        clip.fade_out_frames = 8;
        clip.fade_in_interpolation = Interpolation::Smooth;
        clip.fade_out_interpolation = Interpolation::Linear;
        clip.link_group_id = Some("grp-9".into());

        let mut track = Track::new(ClipType::Audio);
        track.id = "track-1".into();
        track.muted = true;
        track.hidden = false;
        track.sync_locked = false; // non-default (default is true)
        track.display_height = 80.0;
        track.clips.push(clip);

        let mut tl = Timeline::new();
        tl.tracks.push(track);
        let st = state_with(tl);

        let v = full_timeline_json(&st);
        let tracks = v.get("tracks").and_then(Value::as_array).expect("tracks array");
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];

        // Track fields — including the otherwise-skipped displayHeight, and the
        // non-default flags that the COMPACT summary would strip.
        assert_eq!(t.get("id").and_then(Value::as_str), Some("track-1"));
        assert_eq!(t.get("type").and_then(Value::as_str), Some("audio"));
        assert_eq!(t.get("muted").and_then(Value::as_bool), Some(true));
        assert_eq!(t.get("hidden").and_then(Value::as_bool), Some(false));
        assert_eq!(t.get("syncLocked").and_then(Value::as_bool), Some(false));
        assert_eq!(t.get("displayHeight").and_then(Value::as_f64), Some(80.0));

        let clips = t.get("clips").and_then(Value::as_array).expect("clips array");
        assert_eq!(clips.len(), 1);
        let c = &clips[0];
        assert_eq!(c.get("id").and_then(Value::as_str), Some("clip-1"));
        assert_eq!(c.get("mediaRef").and_then(Value::as_str), Some("asset-1"));
        assert_eq!(c.get("mediaType").and_then(Value::as_str), Some("audio"));
        assert_eq!(c.get("sourceClipType").and_then(Value::as_str), Some("video"));
        assert_eq!(c.get("startFrame").and_then(Value::as_i64), Some(10));
        assert_eq!(c.get("durationFrames").and_then(Value::as_i64), Some(90));
        assert_eq!(c.get("trimStartFrame").and_then(Value::as_i64), Some(7));
        assert_eq!(c.get("trimEndFrame").and_then(Value::as_i64), Some(3));
        assert_eq!(c.get("speed").and_then(Value::as_f64), Some(2.0));
        assert_eq!(c.get("volume").and_then(Value::as_f64), Some(0.42));
        assert_eq!(c.get("opacity").and_then(Value::as_f64), Some(0.6));
        assert_eq!(c.get("fadeInFrames").and_then(Value::as_i64), Some(12));
        assert_eq!(c.get("fadeOutFrames").and_then(Value::as_i64), Some(8));
        assert_eq!(c.get("fadeInInterpolation").and_then(Value::as_str), Some("smooth"));
        assert_eq!(c.get("fadeOutInterpolation").and_then(Value::as_str), Some("linear"));
        assert_eq!(c.get("linkGroupId").and_then(Value::as_str), Some("grp-9"));
    }

    #[test]
    fn full_timeline_emits_keyframe_tracks_verbatim() {
        // A clip with a dB volume track and a position (AnimPair) track.
        let mut clip = Clip::new("asset-2", 0, 100);
        clip.id = "clip-kf".into();

        let mut vol = KeyframeTrack::new();
        vol.upsert(Keyframe::with_interpolation(0, -6.0, Interpolation::Linear));
        vol.upsert(Keyframe::with_interpolation(50, 6.0, Interpolation::Smooth));
        clip.volume_track = Some(vol);

        let mut pos = KeyframeTrack::new();
        pos.upsert(Keyframe::new(0, AnimPair::new(0.1, 0.2)));
        clip.position_track = Some(pos);

        let mut track = Track::new(ClipType::Video);
        track.clips.push(clip);
        let mut tl = Timeline::new();
        tl.tracks.push(track);
        let st = state_with(tl);

        let v = full_timeline_json(&st);
        let c = &v["tracks"][0]["clips"][0];

        // volumeTrack survives as { keyframes: [{ frame, value, interpolationOut }] }.
        let vkfs = c["volumeTrack"]["keyframes"].as_array().expect("volume keyframes");
        assert_eq!(vkfs.len(), 2);
        assert_eq!(vkfs[0].get("frame").and_then(Value::as_i64), Some(0));
        assert_eq!(vkfs[0].get("value").and_then(Value::as_f64), Some(-6.0));
        assert_eq!(vkfs[0].get("interpolationOut").and_then(Value::as_str), Some("linear"));
        assert_eq!(vkfs[1].get("value").and_then(Value::as_f64), Some(6.0));
        assert_eq!(vkfs[1].get("interpolationOut").and_then(Value::as_str), Some("smooth"));

        // positionTrack keeps the AnimPair { a, b } value shape verbatim.
        let pkfs = c["positionTrack"]["keyframes"].as_array().expect("position keyframes");
        assert_eq!(pkfs.len(), 1);
        assert_eq!(pkfs[0]["value"].get("a").and_then(Value::as_f64), Some(0.1));
        assert_eq!(pkfs[0]["value"].get("b").and_then(Value::as_f64), Some(0.2));
    }
}
