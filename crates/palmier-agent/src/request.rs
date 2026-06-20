//! The Anthropic Messages **request-body builder** (`AnthropicRequestBody::build`).
//!
//! Ports `AnthropicRequestBody.build` from
//! `Agent/Clients/AgentClientTypes.swift`. Serializes an [`AgentRequest`] into the
//! exact Anthropic Messages JSON both transports send — **identical wire bytes for
//! both clients** so prompt-cache hashing is deterministic across BYOK and proxied
//! paths (`agent-panel.md` §"Request body", lines 59-67).
//!
//! ## Shape (`POST /v1/messages` body)
//! - `model` — the model wire-id ([`AnthropicModel::wire_id`]).
//! - `max_tokens` — `8192` (reference [`DEFAULT_MAX_TOKENS`]).
//! - `stream` — always `true`.
//! - `system` — a single text block:
//!   `[{type:"text", text, cache_control:{type:"ephemeral"}}]`.
//! - `tools` — `[{name, description, input_schema}]`, with
//!   `cache_control:{type:"ephemeral"}` on **only the last tool** (omitted entirely
//!   when there are no tools, matching the reference `if !toolBlocks.isEmpty`).
//! - `messages` — `[{role, content:[…]}]`, with `cache_control:{type:"ephemeral"}`
//!   on **only the last content block of the last message**.
//!
//! ## Cache-control placement (load-bearing)
//! The reference emits **three** `cache_control` markers — on the system block, the
//! last tool, and the conversation tail — which Anthropic coalesces into **two
//! logical cache breakpoints**: the *system + tools prefix* (the system marker and
//! the last-tool marker bracket the same stable prefix) and the *conversation tail*
//! (`agent-panel.md` lines 61-66, 198-200; reconciliation carry-forward "exactly 2
//! ephemeral cache breakpoints"). We reproduce the reference's three wire markers
//! **verbatim** — diverging (e.g. dropping the system marker to make the literal
//! count `2`) would change the bytes and break cache-hit parity with the macOS
//! reference and any recorded exchange, which is exactly what this story exists to
//! preserve. [`cache_control_marker_count`] returns `3` for a request with tools so
//! the relationship is asserted explicitly in tests.
//!
//! ## Canonical (`.sortedKeys`) ordering
//! [`build`] returns a [`serde_json::Value`] whose every object is a
//! [`serde_json::Map`] — with `serde_json`'s default `BTreeMap` backing (no
//! `preserve_order` feature), keys serialize in sorted order at every depth,
//! matching the reference `JSONSerialization` `.sortedKeys`. [`build_bytes`] is the
//! canonical string the transport (E8-S3) puts on the wire.
//!
//! ## Headers (data for the next story)
//! The HTTP headers are a transport concern (E8-S3 `AnthropicClient`), but the
//! contract is fixed here as [`anthropic_headers`] so both the body and its headers
//! are specified in one place: `x-api-key`, `anthropic-version: 2023-06-01`,
//! `content-type: application/json`, `accept: text/event-stream`
//! (`agent-panel.md` lines 70-71).

use crate::client::AgentRequest;
use serde_json::{json, Map, Value};

/// The fixed `anthropic-version` header value (`agent-panel.md` line 71).
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// An ephemeral `cache_control` object: `{"type":"ephemeral"}`.
#[must_use]
fn ephemeral() -> Value {
    json!({ "type": "ephemeral" })
}

/// Build the Anthropic Messages request body for `request` as a
/// [`serde_json::Value`] (reference `AnthropicRequestBody.build`).
///
/// The returned value serializes to canonical sorted-key JSON via
/// [`serde_json::to_string`] / [`build_bytes`] (no `preserve_order` feature → keys
/// are sorted at every depth). The 2 logical cache breakpoints (3 wire markers) are
/// placed on the system block, the last tool, and the last content block of the
/// last message — see the module docs.
///
/// `tools` is omitted entirely when empty (reference `if !toolBlocks.isEmpty`).
#[must_use]
pub fn build(request: &AgentRequest) -> Value {
    // --- tools: [{name, description, input_schema}], cache_control on the LAST ---
    let mut tools: Vec<Value> = request
        .tools
        .iter()
        .map(|t| {
            let mut obj = Map::new();
            obj.insert("name".to_string(), Value::String(t.name.clone()));
            obj.insert(
                "description".to_string(),
                Value::String(t.description.clone()),
            );
            obj.insert("input_schema".to_string(), t.input_schema.clone());
            Value::Object(obj)
        })
        .collect();
    if let Some(Value::Object(last)) = tools.last_mut() {
        last.insert("cache_control".to_string(), ephemeral());
    }

    // --- messages: [{role, content:[…]}], cache_control on the LAST content block
    //     of the LAST message ---
    let mut messages: Vec<Value> = request
        .messages
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content.clone() }))
        .collect();
    if let Some(Value::Object(last_msg)) = messages.last_mut()
        && let Some(Value::Array(content)) = last_msg.get_mut("content")
        && let Some(Value::Object(last_block)) = content.last_mut()
    {
        last_block.insert("cache_control".to_string(), ephemeral());
    }

    // --- system: single text block with cache_control ---
    let system = Value::Array(vec![json!({
        "type": "text",
        "text": request.system,
        "cache_control": ephemeral(),
    })]);

    let mut body = Map::new();
    body.insert(
        "model".to_string(),
        Value::String(request.model.wire_id().to_string()),
    );
    body.insert("max_tokens".to_string(), Value::from(request.max_tokens));
    body.insert("stream".to_string(), Value::Bool(true));
    body.insert("system".to_string(), system);
    body.insert("messages".to_string(), Value::Array(messages));
    if !tools.is_empty() {
        body.insert("tools".to_string(), Value::Array(tools));
    }
    Value::Object(body)
}

