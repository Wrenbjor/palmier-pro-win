//! TEXT + CAPTION tool bodies — `add_texts`, `add_captions` (E7-S8; reference
//! `ToolExecutor+Texts.swift` + `ToolExecutor+Captions.swift`).
//!
//! - **`add_texts`** creates text overlay clips (titles / lower-thirds) as **one**
//!   named agent-undo step. `track_index` is all-or-none: omitting it on every entry
//!   auto-creates one new top video track for the batch; setting it targets an
//!   existing non-audio track. Defaults: `font_name` `Helvetica-Bold`, `font_size`
//!   96, `color` `#FFFFFF`, `alignment` center (reference `addTexts`). Same-track
//!   overlap → overwrite via [`clear_region`](crate::clips::clear_region).
//! - **`add_captions`** is the dispatch seam for the on-device transcription →
//!   styled-caption pipeline. Its real Whisper + CaptionBuilder backing is **Epic 10
//!   (M3)**; in M2 there is no transcription backend, so the styling/placement is
//!   validated (including ruling #18: `text_case ∈ {auto, upper, lower}` — no
//!   title-case) and the tool returns a "transcription not available" result until
//!   Epic 10 lands. When Epic 10 is present, the CaptionBuilder path must satisfy the
//!   14 verbatim CaptionBuilder tests (SM-13) it owns — this tool is the seam, not
//!   the builder.
//!
//! Colors parse through the shared [`parse_rgba`](crate::validate::parse_rgba) hex
//! parser (`#RRGGBB` / `#RRGGBBAA`, E7-S3). Mutating-tool undo semantics match
//! E7-S6/S7 (one agent step via [`agent_edit`](crate::undo::agent_edit)).

use serde_json::Value;

use palmier_model::{
    Clip, ClipType, FontName, Rgba, TextAlignment, TextStyle, Timeline, Transform,
};

use crate::clips::{clear_region, insert_track};
use crate::editor::EditorState;
use crate::result::ToolResult;
use crate::undo::agent_edit;
use crate::validate::parse_rgba;

// ─────────────────────────────────────────────────────────────────────────────
// add_texts
// ─────────────────────────────────────────────────────────────────────────────

/// Reference `addTexts` defaults (`ToolExecutor+Texts.swift`, `AppTheme` text
/// tokens). `font_name` default `Helvetica-Bold`; `font_size` default 96; `color`
/// default white; `alignment` default center.
const DEFAULT_FONT_NAME: &str = "Helvetica-Bold";
const DEFAULT_FONT_SIZE: f64 = 96.0;

/// One resolved text clip to place: where + what + how. Owned so it survives into
/// the agent-swap closure (which only gets `&mut Timeline`).
struct TextSpec {
    track_index: Option<usize>,
    start_frame: i32,
    duration_frames: i32,
    content: String,
    style: TextStyle,
    transform: Option<Transform>,
}

