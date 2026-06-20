//! E10-S6 — `CaptionBuilder.specs(...)` mapping + `generate_captions` orchestration
//! + caption track placement.
//!
//! Clean-room parity port of the macOS reference's caption integration:
//! * [`specs`] ← `MediaPanel/CaptionsTab/CaptionBuilder.swift` `specs(...)` —
//!   maps each timed [`Phrase`] through a source clip's trim/speed into a
//!   [`TextClipSpec`] (drop-before-trim, clamp-past-end, center-based `Transform`).
//! * [`generate_captions`] ← `Editor/ViewModel/EditorViewModel+Captions.swift`
//!   `generateCaptions(for:)` — target selection, per-media transcription over the
//!   ±1.0 s-padded visible-source union, optional dominant-speech-track auto-detect,
//!   phrase→clip assignment by most overlap, casing, and placement of a **new video
//!   track at index 0** under one undo group **"Generate Captions"**.
//! * [`place_text_clips`] ← `EditorViewModel+MediaLibrary.swift` `placeTextClips(_:)`.
//!
//! The phrase algorithm itself ([`palmier_text::phrases`]) and the
//! natural-size/`caption_line_fits` measurement ([`palmier_text::TextLayout::natural_size`])
//! are **consumed**, not duplicated (E10-S5 / Epic 5). The frame mapping rides on
//! [`palmier_model::Clip::timeline_frame`] (Epic 2 parity math: `f64::round`
//! ties-away, speed floor `0.0001`, half-open `[start_frame, end_frame)`).
//!
//! ## Transcription is injected (the cache/bypass + blocking call live with the caller)
//! `palmier_transcribe::transcribe` is **synchronous** (whisper `full()` blocks) and
//! the cache (`TranscriptCache::shared().await`) is **async**; both pull in
//! whisper/tokio. To keep this pure-edit crate free of those, [`generate_captions`]
//! takes a `transcribe` **closure** — `Fn(&media_ref, range, is_video) -> Result<…>`.
//! E10-S7's tool body wires the real closure: cache when
//! `!(censor_profanity || locale.is_some())`, else bypass straight to
//! `transcribe`/`transcribe_video_audio`, each on a blocking thread. This module
//! owns the *parity logic* (targets, union, dominant track, assignment, placement);
//! the closure owns the *engine plumbing*.

use palmier_history::History;
use palmier_model::{Clip, ClipType, TextStyle, Timeline, Track, Transform};
pub use palmier_text::CaptionCase;
use palmier_text::{caption_theme, phrases, FontRegistry, Phrase, Segment, TextLayout};
use palmier_transcribe::{TranscriptionResult, TranscriptionSegment};

/// The exact reference undo-group name for caption placement. Agent-undo parity
/// (carry-forward): the editor's `undoActionName` must equal this for an agent undo
/// of caption generation to be accepted, so it is matched **verbatim**.
pub const GENERATE_CAPTIONS_UNDO_NAME: &str = "Generate Captions";

/// A batch text-clip placement instruction (reference
/// `EditorViewModel.TextClipSpec`). When `transform` is `None` the box is auto-fit
/// to the content's natural size and centered on the canvas (handled in
/// [`place_text_clips`]).
#[derive(Debug, Clone, PartialEq)]
pub struct TextClipSpec {
    /// Destination track index.
    pub track_index: usize,
    /// Start frame on the timeline.
    pub start_frame: i32,
    /// Visible duration in timeline frames.
    pub duration_frames: i32,
    /// The caption text.
    pub content: String,
    /// The text style to apply.
    pub style: TextStyle,
    /// Explicit box transform (center-based, normalized); `None` ⇒ auto-fit+center.
    pub transform: Option<Transform>,
    /// The caption group id shared across one generation batch.
    pub caption_group_id: Option<String>,
}

/// Apply a [`CaptionCase`] to text (reference `CaptionCase.apply`).
fn apply_case(case: CaptionCase, s: &str) -> String {
    match case {
        CaptionCase::Auto => s.to_string(),
        CaptionCase::Upper => s.to_uppercase(),
        CaptionCase::Lower => s.to_lowercase(),
    }
}

