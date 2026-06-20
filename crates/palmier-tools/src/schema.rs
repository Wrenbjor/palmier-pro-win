//! Tool schema catalogue — `ToolName` + `ToolDefinition` + the 30 verbatim
//! definitions (E7-S1).
//!
//! This is the **parity contract surface**. The reference registers tools from
//! exactly one registry (`ToolDefinitions.all`, `ToolDefinitions.swift:44`),
//! enumerated by the `ToolName` enum (`ToolDefinitions.swift:5-34`). Both the MCP
//! server and the in-app agent iterate that same array — there is no second tool
//! source. The count is **30, not 36** (docs/phase0-reconciliation.md ruling #1;
//! docs/reference/mcp-tools.md "TOOL COUNT").
//!
//! ## What is ported verbatim
//! - **Tool names** (snake_case wire names) — the `ToolName` raw values.
//! - **Tool descriptions** — load-bearing prompt text the LLM was tuned against.
//!   Stored as `const &str` to preserve Unicode (`×` U+00D7, `–` U+2013,
//!   `•` U+2022, `…`, curly quotes) as UTF-8; do **not** ASCII-fold. Ported
//!   byte-for-byte from `ToolDefinitions.swift` per ruling #2 / R-5.
//! - **JSON input schemas** — built by [`object_schema`], which **omits empty
//!   `properties`/`required`** exactly like the reference `objectSchema` helper.
//!
//! ## What this story does NOT do (deferred to later E7 stories)
//! The 30 tool *bodies* are E7-S5..S10. Here every definition is data only; the
//! dispatch seam ([`crate::dispatch`]) routes each name to a not-yet-implemented
//! result. The schema/description/registry IS complete and client-compatible now.
//!
//! ## mutation / async flags
//! Carried on every [`ToolDefinition`] from the mcp-tools.md classification table
//! ("The 30 tools | mutation | async | …"). `mutation` = changes the
//! timeline/library (pushes the agent-undo stack, E7-S12). `is_async` = kicks off
//! background work / returns a placeholder immediately. These drive the
//! executor's post-run undo bookkeeping and are advertised here as the single
//! source of truth.

use serde_json::{json, Map, Value};

/// The 30 MCP tool wire names (reference `ToolName`, `ToolDefinitions.swift:5-34`).
///
/// Snake_case raw values are the wire names. **Exactly 30 variants** — adding a
/// 31st fails the count gate (SM-C2). Order mirrors [`ToolName::ALL`] / the
/// reference enum declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolName {
    GetTimeline,
    GetMedia,
    AddClips,
    RemoveClips,
    RemoveTracks,
    MoveClips,
    SetClipProperties,
    SetKeyframes,
    SplitClip,
    RippleDeleteRanges,
    Undo,
    AddTexts,
    AddCaptions,
    GenerateVideo,
    GenerateImage,
    GenerateAudio,
    UpscaleMedia,
    ImportMedia,
    ListModels,
    InspectMedia,
    GetTranscript,
    InspectTimeline,
    SearchMedia,
    ListFolders,
    CreateFolder,
    MoveToFolder,
    RenameMedia,
    RenameFolder,
    DeleteMedia,
    DeleteFolder,
}

impl ToolName {
    /// All 30 variants in reference declaration order. The single source the count
    /// gate and the registry iterate. **Exactly 30.**
    pub const ALL: [ToolName; 30] = [
        ToolName::GetTimeline,
        ToolName::GetMedia,
        ToolName::AddClips,
        ToolName::RemoveClips,
        ToolName::RemoveTracks,
        ToolName::MoveClips,
        ToolName::SetClipProperties,
        ToolName::SetKeyframes,
        ToolName::SplitClip,
        ToolName::RippleDeleteRanges,
        ToolName::Undo,
        ToolName::AddTexts,
        ToolName::AddCaptions,
        ToolName::GenerateVideo,
        ToolName::GenerateImage,
        ToolName::GenerateAudio,
        ToolName::UpscaleMedia,
        ToolName::ImportMedia,
        ToolName::ListModels,
        ToolName::InspectMedia,
        ToolName::GetTranscript,
        ToolName::InspectTimeline,
        ToolName::SearchMedia,
        ToolName::ListFolders,
        ToolName::CreateFolder,
        ToolName::MoveToFolder,
        ToolName::RenameMedia,
        ToolName::RenameFolder,
        ToolName::DeleteMedia,
        ToolName::DeleteFolder,
    ];

    /// The snake_case wire name (reference enum raw value).
    pub const fn wire_name(self) -> &'static str {
        match self {
            ToolName::GetTimeline => "get_timeline",
            ToolName::GetMedia => "get_media",
            ToolName::AddClips => "add_clips",
            ToolName::RemoveClips => "remove_clips",
            ToolName::RemoveTracks => "remove_tracks",
            ToolName::MoveClips => "move_clips",
            ToolName::SetClipProperties => "set_clip_properties",
            ToolName::SetKeyframes => "set_keyframes",
            ToolName::SplitClip => "split_clip",
            ToolName::RippleDeleteRanges => "ripple_delete_ranges",
            ToolName::Undo => "undo",
            ToolName::AddTexts => "add_texts",
            ToolName::AddCaptions => "add_captions",
            ToolName::GenerateVideo => "generate_video",
            ToolName::GenerateImage => "generate_image",
            ToolName::GenerateAudio => "generate_audio",
            ToolName::UpscaleMedia => "upscale_media",
            ToolName::ImportMedia => "import_media",
            ToolName::ListModels => "list_models",
            ToolName::InspectMedia => "inspect_media",
            ToolName::GetTranscript => "get_transcript",
            ToolName::InspectTimeline => "inspect_timeline",
            ToolName::SearchMedia => "search_media",
            ToolName::ListFolders => "list_folders",
            ToolName::CreateFolder => "create_folder",
            ToolName::MoveToFolder => "move_to_folder",
            ToolName::RenameMedia => "rename_media",
            ToolName::RenameFolder => "rename_folder",
            ToolName::DeleteMedia => "delete_media",
            ToolName::DeleteFolder => "delete_folder",
        }
    }

    /// Resolve a wire name → `ToolName` (reference `ToolName(rawValue:)`).
    /// Unknown names return `None`; the dispatcher maps that to the tool-error shape.
    pub fn from_wire(name: &str) -> Option<ToolName> {
        ToolName::ALL.into_iter().find(|t| t.wire_name() == name)
    }

    /// Whether this tool mutates the timeline/library (reference: pushes agent-undo).
    /// Source: mcp-tools.md "The 30 tools" mutation column.
    pub const fn is_mutation(self) -> bool {
        use ToolName::*;
        matches!(
            self,
            AddClips
                | RemoveClips
                | RemoveTracks
                | MoveClips
                | SetClipProperties
                | SetKeyframes
                | SplitClip
                | RippleDeleteRanges
                | Undo
                | AddTexts
                | AddCaptions
                | GenerateVideo
                | GenerateImage
                | GenerateAudio
                | UpscaleMedia
                | ImportMedia
                | CreateFolder
                | MoveToFolder
                | RenameMedia
                | RenameFolder
                | DeleteMedia
                | DeleteFolder
        )
    }

    /// Whether this tool kicks off background work / may return a placeholder
    /// immediately (reference async column). Note `import_media` is async **only**
    /// for the `url` source (path/bytes are synchronous) — flagged async here
    /// because the dispatch seam is `async fn` regardless; the per-source split is
    /// enforced inside the body (E7-S10).
    pub const fn is_async(self) -> bool {
        use ToolName::*;
        matches!(
            self,
            InspectMedia
                | InspectTimeline
                | SearchMedia
                | AddCaptions
                | GenerateVideo
                | GenerateImage
                | GenerateAudio
                | UpscaleMedia
                | ImportMedia
        )
    }
}

