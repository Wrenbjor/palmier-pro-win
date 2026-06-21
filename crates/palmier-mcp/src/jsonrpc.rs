//! JSON-RPC 2.0 protocol layer for `POST /mcp` — parse, dispatch, serialize.
//!
//! Handles **single-shot AND batched** requests (a top-level JSON array is a batch).
//! Methods are routed into the shared [`palmier_tools`] dispatcher / schema catalogue
//! / resource descriptors:
//!
//! | Method                     | Routes to                                              |
//! |----------------------------|--------------------------------------------------------|
//! | `initialize`               | server identity + capabilities + verbatim instructions |
//! | `tools/list`               | [`palmier_tools::tool_definitions`]                    |
//! | `tools/call`               | [`palmier_tools::ToolExecutor::execute`]               |
//! | `resources/list`           | [`palmier_tools::RESOURCE_DESCRIPTORS`]                |
//! | `resources/read`           | the two `palmier://models/*` resource bodies           |
//! | `ping`                     | empty result                                           |
//! | `notifications/initialized`| acknowledged (no response — it is a notification)      |
//!
//! Notifications (no `id`) produce no response, per JSON-RPC 2.0. In a batch, a
//! response array is returned containing one entry per *request* (notifications are
//! omitted); an all-notification batch yields no HTTP body.

use std::sync::Arc;

use palmier_tools::{
    tool_definitions, IdUniverse, ToolContext, ToolDispatch, ToolExecutor, ToolName,
    RESOURCE_DESCRIPTORS,
};
use serde_json::{json, Value};

use crate::server::MutationCallback;
use crate::validators::MCP_PROTOCOL_VERSION;
use crate::{AGENT_INSTRUCTIONS, SERVER_NAME, SERVER_VERSION};

/// JSON-RPC standard error codes (subset used here).
mod codes {
    pub const PARSE_ERROR: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
}

/// A trivial [`ToolContext`]. [`ToolExecutor::execute`] owns its own `EditorState`
/// and snapshots the id universe internally, so it ignores the passed context — we
/// supply an empty universe to satisfy the trait. (The seam exists for the
/// `ScaffoldDispatcher`, which has no state of its own.)
struct EmptyCtx;
impl ToolContext for EmptyCtx {
    fn id_universe(&self) -> IdUniverse {
        IdUniverse::default()
    }
}

/// Dispatch a raw request body (single object or batch array) and return the
/// response body string, or `None` if the whole payload was notifications (→ HTTP
/// 202 with no body).
///
/// `on_mutation`, if present, is invoked **once per successful mutating
/// `tools/call`** (after the executor has applied the edit). "Mutating" is the
/// shared [`ToolName::is_mutation`] classification — the SAME source the executor /
/// in-app agent use, so read tools (`get_*`/`list_*`/`inspect_*`/`search_media`)
/// never fire it. A tool call that returns `isError` (validation failure, unknown
/// tool, etc.) does NOT fire it (nothing changed). In a batch, the callback fires
/// once per qualifying entry. The Tauri layer passes a closure that emits
/// `timeline://changed`; `palmier-mcp` itself stays Tauri-free.
pub fn handle_body(
    body: &str,
    executor: &Arc<ToolExecutor>,
    on_mutation: Option<&MutationCallback>,
) -> Option<String> {
    let parsed: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            return Some(
                serde_json::to_string(&error_response(Value::Null, codes::PARSE_ERROR, "Parse error"))
                    .expect("serialize parse error"),
            )
        }
    };

    match parsed {
        Value::Array(items) => {
            if items.is_empty() {
                return Some(
                    serde_json::to_string(&error_response(
                        Value::Null,
                        codes::INVALID_REQUEST,
                        "Empty batch",
                    ))
                    .expect("serialize"),
                );
            }
            let responses: Vec<Value> = items
                .into_iter()
                .filter_map(|item| handle_one(item, executor, on_mutation))
                .collect();
            if responses.is_empty() {
                None // all notifications
            } else {
                Some(serde_json::to_string(&responses).expect("serialize batch"))
            }
        }
        single => handle_one(single, executor, on_mutation)
            .map(|resp| serde_json::to_string(&resp).expect("serialize response")),
    }
}