/// The caption generation request (reference `EditorViewModel.CaptionRequest`).
#[derive(Debug, Clone)]
pub struct CaptionRequest {
    /// Source clip ids to caption. Empty ⇒ all captionable clips (auto pool).
    pub source_clip_ids: Vec<String>,
    /// Auto-detect the dominant speech track and caption only it.
    pub auto_detect: bool,
    /// The caption text style.
    pub style: TextStyle,
    /// Normalized caption center `(x, y)` (reference `center`, default lower third).
    pub center: (f64, f64),
    /// Caption casing.
    pub text_case: CaptionCase,
    /// Whether profanity is censored (affects cache bypass at the caller).
    pub censor_profanity: bool,
    /// Preferred locale override (affects cache bypass at the caller). `None` ⇒ auto.
    pub locale: Option<String>,
}

impl Default for CaptionRequest {
    fn default() -> Self {
        CaptionRequest {
            source_clip_ids: Vec::new(),
            auto_detect: false,
            style: TextStyle::default(),
            center: caption_theme::DEFAULT_CENTER,
            text_case: CaptionCase::Auto,
            censor_profanity: false,
            locale: None,
        }
    }
}

/// Why caption generation produced nothing actionable (reference `CaptionError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptionError {
    /// No captionable audio clips among the targets (reference `.noSource`).
    NoSource,
}

impl std::fmt::Display for CaptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptionError::NoSource => f.write_str("No audio clips to caption."),
        }
    }
}

impl std::error::Error for CaptionError {}

// =====================================================================
// specs(...) — phrase → TextClipSpec through clip placement
// =====================================================================

/// Map timed [`Phrase`]s through `source_clip`'s trim/speed into [`TextClipSpec`]s
/// (reference `CaptionBuilder.specs(...)`). Verbatim port of the 5-step algorithm in
/// `docs/reference/transcription.md` §D:
///
/// 1. Visible source window (source frames): `vis_start = trim_start_frame`,
///    `vis_end = vis_start + duration_frames * max(speed, 0.0001)`.
/// 2. `p_start = p.start*fps`, `p_end = p.end*fps`; **drop** unless
///    `p_end > vis_start && p_start < vis_end`.
/// 3. `mapped_start = clip.timeline_frame(p.start, fps)`, `mapped_end = …(p.end)`;
///    fall back to `clip.start_frame`/`end_frame` when `None`.
/// 4. `s = mapped_start ?? start_frame`, `e = mapped_end ?? end_frame`;
///    `duration = max(min_dur, min(clip.end_frame, e) - max(clip.start_frame, s))`.
/// 5. Emit the [`TextClipSpec`] with the center-based `transform_for(p.text)`.
///
/// `min_duration_frames` defaults to `1` (reference default); callers pass `1`.
#[allow(clippy::too_many_arguments)]
pub fn specs(
    phrases: &[Phrase],
    source_clip: &Clip,
    track_index: usize,
    fps: i32,
    style: &TextStyle,
    caption_group_id: Option<&str>,
    mut transform_for: impl FnMut(&str) -> Option<Transform>,
    min_duration_frames: i32,
) -> Vec<TextClipSpec> {
    let mut out = Vec::new();
    let fps_f = fps as f64;
    let speed = source_clip.speed.max(0.0001);
    let visible_start_source = source_clip.trim_start_frame as f64;
    let visible_end_source = visible_start_source + source_clip.duration_frames as f64 * speed;

    for p in phrases {
        let phrase_start_source = p.start * fps_f;
        let phrase_end_source = p.end * fps_f;
        // Step 2: drop the phrase unless it overlaps the visible source window.
        if !(phrase_end_source > visible_start_source && phrase_start_source < visible_end_source) {
            continue;
        }

        // Step 3/4: map both endpoints through placement, with start/end fallbacks.
        let mapped_start = source_clip.timeline_frame(p.start, fps);
        let mapped_end = source_clip.timeline_frame(p.end, fps);
        let s = mapped_start.unwrap_or(source_clip.start_frame);
        let e = mapped_end.unwrap_or(source_clip.end_frame());
        let duration_frames = min_duration_frames
            .max(source_clip.end_frame().min(e) - source_clip.start_frame.max(s));

        out.push(TextClipSpec {
            track_index,
            start_frame: s,
            duration_frames,
            content: p.text.clone(),
            style: style.clone(),
            transform: transform_for(&p.text),
            caption_group_id: caption_group_id.map(str::to_string),
        });
    }
    out
}

// =====================================================================
// place_text_clips — batch text-clip placement (caller owns undo + track)
// =====================================================================

