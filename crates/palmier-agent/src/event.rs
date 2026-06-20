//! Streaming model + event types.
//!
//! Ports the value types from `Agent/Clients/AgentClientTypes.swift`
//! (`AnthropicModel`, `AnthropicStopReason`, `AnthropicStreamEvent`) into Rust.
//! These are the **shape the (later) SSE parser emits** — E8-S1 lands the types;
//! E8-S2 wires the real `parse(byte_stream)`.
//!
//! Only **three** stream events reach the agentic loop (`agent-panel.md` lines
//! 33-38, 82-94): a text delta, a completed tool-use block, and the message-stop
//! signal. `message_start` usage is logged internally and never surfaces; `ping`
//! and other event types are dropped by the parser. We add one extra terminal
//! [`StreamEvent::Error`] / informational [`StreamEvent::MessageStart`] variant
//! for the Rust port: the Swift parser carried usage via a side channel
//! (`AgentUsageLog`) and errors via `continuation.finish(throwing:)`; in Rust a
//! `Stream` of events is the idiomatic carrier, so usage + error become explicit
//! variants the loop can match on.

use serde::{Deserialize, Serialize};

/// The Anthropic models the agent can target.
///
/// Reference `AnthropicModel` (`AgentClientTypes.swift`): the `rawValue` is the
/// exact wire model id sent in the request body — **do not** abbreviate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnthropicModel {
    /// `claude-sonnet-4-6` — default paid / catalog model.
    #[serde(rename = "claude-sonnet-4-6")]
    Sonnet46,
    /// `claude-opus-4-8` — BYOK + catalog-enabled paid (ruling #20).
    #[serde(rename = "claude-opus-4-8")]
    Opus48,
    /// `claude-haiku-4-5-20251001` — free tier + BYOK.
    #[serde(rename = "claude-haiku-4-5-20251001")]
    Haiku45,
}

impl AnthropicModel {
    /// All models in reference declaration order (`CaseIterable`).
    pub const ALL: [AnthropicModel; 3] = [
        AnthropicModel::Sonnet46,
        AnthropicModel::Opus48,
        AnthropicModel::Haiku45,
    ];

    /// Exact wire model id (Swift `rawValue`). This is the value placed in the
    /// request body's `model` field.
    #[must_use]
    pub fn wire_id(self) -> &'static str {
        match self {
            AnthropicModel::Sonnet46 => "claude-sonnet-4-6",
            AnthropicModel::Opus48 => "claude-opus-4-8",
            AnthropicModel::Haiku45 => "claude-haiku-4-5-20251001",
        }
    }

    /// Human label for the model picker (reference `displayName`).
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            AnthropicModel::Sonnet46 => "Sonnet 4.6",
            AnthropicModel::Opus48 => "Opus 4.8",
            AnthropicModel::Haiku45 => "Haiku 4.5",
        }
    }

    /// Parse a wire model id back into a model (inverse of [`Self::wire_id`]).
    #[must_use]
    pub fn from_wire_id(id: &str) -> Option<Self> {
        AnthropicModel::ALL.into_iter().find(|m| m.wire_id() == id)
    }
}

/// Why the model ended its turn (reference `AnthropicStopReason`).
///
/// The agentic loop branches on this: `ToolUse` → run pending tools and resume;
/// anything else → end the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnthropicStopReason {
    /// The model finished its turn normally.
    EndTurn,
    /// The model emitted tool calls and is waiting on results.
    ToolUse,
    /// The response hit `max_tokens`.
    MaxTokens,
    /// A configured stop sequence was hit.
    StopSequence,
    /// A server-side pause (long-running turn).
    PauseTurn,
    /// The model refused.
    Refusal,
    /// Any unrecognized stop reason (reference `.other` fallback).
    #[serde(other)]
    Other,
}

impl AnthropicStopReason {
    /// Map a raw wire string to a stop reason, defaulting to [`Self::Other`]
    /// (reference `AnthropicStopReason(rawValue:) ?? .other`).
    #[must_use]
    pub fn from_wire(raw: &str) -> Self {
        match raw {
            "end_turn" => Self::EndTurn,
            "tool_use" => Self::ToolUse,
            "max_tokens" => Self::MaxTokens,
            "stop_sequence" => Self::StopSequence,
            "pause_turn" => Self::PauseTurn,
            "refusal" => Self::Refusal,
            _ => Self::Other,
        }
    }
}

/// Token-usage snapshot from the `message_start` event (reference reads
/// `message.usage`, logs via `AgentUsageLog`).
///
/// In the Swift port this never reached the loop — it was a DEBUG print only.
/// The Rust port surfaces it as the [`StreamEvent::MessageStart`] payload so the
/// loop (or a telemetry seam) can choose to log it; the default behavior stays
/// DEBUG-only logging.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Non-cached input tokens billed.
    #[serde(default, rename = "input_tokens")]
    pub input_tokens: u64,
    /// Tokens written to the prompt cache this request.
    #[serde(default, rename = "cache_creation_input_tokens")]
    pub cache_creation_input_tokens: u64,
    /// Tokens served from the prompt cache this request.
    #[serde(default, rename = "cache_read_input_tokens")]
    pub cache_read_input_tokens: u64,
    /// Output tokens generated.
    #[serde(default, rename = "output_tokens")]
    pub output_tokens: u64,
}

impl Usage {
    /// Percentage of billed input tokens served from cache (reference
    /// `AgentUsageLog` `readPct`). 0 when nothing was billed.
    #[must_use]
    pub fn cache_read_pct(self) -> u32 {
        let billed =
            self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens;
        if billed == 0 {
            0
        } else {
            ((self.cache_read_input_tokens as f64 / billed as f64) * 100.0) as u32
        }
    }
}

