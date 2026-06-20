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

use palmier_edit::{
    generate_captions, CaptionCase, CaptionRequest, GENERATE_CAPTIONS_UNDO_NAME,
};
use palmier_model::{
    Clip, ClipType, FontName, Rgba, TextAlignment, TextStyle, Timeline, Transform,
};
use palmier_text::{caption_theme, FontRegistry, TextLayout};
use palmier_transcribe::TranscriptionError;

use crate::caption_transcribe::{
    acquire_cache, build_runtime, resolve_cache_language, transcribe_blocking, validate_language,
    LibraryAssets, WHISPER_MODEL_ID,
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

/// `add_captions` (optional `clipIds`, `language`, `fontName`, `fontSize` default 48,
/// `color`, `centerX` default .5, `centerY` default .9, `textCase ∈ {auto, upper,
/// lower}` (ruling #18), `censorProfanity`): on-device transcribe + styled caption
/// clips on a **new video track at index 0**, as ONE undoable agent action named
/// `"Generate Captions"`. Reference `addCaptions` → `EditorViewModel.generateCaptions`.
///
/// Wraps E10-S6's [`generate_captions`]: this body parses the styling/placement args,
/// builds the [`CaptionRequest`], and supplies the `transcribe` closure that applies
/// the **cache-vs-bypass** rule and runs the (blocking) whisper engine on a blocking
/// thread (see [`crate::caption_transcribe`]). The undo-group name is matched verbatim
/// (`GENERATE_CAPTIONS_UNDO_NAME`) for agent-undo parity.
pub fn add_captions(state: &mut EditorState, args: &Value) -> ToolResult {
    let obj = match args.as_object() {
        Some(o) => o,
        None => return ToolResult::error("add_captions: arguments must be a JSON object"),
    };

    // ── Parse + validate the styling/placement args (parity: the reference parses
    // these before kicking off transcription; a malformed call rejects with no work).

    // clipIds: empty ⇒ auto-detect the dominant speech track.
    let clip_ids: Vec<String> = obj
        .get("clipIds")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();

    // Style: defaults Helvetica-Bold (reference falls back to bold system font) at the
    // caption default font size 48; white. Overrides from fontName/fontSize/color.
    let mut style = TextStyle::default();
    style.font_name = FontName::from_str(DEFAULT_FONT_NAME);
    style.font_size = caption_theme::DEFAULT_FONT_SIZE;
    if let Some(f) = obj.get("fontName").and_then(Value::as_str) {
        style.font_name = FontName::from_str(f);
    }
    if let Some(s) = obj.get("fontSize").and_then(Value::as_f64) {
        style.font_size = s;
    }
    if let Some(c) = obj.get("color").and_then(Value::as_str) {
        match parse_rgba(c) {
            Ok((r, g, b, a)) => {
                style.color =
                    Rgba::new(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a as f64 / 255.0);
            }
            Err(e) => return ToolResult::error(e.message),
        }
    }

    // language (BCP-47): validate against the on-device model's supported set, exactly
    // like the reference (`matchLocale … ?? throw`). `None` ⇒ system/auto.
    let locale: Option<String> = match obj.get("language").and_then(Value::as_str) {
        Some(lang) => match validate_language(lang) {
            Ok(_) => Some(lang.to_string()),
            Err(msg) => return ToolResult::error(msg),
        },
        None => None,
    };

    // center: default (0.5, 0.9), the lower third.
    let mut center = caption_theme::DEFAULT_CENTER;
    if let Some(x) = obj.get("centerX").and_then(Value::as_f64) {
        center.0 = x;
    }
    if let Some(y) = obj.get("centerY").and_then(Value::as_f64) {
        center.1 = y;
    }

    // textCase: ruling #18 — auto | upper | lower ONLY. "title" is rejected verbatim.
    let text_case = match obj.get("textCase").and_then(Value::as_str) {
        None => CaptionCase::Auto,
        Some("auto") => CaptionCase::Auto,
        Some("upper") => CaptionCase::Upper,
        Some("lower") => CaptionCase::Lower,
        Some(other) => {
            return ToolResult::error(format!(
                "add_captions: textCase must be auto, upper, or lower (got {other})"
            ))
        }
    };

    let censor_profanity = obj.get("censorProfanity").and_then(Value::as_bool).unwrap_or(false);

    let request = CaptionRequest {
        source_clip_ids: clip_ids.clone(),
        auto_detect: clip_ids.is_empty(),
        style,
        center,
        text_case,
        censor_profanity,
        locale: locale.clone(),
    };

    // ── Build the transcription plumbing BEFORE entering agent_edit (which borrows
    // `state.library.timeline` mutably): the runtime, the cache handle, and an owned
    // snapshot of the asset catalog (has-audio / is-video / file path per media_ref).
    let runtime = match build_runtime() {
        Ok(r) => r,
        Err(e) => {
            return ToolResult::error(format!(
                "add_captions: failed to start the transcription runtime: {e}"
            ))
        }
    };
    let cache = runtime.block_on(acquire_cache());
    let assets = LibraryAssets::snapshot(&state.library);

    // The cache-vs-bypass decision is fixed per request (reference: bypass when
    // `censorProfanity || locale != nil`). `cache_language` is the tag the PLAIN path
    // reads/writes under so the read in get_transcript matches.
    let bypass_cache = censor_profanity || locale.is_some();
    let cache_language = resolve_cache_language(locale.as_deref());

    // Mint the shared caption-group UUID inside generate_captions via this closure.
    let new_group_id = || uuid::Uuid::new_v4().to_string();

    // FontRegistry/TextLayout: caption measurement state (natural size / line-fits).
    // Bundled-only fonts (no OS font enumeration) — parity with the caption pipeline.
    let mut registry = FontRegistry::bundled_only();
    let mut layout = TextLayout::new();

    // ── One undoable agent action named "Generate Captions". generate_captions
    // registers its own user-swap on the scratch history; agent_edit captures the ONE
    // agent step from the before/after timeline diff under this exact name.
    agent_edit(state, GENERATE_CAPTIONS_UNDO_NAME, |timeline, history| {
        let transcribe = |media_ref: &str,
                          range: Option<&std::ops::RangeInclusive<f64>>,
                          is_video: bool|
         -> Result<palmier_transcribe::TranscriptionResult, TranscriptionError> {
            let Some(file) = assets.file(media_ref).cloned() else {
                // Unknown asset → nothing to transcribe (skipped by generate_captions).
                return Err(TranscriptionError::AnalysisFailed(format!(
                    "no source file for media '{media_ref}'"
                )));
            };
            let range_owned = range.cloned();

            if bypass_cache {
                // BYPASS: request-specific (censor and/or locale) → engine directly,
                // never cached.
                return runtime.block_on(transcribe_blocking(
                    file,
                    censor_profanity,
                    locale.clone(),
                    range_owned,
                    is_video,
                ));
            }

            // PLAIN PATH → through the cache. Read first (filtered to range, never
            // transcribes). On a miss, transcribe the WHOLE file, store it, then filter.
            if let Ok(Some(hit)) =
                cache.transcript(&file, WHISPER_MODEL_ID, &cache_language, range_owned.as_ref())
            {
                return Ok(hit);
            }
            // Miss: transcribe the full file (no range) so the stored artifact is
            // reusable, store it, then filter to the requested range.
            let full = runtime.block_on(transcribe_blocking(
                file.clone(),
                false,
                locale.clone(),
                None,
                is_video,
            ))?;
            let _ = cache.store(&file, WHISPER_MODEL_ID, &cache_language, &full);
            Ok(match range_owned {
                Some(r) => palmier_transcribe::TranscriptCache::filter(&full, &r),
                None => full,
            })
        };

        match generate_captions(
            timeline,
            history,
            &request,
            &assets,
            &mut registry,
            &mut layout,
            transcribe,
            new_group_id,
        ) {
            Ok(out) => {
                let n = out.clip_ids.len();
                if n == 0 {
                    // No speech detected → reference throws ToolError("No speech detected
                    // to caption."). Surface as the error shape (registers no undo).
                    Err("No speech detected to caption.".to_string())
                } else {
                    Ok(ToolResult::ok(format!(
                        "Added {n} caption{}.",
                        if n == 1 { "" } else { "s" }
                    )))
                }
            }
            // CaptionError::NoSource → "No audio clips to caption." (verbatim Display).
            Err(e) => Err(e.to_string()),
        }
    })
}