/// `add_texts` (`entries[{start_frame, duration_frames, content}]`, all-or-none
/// `track_index`, optional `transform`/`font_name`/`font_size`/`color`/`alignment`):
/// place text clips as ONE undo. Omitting `track_index` on every entry auto-creates
/// one new top video track. Reference `addTexts`.
pub fn add_texts(state: &mut EditorState, args: &Value) -> ToolResult {
    let entries = match args.get("entries").and_then(Value::as_array) {
        Some(e) if !e.is_empty() => e,
        _ => return ToolResult::error("Missing or empty 'entries' array"),
    };

    let mut specs: Vec<TextSpec> = Vec::with_capacity(entries.len());
    for (idx, entry) in entries.iter().enumerate() {
        let path = format!("entries[{idx}]");
        let entry = match entry.as_object() {
            Some(o) => o,
            None => return ToolResult::error(format!("{path} must be an object")),
        };
        let content = match entry.get("content").and_then(Value::as_str) {
            Some(s) => s.to_string(),
            None => return ToolResult::error(format!("{path}: missing 'content'")),
        };
        let start_frame = match entry.get("startFrame").and_then(Value::as_i64) {
            Some(v) => v as i32,
            None => return ToolResult::error(format!("{path}: missing 'startFrame'")),
        };
        let duration_frames = match entry.get("durationFrames").and_then(Value::as_i64) {
            Some(v) => v as i32,
            None => return ToolResult::error(format!("{path}: missing 'durationFrames'")),
        };
        if duration_frames < 1 {
            return ToolResult::error(format!(
                "{path}: durationFrames must be >= 1 (got {duration_frames})"
            ));
        }
        if start_frame < 0 {
            return ToolResult::error(format!(
                "{path}: startFrame must be >= 0 (got {start_frame})"
            ));
        }
        // track_index: must be an existing non-audio track when set.
        let track_index = match entry.get("trackIndex").and_then(Value::as_i64) {
            Some(ti) => {
                let ti = ti as usize;
                let track = match state.library.timeline.tracks.get(ti) {
                    Some(t) => t,
                    None => {
                        return ToolResult::error(format!(
                            "{path}: track index {ti} out of range (0..{})",
                            state.library.timeline.tracks.len().saturating_sub(1)
                        ))
                    }
                };
                if !ClipType::Text.is_compatible(track.track_type) {
                    return ToolResult::error(format!(
                        "{path}: track {ti} is an audio track; text requires a video/image/text track"
                    ));
                }
                Some(ti)
            }
            None => None,
        };

        // Build the text style (defaults + overrides).
        let mut style = TextStyle::default();
        style.font_name = FontName::from_str(DEFAULT_FONT_NAME);
        style.font_size = DEFAULT_FONT_SIZE;
        // color default is opaque white (TextStyle::default already white).
        if let Some(f) = entry.get("fontName").and_then(Value::as_str) {
            style.font_name = FontName::from_str(f);
        }
        if let Some(s) = entry.get("fontSize").and_then(Value::as_f64) {
            style.font_size = s;
        }
        if let Some(c) = entry.get("color").and_then(Value::as_str) {
            match parse_rgba(c) {
                Ok((r, g, b, a)) => {
                    style.color = Rgba::new(
                        r as f64 / 255.0,
                        g as f64 / 255.0,
                        b as f64 / 255.0,
                        a as f64 / 255.0,
                    );
                }
                Err(e) => return ToolResult::error(e.message),
            }
        }
        if let Some(al) = entry.get("alignment").and_then(Value::as_str) {
            style.alignment = match al {
                "left" => TextAlignment::Left,
                "right" => TextAlignment::Right,
                "center" => TextAlignment::Center,
                other => {
                    return ToolResult::error(format!(
                        "{path}: alignment must be 'left', 'center', or 'right' (got '{other}')"
                    ))
                }
            };
        }

        // transform: omit → center+auto-fit (None); {centerX, centerY} → position
        // with auto-fit (we keep default width/height); all four → full override.
        let transform = match entry.get("transform") {
            None => None,
            Some(Value::Object(t)) => {
                let cx = t.get("centerX").and_then(Value::as_f64);
                let cy = t.get("centerY").and_then(Value::as_f64);
                let w = t.get("width").and_then(Value::as_f64);
                let h = t.get("height").and_then(Value::as_f64);
                if cx.is_none() && cy.is_none() && w.is_none() && h.is_none() {
                    None
                } else {
                    let (cx, cy) = match (cx, cy) {
                        (Some(cx), Some(cy)) => (cx, cy),
                        _ => {
                            return ToolResult::error(format!(
                                "{path}: transform must be either {{centerX, centerY}} for auto-fit, \
                                 or all four of {{centerX, centerY, width, height}}"
                            ))
                        }
                    };
                    let mut tf = Transform::default();
                    tf.center_x = cx;
                    tf.center_y = cy;
                    match (w, h) {
                        (Some(w), Some(h)) => {
                            tf.width = w;
                            tf.height = h;
                        }
                        (None, None) => {
                            // auto-fit: keep the default box size (the GPU text pass
                            // refits to the content; a full builder lives in E5-S9).
                        }
                        _ => {
                            return ToolResult::error(format!(
                                "{path}: transform must be either {{centerX, centerY}} for auto-fit, \
                                 or all four of {{centerX, centerY, width, height}}"
                            ))
                        }
                    }
                    Some(tf)
                }
            }
            Some(_) => return ToolResult::error(format!("{path}: transform must be an object")),
        };

        specs.push(TextSpec {
            track_index,
            start_frame,
            duration_frames,
            content,
            style,
            transform,
        });
    }

    // All-or-none track_index (a new track at index 0 would shift explicit indices).
    let omitted = specs.iter().filter(|s| s.track_index.is_none()).count();
    if omitted != 0 && omitted != specs.len() {
        return ToolResult::error(format!(
            "Mixed trackIndex: {omitted} of {} entries omitted trackIndex. Either set it on \
             every entry or omit it on every entry (to auto-create a shared new track).",
            specs.len()
        ));
    }

    let action_name = if specs.len() == 1 { "Add Text (Agent)" } else { "Add Texts (Agent)" };
    let count = specs.len();
    let all_omitted = omitted == count;
    agent_edit(state, action_name, move |timeline, _hist| {
        let mut created_track: Option<String> = None;
        let shared_track = if all_omitted {
            let i = insert_track(timeline, ClipType::Video, 0);
            created_track = Some(format!("track {i} (video)"));
            Some(i)
        } else {
            None
        };

        let mut summaries: Vec<String> = Vec::new();
        for (i, spec) in specs.iter().enumerate() {
            let track_index = match spec.track_index {
                Some(ti) => ti,
                None => shared_track.expect("auto-created text track above"),
            };
            // Overwrite same-track overlap first (reference drag-onto-track behavior).
            clear_region(
                timeline,
                track_index,
                spec.start_frame,
                spec.start_frame + spec.duration_frames,
            );
            let id = place_text_clip(timeline, track_index, spec);
            let Some(id) = id else {
                return Err(format!(
                    "entries[{i}]: failed to place text clip on track {track_index}"
                ));
            };
            summaries.push(format!(
                "{id} on track {track_index} @ frame {} for {}",
                spec.start_frame, spec.duration_frames
            ));
        }
        let prefix = created_track.map(|t| format!("Created {t}. ")).unwrap_or_default();
        Ok(ToolResult::ok(format!(
            "{prefix}Added {count} text clip(s): {}",
            summaries.join("; ")
        )))
    })
}