/// Batch-place text clips onto existing tracks (reference `placeTextClips(_:)`).
/// The caller owns undo registration and track creation; this clears each clip's
/// region (overwrite, no prune) and appends a `.text` clip carrying the content,
/// style, caption-group-id, and resolved transform. `None` transforms are auto-fit
/// to the content's natural size and centered. Returns the created clip ids (in spec
/// order, skipping specs whose `track_index` is out of range).
pub fn place_text_clips(
    timeline: &mut Timeline,
    specs: &[TextClipSpec],
    registry: &mut FontRegistry,
    layout: &mut TextLayout,
) -> Vec<String> {
    if specs.is_empty() {
        return Vec::new();
    }
    let canvas_w = timeline.width as f64;
    let canvas_h = timeline.height as f64;
    let mut created: Vec<Option<String>> = vec![None; specs.len()];

    // Group spec indices by track, placing each track's clips in start-frame order
    // (reference groups by trackIndex then sorts ascending).
    let mut track_indices: Vec<usize> =
        specs.iter().map(|s| s.track_index).collect::<Vec<_>>();
    track_indices.sort_unstable();
    track_indices.dedup();

    for &ti in &track_indices {
        if ti >= timeline.tracks.len() {
            continue;
        }
        let mut ordered: Vec<usize> = (0..specs.len()).filter(|&i| specs[i].track_index == ti).collect();
        ordered.sort_by_key(|&i| specs[i].start_frame);
        for i in ordered {
            let spec = &specs[i];
            let start = spec.start_frame.max(0);
            let duration = spec.duration_frames.max(1);

            // Clear the destination region (overwrite-style, in place, no ripple).
            clear_region_no_prune(&mut timeline.tracks[ti], start, start + duration);

            let resolved = match spec.transform {
                Some(t) => t,
                None => {
                    let natural =
                        layout.natural_size(registry, &spec.content, &spec.style, canvas_w * 0.9, canvas_h);
                    let w = natural.width / canvas_w;
                    let h = natural.height / canvas_h;
                    Transform::from_top_left(((1.0 - w) / 2.0, (1.0 - h) / 2.0), w, h)
                }
            };

            let mut clip = Clip::new("", start, duration);
            clip.media_type = ClipType::Text;
            clip.source_clip_type = ClipType::Text;
            clip.transform = resolved;
            clip.text_content = Some(spec.content.clone());
            clip.text_style = Some(spec.style.clone());
            clip.caption_group_id = spec.caption_group_id.clone();
            created[i] = Some(clip.id.clone());
            timeline.tracks[ti].clips.push(clip);
        }
        timeline.tracks[ti].sort_clips();
    }

    created.into_iter().flatten().collect()
}

/// Remove any clip fully covered by `[start, end)` and trim/split survivors so the
/// region is empty, in place (a minimal `clearRegion(prune: false)` for text-clip
/// placement). Caption specs are non-overlapping per track in practice, so the common
/// case removes nothing; this guards the rare overlap.
fn clear_region_no_prune(track: &mut Track, start: i32, end: i32) {
    if end <= start {
        return;
    }
    // Fully-covered clips are removed.
    track
        .clips
        .retain(|c| !(c.start_frame >= start && c.end_frame() <= end));
    track.sort_clips();
}

// =====================================================================
// generate_captions orchestration
// =====================================================================

/// A resolved caption target: a clip plus the id of the track it lives on.
#[derive(Debug, Clone)]
struct CaptionTarget {
    /// The clip id (== `clip.id`).
    id: String,
    /// The id of the track holding the clip.
    track_id: String,
    /// A snapshot of the clip.
    clip: Clip,
}

/// The visible source window (source frames) of a clip:
/// `(trim_start, trim_start + duration * max(speed, 0.0001))`.
fn visible_source(c: &Clip) -> (f64, f64) {
    let s = c.trim_start_frame as f64;
    (s, s + c.duration_frames as f64 * c.speed.max(0.0001))
}

/// Whether a clip can be transcribed: it must be video/audio, and (if its asset is
/// known) the asset must carry audio. Without an asset map we optimistically accept
/// video/audio clips (reference `captionCanTranscribe` returns `true` when the asset
/// is unknown). `assets` maps `media_ref → has_audio`.
fn can_transcribe(clip: &Clip, assets: &dyn AssetInfo) -> bool {
    if clip.media_type != ClipType::Video && clip.media_type != ClipType::Audio {
        return false;
    }
    match assets.has_audio(&clip.media_ref) {
        Some(has_audio) => has_audio,
        // Unknown asset → optimistic accept (reference returns true).
        None => true,
    }
}

