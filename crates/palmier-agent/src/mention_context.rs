//! `@`-mention context-hint construction + image inlining (E8-S5).
//!
//! Ports `AgentMentionContext.swift` + the `inlineImageBlocks` / context-hint half
//! of `AgentService.apiMessages`. For a user turn that references `@`-mentions,
//! the loop **prepends** to that turn's wire content, at index 0, in order:
//! (1) a **context-hint text block** (a JSON summary of the referenced assets /
//! clips / ranges), then (2) any **inlined image blocks** (base64). This is an
//! **in-app-loop-only** augmentation — it is never pushed into the MCP server's
//! `instructions`, and an external MCP client never receives it
//! (`agent-instructions.md` lines 53-54, 189-190).
//!
//! ## The three mention kinds & their exact JSON (`agent-panel.md` lines 129-141)
//! Each hint entry is `{mention:"@name", kind, …}`:
//! - **`mediaAsset`** — `mediaRef` present, `clipId` absent. Adds `mediaRef`,
//!   `type` (ClipType raw value), and — when the image was inlined / failed —
//!   `inlined:true` / `inlineError:"<reason>"`.
//! - **`timelineClip`** — `clipId` present. Adds `mediaRef`/`type` (as above) plus
//!   `clipId` and a `clip` summary object resolved from the editor (the
//!   [`MentionResolver`] seam).
//! - **`timelineRange`** — `timelineRange` present. Adds a `timelineRange` summary
//!   (half-open frame range: `startFrame` inclusive, `endFrame` exclusive,
//!   `rangeSemantics:"startInclusiveEndExclusive"`).
//!
//! ## The two seams (so this is testable with no editor / no real media)
//! - [`AssetBytesSource`](crate::image_encoder::AssetBytesSource) — resolves a
//!   `media_ref` to image bytes for inlining (reused from [`crate::image_encoder`]).
//! - [`MentionResolver`] — resolves a `clip_id` to its `clip` summary JSON
//!   (reference `clipSummary(for:editor:)`). The real adapter (E8-S9) reads the
//!   editor timeline; tests pass a map. A [`MentionEnricher`] bundles both seams.
//!
//! ## Hint text format (exact — `agent-panel.md` lines 136-138)
//! `"Referenced assets and timeline context in this message: <JSON array>.<notes>"`
//! where `<JSON array>` is the sorted-key JSON of the entries and `<notes>` is a
//! leading space + the space-joined applicable note strings (or empty). The
//! `mentionNotes` strings are ported **verbatim** from the reference.

use serde_json::{json, Map, Value};

use crate::image_encoder::{AssetBytesSource, ImageEncoder};
use crate::model::{AgentMention, AgentTimelineRangeMention};
use palmier_model::ClipType;

/// The note appended when one or more assets were inlined as image blocks
/// (reference `mentionNotes`, verbatim).
pub const NOTE_INLINED: &str = "Assets marked \"inlined\": true are attached as image blocks in this message — do not call inspect_media for them.";

/// The note appended when one or more image inlines failed (reference, verbatim).
pub const NOTE_INLINE_ERROR: &str = "Assets with \"inlineError\" could not be attached; tell the user the image could not be read rather than describing it.";

/// The note appended when any mention references a timeline clip (reference,
/// verbatim).
pub const NOTE_CLIPS: &str = "Entries with \"clipId\" refer to timeline clips; use clipId for timeline edits and pass it to inspect_media when inspecting visible source media.";

/// The note appended when any mention references a timeline range (reference,
/// verbatim).
pub const NOTE_RANGES: &str = "Entries with \"timelineRange\" refer to selected timeline time spans; their frame ranges are half-open: startFrame inclusive, endFrame exclusive.";

/// The exact inline-failure reason when the asset is not in the media library
/// (reference, verbatim).
pub const FAIL_NOT_IN_LIBRARY: &str = "asset not in media library";

/// The exact inline-failure reason when the asset bytes can't be read/decoded
/// (reference, verbatim).
pub const FAIL_UNREADABLE: &str = "could not read or decode image file";