/// A single tool's contract: wire name, verbatim description, JSON input schema,
/// and the mutation/async classification (reference `AgentTool`, extended with the
/// mutation/async flags the mcp-tools.md table carries).
///
/// `description` and `input_schema` are the parity contract — see module docs.
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: ToolName,
    /// Byte-for-byte reference `description` string (load-bearing prompt text).
    pub description: &'static str,
    /// JSON-Schema object built by [`object_schema`] (empty props/required omitted).
    pub input_schema: Value,
    /// Changes the timeline/library — pushes the agent-undo stack on a successful,
    /// timeline-changing run (E7-S12). Mirrors [`ToolName::is_mutation`].
    pub mutation: bool,
    /// Kicks off background work / may return a placeholder. Mirrors
    /// [`ToolName::is_async`].
    pub is_async: bool,
}

/// Build a JSON-Schema `object` from `properties` and `required`, **omitting**
/// either when empty — exact port of the reference `objectSchema` helper
/// (`ToolDefinitions.swift:562`). An empty object becomes `{"type":"object"}`.
///
/// `properties` is an ordered slice of `(key, schema)` so the emitted JSON key
/// order is stable and matches the reference declaration order.
pub fn object_schema(properties: &[(&str, Value)], required: &[&str]) -> Value {
    let mut dict = Map::new();
    dict.insert("type".to_string(), Value::String("object".to_string()));
    if !properties.is_empty() {
        let mut props = Map::new();
        for (k, v) in properties {
            props.insert((*k).to_string(), v.clone());
        }
        dict.insert("properties".to_string(), Value::Object(props));
    }
    if !required.is_empty() {
        dict.insert(
            "required".to_string(),
            Value::Array(required.iter().map(|s| Value::String((*s).to_string())).collect()),
        );
    }
    Value::Object(dict)
}

/// The complete 30-tool registry (reference `ToolDefinitions.all`). Returned in
/// reference declaration order. **Exactly 30 entries** — the count is asserted in
/// tests (mirroring the reference's two-way grep verification).
pub fn tool_definitions() -> Vec<ToolDefinition> {
    let defs = vec![
        def(ToolName::GetTimeline, GET_TIMELINE_DESC, schema_get_timeline()),
        def(ToolName::GetMedia, GET_MEDIA_DESC, object_schema(&[], &[])),
        def(ToolName::InspectMedia, INSPECT_MEDIA_DESC, schema_inspect_media()),
        def(ToolName::GetTranscript, GET_TRANSCRIPT_DESC, schema_get_transcript()),
        def(ToolName::InspectTimeline, INSPECT_TIMELINE_DESC, schema_inspect_timeline()),
        def(ToolName::SearchMedia, SEARCH_MEDIA_DESC, schema_search_media()),
        def(ToolName::AddClips, ADD_CLIPS_DESC, schema_add_clips()),
        def(ToolName::RemoveClips, REMOVE_CLIPS_DESC, schema_remove_clips()),
        def(ToolName::RemoveTracks, REMOVE_TRACKS_DESC, schema_remove_tracks()),
        def(ToolName::MoveClips, MOVE_CLIPS_DESC, schema_move_clips()),
        def(ToolName::SetClipProperties, SET_CLIP_PROPERTIES_DESC, schema_set_clip_properties()),
        def(ToolName::SetKeyframes, SET_KEYFRAMES_DESC, schema_set_keyframes()),
        def(ToolName::SplitClip, SPLIT_CLIP_DESC, schema_split_clip()),
        def(ToolName::RippleDeleteRanges, RIPPLE_DELETE_RANGES_DESC, schema_ripple_delete_ranges()),
        def(ToolName::Undo, UNDO_DESC, object_schema(&[], &[])),
        def(ToolName::AddTexts, ADD_TEXTS_DESC, schema_add_texts()),
        def(ToolName::AddCaptions, ADD_CAPTIONS_DESC, schema_add_captions()),
        def(ToolName::GenerateVideo, GENERATE_VIDEO_DESC, schema_generate_video()),
        def(ToolName::GenerateImage, GENERATE_IMAGE_DESC, schema_generate_image()),
        def(ToolName::GenerateAudio, GENERATE_AUDIO_DESC, schema_generate_audio()),
        def(ToolName::UpscaleMedia, UPSCALE_MEDIA_DESC, schema_upscale_media()),
        def(ToolName::ImportMedia, IMPORT_MEDIA_DESC, schema_import_media()),
        def(ToolName::ListFolders, LIST_FOLDERS_DESC, object_schema(&[], &[])),
        def(ToolName::CreateFolder, CREATE_FOLDER_DESC, schema_create_folder()),
        def(ToolName::MoveToFolder, MOVE_TO_FOLDER_DESC, schema_move_to_folder()),
        def(ToolName::RenameMedia, RENAME_MEDIA_DESC, schema_rename_media()),
        def(ToolName::RenameFolder, RENAME_FOLDER_DESC, schema_rename_folder()),
        def(ToolName::DeleteMedia, DELETE_MEDIA_DESC, schema_delete_media()),
        def(ToolName::DeleteFolder, DELETE_FOLDER_DESC, schema_delete_folder()),
        def(ToolName::ListModels, LIST_MODELS_DESC, schema_list_models()),
    ];
    defs
}

fn def(name: ToolName, description: &'static str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name,
        description,
        input_schema,
        mutation: name.is_mutation(),
        is_async: name.is_async(),
    }
}

// ── schema string helpers ───────────────────────────────────────────────────
// `str_prop` / `int_prop` / `num_prop` / `bool_prop` mirror the inline
// `["type": .., "description": ..]` dicts in the reference. `enum_prop` adds the
// `enum` array. These keep the 30 schema builders readable and 1:1 with Swift.