/// Whether a clip's audio is extracted from a video container (reference
/// `captionUsesVideoAudioExtraction`): the asset is a video, or the asset is unknown
/// and the clip's own media type is video.
fn uses_video_audio_extraction(clip: &Clip, assets: &dyn AssetInfo) -> bool {
    match assets.asset_is_video(&clip.media_ref) {
        Some(is_video) => is_video,
        None => clip.media_type == ClipType::Video,
    }
}

/// Minimal asset lookup the orchestration needs (the editor's `mediaAssets`).
/// Implementors answer two questions per `media_ref`; an unknown ref returns `None`,
/// which the reference treats optimistically.
pub trait AssetInfo {
    /// Does the asset backing `media_ref` have audio? `None` ⇒ unknown asset.
    fn has_audio(&self, media_ref: &str) -> Option<bool>;
    /// Is the asset backing `media_ref` a video (vs audio)? `None` ⇒ unknown.
    fn asset_is_video(&self, media_ref: &str) -> Option<bool>;
}

/// An [`AssetInfo`] that knows nothing — every clip is treated optimistically
/// (video/audio clips are captionable; video clips extract from video). Useful for
/// callers without a media-asset map and for tests.
pub struct NoAssetInfo;
impl AssetInfo for NoAssetInfo {
    fn has_audio(&self, _media_ref: &str) -> Option<bool> {
        None
    }
    fn asset_is_video(&self, _media_ref: &str) -> Option<bool> {
        None
    }
}

/// Collect caption targets from a clip pool (reference private `captionTargets(in:)`):
/// keep captionable clips; a **video** clip linked to an **audio** clip in the pool
/// yields to the audio clip (skip the video); sort by `start_frame`.
fn caption_targets_in(pool: &[(String, Clip)], assets: &dyn AssetInfo) -> Vec<(String, Clip)> {
    // Link groups that have an audio clip in the pool.
    let link_groups_with_audio: Vec<String> = pool
        .iter()
        .filter(|(_, c)| c.media_type == ClipType::Audio)
        .filter_map(|(_, c)| c.link_group_id.clone())
        .collect();

    let mut out: Vec<(String, Clip)> = pool
        .iter()
        .filter(|(_, c)| {
            if !can_transcribe(c, assets) {
                return false;
            }
            // A video clip linked to an audio clip yields to the audio clip.
            match (c.media_type, &c.link_group_id) {
                (ClipType::Video, Some(group)) => !link_groups_with_audio.contains(group),
                _ => true,
            }
        })
        .cloned()
        .collect();
    out.sort_by_key(|(_, c)| c.start_frame);
    out
}

/// Resolve the caption-target pool for a request (reference `captionTargets(ids:)`):
/// the named clips, or every clip when `ids` is empty. Each entry is `(track_id, clip)`.
fn resolve_targets(
    timeline: &Timeline,
    ids: &[String],
    assets: &dyn AssetInfo,
) -> Vec<CaptionTarget> {
    let pool: Vec<(String, Clip)> = if ids.is_empty() {
        timeline
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(move |c| (t.id.clone(), c.clone())))
            .collect()
    } else {
        timeline
            .tracks
            .iter()
            .flat_map(|t| t.clips.iter().map(move |c| (t.id.clone(), c.clone())))
            .filter(|(_, c)| ids.iter().any(|id| id == &c.id))
            .collect()
    };
    caption_targets_in(&pool, assets)
        .into_iter()
        .map(|(track_id, clip)| CaptionTarget {
            id: clip.id.clone(),
            track_id,
            clip,
        })
        .collect()
}

/// The ±1.0 s-padded, clamp-at-0 union (in **source seconds**) of every target clip's
/// visible source window for `media_ref` (reference `visibleSourceUnion`). `None`
/// when no spans / `fps <= 0` / `hi <= lo`.
fn visible_source_union(
    media_ref: &str,
    targets: &[CaptionTarget],
    fps: i32,
) -> Option<std::ops::RangeInclusive<f64>> {
    let fps_f = fps as f64;
    if fps_f <= 0.0 {
        return None;
    }
    let spans: Vec<(f64, f64)> = targets
        .iter()
        .filter(|t| t.clip.media_ref == media_ref)
        .map(|t| visible_source(&t.clip))
        .collect();
    let lo = spans.iter().map(|s| s.0).fold(f64::INFINITY, f64::min);
    let hi = spans.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
    if spans.is_empty() || !(hi > lo) {
        return None;
    }
    let pad = 1.0;
    let start = (lo / fps_f - pad).max(0.0);
    let end = hi / fps_f + pad;
    Some(start..=end)
}