/// Handle a single JSON-RPC message. Returns `None` for notifications (no `id`).
///
/// `on_mutation` fires after a successful mutating `tools/call` (see
/// [`handle_body`]). Notifications still run their side effect: an MCP
/// `tools/call` notification (no `id`) that mutates DOES fire the callback even
/// though it returns no JSON-RPC response.
fn handle_one(
    msg: Value,
    executor: &Arc<ToolExecutor>,
    on_mutation: Option<&MutationCallback>,
) -> Option<Value> {
    let Some(obj) = msg.as_object() else {
        return Some(error_response(
            Value::Null,
            codes::INVALID_REQUEST,
            "Request is not a JSON object",
        ));
    };

    let method = obj.get("method").and_then(Value::as_str).unwrap_or("");
    let id = obj.get("id").cloned();
    let params = obj.get("params").cloned().unwrap_or(Value::Null);

    // A message with no `id` is a notification: act on it, return nothing.
    let is_notification = id.is_none();

    match method {
        "notifications/initialized" | "initialized" => None,
        "initialize" => respond(id, is_notification, initialize_result()),
        "ping" => respond(id, is_notification, json!({})),
        "tools/list" => respond(id, is_notification, tools_list_result()),
        "tools/call" => match call_tool(&params, executor) {
            Ok(result) => {
                // Fire the mutation hook iff this was a successful MUTATING tool.
                // Classification reuses the shared `ToolName::is_mutation` (the same
                // source the executor uses); a result with `isError:true` (validation
                // failure / unknown tool) changed nothing, so it never fires.
                if let Some(cb) = on_mutation
                    && tool_call_mutated(&params, &result)
                {
                    cb();
                }
                respond(id, is_notification, result)
            }
            Err((code, msg)) => respond_err(id, is_notification, code, msg),
        },
        "resources/list" => respond(id, is_notification, resources_list_result()),
        "resources/read" => match read_resource(&params) {
            Ok(result) => respond(id, is_notification, result),
            Err((code, msg)) => respond_err(id, is_notification, code, msg),
        },
        other => respond_err(
            id,
            is_notification,
            codes::METHOD_NOT_FOUND,
            format!("Method not found: {other}"),
        ),
    }
}

/// Wrap a successful result for a request; `None` for a notification.
fn respond(id: Option<Value>, is_notification: bool, result: Value) -> Option<Value> {
    if is_notification {
        return None;
    }
    Some(json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "result": result,
    }))
}

/// Wrap an error for a request; `None` for a notification.
fn respond_err(
    id: Option<Value>,
    is_notification: bool,
    code: i64,
    message: impl Into<String>,
) -> Option<Value> {
    if is_notification {
        return None;
    }
    Some(error_response(id.unwrap_or(Value::Null), code, message))
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message.into() },
    })
}

/// The `initialize` result — server identity, capabilities, verbatim instructions.
/// Mirrors the reference `Server(name:"palmier-pro", version:"1.0.0",
/// instructions:AgentInstructions.serverInstructions, capabilities:…)`.
fn initialize_result() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        "instructions": AGENT_INSTRUCTIONS,
        "capabilities": {
            "resources": { "subscribe": false, "listChanged": false },
            "tools": { "listChanged": false }
        }
    })
}

/// `tools/list` — the 30 tools from the shared schema catalogue, in the MCP wire
/// shape `{ name, description, inputSchema }`.
fn tools_list_result() -> Value {
    let tools: Vec<Value> = tool_definitions()
        .into_iter()
        .map(|def| {
            json!({
                "name": def.name.wire_name(),
                "description": def.description,
                "inputSchema": def.input_schema,
            })
        })
        .collect();
    json!({ "tools": tools })
}

/// `tools/call` — route into the shared dispatcher and return its
/// [`palmier_tools::ToolResult`] in MCP `CallTool.Result` shape.
fn call_tool(params: &Value, executor: &Arc<ToolExecutor>) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((codes::INVALID_PARAMS, "tools/call requires a 'name'".to_string()))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));
    let result = executor.execute(name, args, &EmptyCtx);
    Ok(result.to_mcp_json())
}