fn str_prop(desc: &str) -> Value {
    json!({ "type": "string", "description": desc })
}
fn int_prop(desc: &str) -> Value {
    json!({ "type": "integer", "description": desc })
}
fn num_prop(desc: &str) -> Value {
    json!({ "type": "number", "description": desc })
}
fn bool_prop(desc: &str) -> Value {
    json!({ "type": "boolean", "description": desc })
}
fn enum_str_prop(variants: &[&str], desc: &str) -> Value {
    json!({ "type": "string", "enum": variants, "description": desc })
}

// ── per-tool input schemas (ported from ToolDefinitions.swift) ───────────────

fn schema_get_timeline() -> Value {
    object_schema(
        &[
            ("startFrame", int_prop("Optional. Window start (inclusive); only clips intersecting [startFrame, endFrame) are returned. Tracks report totalClips when the window hides some.")),
            ("endFrame", int_prop("Optional. Window end (exclusive).")),
        ],
        &[],
    )
}

fn schema_inspect_media() -> Value {
    object_schema(
        &[
            ("mediaRef", str_prop("Asset ID from get_media.")),
            ("clipId", str_prop("Optional. A clip referencing this mediaRef; transcript times come back as project frames for that clip (out-of-range entries dropped).")),
            ("maxFrames", int_prop("Video and Lottie. Sample frame count (default 6, max 12).")),
            ("startSeconds", num_prop("Video/audio. Source-time window start; scopes frames and transcription.")),
            ("endSeconds", num_prop("Video/audio. Window end (default: asset duration).")),
            ("wordTimestamps", bool_prop("Video/audio. Add word-level [text, start, end] tuples (capped at 10000 — most clips return all words at once; narrow with startSeconds/endSeconds only for very long media). Use for word-boundary edits like filler-word removal.")),
            ("overview", bool_prop("Video only. One storyboard grid of visually distinct, timestamped moments instead of frames — far more coverage per token; few tiles means static footage. maxFrames ignored.")),
        ],
        &["mediaRef"],
    )
}

fn schema_get_transcript() -> Value {
    object_schema(
        &[
            ("startFrame", int_prop("Optional. Only return words ending after this project frame. Use with the returned nextStartFrame to page a long timeline.")),
            ("endFrame", int_prop("Optional. Only return words starting before this project frame.")),
            ("clipId", str_prop("Scope the transcript to a single clip — returns only what that clip says, in project frames. Answers \"what's in clip X?\" without scanning the whole timeline.")),
        ],
        &[],
    )
}

fn schema_inspect_timeline() -> Value {
    object_schema(
        &[
            ("startFrame", int_prop("Project frame to render (default 0). With no endFrame, a single frame is returned.")),
            ("endFrame", int_prop("Optional. Sample maxFrames evenly across [startFrame, endFrame) instead of one frame.")),
            ("maxFrames", int_prop("Frames to sample when endFrame is set (default 6, max 12).")),
        ],
        &[],
    )
}

fn schema_search_media() -> Value {
    object_schema(
        &[
            ("query", str_prop("What to find. Visual: a caption-style scene description. Spoken: the words to match.")),
            ("scope", enum_str_prop(&["visual", "spoken", "both"], "Optional. Default both.")),
            ("mediaRef", str_prop("Optional. Restrict the search to one asset from get_media.")),
            ("limit", int_prop("Optional. Max hits per group (default 10, max 50).")),
        ],
        &["query"],
    )
}

fn schema_add_clips() -> Value {
    let entry_item = json!({
        "type": "object",
        "properties": {
            "mediaRef": { "type": "string", "description": "ID of the media asset from get_media" },
            "trackIndex": { "type": "integer", "description": "Optional. Track index (0-based). Omit on every entry to auto-create one shared track per asset zone (video/audio)." },
            "startFrame": { "type": "integer", "description": "Frame position to place the clip" },
            "durationFrames": { "type": "integer", "description": "Duration in frames" }
        },
        "required": ["mediaRef", "startFrame", "durationFrames"]
    });
    object_schema(
        &[(
            "entries",
            json!({
                "type": "array",
                "description": "Clips to add. Each entry is validated up front; one bad entry rejects the whole call with no partial state.",
                "items": entry_item
            }),
        )],
        &["entries"],
    )
}

fn schema_remove_clips() -> Value {
    object_schema(
        &[(
            "clipIds",
            json!({
                "type": "array",
                "description": "Clip IDs to remove.",
                "items": { "type": "string" }
            }),
        )],
        &["clipIds"],
    )
}

fn schema_remove_tracks() -> Value {
    object_schema(
        &[(
            "trackIndexes",
            json!({
                "type": "array",
                "items": { "type": "integer" },
                "description": "Track indexes (0-based, from get_timeline) to remove."
            }),
        )],
        &["trackIndexes"],
    )
}

fn schema_move_clips() -> Value {
    let move_item = json!({
        "type": "object",
        "properties": {
            "clipId": { "type": "string", "description": "The clip ID to move." },
            "toTrack": { "type": "integer", "description": "Destination track index (0-based). Omit to keep the clip on its current track." },
            "toFrame": { "type": "integer", "description": "Destination start frame. Omit to keep the clip at its current start." }
        },
        "required": ["clipId"]
    });
    object_schema(
        &[(
            "moves",
            json!({
                "type": "array",
                "description": "Per-clip move requests. At least one of toTrack or toFrame is required per entry.",
                "items": move_item
            }),
        )],
        &["moves"],
    )
}

fn schema_set_clip_properties() -> Value {
    let transform = json!({
        "type": "object",
        "description": "Partial transform. Any combination of centerX, centerY, width, height, flipHorizontal, flipVertical; omitted fields keep their current value.",
        "properties": {
            "centerX": { "type": "number" },
            "centerY": { "type": "number" },
            "width": { "type": "number" },
            "height": { "type": "number" },
            "flipHorizontal": { "type": "boolean", "description": "Mirror across the vertical axis." },
            "flipVertical": { "type": "boolean", "description": "Mirror across the horizontal axis." }
        }
    });
    object_schema(
        &[
            ("clipIds", json!({
                "type": "array",
                "description": "Clip IDs to update. The property values below apply to every clip in this list.",
                "items": { "type": "string" }
            })),
            ("durationFrames", int_prop("New duration in frames.")),
            ("trimStartFrame", int_prop("SOURCE-media offset, NOT a timeline frame: frames trimmed off the start of the source. To turn a get_transcript project frame P into this clip's source offset, use trimStartFrame + (P − startFrame) × speed; setting trimStartFrame to that value makes the clip begin at P's source content.")),
            ("trimEndFrame", int_prop("SOURCE-media offset, NOT a timeline frame: frames trimmed off the end of the source. Maps the same way as trimStartFrame via startFrame/speed.")),
            ("speed", num_prop("Playback speed multiplier (default 1.0). >1 speeds up, <1 slows down. The clip's timeline length is rescaled to keep the same source content (2x speed → half the frames), unless you also pass durationFrames to set the length explicitly.")),
            ("volume", num_prop("Volume 0.0-1.0. Clears any existing volume keyframes.")),
            ("opacity", num_prop("Opacity 0.0-1.0. Clears any existing opacity keyframes.")),
            ("transform", transform),
            ("content", str_prop("Text clips only. New text content.")),
            ("fontName", str_prop("Text clips only. Font PostScript or family name.")),
            ("fontSize", num_prop("Text clips only. Font size in canvas points.")),
            ("color", str_prop("Text clips only. Hex '#RRGGBB' or '#RRGGBBAA'.")),
            ("alignment", enum_str_prop(&["left", "center", "right"], "Text clips only.")),
        ],
        &["clipIds"],
    )
}

