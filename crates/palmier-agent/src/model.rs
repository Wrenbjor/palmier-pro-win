//! Message / content / session data model + JSON (de)serialization.
//!
//! Ports the value types from `Agent/AgentService.swift` (`AgentMessage`,
//! `AgentContentBlock`, `Role`), `Agent/Tools/ToolResult.swift`
//! (`ToolResult.Block`), `Agent/ChatSessionStore.swift` (`ChatSession`), and
//! `Agent/AgentMentionContext.swift` (`AgentMention`,
//! `AgentTimelineRangeMention`). Serde matches the reference `Codable` wire JSON
//! exactly — the **`kind` discriminator** values (`text`/`toolUse`/`toolResult`,
//! and `text`/`image` for tool-result blocks) and the camelCase keys are
//! load-bearing for round-tripping persisted `chat/*.json` and for the wire
//! projection (E8-S5).
//!
//! ## `input_json` is stored and forwarded verbatim
//! [`AgentContentBlock::ToolUse::input_json`] is a **raw JSON string**, never a
//! parsed object. The serializer MUST NOT round-trip / normalize it: re-encoding
//! would change bytes and break prompt-cache determinism (`agent-panel.md` lines
//! 41-42, 201-203; reconciliation carry-forward "store and forward verbatim").
//! Empty → `"{}"`. Storing it under a `String`-typed field (key `input`) keeps
//! the exact bytes — serde emits it JSON-string-escaped, identical to Swift's
//! `c.encode(inputJSON, forKey: .input)`.

