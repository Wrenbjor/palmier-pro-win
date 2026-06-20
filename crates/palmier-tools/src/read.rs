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

use crate::editor::EditorState;
use crate::json_round::{round_json_numbers, JSON_ROUND_PLACES};
use crate::result::ToolResult;

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

/// `get_transcript` (`startFrame?`, `endFrame?`, `clipId?`): the spoken transcript
/// of the CURRENT timeline in project frames — clips in timeline order, words
/// `[text, startFrame, endFrame]` capped at 10000, paged via `nextStartFrame`.
///
/// The real word data lives in **Epic 10's transcription store** (M3). In M2 there
/// is no store, so every audio/video clip yields zero words and the transcript is
/// empty — matching the reference's "no transcription → empty → agent tells the
/// user to transcribe" (UJ-1 edge case). The clip-walk, ordering, window
/// validation, and the `spanFrames` source→timeline mapping
/// ([`crate::transcript`]) are all implemented now so Epic 10 only supplies the
/// words.
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

    // Walk every audio/video clip in timeline order. Without Epic 10's store there
    // are no words; we still emit clip entries scoped/ordered correctly so the
    // shape is stable. (When a clip filter is set and matches nothing → error, per
    // the reference.)
    let mut frags: Vec<(usize, &Clip)> = Vec::new();
    for (track_index, track) in timeline.tracks.iter().enumerate() {
        for clip in &track.clips {
            // Only audio/video clips carry speech.
            use palmier_model::ClipType::*;
            if !matches!(clip.media_type, Video | Audio) {
                continue;
            }
            if let Some(filter) = clip_filter {
                if clip.id != filter {
                    continue;
                }
            }
            frags.push((track_index, clip));
        }
    }
    frags.sort_by_key(|(_, c)| c.start_frame);

    if clip_filter.is_some() && frags.is_empty() {
        return ToolResult::error(format!(
            "Clip {} not found, or it has no audio/video to transcribe.",
            clip_filter.unwrap()
        ));
    }

    // In M2 there is no transcription backend, so words are empty for every clip.
    // We emit clip entries with empty word lists (the reference emits clips with
    // their `words` slices — here always empty until Epic 10).
    let clips_out: Vec<Value> = frags
        .iter()
        .map(|(track_index, clip)| {
            json!({
                "clipId": clip.id,
                "trackIndex": track_index,
                "startFrame": clip.start_frame,
                "endFrame": clip.end_frame(),
                "words": [],
            })
        })
        .collect();

    // totalWords is 0 in M2 (no store) → no paging note.
    let body = json!({
        "fps": fps,
        "timing": "projectFrames",
        "wordFormat": ["text", "start", "end"],
        "clips": clips_out,
    });

    // get_transcript is NOT rounded (integer frames).
    match serde_json::to_string(&body) {
        Ok(s) => ToolResult::ok(s),
        Err(_) => ToolResult::error("Failed to serialize transcript"),
    }
}