fn schema_set_keyframes() -> Value {
    object_schema(
        &[
            ("clipId", str_prop("The clip ID.")),
            ("property", enum_str_prop(
                &["volume", "opacity", "rotation", "position", "scale", "crop"],
                "Which property's keyframe track to set.",
            )),
            ("keyframes", json!({
                "type": "array",
                "description": "Replacement keyframe rows. Empty array clears the track. Row shape depends on property — see tool description.",
                "items": { "type": "array" }
            })),
        ],
        &["clipId", "property", "keyframes"],
    )
}

fn schema_split_clip() -> Value {
    object_schema(
        &[
            ("clipId", str_prop("The clip ID to split")),
            ("atFrame", int_prop("Frame position to split at (must be between clip start and end)")),
        ],
        &["clipId", "atFrame"],
    )
}

fn schema_ripple_delete_ranges() -> Value {
    object_schema(
        &[
            ("trackIndex", int_prop("Cut project-frame ranges spanning every clip they cross on this track, in one call. From get_transcript's clips array. Mutually exclusive with clipId; requires units 'frames'.")),
            ("clipId", str_prop("Cut ranges within this single clip only, clamped to its visible span. Mutually exclusive with trackIndex.")),
            ("ranges", json!({
                "type": "array",
                "description": "Ranges to remove, each a [start, end] pair (end > start). In the unit given by 'units'.",
                "items": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2 }
            })),
            ("units", enum_str_prop(&["seconds", "frames"], "Interpretation of range values. 'frames' (default) = project/timeline frames, matching get_transcript and inspect_media-with-clipId. 'seconds' = source-media seconds (clipId mode only).")),
        ],
        &["ranges"],
    )
}

fn schema_add_texts() -> Value {
    let transform = json!({
        "type": "object",
        "description": "Optional position/size. Omit for center + auto-fit. Pass centerX+centerY only for a specific position with auto-fit size. Pass all four for full override.",
        "properties": {
            "centerX": { "type": "number", "description": "Horizontal center 0–1 (0=left edge, 1=right edge)" },
            "centerY": { "type": "number", "description": "Vertical center 0–1 (0=top, 1=bottom)" },
            "width": { "type": "number", "description": "Width 0–1 (optional; omit for auto-fit)" },
            "height": { "type": "number", "description": "Height 0–1 (optional; omit for auto-fit)" }
        }
    });
    let entry_item = json!({
        "type": "object",
        "properties": {
            "trackIndex": { "type": "integer", "description": "Optional. Track index (0-based) for an existing non-audio track. Omit on every entry to auto-create one new track for the batch." },
            "startFrame": { "type": "integer", "description": "Frame position to place the clip" },
            "durationFrames": { "type": "integer", "description": "Duration in frames (>= 1)" },
            "content": { "type": "string", "description": "Text to display. Supports \\n for line breaks." },
            "transform": transform,
            "fontName": { "type": "string", "description": "Font PostScript or family name, e.g. 'Helvetica-Bold', 'Georgia-Bold'. Default 'Helvetica-Bold'. Falls back to bold system font if not found." },
            "fontSize": { "type": "number", "description": "Font size in canvas points (default 96). On a 1080p canvas ~50 is a caption, ~120 is a title." },
            "color": { "type": "string", "description": "Hex '#RRGGBB' or '#RRGGBBAA' (default '#FFFFFF')" },
            "alignment": { "type": "string", "enum": ["left", "center", "right"], "description": "Text alignment (default 'center')" }
        },
        "required": ["startFrame", "durationFrames", "content"]
    });
    object_schema(
        &[(
            "entries",
            json!({
                "type": "array",
                "description": "Text clips to add. Each entry is independent.",
                "items": entry_item
            }),
        )],
        &["entries"],
    )
}

fn schema_add_captions() -> Value {
    object_schema(
        &[
            ("clipIds", json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Optional. Audio/video clips to caption. Omit to auto-detect the primary spoken track."
            })),
            ("language", str_prop("Optional BCP-47 language of the speech (e.g. 'es', 'ja', 'en-GB'). Defaults to the system language — set this when the footage is in another language, or transcription will be garbage.")),
            ("fontName", str_prop("Optional font PostScript or family name (default 'Helvetica-Bold'). Falls back to bold system font if not found.")),
            ("fontSize", num_prop("Optional font size in canvas points (default 48).")),
            ("color", str_prop("Optional hex '#RRGGBB' or '#RRGGBBAA' (default white).")),
            ("centerX", num_prop("Optional horizontal center 0–1 (default 0.5).")),
            ("centerY", num_prop("Optional vertical center 0–1 (default 0.9, near the bottom).")),
            ("textCase", enum_str_prop(&["auto", "upper", "lower"], "Optional letter case (default auto).")),
            ("censorProfanity", bool_prop("Optional. Mask profanity (default false).")),
        ],
        &[],
    )
}

fn schema_generate_video() -> Value {
    object_schema(
        &[
            ("prompt", str_prop("Text description of the video to generate")),
            ("name", str_prop("Display name for the asset in the media library. Defaults to first 30 chars of prompt.")),
            ("model", str_prop("Model ID (e.g. 'veo3.1-fast'). Use list_models to see options. Defaults to first available model.")),
            ("duration", int_prop("Duration in seconds. Valid values depend on model.")),
            ("aspectRatio", str_prop("Aspect ratio (e.g. '16:9', '9:16', '1:1')")),
            ("resolution", str_prop("Resolution (e.g. '720p', '1080p', '4k')")),
            ("startFrameMediaRef", str_prop("Media asset ID to use as the first frame (image-to-video)")),
            ("endFrameMediaRef", str_prop("Media asset ID to use as the last frame (supported by some models)")),
            ("sourceVideoMediaRef", str_prop("Media asset ID of a source video (required by video-to-video edit models; ignores duration/aspectRatio/resolution)")),
            ("sourceClipId", str_prop("Optional. Clip id (from get_timeline) referencing sourceVideoMediaRef. When set and the clip is trimmed, only the clip's visible range is sent to the model, not the full source — matches the UI's 'Use trimmed portion only'.")),
            ("referenceImageMediaRefs", json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Media asset IDs of image references. Covers both reference-to-video generation (Seedance, Kling V3/O3 elements, Grok — refer as @Image1/@Element1 in prompt) and the single-image ref used by video-to-video edit models (Kling V3 Motion Control). See list_models maxReferenceImages for per-model cap."
            })),
            ("referenceVideoMediaRefs", json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Media asset IDs of video references (Seedance only). Refer to them as @Video1, @Video2. See maxReferenceVideos and maxCombinedVideoRefSeconds."
            })),
            ("referenceAudioMediaRefs", json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Media asset IDs of audio references (Seedance only). Refer to them as @Audio1, @Audio2. See maxReferenceAudios and maxCombinedAudioRefSeconds."
            })),
            ("folderId", str_prop("Optional. Folder id (from list_folders or create_folder) to place the result in. Omit for the project root.")),
        ],
        &["prompt"],
    )
}