/// Build + push one text [`Clip`] onto `track_index`, returning its id (or `None`
/// if the track vanished). Reference `placeTextClips` per-spec.
fn place_text_clip(timeline: &mut Timeline, track_index: usize, spec: &TextSpec) -> Option<String> {
    timeline.tracks.get(track_index)?;
    let mut clip = Clip::new(String::new(), spec.start_frame, spec.duration_frames);
    clip.media_type = ClipType::Text;
    clip.source_clip_type = ClipType::Text;
    clip.text_content = Some(spec.content.clone());
    clip.text_style = Some(spec.style.clone());
    if let Some(tf) = spec.transform {
        clip.transform = tf;
    }
    let id = clip.id.clone();
    timeline.tracks[track_index].clips.push(clip);
    timeline.tracks[track_index].sort_clips();
    Some(id)
}

// ─────────────────────────────────────────────────────────────────────────────
// add_captions
// ─────────────────────────────────────────────────────────────────────────────

/// `add_captions` (optional `clip_ids`, `language`, `font_name`, `font_size` default
/// 48, `color`, `center_x` default .5, `center_y` default .9, `text_case ∈ {auto,
/// upper, lower}` (ruling #18), `censor_profanity`): on-device transcribe + styled
/// caption clips on a new track. **Async**. Reference `addCaptions`.
///
/// The real Whisper + CaptionBuilder backing is **Epic 10 (M3)**. M2 validates the
/// styling/placement args (so a malformed call is rejected with the same messages the
/// reference emits) and then reports that on-device transcription is not yet
/// available — the reference returns caption-clip ids once Epic 10's pipeline lands.
pub fn add_captions(state: &mut EditorState, args: &Value) -> ToolResult {
    let _ = state;
    let obj = match args.as_object() {
        Some(o) => o,
        None => return ToolResult::error("add_captions: arguments must be a JSON object"),
    };

    // Validate the optional styling/placement args up front (parity with the
    // reference, which parses these before kicking off transcription).
    if let Some(c) = obj.get("color").and_then(Value::as_str) {
        if let Err(e) = parse_rgba(c) {
            return ToolResult::error(e.message);
        }
    }
    // ruling #18: text_case is auto | upper | lower — NO title-case.
    if let Some(tc) = obj.get("textCase").and_then(Value::as_str) {
        if !matches!(tc, "auto" | "upper" | "lower") {
            return ToolResult::error(format!(
                "add_captions: textCase must be auto, upper, or lower (got {tc})"
            ));
        }
    }

    // The transcription backend (Whisper + CaptionBuilder) lands in Epic 10 (M3).
    // Until then there is nothing to transcribe; report it the way the reference's
    // "no speech / not available" path would, so the agent tells the user.
    ToolResult::error(
        "add_captions: on-device transcription is not yet available in this build \
         (the caption pipeline lands in a later milestone). To add text by hand, use add_texts.",
    )
}