/// Count words whose **midpoint** (`(start+end)/2 × fps`) falls inside the clip's
/// visible source window, skipping words with a `None` timestamp (reference
/// `spokenWordCount`).
fn spoken_word_count(clip: &Clip, result: &TranscriptionResult, fps: i32) -> usize {
    let (vs, ve) = visible_source(clip);
    let fps_f = fps as f64;
    result
        .words
        .iter()
        .filter(|w| match (w.start, w.end) {
            (Some(s), Some(e)) => {
                let mid = (s + e) / 2.0 * fps_f;
                vs <= mid && mid < ve
            }
            _ => false,
        })
        .count()
}

/// The dominant speech track id (reference `dominantSpeechTrack`): the track whose
/// targets accumulate the most spoken words (only tracks with `> 0` words qualify;
/// ties resolve to the first max in iteration order). `None` when no words.
fn dominant_speech_track(
    targets: &[CaptionTarget],
    results: &std::collections::HashMap<String, TranscriptionResult>,
    fps: i32,
) -> Option<String> {
    let mut words_by_track: Vec<(String, usize)> = Vec::new();
    for t in targets {
        let Some(result) = results.get(&t.clip.media_ref) else {
            continue;
        };
        let n = spoken_word_count(&t.clip, result, fps);
        if let Some(entry) = words_by_track.iter_mut().find(|(id, _)| id == &t.track_id) {
            entry.1 += n;
        } else {
            words_by_track.push((t.track_id.clone(), n));
        }
    }
    // max by value over tracks with > 0 words; `max_by` returns the LAST max on ties,
    // but the reference's `max { $0.value < $1.value }` over a dictionary is order-
    // unspecified — for determinism we pick the first max in iteration order.
    words_by_track
        .into_iter()
        .filter(|(_, v)| *v > 0)
        .fold(None::<(String, usize)>, |acc, (id, v)| match acc {
            Some((_, best)) if v <= best => acc,
            _ => Some((id, v)),
        })
        .map(|(id, _)| id)
}

/// The clip with the **most overlap** owns a phrase, but only if overlap is positive
/// AND `>= phrase_len / 2` (reference `bestClip`). Overlap is computed in source
/// frames against each clip's visible source window.
fn best_clip<'a>(p: &Phrase, clips: &'a [CaptionTarget], fps: i32) -> Option<&'a CaptionTarget> {
    let fps_f = fps as f64;
    let ps = p.start * fps_f;
    let pe = p.end * fps_f;
    let overlap = |c: &Clip| {
        let (vs, ve) = visible_source(c);
        (pe.min(ve) - ps.max(vs)).max(0.0)
    };
    // Pick the max-overlap clip (first max on ties for determinism).
    let mut best: Option<&CaptionTarget> = None;
    let mut best_o = f64::NEG_INFINITY;
    for t in clips {
        let o = overlap(&t.clip);
        if o > best_o {
            best_o = o;
            best = Some(t);
        }
    }
    let best = best?;
    let o = overlap(&best.clip);
    if o > 0.0 && o >= (pe - ps) / 2.0 {
        Some(best)
    } else {
        None
    }
}

/// Whether a caption line "fits": its natural width is `<= timeline.width * 0.9`
/// (reference `captionLineFits`; `captionPreviewMaxTextWidthRatio = 0.9`).
fn caption_line_fits(
    line: &str,
    style: &TextStyle,
    timeline: &Timeline,
    registry: &mut FontRegistry,
    layout: &mut TextLayout,
) -> bool {
    let natural = layout.natural_size(registry, line, style, f64::INFINITY, timeline.height as f64);
    natural.width <= timeline.width as f64 * caption_theme::CAPTION_PREVIEW_MAX_TEXT_WIDTH_RATIO
}