fn schema_generate_image() -> Value {
    object_schema(
        &[
            ("prompt", str_prop("Text description of the image to generate")),
            ("name", str_prop("Display name for the asset in the media library. Defaults to first 30 chars of prompt.")),
            ("model", str_prop("Model ID (e.g. 'nano-banana-pro'). Use list_models to see options. Defaults to first available model.")),
            ("aspectRatio", str_prop("Aspect ratio (e.g. '16:9', '9:16')")),
            ("resolution", str_prop("Resolution (e.g. '2K', '4K')")),
            ("quality", str_prop("Image quality (e.g. 'low', 'medium', 'high'). Only supported by some models — see list_models.")),
            ("referenceMediaRefs", json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Media asset IDs to use as reference images"
            })),
            ("folderId", str_prop("Optional. Folder id (from list_folders or create_folder) to place the result in. Omit for the project root.")),
        ],
        &["prompt"],
    )
}

fn schema_generate_audio() -> Value {
    // NOTE: `required: []` — generate_audio has NO required field (prompt is
    // optional, for video-to-music). Easy to wrongly mark prompt required
    // (mcp-tools.md "Port risks", ruling carry-forward).
    object_schema(
        &[
            ("prompt", str_prop("Required for TTS (the text to speak) and text-to-music (style/mood/genre; MiniMax needs ≥10 chars). For Lyria 3 Pro, include lyrics, tempo, language, and vocal style directly in the prompt. Optional style guide for video-to-music models.")),
            ("name", str_prop("Display name for the asset in the media library. Defaults to first 30 chars of prompt.")),
            ("model", str_prop("Model ID. Use list_models with type='audio' to see options and their 'inputs'. Defaults to the first model.")),
            ("voice", str_prop("TTS only. Voice preset name. list_models shows voicesSample (first 3) + voiceCount; any voice supported by the model is accepted. Defaults to the model's defaultVoice. Ignored by music models.")),
            ("lyrics", str_prop("MiniMax Music only. Lyrics with optional [Verse]/[Chorus] section tags. If omitted and instrumental=false, MiniMax auto-writes lyrics from the prompt.")),
            ("styleInstructions", str_prop("Gemini TTS only. Optional delivery instructions (e.g. 'warm and slow', 'British accent').")),
            ("instrumental", bool_prop("Music models only. true = no vocals when the selected model supports it. Defaults to false.")),
            ("duration", int_prop("Length in seconds. ElevenLabs Music: 3–600. Sonilo text-to-music: up to 600. For a video source, defaults to the span/clip length. Ignored by TTS, MiniMax, and Lyria 3 Pro.")),
            ("videoSourceStartFrame", int_prop("Video-to-audio models only. Start frame (timeline) of a span to render and score — pair with videoSourceEndFrame. Use get_timeline for frame numbers; for the whole timeline use 0 to the timeline's end frame.")),
            ("videoSourceEndFrame", int_prop("Video-to-audio models only. End frame (exclusive) of the span to score. Must be > videoSourceStartFrame.")),
            ("videoSourceMediaRef", str_prop("Video-to-audio models only. Score this existing video asset instead of a timeline span. Mutually exclusive with the videoSource frames.")),
            ("folderId", str_prop("Optional. Folder id (from list_folders or create_folder) to place the result in. Omit for the project root.")),
        ],
        &[],
    )
}

fn schema_upscale_media() -> Value {
    object_schema(
        &[
            ("mediaRef", str_prop("ID of the video or image asset to upscale")),
            ("model", str_prop("Upscaler model ID (e.g. 'bytedance-upscaler', 'seedvr-image-upscaler'). Defaults to the first model that supports the asset's type.")),
            ("sourceClipId", str_prop("Optional. Video clip id (from get_timeline) referencing mediaRef. When set and the clip is trimmed, only the clip's visible range is upscaled, not the full source.")),
        ],
        &["mediaRef"],
    )
}

fn schema_import_media() -> Value {
    let source = json!({
        "type": "object",
        "description": "Exactly one of url, path, or bytes must be set. mimeType is required when bytes is set; for url it acts as a type-inference override.",
        "properties": {
            "url": { "type": "string", "description": "HTTPS URL. Pre-signed URLs are fine but must not expire mid-download." },
            "path": { "type": "string", "description": "Absolute local file or directory path, readable by the Palmier process. A directory is imported recursively — every openable file is pulled in and the folder structure is replicated as media folders." },
            "bytes": { "type": "string", "description": "Base64-encoded media data. Prefer url or path for anything over ~10MB." },
            "mimeType": { "type": "string", "description": "Required when bytes is set. Optional override for url when its path has no usable extension (e.g. signed URLs). Accepted: video/mp4, video/quicktime, audio/mpeg, audio/wav, audio/aac, audio/mp4, image/png, image/jpeg, image/tiff, image/heic." }
        }
    });
    object_schema(
        &[
            ("source", source),
            ("name", str_prop("Display name in the library. Defaults to the filename derived from url/path, or 'Imported asset' for bytes.")),
            ("folderId", str_prop("Optional. Folder id (from list_folders or create_folder) to place the result in. Omit for the project root.")),
        ],
        &["source"],
    )
}

fn schema_create_folder() -> Value {
    let entry_item = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string", "description": "Folder name." },
            "parentFolderId": { "type": "string", "description": "Optional parent folder id; omit for top level." }
        },
        "required": ["name"]
    });
    object_schema(
        &[
            ("name", str_prop("Folder name.")),
            ("parentFolderId", str_prop("Optional parent folder id; omit for top level.")),
            ("entries", json!({
                "type": "array",
                "description": "Folders to create in one undoable action.",
                "items": entry_item
            })),
        ],
        &[],
    )
}