/// The canonical (sorted-key) JSON string the transport sends as the request body.
///
/// # Errors
/// Propagates a `serde_json` serialization error (only on a non-serializable
/// `input_schema`, which cannot occur for valid [`serde_json::Value`]s).
pub fn build_bytes(request: &AgentRequest) -> Result<String, serde_json::Error> {
    serde_json::to_string(&build(request))
}

/// Count the `cache_control` objects in `body` (recursively). The reference emits
/// **3** for a request with at least one tool (system block + last tool +
/// conversation tail) — the two logical breakpoints "system+tools" and
/// "conversation tail". Used by the golden test to pin the placement contract.
#[must_use]
pub fn cache_control_marker_count(body: &Value) -> usize {
    match body {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| usize::from(k == "cache_control") + cache_control_marker_count(v))
            .sum(),
        Value::Array(items) => items.iter().map(cache_control_marker_count).sum(),
        _ => 0,
    }
}

/// The HTTP headers the transport (E8-S3) sends with the request body, as
/// `(name, value)` pairs. `x-api-key` is the BYOK key; the proxied client (E8-S6)
/// swaps it for `Authorization: Bearer <jwt>` but keeps the rest
/// (`agent-panel.md` lines 70-74). This is **data**, not a live request — the
/// transport owns the actual send.
#[must_use]
pub fn anthropic_headers(api_key: &str) -> Vec<(&'static str, String)> {
    vec![
        ("x-api-key", api_key.to_string()),
        ("anthropic-version", ANTHROPIC_VERSION.to_string()),
        ("content-type", "application/json".to_string()),
        ("accept", "text/event-stream".to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{AnthropicToolSchema, WireMessage};
    use crate::event::AnthropicModel;

    fn tool(name: &str) -> AnthropicToolSchema {
        AnthropicToolSchema {
            name: name.to_string(),
            description: format!("The {name} tool."),
            input_schema: json!({
                "type": "object",
                "properties": { "page": { "type": "integer" } },
                "required": [],
            }),
        }
    }

    fn user_msg(text: &str) -> WireMessage {
        WireMessage {
            role: "user".to_string(),
            content: vec![json!({ "type": "text", "text": text })],
        }
    }

    fn assistant_tool_use(id: &str, name: &str, input: Value) -> WireMessage {
        WireMessage {
            role: "assistant".to_string(),
            content: vec![json!({ "type": "tool_use", "id": id, "name": name, "input": input })],
        }
    }

    fn sample_request() -> AgentRequest {
        AgentRequest {
            model: AnthropicModel::Sonnet46,
            max_tokens: 8192,
            system: "You are the Palmier editor agent.".to_string(),
            tools: vec![tool("get_timeline"), tool("split_clip"), tool("ripple_delete")],
            messages: vec![
                user_msg("Cut the intro."),
                assistant_tool_use("toolu_1", "get_timeline", json!({"page": 1})),
            ],
        }
    }

    #[test]
    fn body_top_level_shape() {
        let body = build(&sample_request());
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["stream"], true);
        assert!(body["system"].is_array());
        assert!(body["tools"].is_array());
        assert!(body["messages"].is_array());
    }

    #[test]
    fn max_tokens_is_8192_from_default() {
        let req = AgentRequest::new(AnthropicModel::Opus48, "sys");
        assert_eq!(req.max_tokens, 8192);
        assert_eq!(build(&req)["max_tokens"], 8192);
    }

    #[test]
    fn system_block_carries_cache_control() {
        let body = build(&sample_request());
        let sys = &body["system"][0];
        assert_eq!(sys["type"], "text");
        assert_eq!(sys["text"], "You are the Palmier editor agent.");
        assert_eq!(sys["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn only_last_tool_carries_cache_control() {
        let body = build(&sample_request());
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        // First two tools have NO cache_control.
        assert!(tools[0].get("cache_control").is_none());
        assert!(tools[1].get("cache_control").is_none());
        // The last tool carries the breakpoint.
        assert_eq!(tools[2]["cache_control"]["type"], "ephemeral");
        assert_eq!(tools[2]["name"], "ripple_delete");
    }

    #[test]
    fn only_last_content_block_of_last_message_carries_cache_control() {
        let body = build(&sample_request());
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        // First message's content has NO cache_control.
        assert!(messages[0]["content"][0].get("cache_control").is_none());
        // Last message's last (only) content block carries the breakpoint.
        let last_content = messages[1]["content"].as_array().unwrap();
        let last_block = last_content.last().unwrap();
        assert_eq!(last_block["cache_control"]["type"], "ephemeral");
        assert_eq!(last_block["type"], "tool_use");
    }

    #[test]
    fn cache_control_appears_on_exactly_the_three_reference_nodes() {
        // The reference emits 3 wire markers = the 2 logical breakpoints
        // "system+tools" (system block + last tool) and "conversation tail"
        // (last content block of the last message). Byte parity with the
        // reference requires all three; see the module docs.
        let body = build(&sample_request());
        assert_eq!(cache_control_marker_count(&body), 3);
    }

    #[test]
    fn no_tools_omits_tools_key_and_drops_one_breakpoint() {
        // Reference: `if !toolBlocks.isEmpty { body["tools"] = … }`.
        let req = AgentRequest {
            model: AnthropicModel::Haiku45,
            max_tokens: 8192,
            system: "sys".to_string(),
            tools: vec![],
            messages: vec![user_msg("hi")],
        };
        let body = build(&req);
        assert!(body.get("tools").is_none());
        // Only the system + conversation-tail markers remain (no last-tool marker).
        assert_eq!(cache_control_marker_count(&body), 2);
    }

    #[test]
    fn canonical_json_is_byte_stable_and_sorted() {
        let req = sample_request();
        let a = build_bytes(&req).unwrap();
        let b = build_bytes(&req).unwrap();
        assert_eq!(a, b, "build must be deterministic");

        // Top-level keys are emitted in sorted order (serde_json BTreeMap backing).
        let max_tokens_pos = a.find("\"max_tokens\"").unwrap();
        let messages_pos = a.find("\"messages\"").unwrap();
        let model_pos = a.find("\"model\"").unwrap();
        let stream_pos = a.find("\"stream\"").unwrap();
        let system_pos = a.find("\"system\"").unwrap();
        let tools_pos = a.find("\"tools\"").unwrap();
        // max_tokens < messages < model < stream < system < tools
        assert!(max_tokens_pos < messages_pos);
        assert!(messages_pos < model_pos);
        assert!(model_pos < stream_pos);
        assert!(stream_pos < system_pos);
        assert!(system_pos < tools_pos);
    }

    #[test]
    fn tool_use_input_object_is_forwarded_verbatim() {
        // The wire-projected tool_use block carries `input` as a parsed object
        // (the E8-S5 projection re-parses input_json). The builder must not
        // touch it — only append cache_control to the LAST block of the LAST msg.
        let req = AgentRequest {
            model: AnthropicModel::Sonnet46,
            max_tokens: 8192,
            system: "sys".to_string(),
            tools: vec![tool("x")],
            messages: vec![assistant_tool_use("toolu_9", "split_clip", json!({"frame": 120}))],
        };
        let body = build(&req);
        let block = &body["messages"][0]["content"][0];
        assert_eq!(block["input"]["frame"], 120);
    }

    #[test]
    fn golden_body_matches_committed_fixture() {
        // Byte-compare the built body for a fixed (system, 3-tool, 2-message)
        // fixture against a committed golden. This pins the exact wire bytes —
        // any change to key order, cache_control placement, or field naming
        // breaks this test (the whole point: deterministic cache hashing).
        let body = build_bytes(&sample_request()).unwrap();
        let golden = include_str!("../tests/fixtures/request_body_golden.json");
        // The golden is stored pretty-printed for review; compare canonical forms.
        let golden_canonical =
            serde_json::to_string(&serde_json::from_str::<Value>(golden).unwrap()).unwrap();
        assert_eq!(
            body, golden_canonical,
            "built request body diverged from the committed golden"
        );
    }

    #[test]
    fn headers_contract() {
        let h = anthropic_headers("sk-ant-test");
        assert!(h.contains(&("x-api-key", "sk-ant-test".to_string())));
        assert!(h.contains(&("anthropic-version", "2023-06-01".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));
        assert!(h.contains(&("accept", "text/event-stream".to_string())));
    }
}