/// One event from the streaming parser, consumed by the agentic loop.
///
/// Reference `AnthropicStreamEvent` carried only `textDelta` / `toolUseComplete`
/// / `messageStop`. The Rust port adds two variants that the Swift code expressed
/// out-of-band:
/// - [`Self::MessageStart`] carries the usage that Swift logged via `AgentUsageLog`.
/// - [`Self::Error`] carries the stream error Swift raised via
///   `continuation.finish(throwing:)`.
///
/// The loop (E8-S4) only acts on `TextDelta` / `ToolUseComplete` / `MessageStop`;
/// `MessageStart` is informational and `Error` is terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// `message_start` — usage counts for this request (logged, DEBUG-only).
    MessageStart {
        /// Token usage for the request.
        usage: Usage,
    },
    /// `content_block_delta` (text_delta) — a non-empty chunk of assistant text.
    TextDelta(String),
    /// `content_block_stop` for a `tool_use` block — the fully-accumulated call.
    ///
    /// `json` is the **raw, verbatim** accumulated `input_json` string (empty →
    /// `"{}"`); it is forwarded without normalization (`agent-panel.md` lines
    /// 41-42, 201-203).
    ToolUseComplete {
        /// The tool-use block id.
        id: String,
        /// The tool name.
        name: String,
        /// The raw accumulated input JSON (empty → `"{}"`).
        json: String,
    },
    /// `message_delta` — the turn ended; carries the stop reason.
    MessageStop {
        /// Why the turn ended.
        reason: AnthropicStopReason,
    },
    /// Terminal stream error (reference `streamError`).
    Error(String),
}

impl StreamEvent {
    /// A `ToolUseComplete` with the empty-json default applied (`"{}"`), matching
    /// the reference `content_block_stop` rule. Helper for the (later) parser and
    /// for tests.
    #[must_use]
    pub fn tool_use_complete(
        id: impl Into<String>,
        name: impl Into<String>,
        json: impl Into<String>,
    ) -> Self {
        let json = json.into();
        let json = if json.is_empty() {
            "{}".to_string()
        } else {
            json
        };
        StreamEvent::ToolUseComplete {
            id: id.into(),
            name: name.into(),
            json,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_wire_ids_match_reference() {
        assert_eq!(AnthropicModel::Sonnet46.wire_id(), "claude-sonnet-4-6");
        assert_eq!(AnthropicModel::Opus48.wire_id(), "claude-opus-4-8");
        assert_eq!(
            AnthropicModel::Haiku45.wire_id(),
            "claude-haiku-4-5-20251001"
        );
    }

    #[test]
    fn model_round_trips_through_wire_id() {
        for m in AnthropicModel::ALL {
            assert_eq!(AnthropicModel::from_wire_id(m.wire_id()), Some(m));
        }
        assert_eq!(AnthropicModel::from_wire_id("claude-unknown"), None);
    }

    #[test]
    fn model_serde_uses_wire_id() {
        let json = serde_json::to_string(&AnthropicModel::Sonnet46).unwrap();
        assert_eq!(json, "\"claude-sonnet-4-6\"");
        let m: AnthropicModel = serde_json::from_str("\"claude-haiku-4-5-20251001\"").unwrap();
        assert_eq!(m, AnthropicModel::Haiku45);
    }

    #[test]
    fn stop_reason_from_wire() {
        assert_eq!(
            AnthropicStopReason::from_wire("end_turn"),
            AnthropicStopReason::EndTurn
        );
        assert_eq!(
            AnthropicStopReason::from_wire("tool_use"),
            AnthropicStopReason::ToolUse
        );
        assert_eq!(
            AnthropicStopReason::from_wire("pause_turn"),
            AnthropicStopReason::PauseTurn
        );
        // Unknown → Other (reference fallback).
        assert_eq!(
            AnthropicStopReason::from_wire("nonsense"),
            AnthropicStopReason::Other
        );
    }

    #[test]
    fn stop_reason_serde_snake_case_with_other_fallback() {
        let r: AnthropicStopReason = serde_json::from_str("\"max_tokens\"").unwrap();
        assert_eq!(r, AnthropicStopReason::MaxTokens);
        // serde(other) absorbs unknown wire variants.
        let r: AnthropicStopReason = serde_json::from_str("\"brand_new_reason\"").unwrap();
        assert_eq!(r, AnthropicStopReason::Other);
    }

    #[test]
    fn tool_use_complete_defaults_empty_json() {
        let e = StreamEvent::tool_use_complete("toolu_1", "get_timeline", "");
        match e {
            StreamEvent::ToolUseComplete { json, .. } => assert_eq!(json, "{}"),
            _ => panic!("wrong variant"),
        }
        let e = StreamEvent::tool_use_complete("toolu_1", "x", "{\"a\":1}");
        match e {
            StreamEvent::ToolUseComplete { json, .. } => assert_eq!(json, "{\"a\":1}"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn usage_cache_read_pct() {
        let u = Usage {
            input_tokens: 100,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 300,
            output_tokens: 50,
        };
        // billed = 400, read = 300 → 75%.
        assert_eq!(u.cache_read_pct(), 75);
        assert_eq!(Usage::default().cache_read_pct(), 0);
    }

    #[test]
    fn usage_decodes_from_message_start_payload() {
        let json = r#"{"input_tokens":10,"cache_creation_input_tokens":5,"cache_read_input_tokens":85,"output_tokens":2}"#;
        let u: Usage = serde_json::from_str(json).unwrap();
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.cache_read_input_tokens, 85);
        assert_eq!(u.cache_read_pct(), 85);
    }
}