fn schema_move_to_folder() -> Value {
    let entry_item = json!({
        "type": "object",
        "properties": {
            "assetIds": { "type": "array", "items": { "type": "string" }, "description": "Media asset ids to move." },
            "folderId": { "type": "string", "description": "Destination folder id. Omit to move to the project root." }
        },
        "required": ["assetIds"]
    });
    object_schema(
        &[
            ("assetIds", json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Media asset ids to move."
            })),
            ("folderId", str_prop("Destination folder id. Omit to move to the project root.")),
            ("entries", json!({
                "type": "array",
                "description": "Move operations to apply in one undoable action. Each entry can target a different folder.",
                "items": entry_item
            })),
        ],
        &[],
    )
}

fn schema_rename_media() -> Value {
    let entry_item = json!({
        "type": "object",
        "properties": {
            "mediaRef": { "type": "string", "description": "Media asset id from get_media." },
            "name": { "type": "string", "description": "New display name." }
        },
        "required": ["mediaRef", "name"]
    });
    object_schema(
        &[
            ("mediaRef", str_prop("Media asset id from get_media.")),
            ("name", str_prop("New display name.")),
            ("entries", json!({
                "type": "array",
                "description": "Media assets to rename in one undoable action.",
                "items": entry_item
            })),
        ],
        &[],
    )
}

fn schema_rename_folder() -> Value {
    let entry_item = json!({
        "type": "object",
        "properties": {
            "folderId": { "type": "string", "description": "Folder id from list_folders." },
            "name": { "type": "string", "description": "New folder name." }
        },
        "required": ["folderId", "name"]
    });
    object_schema(
        &[
            ("folderId", str_prop("Folder id from list_folders.")),
            ("name", str_prop("New folder name.")),
            ("entries", json!({
                "type": "array",
                "description": "Folders to rename in one undoable action.",
                "items": entry_item
            })),
        ],
        &[],
    )
}

fn schema_delete_media() -> Value {
    object_schema(
        &[(
            "assetIds",
            json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Media asset ids to delete."
            }),
        )],
        &["assetIds"],
    )
}

fn schema_delete_folder() -> Value {
    object_schema(
        &[(
            "folderIds",
            json!({
                "type": "array",
                "items": { "type": "string" },
                "description": "Folder ids to delete."
            }),
        )],
        &["folderIds"],
    )
}

fn schema_list_models() -> Value {
    object_schema(
        &[(
            "type",
            enum_str_prop(&["video", "image", "audio", "upscale"], "Filter by type. Omit to list all models."),
        )],
        &[],
    )
}

// ── verbatim descriptions (byte-for-byte from ToolDefinitions.swift) ─────────
// Preserve Unicode (× U+00D7, – U+2013, • U+2022, …, curly quotes) as UTF-8.
// Do NOT ASCII-fold or paraphrase — these are LLM-tuned contract text (ruling #2).

const GET_TIMELINE_DESC: &str = "Always call at the start of a session. Returns project settings (fps, resolution, totalFrames), track list with types and order, all clips with their frames and properties, and canGenerate (if false, generation/upscale tools will fail — tell the user to sign in to Palmier and subscribe before attempting them). The clipId/trackId values here are what every other tool accepts.\n\nClip and track fields equal to their defaults are omitted: mediaType 'video', sourceClipType = mediaType, speed 1, volume 1, opacity 1, trims/fades 0, identity transform/crop, default textStyle, track muted/hidden false. Text clips never report trims (no source media).\n\nCaption clips (sharing a captionGroupId) come back per track as captionGroups instead of clips entries: properties common to the group are hoisted into 'shared' and each clip is a [clipId, startFrame, durationFrames, text] row (caption box width/height are auto-fit per text and omitted). Rows are capped at 200 per group — when clipCount exceeds the rows shown, page with startFrame/endFrame. Caption clips whose properties deviate from the group appear individually in clips.";

const GET_MEDIA_DESC: &str = "Call before referencing any asset. Every mediaRef/reference ID in other tools comes from the IDs returned here. Also exposes generationStatus (generating | downloading | failed | none) for async-generated and -imported assets.";

const INSPECT_MEDIA_DESC: &str = "Look at a media asset before referencing or editing it. Images: the image plus dimensions and EXIF. Video: sample frames plus a transcription of the audio track. Audio: transcription. Lottie: frames sampled evenly across the animation (over gray), plus framerate and duration — use this to verify a Lottie you wrote looks and moves right. Transcription is sentence-level segments — [text, start, end] tuples, capped at 400 — in source seconds, or project frames when clipId is set. When capped, pass the returned nextStartSeconds as startSeconds for the next page.\n\nLong media: pass overview=true for a one-image storyboard, read the segments, then re-call with startSeconds/endSeconds to zoom — windowed calls only transcribe that span, so they are fast.";

const GET_TRANSCRIPT_DESC: &str = "Returns the spoken transcript of the CURRENT timeline in project frames — the post-edit caption track in one call. Unlike inspect_media (which transcribes one source asset in isolation, in source seconds), this walks every audio/video clip on the timeline, maps each word through that clip's trim/speed/position, and concatenates in timeline order. Deleted ranges are gone by construction, so after cuts this always reflects what's actually audible — no stale results, no per-clip frame math.\n\nReturns clips in timeline order, each with its words nested as compact [text, startFrame, endFrame] rows (the field order is given once in wordFormat) — clipId and trackIndex are stated once per clip, not repeated per word. Words are monotonic and non-overlapping; each is attributed to one clip, so a word split across a clip seam is emitted once, not re-emitted per clip. Pass a clip's clipId and a word's frames straight to ripple_delete_ranges. Capped at 10000 words total; page with startFrame/endFrame using nextStartFrame. Pass clipId to scope to a single clip (\"what does this clip say?\"). Transcription runs on-device.\n\nUse for transcript-driven edits (filler-word / dead-air removal, locating a quote, take selection) and to verify what remains after cutting.";

const INSPECT_TIMELINE_DESC: &str = "See the composited timeline — what the user actually sees in the preview at a given frame: all video tracks stacked with their transforms, opacity, crop, and keyframes applied, plus text and caption overlays baked in. Use this to verify your edits landed (a PIP's position, a title's placement, layer order) — inspect_media shows the raw source asset, not the cut.\n\nFrames are project frames (from get_timeline). Pass a single startFrame for one composited frame; add endFrame to sample maxFrames evenly across [startFrame, endFrame) for a transition or sequence. Frames past content render black. Returns frames downscaled for token efficiency, with the frameNumbers sampled.";

const SEARCH_MEDIA_DESC: &str = "Search the media library by content: what's on screen (visual) and what's said (spoken). Visual matching is semantic and on-device — phrase the query like an image caption ('a wide shot of a harbor at sunset'), not keywords; covers videos and stills. Spoken matching layers exact keywords over on-device semantic matching of transcript segments — quote the words said, or paraphrase them; transcripts are created automatically while indexing (and by inspect_media and add_captions), so coverage grows as indexing completes. The two groups rank independently and are never blended. Scores are uncalibrated — use them for ordering only.\n\nHits are source-second ranges. To place exactly that moment, multiply by fps and pass as trimStartFrame/trimEndFrame with a matching durationFrames to add_clips or set_clip_properties. Image hits have no time range.\n\nstatus reports the visual index: ready | indexing | modelNotInstalled | downloadingModel | preparing | disabled | failed. When not ready, moments may be empty or incomplete (compare indexedAssets to indexableAssets) — report that instead of concluding the footage doesn't exist, and don't poll in a loop. Spoken results work regardless of status.";