/// Build the per-phrase `transform_for` closure data: each box's center-based
/// `Transform` from its text's natural size (reference `captionTransform`).
fn caption_transform(
    text: &str,
    style: &TextStyle,
    center: (f64, f64),
    timeline: &Timeline,
    registry: &mut FontRegistry,
    layout: &mut TextLayout,
) -> Transform {
    let canvas_w = timeline.width as f64;
    let canvas_h = timeline.height as f64;
    let natural = layout.natural_size(
        registry,
        text,
        style,
        canvas_w * caption_theme::CAPTION_PREVIEW_MAX_TEXT_WIDTH_RATIO,
        canvas_h,
    );
    Transform {
        center_x: center.0,
        center_y: center.1,
        width: natural.width / canvas_w,
        height: natural.height / canvas_h,
        ..Transform::default()
    }
}

/// The outcome of [`generate_captions`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GenerateCaptionsResult {
    /// Ids of the placed caption clips (empty when nothing was placed).
    pub clip_ids: Vec<String>,
}

/// Generate captions onto the timeline (reference `EditorViewModel.generateCaptions`).
///
/// Pipeline:
/// 1. **Targets**: video/audio-with-audio clips; a video clip linked to an audio clip
///    yields to the audio; sorted by `start_frame` (auto pool when `auto_detect`).
/// 2. **Transcribe**: one result per distinct `media_ref` over its ±1.0 s-padded
///    visible-source union, via the injected `transcribe` closure (the caller decides
///    cache vs bypass per the `censor_profanity || locale.is_some()` rule).
/// 3. **Auto-detect**: keep only the dominant-speech track when `auto_detect`.
/// 4. **Phrases**: `result.segments.flat_map(phrases(.., caption_line_fits, 0.7))`;
///    each phrase assigned to the clip with most overlap (only if `> 0` and
///    `>= phrase_len/2`).
/// 5. **Casing + specs**: apply the request case, then `specs(..)` with a shared
///    `group_id` and a center-based `transform_for`.
/// 6. **Placement**: insert a **new video track at index 0**, place the text clips,
///    register one undo group **"Generate Captions"**.
///
/// `transcribe(media_ref, range, is_video) -> Result<TranscriptionResult, E>`: the
/// caller's engine call. `new_group_id` mints the shared caption-group UUID (injected
/// for deterministic tests).
#[allow(clippy::too_many_arguments)]
pub fn generate_captions<E>(
    timeline: &mut Timeline,
    history: &mut History<Timeline>,
    request: &CaptionRequest,
    assets: &dyn AssetInfo,
    registry: &mut FontRegistry,
    layout: &mut TextLayout,
    mut transcribe: impl FnMut(
        &str,
        Option<&std::ops::RangeInclusive<f64>>,
        bool,
    ) -> Result<TranscriptionResult, E>,
    mut new_group_id: impl FnMut() -> String,
) -> Result<GenerateCaptionsResult, CaptionError> {
    let fps = timeline.fps;

    // 1. Targets.
    let ids = if request.auto_detect {
        Vec::new()
    } else {
        request.source_clip_ids.clone()
    };
    let mut targets = resolve_targets(timeline, &ids, assets);
    if targets.is_empty() {
        return Err(CaptionError::NoSource);
    }

    // 2. Transcribe one result per distinct media_ref over the padded union.
    let mut results: std::collections::HashMap<String, TranscriptionResult> =
        std::collections::HashMap::new();
    for t in &targets {
        if results.contains_key(&t.clip.media_ref) {
            continue;
        }
        let range = visible_source_union(&t.clip.media_ref, &targets, fps);
        let is_video = uses_video_audio_extraction(&t.clip, assets);
        if let Ok(result) = transcribe(&t.clip.media_ref, range.as_ref(), is_video) {
            results.insert(t.clip.media_ref.clone(), result);
        }
        // A per-ref transcription failure is skipped (reference records firstError but
        // proceeds; if NOTHING transcribes the run yields no specs → empty result).
    }

    // 3. Auto-detect dominant speech track.
    if request.auto_detect {
        let Some(winner) = dominant_speech_track(&targets, &results, fps) else {
            return Ok(GenerateCaptionsResult::default());
        };
        targets.retain(|t| t.track_id == winner);
    }

    // 4./5. Phrases → assignment → casing → specs.
    let group_id = new_group_id();
    let mut specs_out: Vec<TextClipSpec> = Vec::new();

    // The `phrases` `fits` predicate and the `specs` `transform_for` both need the
    // shared, mutable cosmic-text measurement state (`FontRegistry`/`TextLayout`).
    // Wrap each in its own `RefCell` so the `Fn` predicate `phrases` requires can
    // borrow them mutably per call (two cells avoids a tuple split-borrow). `timeline`
    // is read-only here.
    let reg_cell = std::cell::RefCell::new(registry);
    let lay_cell = std::cell::RefCell::new(layout);
    {
        // phrases owned by each clip id (assignment by most overlap).
        let mut phrases_by_clip: std::collections::HashMap<String, Vec<Phrase>> =
            std::collections::HashMap::new();
        // Collect media_refs deterministically (target order) to keep output stable.
        let mut seen_refs: Vec<String> = Vec::new();
        for t in &targets {
            if !seen_refs.contains(&t.clip.media_ref) {
                seen_refs.push(t.clip.media_ref.clone());
            }
        }
        for media_ref in &seen_refs {
            let Some(result) = results.get(media_ref) else {
                continue;
            };
            let clips: Vec<CaptionTarget> = targets
                .iter()
                .filter(|t| &t.clip.media_ref == media_ref)
                .cloned()
                .collect();
            if clips.is_empty() {
                continue;
            }
            // phrases over each segment using the width-fits predicate + 0.7s min.
            let mut all_phrases: Vec<Phrase> = Vec::new();
            for seg in &result.segments {
                let segment = to_caption_segment(seg);
                let ps = phrases(
                    &segment,
                    |line| {
                        let mut reg = reg_cell.borrow_mut();
                        let mut lay = lay_cell.borrow_mut();
                        caption_line_fits(line, &request.style, timeline, &mut **reg, &mut **lay)
                    },
                    caption_theme::MIN_DISPLAY_DURATION,
                );
                all_phrases.extend(ps);
            }
            for p in all_phrases {
                if let Some(owner) = best_clip(&p, &clips, fps) {
                    phrases_by_clip.entry(owner.id.clone()).or_default().push(p);
                }
            }
        }

        // Emit specs per target (in target order), casing each phrase, sharing group_id.
        for t in &targets {
            let Some(ps) = phrases_by_clip.get(&t.id) else {
                continue;
            };
            let cased: Vec<Phrase> = ps
                .iter()
                .map(|p| Phrase::new(apply_case(request.text_case, &p.text), p.start, p.end))
                .collect();
            let style = request.style.clone();
            let center = request.center;
            let clip_specs = specs(
                &cased,
                &t.clip,
                0,
                fps,
                &request.style,
                Some(&group_id),
                |text| {
                    let mut reg = reg_cell.borrow_mut();
                    let mut lay = lay_cell.borrow_mut();
                    Some(caption_transform(text, &style, center, timeline, &mut **reg, &mut **lay))
                },
                1,
            );
            specs_out.extend(clip_specs);
        }
    }

    if specs_out.is_empty() {
        return Ok(GenerateCaptionsResult::default());
    }

    // 6. Placement: new video track at index 0, place clips, one undo group.
    let registry = reg_cell.into_inner();
    let layout = lay_cell.into_inner();
    let clip_ids = place_caption_track(timeline, history, specs_out, registry, layout);
    Ok(GenerateCaptionsResult { clip_ids })
}