/// Whether a completed `tools/call` actually mutated the shared editor state — i.e.
/// it named a [mutating tool](ToolName::is_mutation) AND its MCP result did not carry
/// `isError`. Drives the [`handle_body`] mutation hook (so the live UI refetches only
/// when an external client's edit changed the timeline/library).
fn tool_call_mutated(params: &Value, result: &Value) -> bool {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return false;
    };
    let is_mutating = ToolName::from_wire(name).is_some_and(ToolName::is_mutation);
    // `to_mcp_json` only emits `isError` on failure (success omits it), so an absent
    // or non-`true` flag means the tool succeeded.
    let succeeded = result.get("isError") != Some(&Value::Bool(true));
    is_mutating && succeeded
}

/// `resources/list` — the two `palmier://models/*` descriptors (NOT tools; SM-C2).
fn resources_list_result() -> Value {
    let resources: Vec<Value> = RESOURCE_DESCRIPTORS
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "uri": r.uri,
                "description": r.description,
                "mimeType": r.mime_type,
            })
        })
        .collect();
    json!({ "resources": resources })
}

/// `resources/read` — return the JSON body for a `palmier://models/*` URI. Until
/// Epic 9 supplies the catalog the bodies are empty arrays (clients tolerate empty,
/// reference `jsonString(...) ?? "[]"`).
fn read_resource(params: &Value) -> Result<Value, (i64, String)> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or((codes::INVALID_PARAMS, "resources/read requires a 'uri'".to_string()))?;

    let known = RESOURCE_DESCRIPTORS.iter().any(|r| r.uri == uri);
    if !known {
        return Err((codes::INVALID_PARAMS, format!("Unknown resource: {uri}")));
    }
    // Epic 9 wires the real catalog; M2 returns an empty array.
    let body = "[]";
    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": "application/json",
            "text": body,
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exec() -> Arc<ToolExecutor> {
        Arc::new(ToolExecutor::new())
    }

    #[test]
    fn initialize_carries_identity_and_verbatim_instructions() {
        let r = initialize_result();
        assert_eq!(r["serverInfo"]["name"], "palmier-pro");
        assert_eq!(r["serverInfo"]["version"], "1.0.0");
        assert_eq!(r["capabilities"]["resources"]["subscribe"], false);
        assert_eq!(r["capabilities"]["resources"]["listChanged"], false);
        assert_eq!(r["capabilities"]["tools"]["listChanged"], false);
        let instr = r["instructions"].as_str().unwrap();
        assert!(instr.starts_with("You are a creative AI assistant connected to palmier-pro"));
        assert_eq!(instr.len(), 8694);
    }

    #[test]
    fn tools_list_returns_exactly_30() {
        let r = tools_list_result();
        let tools = r["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 30);
        // Every entry carries the three MCP fields.
        for t in tools {
            assert!(t["name"].is_string());
            assert!(t["description"].is_string());
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn resources_list_returns_exactly_two() {
        let r = resources_list_result();
        let res = r["resources"].as_array().unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0]["uri"], "palmier://models/video");
        assert_eq!(res[1]["uri"], "palmier://models/image");
    }

    #[test]
    fn resources_read_known_uri_returns_body() {
        let p = json!({ "uri": "palmier://models/video" });
        let r = read_resource(&p).unwrap();
        assert_eq!(r["contents"][0]["uri"], "palmier://models/video");
        assert_eq!(r["contents"][0]["mimeType"], "application/json");
        assert_eq!(r["contents"][0]["text"], "[]");
    }

    #[test]
    fn resources_read_unknown_uri_errors() {
        let p = json!({ "uri": "palmier://nope" });
        assert!(read_resource(&p).is_err());
    }

    #[test]
    fn single_request_round_trips() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let out = handle_body(body, &exec(), None).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["result"]["tools"].as_array().unwrap().len(), 30);
    }

    #[test]
    fn batch_request_returns_array_of_responses() {
        let body = r#"[
            {"jsonrpc":"2.0","id":1,"method":"initialize"},
            {"jsonrpc":"2.0","id":2,"method":"tools/list"}
        ]"#;
        let out = handle_body(body, &exec(), None).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let arr = v.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[1]["id"], 2);
    }

    #[test]
    fn notification_produces_no_response() {
        // `initialized` notification has no id → no body.
        let body = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(handle_body(body, &exec(), None).is_none());
    }

    #[test]
    fn batch_of_only_notifications_produces_no_body() {
        let body = r#"[{"jsonrpc":"2.0","method":"notifications/initialized"}]"#;
        assert!(handle_body(body, &exec(), None).is_none());
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let body = r#"{"jsonrpc":"2.0","id":7,"method":"does/not/exist"}"#;
        let out = handle_body(body, &exec(), None).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], codes::METHOD_NOT_FOUND);
    }

    #[test]
    fn malformed_json_returns_parse_error() {
        let out = handle_body("{not json", &exec(), None).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], codes::PARSE_ERROR);
    }

    #[test]
    fn tools_call_routes_into_the_executor() {
        // get_timeline is a real READ body on an empty editor → not an error.
        let body =
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_timeline","arguments":{}}}"#;
        let out = handle_body(body, &exec(), None).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["result"]["content"].is_array());
        // success result omits isError
        assert!(v["result"].get("isError").is_none());
    }

    #[test]
    fn tools_call_unknown_tool_returns_tool_error_shape() {
        let body =
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"not_a_tool","arguments":{}}}"#;
        let out = handle_body(body, &exec(), None).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        // Tool errors live INSIDE the result (isError:true), not as a JSON-RPC error.
        assert_eq!(v["result"]["isError"], true);
        assert!(v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Unknown tool"));
    }

    #[test]
    fn tools_call_missing_name_is_invalid_params() {
        let body = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{}}"#;
        let out = handle_body(body, &exec(), None).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["error"]["code"], codes::INVALID_PARAMS);
    }

    // ── on_mutation hook (the live-UI seam) ───────────────────────────────────

    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Build an `(callback, counter)` pair: the callback bumps an `AtomicUsize` each
    /// time it fires, the counter reads it back.
    fn counting_hook() -> (super::MutationCallback, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        let cb: super::MutationCallback = Arc::new(move || {
            c.fetch_add(1, Ordering::SeqCst);
        });
        (cb, count)
    }

    #[test]
    fn mutating_tool_call_fires_the_hook_once() {
        // create_folder is a MUTATING tool that succeeds on a fresh editor.
        let (cb, count) = counting_hook();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_folder","arguments":{"name":"Clips"}}}"#;
        let out = handle_body(body, &exec(), Some(&cb)).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        // The call itself succeeded (no isError) …
        assert!(v["result"].get("isError").is_none(), "create_folder should succeed");
        // … and the hook fired exactly once.
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn read_tool_call_does_not_fire_the_hook() {
        // get_timeline is a READ tool → the hook must NOT fire even on success.
        let (cb, count) = counting_hook();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_timeline","arguments":{}}}"#;
        let out = handle_body(body, &exec(), Some(&cb)).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["result"].get("isError").is_none(), "get_timeline should succeed");
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn failed_mutating_tool_call_does_not_fire_the_hook() {
        // create_folder with NO name → tool-error (isError:true); nothing changed,
        // so a mutating-but-failed call must NOT fire the hook.
        let (cb, count) = counting_hook();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_folder","arguments":{}}}"#;
        let out = handle_body(body, &exec(), Some(&cb)).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["result"]["isError"], true, "create_folder w/o name should error");
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unknown_tool_call_does_not_fire_the_hook() {
        // Unknown tool → isError + unknown name → never mutating.
        let (cb, count) = counting_hook();
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"not_a_tool","arguments":{}}}"#;
        let _ = handle_body(body, &exec(), Some(&cb)).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn batch_fires_the_hook_per_qualifying_mutation() {
        // Two mutations (create_folder ×2) + one read (get_timeline) → hook fires twice.
        let (cb, count) = counting_hook();
        let body = r#"[
            {"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_folder","arguments":{"name":"A"}}},
            {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_timeline","arguments":{}}},
            {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_folder","arguments":{"name":"B"}}}
        ]"#;
        let _ = handle_body(body, &exec(), Some(&cb)).unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn mutating_tools_call_notification_still_fires_the_hook() {
        // A `tools/call` with NO id is a notification (no JSON-RPC response) — but the
        // side effect (the edit) still happened, so the hook fires for live refetch.
        let (cb, count) = counting_hook();
        let body = r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"create_folder","arguments":{"name":"Clips"}}}"#;
        assert!(handle_body(body, &exec(), Some(&cb)).is_none(), "notification → no body");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