const ADD_CLIPS_DESC: &str = "Places one or more media assets on the timeline as a single undoable action. Each entry's asset type must be compatible with its target track (video/image are interchangeable across video/image tracks; audio requires an audio track). When a video asset with audio is placed on a video track, a linked audio clip is automatically created on an audio track (an existing one if available, otherwise a new one). The whole batch is one undo step.\n\ntrackIndex is optional. Omit it on all entries and the tool auto-creates the needed tracks — one shared video track for visual entries and one shared audio track for audio entries (matches the captioning pattern in add_texts). To target existing tracks, set trackIndex on every entry. Mixing (some entries specify, others omit) is rejected — split into two calls.\n\nTracks work as layers: clips on the SAME track are sequential — if a new clip's range overlaps an existing clip on that track, the existing clip is trimmed/split/removed to make room, matching the UI's drag-onto-track overwrite behavior.";

const REMOVE_CLIPS_DESC: &str = "Removes one or more clips by ID as a single undoable action. Any clip that belongs to a link group (e.g. a video with its paired audio) takes its whole group with it, matching the UI's linked-delete behavior.";

const REMOVE_TRACKS_DESC: &str = "Removes whole tracks and every clip on them in one undoable action. Linked partners on OTHER tracks are not removed. Remaining track indexes shift down after removal.";

const MOVE_CLIPS_DESC: &str = "Moves one or more clips to a new track and/or frame position. Single undoable action. Each move specifies the clip ID and at least one of toTrack (must be compatible with the clip's media type) and toFrame. Overlap on the destination is resolved as in add_clips (existing clips on the destination track are trimmed/split/removed). Linked partners follow the named clip: startFrame propagates as a delta to preserve l-cut / j-cut offsets; tracks stay with the named clip.";

const SET_CLIP_PROPERTIES_DESC: &str = "Apply the same property values to one or more clips in a single undoable action. Pass any combination of durationFrames, trimStartFrame, trimEndFrame, speed, volume, opacity, transform, or — for text clips only — content, fontName, fontSize, color, alignment. All values are applied to every clip in clipIds; for per-clip differences, make separate calls. trimStartFrame/trimEndFrame are offsets from the source media, not the timeline. speed 1.0 is normal, <1.0 slows (clip gets longer on the timeline), >1.0 speeds up. volume and opacity are 0.0–1.0. transform uses 0–1 normalized canvas coords, partial merge (pass only centerY to reposition vertically); flipHorizontal/flipVertical mirror the clip across the corresponding axis (no effect on text clips). When a text clip's content or font changes without an explicit transform, the bounding box auto-refits. Text-only fields with any non-text clip in clipIds are rejected.\n\nFor moves and start-frame changes, use move_clips. For animated values (keyframes), use set_keyframes — setting volume or opacity here clears any existing keyframe track on that property.\n\nTiming changes (durationFrames, trimStartFrame, trimEndFrame, speed) on a linked clip carry over to its linked partner so audio/video stay in sync — same as the timeline UI. Per-clip fields (volume, opacity, transform, text*) don't propagate. trim and speed are skipped for text partners.";

const SET_KEYFRAMES_DESC: &str = "Set animated keyframes on one property of one clip. Replaces the existing keyframe track for that property (pass an empty array to clear). Frames are CLIP-RELATIVE offsets (0 = first frame of the clip), so keyframes follow the clip when it moves. Rows are sorted by frame internally and the LAST row for any duplicate frame wins. Values must be finite numbers. Each row is `[frame, ...values, interp?]` where interp ∈ {linear, hold, smooth} (default smooth).\n\nProperties and their value layouts:\n  • volume `[frame, value]` — value 0.0–1.0\n  • opacity `[frame, value]` — value 0.0–1.0\n  • rotation `[frame, degrees]` — clockwise degrees\n  • position `[frame, topLeftX, topLeftY]` — TOP-LEFT corner in 0–1 normalized canvas coords. NOT the center. (Default static transform centers a full-canvas clip, so top-left of the static is (0, 0); a centered half-size clip has top-left (0.25, 0.25).)\n  • scale `[frame, width, height]` — clip's normalized width and height in 0–1 canvas coords (1.0 = fills the canvas axis). NOT a scale factor.\n  • crop `[frame, top, right, bottom, left]` — side insets in 0–1 of the source media.\n\nMotion keyframes (position/scale/rotation) override the static `transform` value when active.";

const SPLIT_CLIP_DESC: &str = "Splits a clip into two at atFrame. The frame must be strictly between the clip's start and end — use get_timeline to confirm the range.";

const RIPPLE_DELETE_RANGES_DESC: &str = "Cuts one or more ranges out and closes the gaps in one undoable action — the fast path for filler-word/dead-air removal. Replaces hand-cranked split_clip → split_clip → remove_clips → move_clips loops: pass every range at once.\n\nTwo modes — pass exactly one of clipId or trackIndex:\n• trackIndex (preferred for transcript-driven cuts): ranges are PROJECT frames and may span any number of clips on that track. get_transcript returns a clips array with nested words in project frames — collect every cut across the whole timeline and pass them in ONE call, no per-clip splitting and no re-reading the timeline between cuts. units must be 'frames'.\n• clipId: ranges are cut within that single clip only, clamped to its visible span. Allows units 'seconds' (source-media seconds, e.g. inspect_media WITHOUT a clipId or search_media hits); 'frames' = project frames. Use when you already have one clip's per-word timestamps.\n\nOverlapping ranges merge. Linked audio/video partners of every touched clip are cut on the same span so A/V stays in sync. Remaining clips shift left to close every gap; sync-locked tracks shift along to preserve alignment (their content isn't cut). Refuses without changing anything if a sync-locked track can't absorb the shift (e.g. it would move past frame 0). Returns the anchor track's post-cut layout (clip ids/frames) so you don't need to re-read.";

const UNDO_DESC: &str = "Reverts the assistant's most recent timeline edit (a cut, move, trim, split, or clip/text/caption add) as one step. The recovery path when an edit went too far — e.g. a ripple_delete_ranges removed more than intended. Verify a cut first (get_transcript reflects the post-cut audio), then undo if it overshot, then retry with corrected ranges.\n\nUndoes only edits the assistant made this session, most-recent-first — it never touches the user's own manual edits, and refuses if the latest change wasn't the assistant's. After undoing, the timeline is restored to its state before that edit; the ids/frames the edit returned are no longer valid, so re-read with get_timeline or get_transcript if you'll edit again. Takes no arguments.";