impl AgentTimelineRangeMention {
    /// The `timelineRange` summary object the hint embeds (reference
    /// `AgentTimelineRangeMention.summary`). Keys match the reference 1:1; serde's
    /// `Value` map sorts them (parity with `.sortedKeys`).
    #[must_use]
    pub fn summary(&self) -> Value {
        json!({
            "startFrame": self.start_frame,
            "endFrame": self.end_frame,
            "durationFrames": self.duration_frames,
            "fps": self.fps,
            "startTimecode": self.start_timecode,
            "endTimecode": self.end_timecode,
            "durationTimecode": self.duration_timecode,
            "rangeSemantics": self.range_semantics,
        })
    }
}

/// The `clip` summary the editor resolves for a `timelineClip` mention (reference
/// `clipSummary(for:editor:)`).
pub trait MentionResolver: Send + Sync {
    /// Resolve `clip_id` to its `clip` summary JSON object. Returns the reference's
    /// error shapes when the clip can't be resolved:
    /// `{clipId, error:"editor unavailable"}` / `{clipId, error:"clip not found"}`.
    fn clip_summary(&self, clip_id: &str) -> Value;
}

/// Bundles the two seams the enrichment needs: clip-summary resolution
/// ([`MentionResolver`]) and image-byte resolution
/// ([`AssetBytesSource`](crate::image_encoder::AssetBytesSource)).
///
/// The real implementation (E8-S9) wraps the editor + media library; tests pass a
/// pair of in-memory stubs. [`AgentLoop`](crate::loop_run::AgentLoop) holds an
/// `Option<&dyn MentionEnricher>` so a loop with no editor still projects
/// (mentions just carry no `clip` summary / no inlined image — matching the
/// reference's `editor == nil` branches).
pub trait MentionEnricher: MentionResolver + AssetBytesSource {
    /// Borrow `self` as its [`MentionResolver`] half (clip-summary resolution).
    fn as_resolver(&self) -> &dyn MentionResolver;
    /// Borrow `self` as its [`AssetBytesSource`] half (image inlining).
    fn as_asset_source(&self) -> &dyn AssetBytesSource;
}

/// Blanket impl: anything that is both a resolver and an asset source is an
/// enricher. The upcast accessors avoid relying on `dyn`-trait upcasting at the
/// call sites (the loop's `api_messages` needs each half as a separate `&dyn`).
impl<T: MentionResolver + AssetBytesSource> MentionEnricher for T {
    fn as_resolver(&self) -> &dyn MentionResolver {
        self
    }
    fn as_asset_source(&self) -> &dyn AssetBytesSource {
        self
    }
}

/// The result of inlining the image mentions of one user turn (reference
/// `AgentMentionContext.InlinedMentions`).
#[derive(Debug, Default, Clone)]
pub struct InlinedMentions {
    /// The Anthropic `image` wire blocks to prepend, in mention order.
    pub blocks: Vec<Value>,
    /// `media_ref`s that were successfully inlined (→ `inlined:true` in the hint).
    pub inlined_ids: std::collections::HashSet<String>,
    /// `media_ref → reason` for inlines that failed (→ `inlineError` in the hint).
    pub failures: std::collections::HashMap<String, String>,
}

/// Mentions whose `@displayName` literally appears in `text` (reference
/// `referencedMentions` — `text.contains("@\(displayName)")`).
#[must_use]
pub fn referenced_mentions(mentions: &[AgentMention], text: &str) -> Vec<AgentMention> {
    mentions
        .iter()
        .filter(|m| text.contains(&format!("@{}", m.display_name)))
        .cloned()
        .collect()
}

/// Collapse spaces and `-` runs to a single `-`, trimmed, so the inserted
/// `@token` stays one word (reference `AgentMention.makeDisplayName(from:)`).
#[must_use]
pub fn make_display_name(raw: &str) -> String {
    let mut result = String::new();
    let mut last_was_dash = false;
    for ch in raw.chars() {
        if ch.is_whitespace() || ch == '-' {
            if !last_was_dash {
                result.push('-');
            }
            last_was_dash = true;
        } else {
            result.push(ch);
            last_was_dash = false;
        }
    }
    result.trim_matches('-').to_string()
}