use palmier_model::ClipType;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Message author (reference `AgentMessage.Role`). Wire = `"user"`/`"assistant"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// A user turn.
    User,
    /// An assistant (model) turn.
    Assistant,
}

/// One block inside a `tool_result` (reference `ToolResult.Block`).
///
/// Codable with a `kind` discriminator (`text`/`image`) and keys `text` /
/// `base64` / `mediaType` (`agent-panel.md` line 43, `ToolResult.swift`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ToolResultBlock {
    /// Plain-text tool output.
    Text {
        /// The text payload.
        text: String,
    },
    /// An inlined image (base64 + media type), e.g. a rendered frame.
    Image {
        /// Base64-encoded image bytes.
        base64: String,
        /// IANA media type (e.g. `image/jpeg`).
        #[serde(rename = "mediaType")]
        media_type: String,
    },
}

impl ToolResultBlock {
    /// Convenience constructor for a text block.
    #[must_use]
    pub fn text(s: impl Into<String>) -> Self {
        ToolResultBlock::Text { text: s.into() }
    }

    /// Convenience constructor for an image block.
    #[must_use]
    pub fn image(base64: impl Into<String>, media_type: impl Into<String>) -> Self {
        ToolResultBlock::Image {
            base64: base64.into(),
            media_type: media_type.into(),
        }
    }
}

/// One content block of an [`AgentMessage`] (reference `AgentContentBlock`).
///
/// Codable with a `kind` discriminator (`text`/`toolUse`/`toolResult`). Note the
/// reference quirk: the **wire key for the tool-use raw JSON is `input`** (the
/// field is named `inputJSON`), and tool-result uses `toolUseId` / `content` /
/// `isError`. We reproduce those keys exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AgentContentBlock {
    /// Assistant or user text (reference `.text`).
    #[serde(rename = "text")]
    Text {
        /// The text payload.
        text: String,
    },
    /// A tool call the model emitted (reference `.toolUse`).
    ///
    /// `input_json` is the **raw, verbatim** input JSON string (key `input`);
    /// empty → `"{}"`; never normalized.
    #[serde(rename = "toolUse")]
    ToolUse {
        /// The tool-use block id (e.g. `toolu_…`).
        id: String,
        /// The tool name.
        name: String,
        /// The raw input JSON string, stored and forwarded verbatim.
        #[serde(rename = "input")]
        input_json: String,
    },
    /// The result of executing a tool, fed back to the model (reference
    /// `.toolResult`).
    #[serde(rename = "toolResult")]
    ToolResult {
        /// The `id` of the [`AgentContentBlock::ToolUse`] this answers.
        #[serde(rename = "toolUseId")]
        tool_use_id: String,
        /// The result blocks (text and/or images).
        content: Vec<ToolResultBlock>,
        /// Whether the tool errored.
        #[serde(rename = "isError")]
        is_error: bool,
    },
}

impl AgentContentBlock {
    /// A text block.
    #[must_use]
    pub fn text(s: impl Into<String>) -> Self {
        AgentContentBlock::Text { text: s.into() }
    }

    /// A tool-use block. Empties the `input_json` to `"{}"` (reference rule), then
    /// stores it verbatim.
    #[must_use]
    pub fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input_json: impl Into<String>,
    ) -> Self {
        let input_json = input_json.into();
        let input_json = if input_json.is_empty() {
            "{}".to_string()
        } else {
            input_json
        };
        AgentContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input_json,
        }
    }

    /// A tool-result block.
    #[must_use]
    pub fn tool_result(
        tool_use_id: impl Into<String>,
        content: Vec<ToolResultBlock>,
        is_error: bool,
    ) -> Self {
        AgentContentBlock::ToolResult {
            tool_use_id: tool_use_id.into(),
            content,
            is_error,
        }
    }

    /// The discriminator string for this block (`text`/`toolUse`/`toolResult`).
    /// Mirrors the reference `Kind` raw values.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            AgentContentBlock::Text { .. } => "text",
            AgentContentBlock::ToolUse { .. } => "toolUse",
            AgentContentBlock::ToolResult { .. } => "toolResult",
        }
    }
}

/// Half-open frame range selection a `timelineRange` mention captures (reference
/// `AgentTimelineRangeMention`).
///
/// `startFrame` is inclusive, `endFrame` is **exclusive**; `rangeSemantics` is
/// the literal `"startInclusiveEndExclusive"` (`agent-panel.md` lines 129-132).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTimelineRangeMention {
    #[serde(rename = "startFrame")]
    pub start_frame: i64,
    #[serde(rename = "endFrame")]
    pub end_frame: i64,
    #[serde(rename = "durationFrames")]
    pub duration_frames: i64,
    pub fps: i64,
    #[serde(rename = "startTimecode")]
    pub start_timecode: String,
    #[serde(rename = "endTimecode")]
    pub end_timecode: String,
    #[serde(rename = "durationTimecode")]
    pub duration_timecode: String,
    #[serde(rename = "rangeSemantics")]
    pub range_semantics: String,
}

/// An `@`-mention attached to a user turn (reference `AgentMention`).
///
/// One of three kinds, distinguished by which optional fields are set:
/// `mediaAsset` (`media_ref` + `type`), `timelineClip` (adds `clip_id`), or
/// `timelineRange` (`timeline_range`). The wire shape uses `decodeIfPresent`
/// (optional) for everything but `id` + `displayName`, so absent fields are
/// `skip_serializing_if = None` for round-trip stability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMention {
    pub id: Uuid,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "mediaRef", default, skip_serializing_if = "Option::is_none")]
    pub media_ref: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub clip_type: Option<ClipType>,
    #[serde(rename = "clipId", default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<String>,
    #[serde(
        rename = "timelineRange",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeline_range: Option<AgentTimelineRangeMention>,
}

impl AgentMention {
    /// True when this mention references a timeline clip (reference
    /// `referencesTimelineClips`).
    #[must_use]
    pub fn references_timeline_clips(&self) -> bool {
        self.clip_id.is_some()
    }

    /// True when this mention references a timeline range (reference
    /// `referencesTimelineRange`).
    #[must_use]
    pub fn references_timeline_range(&self) -> bool {
        self.timeline_range.is_some()
    }
}

/// A single chat turn (reference `AgentMessage`).
///
/// `mentions` defaults to empty and `context_hint` to absent (reference's
/// `init` defaults). The wire shape keeps the reference field names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMessage {
    pub id: Uuid,
    pub role: Role,
    pub blocks: Vec<AgentContentBlock>,
    #[serde(default)]
    pub mentions: Vec<AgentMention>,
    #[serde(
        rename = "contextHint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub context_hint: Option<String>,
}

impl AgentMessage {
    /// New message with a fresh `id` and no mentions / context hint (reference
    /// `init(role:blocks:)`).
    #[must_use]
    pub fn new(role: Role, blocks: Vec<AgentContentBlock>) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            blocks,
            mentions: Vec::new(),
            context_hint: None,
        }
    }

    /// New message carrying mentions + an optional context hint.
    #[must_use]
    pub fn with_context(
        role: Role,
        blocks: Vec<AgentContentBlock>,
        mentions: Vec<AgentMention>,
        context_hint: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            blocks,
            mentions,
            context_hint,
        }
    }

    /// First user text in this message, if any — used to auto-derive a session
    /// title (reference `ChatSession` title derivation).
    #[must_use]
    pub fn first_text(&self) -> Option<&str> {
        self.blocks.iter().find_map(|b| match b {
            AgentContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
    }
}

/// Default title for a fresh chat session (reference `"New chat"`).
pub const DEFAULT_SESSION_TITLE: &str = "New chat";

/// Serialize `value` as **pretty-printed, sorted-key** JSON — the reference
/// `JSONEncoder` `[.prettyPrinted, .sortedKeys]` canonical form used for
/// `chat/*.json` (`agent-panel.md` lines 158-159).
///
/// `serde`'s struct serializer emits fields in *declaration* order; routing
/// through [`serde_json::Value`] (whose object map is a `BTreeMap` when the
/// `preserve_order` feature is off) yields **alphabetically sorted** keys at
/// every depth, matching Swift's `.sortedKeys`. Nested `input_json` strings stay
/// verbatim because they are JSON *strings*, not parsed objects.
///
/// # Errors
/// Propagates a `serde_json` error if `value` is not serializable.
pub fn to_canonical_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    let v = serde_json::to_value(value)?;
    serde_json::to_string_pretty(&v)
}

/// A persisted chat tab (reference `ChatSession`).
///
/// `updated_at` is encoded as **ISO-8601** (chat uses RFC3339 strings, distinct
/// from project/media Apple-epoch doubles — reconciliation "Project I/O Date
/// encoding"; `serde_date::iso8601`). `is_open` decodes-if-present defaulting
/// `true` (`agent-panel.md` lines 149-150).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: Uuid,
    pub title: String,
    #[serde(rename = "updatedAt", with = "palmier_model::serde_date::iso8601")]
    pub updated_at: OffsetDateTime,
    pub messages: Vec<AgentMessage>,
    #[serde(rename = "isOpen", default = "default_is_open")]
    pub is_open: bool,
}

/// `isOpen` decode-if-present default (reference: `decodeIfPresent ?? true`).
fn default_is_open() -> bool {
    true
}

impl ChatSession {
    /// A fresh empty open session with title `"New chat"` and `updated_at = now`
    /// (reference `ChatSession.init`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            updated_at: OffsetDateTime::now_utc(),
            messages: Vec::new(),
            is_open: true,
        }
    }

    /// Whether this session has no messages (filtered out on both load and save —
    /// `agent-panel.md` lines 151, 158).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Copy `messages` in, bump `updated_at`, and if the title is still the
    /// default derive it from the first 40 chars of the first user text (reference
    /// `syncMessagesIntoCurrentSession`). Returns `self` for chaining.
    pub fn sync_messages(&mut self, messages: Vec<AgentMessage>) {
        self.messages = messages;
        self.updated_at = OffsetDateTime::now_utc();
        if self.title == DEFAULT_SESSION_TITLE
            && let Some(title) = self.derive_title()
        {
            self.title = title;
        }
    }

    /// First-40-chars-of-first-user-text title (reference title derivation).
    /// `None` when there is no non-empty user text yet (title stays `"New chat"`).
    fn derive_title(&self) -> Option<String> {
        let first_user = self
            .messages
            .iter()
            .find(|m| m.role == Role::User)
            .and_then(AgentMessage::first_text)?;
        let trimmed = first_user.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.chars().take(40).collect())
    }
}

impl Default for ChatSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn round_trip_block(block: &AgentContentBlock) -> AgentContentBlock {
        let json = serde_json::to_string(block).unwrap();
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn text_block_round_trips_with_kind() {
        let b = AgentContentBlock::text("hello");
        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(json["kind"], "text");
        assert_eq!(json["text"], "hello");
        assert_eq!(round_trip_block(&b), b);
    }

    #[test]
    fn tool_use_block_round_trips_with_input_key() {
        let b = AgentContentBlock::tool_use("toolu_1", "get_timeline", r#"{"page":2}"#);
        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(json["kind"], "toolUse");
        assert_eq!(json["id"], "toolu_1");
        assert_eq!(json["name"], "get_timeline");
        // Raw JSON stored under the reference key `input`, as a STRING (escaped).
        assert_eq!(json["input"], r#"{"page":2}"#);
        assert_eq!(round_trip_block(&b), b);
    }

    #[test]
    fn tool_result_block_round_trips_with_reference_keys() {
        let b = AgentContentBlock::tool_result(
            "toolu_1",
            vec![
                ToolResultBlock::text("ok"),
                ToolResultBlock::image("YWJj", "image/jpeg"),
            ],
            false,
        );
        let json = serde_json::to_value(&b).unwrap();
        assert_eq!(json["kind"], "toolResult");
        assert_eq!(json["toolUseId"], "toolu_1");
        assert_eq!(json["isError"], false);
        assert_eq!(json["content"][0]["kind"], "text");
        assert_eq!(json["content"][1]["kind"], "image");
        assert_eq!(json["content"][1]["mediaType"], "image/jpeg");
        assert_eq!(round_trip_block(&b), b);
    }

    #[test]
    fn kind_discriminator_values() {
        assert_eq!(AgentContentBlock::text("x").kind(), "text");
        assert_eq!(
            AgentContentBlock::tool_use("i", "n", "{}").kind(),
            "toolUse"
        );
        assert_eq!(
            AgentContentBlock::tool_result("i", vec![], true).kind(),
            "toolResult"
        );
    }

    #[test]
    fn input_json_passes_through_unmodified_including_whitespace() {
        // Deliberately-non-canonical JSON with odd spacing: must survive byte-exact.
        let weird = "{  \"b\" : 2,\n  \"a\":1 }";
        let b = AgentContentBlock::tool_use("toolu_x", "ripple_delete", weird);
        let json = serde_json::to_string(&b).unwrap();
        let back: AgentContentBlock = serde_json::from_str(&json).unwrap();
        match back {
            AgentContentBlock::ToolUse { input_json, .. } => {
                // Bytes identical — NOT normalized / re-ordered.
                assert_eq!(input_json, weird);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn empty_input_json_defaults_to_braces() {
        let b = AgentContentBlock::tool_use("toolu_x", "noop", "");
        match b {
            AgentContentBlock::ToolUse { input_json, .. } => assert_eq!(input_json, "{}"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn role_wire_values() {
        assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
    }

    #[test]
    fn agent_message_round_trips() {
        let msg = AgentMessage::new(
            Role::Assistant,
            vec![
                AgentContentBlock::text("done"),
                AgentContentBlock::tool_use("toolu_2", "split_clip", r#"{"frame":120}"#),
            ],
        );
        let json = serde_json::to_string(&msg).unwrap();
        let back: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn agent_message_omits_empty_mentions_default_on_decode() {
        // A persisted message without `mentions` / `contextHint` decodes cleanly.
        let json = r#"{
            "id": "11111111-1111-1111-1111-111111111111",
            "role": "user",
            "blocks": [ { "kind": "text", "text": "hi" } ]
        }"#;
        let msg: AgentMessage = serde_json::from_str(json).unwrap();
        assert!(msg.mentions.is_empty());
        assert_eq!(msg.context_hint, None);
        assert_eq!(msg.role, Role::User);
    }

    #[test]
    fn mention_kinds_round_trip() {
        let media = AgentMention {
            id: Uuid::nil(),
            display_name: "clip-a".to_string(),
            media_ref: Some("media_1".to_string()),
            clip_type: Some(ClipType::Video),
            clip_id: None,
            timeline_range: None,
        };
        let range = AgentMention {
            id: Uuid::nil(),
            display_name: "range-1".to_string(),
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
        };
        for m in [&media, &range] {
            let json = serde_json::to_string(m).unwrap();
            let back: AgentMention = serde_json::from_str(&json).unwrap();
            assert_eq!(&back, m);
        }
        // Half-open range semantics preserved verbatim.
        let v = serde_json::to_value(&range).unwrap();
        assert_eq!(
            v["timelineRange"]["rangeSemantics"],
            "startInclusiveEndExclusive"
        );
    }

    #[test]
    fn chat_session_round_trips_byte_stable_and_sorted_keys() {
        let mut session = ChatSession {
            id: uuid::uuid!("22222222-2222-2222-2222-222222222222"),
            title: "Cut the intro".to_string(),
            updated_at: datetime!(2026-06-20 12:00:00 UTC),
            messages: vec![AgentMessage::with_context(
                Role::User,
                vec![AgentContentBlock::text("cut the intro")],
                vec![],
                None,
            )],
            is_open: true,
        };
        // Stable id on the message so the encode is deterministic.
        session.messages[0].id = uuid::uuid!("33333333-3333-3333-3333-333333333333");

        let encoded = to_canonical_json(&session).unwrap();
        // Re-encode is byte-identical (sorted-keys via serde_json's BTreeMap-backed
        // Map; pretty-printed).
        let decoded: ChatSession = serde_json::from_str(&encoded).unwrap();
        let reencoded = to_canonical_json(&decoded).unwrap();
        assert_eq!(encoded, reencoded, "session encode must be byte-stable");

        // Top-level keys appear in deterministic (sorted) order.
        let id_pos = encoded.find("\"id\"").unwrap();
        let is_open_pos = encoded.find("\"isOpen\"").unwrap();
        let messages_pos = encoded.find("\"messages\"").unwrap();
        let title_pos = encoded.find("\"title\"").unwrap();
        let updated_pos = encoded.find("\"updatedAt\"").unwrap();
        assert!(id_pos < is_open_pos);
        assert!(is_open_pos < messages_pos);
        assert!(messages_pos < title_pos);
        assert!(title_pos < updated_pos);
    }

    #[test]
    fn chat_session_updated_at_is_iso8601() {
        let session = ChatSession {
            id: Uuid::nil(),
            title: "x".to_string(),
            updated_at: datetime!(2026-06-20 12:00:00 UTC),
            messages: vec![],
            is_open: true,
        };
        let v = serde_json::to_value(&session).unwrap();
        assert_eq!(v["updatedAt"], "2026-06-20T12:00:00Z");
    }

    #[test]
    fn chat_session_is_open_defaults_true_when_absent() {
        let json = r#"{
            "id": "44444444-4444-4444-4444-444444444444",
            "title": "x",
            "updatedAt": "2026-06-20T12:00:00Z",
            "messages": []
        }"#;
        let s: ChatSession = serde_json::from_str(json).unwrap();
        assert!(s.is_open);
    }

    #[test]
    fn sync_messages_derives_title_once_then_stops() {
        let mut s = ChatSession::new();
        assert_eq!(s.title, DEFAULT_SESSION_TITLE);
        s.sync_messages(vec![AgentMessage::new(
            Role::User,
            vec![AgentContentBlock::text(
                "Please trim the first ten seconds of the opening shot",
            )],
        )]);
        // Derived from first 40 chars.
        assert_eq!(s.title, "Please trim the first ten seconds of the");
        assert!(!s.is_empty());

        // A later sync with different first text does NOT re-derive (title no
        // longer the default).
        s.sync_messages(vec![AgentMessage::new(
            Role::User,
            vec![AgentContentBlock::text("a completely different message")],
        )]);
        assert_eq!(s.title, "Please trim the first ten seconds of the");
    }

    #[test]
    fn sync_messages_keeps_default_title_when_no_user_text() {
        let mut s = ChatSession::new();
        s.sync_messages(vec![AgentMessage::new(
            Role::Assistant,
            vec![AgentContentBlock::text("assistant only")],
        )]);
        assert_eq!(s.title, DEFAULT_SESSION_TITLE);
    }
}