const ADD_TEXTS_DESC: &str = "Adds one or more text clips (titles, captions, lower-thirds) in a single undoable action. Text renders as an overlay on top of visual media. Transform uses 0–1 normalized canvas coords: (0.5,0.5) is center, (0.5,0.1) top-center, (0.5,0.9) bottom-center. Omit transform to center + auto-fit. Pass only centerX/centerY to reposition with auto-fit size (common for lower-thirds). Pass all four fields to override the box entirely. Colors are hex '#RRGGBB' or '#RRGGBBAA'.\n\ntrackIndex is optional. Omit it on all entries and the tool auto-creates one new video track at the top and places all text clips there — the common case for captions. To target existing tracks, set trackIndex on every entry (audio tracks rejected). Mixing (some entries specify, others omit) is rejected — split into two calls.\n\nTracks work as layers: clips on the SAME track are sequential — if a new clip's range overlaps an existing (or earlier-batch) clip on that track, the existing clip is trimmed/split/removed to make room, matching the UI's drag-onto-track overwrite behavior. To show multiple text clips at the same time (stacked titles, simultaneous labels), put each on a DIFFERENT trackIndex so they layer instead of trimming each other.\n\nFor captioning spoken audio, prefer add_captions — it transcribes and places styled caption clips in one call. Use add_texts only for bespoke text (titles, lower-thirds) or captioning a custom range by hand. Unknown fields are rejected.";

const ADD_CAPTIONS_DESC: &str = "Auto-caption spoken audio: transcribes on-device and places styled caption clips on a new track — the same pipeline as the editor's Captions tab. This is the reliable path for 'caption this'; prefer it over hand-placing add_texts from a transcript. Omit clipIds to auto-pick the track with the most speech; pass clipIds to caption specific clips (e.g. only the interview).";

const GENERATE_VIDEO_DESC: &str = "Starts an async AI video generation. Returns a placeholder asset ID immediately; generation runs in the background and the asset becomes usable in add_clips once ready. Costs real money and is not undoable.";

const GENERATE_IMAGE_DESC: &str = "Starts an async AI image generation. Returns a placeholder asset ID immediately; generation runs in the background. Costs real money and is not undoable.";

const GENERATE_AUDIO_DESC: &str = "Starts an async AI audio generation: text-to-speech, text-to-music, or video-to-music (scoring a video). Returns a placeholder asset ID immediately; the asset appears in get_media and becomes usable in add_clips once ready. TTS models (elevenlabs-tts-v3, gemini-3.1-flash-tts) convert the prompt into speech and accept a 'voice'. Music models (lyria3-pro, minimax-music-v2.6, elevenlabs-music, sonilo-v1.1-video-to-music) generate tracks from a prompt; include lyrics/tempo/vocal style in the prompt for Lyria 3 Pro, pass 'lyrics' for MiniMax vocals, or set 'instrumental' true when the selected model supports it. Video-to-audio models (inputs include 'video' — see list_models, e.g. sonilo-v1.1-video-to-music, mirelo-sfx-v1.5-video-to-audio) generate audio that matches a VIDEO: provide a timeline span via videoSourceStartFrame+videoSourceEndFrame (e.g. to score the timeline), or a video asset via videoSourceMediaRef; the prompt is then an optional style guide. PLACEMENT: when you pass a timeline span, the result is placed on the timeline automatically at that span (no add_clips needed); for a media-asset source or a plain text-to-speech/music result, the asset lands in the library and you place it with add_clips. Use list_models with type='audio' to see each model's 'inputs', category, and voices. Costs real money and is not undoable.";

const UPSCALE_MEDIA_DESC: &str = "Upscales an existing video or image asset to higher resolution using an AI upscaler. Returns a placeholder asset ID immediately; the upscaled asset appears in get_media once ready. Use list_models with type='upscale' to pick a model that supports the asset's type. Costs real money and is not undoable.";

const IMPORT_MEDIA_DESC: &str = "Imports external media into the project's library — the bridge for assets coming from other MCP servers (stock libraries, music services, web search) or local files the user already has. The 'source' object must set exactly one of: url (HTTPS only — downloaded in the background, the dominant case; max 1 GB), path (absolute local file path — referenced in place; may also be a directory, which is imported recursively, mirroring its subfolder structure as media folders), or bytes (base64-encoded inline data — max ~15 MB of base64 ≈ 11 MB binary; use url/path for anything larger). For url, type is inferred from the URL path's file extension unless source.mimeType is set as an override (needed for signed URLs whose path has no usable extension). For bytes, source.mimeType is required.\n\nSupported types and extensions: video (mov, mp4, m4v), audio (mp3, wav, aac, m4a), image (png, jpg, jpeg, tiff, heic). Anything else is rejected — the caller must transcode externally.\n\nReturns a placeholder asset id immediately; URL imports run in the background and the asset becomes usable in add_clips once ready (same async pattern as generate_*). Path and bytes imports finalize synchronously. Costs nothing.";

const LIST_FOLDERS_DESC: &str = "Lists every folder in the media panel as {id, name, parentFolderId}. Folders are nested (parentFolderId is nil for top-level). Use to find an existing folder by name before generating new media.";

const CREATE_FOLDER_DESC: &str = "Creates folders in the media panel. Pass either name/parentFolderId for one folder or entries for multiple folders, not both. Direct form returns one folder; entries returns { folders }. Undoable. Use to organize related generations (e.g. 'Hero shot variations'). Don't create folders for unrelated concepts.";

const MOVE_TO_FOLDER_DESC: &str = "Moves media assets to folders. Pass either assetIds/folderId for one destination or entries for multiple destinations, not both. Omit folderId to move to root. Undoable.";

const RENAME_MEDIA_DESC: &str = "Renames media assets in the library. Pass either mediaRef/name for one asset or entries for multiple assets, not both. Undoable.";

const RENAME_FOLDER_DESC: &str = "Renames folders in the media panel. Pass either folderId/name for one folder or entries for multiple folders, not both. Undoable.";

const DELETE_MEDIA_DESC: &str = "Deletes media assets from the library. Any clips referencing them are removed from the timeline in the same undoable action.";

const DELETE_FOLDER_DESC: &str = "Deletes folders and everything inside them (subfolders and assets). Clips referencing any deleted asset are removed from the timeline in the same undoable action.";

const LIST_MODELS_DESC: &str = "Lists AI models with their capabilities (durations, aspect ratios, resolutions, first/last frame support, reference support, voices/category for audio, upscaler speed). Always call before generate_video, generate_image, generate_audio, or upscale_media so the model you pick actually supports the constraints you need. Returns { models, loaded } — if loaded=false the catalog hasn't synced yet (e.g. user not signed in); the models array may be empty even when models exist, so do not conclude no models are available. Retry after the user signs in.";