/// Inline the `image`-type mentions of a turn (reference `inlineImageBlocks`).
///
/// For each mention with `type == image` and a `media_ref`: attempt
/// [`ImageEncoder::encode`]; on success push the base64 image block + mark the id
/// `inlined`; on a missing asset record [`FAIL_NOT_IN_LIBRARY`]; on a
/// present-but-unreadable asset record [`FAIL_UNREADABLE`]. With no asset source
/// (`None` — the reference's `editor == nil` branch), every image mention fails
/// with `"editor unavailable"`.
#[must_use]
pub fn inline_image_blocks(
    mentions: &[AgentMention],
    source: Option<&dyn AssetBytesSource>,
) -> InlinedMentions {
    let mut out = InlinedMentions::default();

    let Some(source) = source else {
        for m in mentions
            .iter()
            .filter(|m| m.clip_type == Some(ClipType::Image))
        {
            if let Some(media_ref) = &m.media_ref {
                out.failures
                    .insert(media_ref.clone(), "editor unavailable".to_string());
            }
        }
        return out;
    };

    for m in mentions
        .iter()
        .filter(|m| m.clip_type == Some(ClipType::Image))
    {
        let Some(media_ref) = &m.media_ref else {
            continue;
        };
        // Distinguish "not in library" (source has no such asset) from
        // "unreadable" (asset present, bytes don't decode / don't fit).
        if source.load(media_ref).is_none() {
            out.failures
                .insert(media_ref.clone(), FAIL_NOT_IN_LIBRARY.to_string());
            continue;
        }
        match ImageEncoder::encode(source, media_ref) {
            Some(encoded) => {
                out.blocks.push(json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": encoded.mime,
                        "data": encoded.base64(),
                    },
                }));
                out.inlined_ids.insert(media_ref.clone());
            }
            None => {
                out.failures
                    .insert(media_ref.clone(), FAIL_UNREADABLE.to_string());
            }
        }
    }
    out
}

/// Build the JSON entry array for the hint (reference `mentionEntries`).
fn mention_entries(
    mentions: &[AgentMention],
    resolver: Option<&dyn MentionResolver>,
    inlined: &InlinedMentions,
) -> Vec<Value> {
    mentions
        .iter()
        .map(|m| {
            let mut entry = Map::new();
            entry.insert("mention".to_string(), json!(format!("@{}", m.display_name)));

            // timelineRange short-circuits (reference returns early).
            if let Some(range) = &m.timeline_range {
                entry.insert("kind".to_string(), json!("timelineRange"));
                entry.insert("timelineRange".to_string(), range.summary());
                return Value::Object(entry);
            }

            // mediaAsset vs timelineClip by clipId presence.
            entry.insert(
                "kind".to_string(),
                json!(if m.clip_id.is_none() {
                    "mediaAsset"
                } else {
                    "timelineClip"
                }),
            );
            if let Some(media_ref) = &m.media_ref {
                entry.insert("mediaRef".to_string(), json!(media_ref));
                if inlined.inlined_ids.contains(media_ref) {
                    entry.insert("inlined".to_string(), json!(true));
                }
                if let Some(reason) = inlined.failures.get(media_ref) {
                    entry.insert("inlineError".to_string(), json!(reason));
                }
            }
            if let Some(clip_type) = m.clip_type {
                // ClipType serializes as its lowercase raw value.
                entry.insert("type".to_string(), json!(clip_type));
            }
            if let Some(clip_id) = &m.clip_id {
                entry.insert("clipId".to_string(), json!(clip_id));
                let summary = match resolver {
                    Some(r) => r.clip_summary(clip_id),
                    None => json!({ "clipId": clip_id, "error": "editor unavailable" }),
                };
                entry.insert("clip".to_string(), summary);
            }
            Value::Object(entry)
        })
        .collect()
}