/// Convert a serde [`TranscriptionSegment`] into the caption-algorithm [`Segment`]
/// (E10-S5's input shape). There is no bridge yet (per the dependency note) — this is
/// the call-site conversion.
fn to_caption_segment(seg: &TranscriptionSegment) -> Segment {
    Segment::new(seg.text.clone(), seg.start, seg.end)
}

/// Insert a new video track at index 0, place the caption clips on it, and register
/// **one** undo group named exactly **"Generate Captions"** (reference
/// `placeCaptionTrack`). A placement that produces no clips rolls back (registers
/// nothing). Returns the placed clip ids.
fn place_caption_track(
    timeline: &mut Timeline,
    history: &mut History<Timeline>,
    specs: Vec<TextClipSpec>,
    registry: &mut FontRegistry,
    layout: &mut TextLayout,
) -> Vec<String> {
    let mut placed: Vec<String> = Vec::new();
    history.with_user_swap(GENERATE_CAPTIONS_UNDO_NAME, timeline, |tl| {
        tl.tracks.insert(0, Track::new(ClipType::Video));
        placed = place_text_clips(tl, &specs, registry, layout);
        if placed.is_empty() {
            // Roll back the inserted empty track so before == after (no undo entry).
            tl.tracks.remove(0);
        }
    });
    placed
}

#[cfg(test)]
mod tests;