/// The applicable note strings, in reference order (reference `mentionNotes`).
fn mention_notes(mentions: &[AgentMention], inlined: &InlinedMentions) -> Vec<&'static str> {
    let mut notes = Vec::new();
    if !inlined.inlined_ids.is_empty() {
        notes.push(NOTE_INLINED);
    }
    if !inlined.failures.is_empty() {
        notes.push(NOTE_INLINE_ERROR);
    }
    if mentions.iter().any(AgentMention::references_timeline_clips) {
        notes.push(NOTE_CLIPS);
    }
    if mentions.iter().any(AgentMention::references_timeline_range) {
        notes.push(NOTE_RANGES);
    }
    notes
}

/// The full context-hint string for a turn (reference `AgentMentionContext.hint`).
///
/// `<JSON array>` is the sorted-key JSON of the entries (serde's `Value` map is
/// `BTreeMap`-backed → sorted, matching `.sortedKeys`); `<notes>` is a leading
/// space + the space-joined applicable notes (empty when none apply).
#[must_use]
pub fn hint(
    mentions: &[AgentMention],
    resolver: Option<&dyn MentionResolver>,
    inlined: &InlinedMentions,
) -> String {
    let entries = mention_entries(mentions, resolver, inlined);
    let json_array = serde_json::to_string(&Value::Array(entries))
        .unwrap_or_else(|_| "[]".to_string());
    let notes = mention_notes(mentions, inlined);
    let suffix = if notes.is_empty() {
        String::new()
    } else {
        format!(" {}", notes.join(" "))
    };
    format!("Referenced assets and timeline context in this message: {json_array}.{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_encoder::test_support::{png, MapSource};
    use crate::model::AgentTimelineRangeMention;
    use uuid::Uuid;

    /// A combined enricher backed by a clip-summary map + an asset-bytes map.
    struct StubEnricher {
        clips: std::collections::HashMap<String, Value>,
        assets: MapSource,
    }

    impl StubEnricher {
        fn new() -> Self {
            Self {
                clips: std::collections::HashMap::new(),
                assets: MapSource::new(),
            }
        }
    }

    impl MentionResolver for StubEnricher {
        fn clip_summary(&self, clip_id: &str) -> Value {
            self.clips
                .get(clip_id)
                .cloned()
                .unwrap_or_else(|| json!({ "clipId": clip_id, "error": "clip not found" }))
        }
    }

    impl AssetBytesSource for StubEnricher {
        fn load(&self, media_ref: &str) -> Option<crate::image_encoder::AssetBytes> {
            self.assets.load(media_ref)
        }
    }

    fn media_asset_mention(name: &str, media_ref: &str, ct: ClipType) -> AgentMention {
        AgentMention {
            id: Uuid::nil(),
            display_name: name.to_string(),
            media_ref: Some(media_ref.to_string()),
            clip_type: Some(ct),
            clip_id: None,
            timeline_range: None,
        }
    }

    fn timeline_clip_mention(name: &str, media_ref: &str, clip_id: &str) -> AgentMention {
        AgentMention {
            id: Uuid::nil(),
            display_name: name.to_string(),
            media_ref: Some(media_ref.to_string()),
            clip_type: Some(ClipType::Video),
            clip_id: Some(clip_id.to_string()),
            timeline_range: None,
        }
    }

    fn timeline_range_mention(name: &str) -> AgentMention {
        AgentMention {
            id: Uuid::nil(),
            display_name: name.to_string(),
            media_ref: None,
            clip_type: None,
            clip_id: None,
            timeline_range: Some(AgentTimelineRangeMention {
                start_frame: 10,
                end_frame: 30,
                duration_frames: 20,
                fps: 30,
                start_timecode: "00:00:00:10".to_string(),
                end_timecode: "00:00:01:00".to_string(),
                duration_timecode: "00:00:00:20".to_string(),
                range_semantics: "startInclusiveEndExclusive".to_string(),
            }),
        }
    }

    // ── referenced_mentions / make_display_name ──────────────────────────────

    #[test]
    fn referenced_mentions_filters_by_at_token_presence() {
        let mentions = vec![
            media_asset_mention("clip-a", "m1", ClipType::Video),
            media_asset_mention("clip-b", "m2", ClipType::Video),
        ];
        let got = referenced_mentions(&mentions, "please use @clip-a here");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].display_name, "clip-a");
    }

    #[test]
    fn make_display_name_collapses_space_and_dash_runs() {
        assert_eq!(make_display_name("My  Cool -- Clip"), "My-Cool-Clip");
        assert_eq!(make_display_name("-leading and trailing-"), "leading-and-trailing");
        assert_eq!(make_display_name("solid"), "solid");
    }

    // ── mediaAsset hint shape ────────────────────────────────────────────────

    #[test]
    fn media_asset_entry_shape() {
        let mentions = vec![media_asset_mention("logo", "media_1", ClipType::Image)];
        let inlined = InlinedMentions::default();
        let entries = mention_entries(&mentions, None, &inlined);
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e["mention"], "@logo");
        assert_eq!(e["kind"], "mediaAsset");
        assert_eq!(e["mediaRef"], "media_1");
        assert_eq!(e["type"], "image");
        // No clipId / clip / inlined / inlineError on a bare mediaAsset.
        assert!(e.get("clipId").is_none());
        assert!(e.get("inlined").is_none());
    }

    // ── timelineClip hint shape (with resolved clip summary) ─────────────────

    #[test]
    fn timeline_clip_entry_includes_resolved_clip_summary() {
        let mut enricher = StubEnricher::new();
        enricher.clips.insert(
            "clip_42".to_string(),
            json!({ "clipId": "clip_42", "label": "Intro", "startFrame": 0, "endFrame": 120 }),
        );
        let mentions = vec![timeline_clip_mention("intro", "media_1", "clip_42")];
        let inlined = InlinedMentions::default();
        let entries = mention_entries(&mentions, Some(&enricher), &inlined);
        let e = &entries[0];
        assert_eq!(e["kind"], "timelineClip");
        assert_eq!(e["clipId"], "clip_42");
        assert_eq!(e["clip"]["label"], "Intro");
        assert_eq!(e["clip"]["endFrame"], 120);
    }

    #[test]
    fn timeline_clip_without_resolver_gets_editor_unavailable() {
        let mentions = vec![timeline_clip_mention("intro", "media_1", "clip_42")];
        let entries = mention_entries(&mentions, None, &InlinedMentions::default());
        assert_eq!(entries[0]["clip"]["error"], "editor unavailable");
    }

    // ── timelineRange hint shape (half-open semantics) ───────────────────────

    #[test]
    fn timeline_range_entry_carries_half_open_summary() {
        let mentions = vec![timeline_range_mention("sel")];
        let entries = mention_entries(&mentions, None, &InlinedMentions::default());
        let e = &entries[0];
        assert_eq!(e["mention"], "@sel");
        assert_eq!(e["kind"], "timelineRange");
        let r = &e["timelineRange"];
        assert_eq!(r["startFrame"], 10);
        assert_eq!(r["endFrame"], 30);
        assert_eq!(r["durationFrames"], 20);
        assert_eq!(r["rangeSemantics"], "startInclusiveEndExclusive");
    }

    // ── full hint string format + notes ──────────────────────────────────────

    #[test]
    fn hint_string_has_exact_prefix_and_range_note() {
        let mentions = vec![timeline_range_mention("sel")];
        let h = hint(&mentions, None, &InlinedMentions::default());
        assert!(
            h.starts_with("Referenced assets and timeline context in this message: ["),
            "hint prefix: {h}"
        );
        // The half-open range note is appended (leading space, single space join).
        assert!(h.contains(NOTE_RANGES), "range note present: {h}");
        // The JSON array is sorted-key (endFrame before startFrame alphabetically).
        let end_pos = h.find("\"endFrame\"").unwrap();
        let start_pos = h.find("\"startFrame\"").unwrap();
        assert!(end_pos < start_pos, "sorted keys: endFrame before startFrame");
    }

    #[test]
    fn hint_notes_order_inlined_then_failed_then_clips_then_ranges() {
        let mut inlined = InlinedMentions::default();
        inlined.inlined_ids.insert("m_in".to_string());
        inlined.failures.insert("m_fail".to_string(), FAIL_UNREADABLE.to_string());
        let mentions = vec![
            media_asset_mention("a", "m_in", ClipType::Image),
            timeline_clip_mention("b", "m_fail", "clip_1"),
            timeline_range_mention("c"),
        ];
        let h = hint(&mentions, None, &inlined);
        let p_inlined = h.find(NOTE_INLINED).unwrap();
        let p_failed = h.find(NOTE_INLINE_ERROR).unwrap();
        let p_clips = h.find(NOTE_CLIPS).unwrap();
        let p_ranges = h.find(NOTE_RANGES).unwrap();
        assert!(p_inlined < p_failed);
        assert!(p_failed < p_clips);
        assert!(p_clips < p_ranges);
    }

    #[test]
    fn hint_with_no_notes_ends_in_bare_period() {
        // A single mediaAsset with no inline / clip / range → no notes → "...].".
        let mentions = vec![media_asset_mention("a", "m1", ClipType::Video)];
        let h = hint(&mentions, None, &InlinedMentions::default());
        assert!(h.ends_with("]."), "no-notes hint ends with `].`: {h}");
    }

    // ── inline_image_blocks: success / not-in-library / unreadable / no-source ─

    #[test]
    fn inline_image_blocks_success_marks_inlined_and_emits_block() {
        let enricher = {
            let e = StubEnricher::new();
            e.assets.insert("media_img", png(64, 64), 1);
            e
        };
        let mentions = vec![media_asset_mention("pic", "media_img", ClipType::Image)];
        let inlined = inline_image_blocks(&mentions, Some(&enricher));
        assert_eq!(inlined.blocks.len(), 1);
        assert_eq!(inlined.blocks[0]["type"], "image");
        assert_eq!(inlined.blocks[0]["source"]["type"], "base64");
        assert_eq!(inlined.blocks[0]["source"]["media_type"], "image/png");
        assert!(inlined.inlined_ids.contains("media_img"));
        assert!(inlined.failures.is_empty());

        // And the hint marks it inlined:true.
        let h = hint(&mentions, None, &inlined);
        assert!(h.contains("\"inlined\":true"), "hint marks inlined: {h}");
        assert!(h.contains(NOTE_INLINED));
    }

    #[test]
    fn inline_image_blocks_missing_asset_records_not_in_library() {
        let enricher = StubEnricher::new(); // empty asset map
        let mentions = vec![media_asset_mention("pic", "media_missing", ClipType::Image)];
        let inlined = inline_image_blocks(&mentions, Some(&enricher));
        assert!(inlined.blocks.is_empty());
        assert_eq!(
            inlined.failures.get("media_missing").map(String::as_str),
            Some(FAIL_NOT_IN_LIBRARY)
        );
        // The hint carries the exact inlineError reason + the failure note.
        let h = hint(&mentions, None, &inlined);
        assert!(h.contains(FAIL_NOT_IN_LIBRARY), "exact reason in hint: {h}");
        assert!(h.contains(NOTE_INLINE_ERROR));
    }

    #[test]
    fn inline_image_blocks_unreadable_asset_records_decode_failure() {
        let enricher = StubEnricher::new();
        enricher
            .assets
            .insert("media_bad", b"garbage not an image".to_vec(), 1);
        let mentions = vec![media_asset_mention("pic", "media_bad", ClipType::Image)];
        let inlined = inline_image_blocks(&mentions, Some(&enricher));
        assert!(inlined.blocks.is_empty());
        assert_eq!(
            inlined.failures.get("media_bad").map(String::as_str),
            Some(FAIL_UNREADABLE)
        );
    }

    #[test]
    fn inline_image_blocks_no_source_fails_with_editor_unavailable() {
        let mentions = vec![media_asset_mention("pic", "media_img", ClipType::Image)];
        let inlined = inline_image_blocks(&mentions, None);
        assert_eq!(
            inlined.failures.get("media_img").map(String::as_str),
            Some("editor unavailable")
        );
    }

    #[test]
    fn non_image_mentions_are_not_inlined() {
        let enricher = StubEnricher::new();
        let mentions = vec![media_asset_mention("vid", "media_v", ClipType::Video)];
        let inlined = inline_image_blocks(&mentions, Some(&enricher));
        assert!(inlined.blocks.is_empty());
        assert!(inlined.failures.is_empty());
    }
}
