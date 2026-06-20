//! Spawned-server integration tests (E7-S11 build gate): drive a real loopback MCP
//! server over HTTP with a real HTTP client.
//!
//! Covers the story's required gates:
//! - JSON-RPC `initialize` → `tools/list` (30) → `tools/call` (a READ tool) → a
//!   `resources/read`.
//! - The three validators reject bad Origin / content-type / protocol version.
//! - The `.well-known/oauth-protected-resource` endpoint body.
//! - Batched JSON-RPC.

use std::sync::Arc;

use palmier_mcp::{McpServer, ServerConfig};
use palmier_tools::ToolExecutor;
use serde_json::{json, Value};

/// Start a server on an OS-assigned loopback port; return it plus the `/mcp` URL.
async fn spawn() -> (McpServer, String, u16) {
    let exec = Arc::new(ToolExecutor::new());
    let server = McpServer::start(exec, ServerConfig { port: 0 })
        .await
        .expect("server starts");
    let port = server.port();
    let url = format!("http://127.0.0.1:{port}/mcp");
    (server, url, port)
}

/// A client that sends valid headers by default.
fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn rpc(client: &reqwest::Client, url: &str, body: Value) -> reqwest::Response {
    client
        .post(url)
        .header("content-type", "application/json")
        .header("mcp-protocol-version", "2025-06-18")
        .body(serde_json::to_string(&body).unwrap())
        .send()
        .await
        .expect("request sends")
}

#[tokio::test]
async fn server_binds_loopback_only() {
    let (mut server, _url, _port) = spawn().await;
    assert!(server.local_addr().ip().is_loopback());
    server.stop().await;
}

#[tokio::test]
async fn full_jsonrpc_round_trip() {
    let (mut server, url, _port) = spawn().await;
    let c = client();

    // initialize
    let resp = rpc(
        &c,
        &url,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["result"]["serverInfo"]["name"], "palmier-pro");
    assert_eq!(v["result"]["serverInfo"]["version"], "1.0.0");
    assert_eq!(v["result"]["capabilities"]["resources"]["subscribe"], false);
    assert_eq!(v["result"]["capabilities"]["tools"]["listChanged"], false);
    // Verbatim instructions are wired into initialize.
    let instr = v["result"]["instructions"].as_str().unwrap();
    assert!(instr.starts_with("You are a creative AI assistant connected to palmier-pro"));

    // tools/list → exactly 30
    let resp = rpc(
        &c,
        &url,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
    )
    .await;
    let v: Value = resp.json().await.unwrap();
    let tools = v["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 30, "tools/list must return exactly 30 tools");

    // tools/call a READ tool (get_timeline on an empty editor → success, not error)
    let resp = rpc(
        &c,
        &url,
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": { "name": "get_timeline", "arguments": {} }
        }),
    )
    .await;
    let v: Value = resp.json().await.unwrap();
    assert!(v["result"]["content"].is_array());
    assert!(v["result"].get("isError").is_none(), "READ tool should not error");

    // resources/list → 2
    let resp = rpc(
        &c,
        &url,
        json!({ "jsonrpc": "2.0", "id": 4, "method": "resources/list" }),
    )
    .await;
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["result"]["resources"].as_array().unwrap().len(), 2);

    // resources/read a known model resource
    let resp = rpc(
        &c,
        &url,
        json!({
            "jsonrpc": "2.0", "id": 5, "method": "resources/read",
            "params": { "uri": "palmier://models/video" }
        }),
    )
    .await;
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["result"]["contents"][0]["uri"], "palmier://models/video");
    assert_eq!(v["result"]["contents"][0]["mimeType"], "application/json");

    server.stop().await;
}

#[tokio::test]
async fn batched_jsonrpc_returns_array() {
    let (mut server, url, _port) = spawn().await;
    let c = client();
    let resp = rpc(
        &c,
        &url,
        json!([
            { "jsonrpc": "2.0", "id": 1, "method": "initialize" },
            { "jsonrpc": "2.0", "id": 2, "method": "tools/list" }
        ]),
    )
    .await;
    assert_eq!(resp.status(), 200);
    let v: Value = resp.json().await.unwrap();
    let arr = v.as_array().expect("batch returns an array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[1]["result"]["tools"].as_array().unwrap().len(), 30);
    server.stop().await;
}

// ── Validators ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn validator_rejects_bad_origin() {
    let (mut server, url, _port) = spawn().await;
    let resp = client()
        .post(&url)
        .header("content-type", "application/json")
        .header("origin", "http://evil.com")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "non-loopback Origin must be rejected");
    server.stop().await;
}

#[tokio::test]
async fn validator_allows_loopback_origin() {
    let (mut server, url, port) = spawn().await;
    let resp = client()
        .post(&url)
        .header("content-type", "application/json")
        .header("origin", format!("http://127.0.0.1:{port}"))
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "loopback Origin must be allowed");
    server.stop().await;
}

#[tokio::test]
async fn validator_rejects_bad_content_type() {
    let (mut server, url, _port) = spawn().await;
    let resp = client()
        .post(&url)
        .header("content-type", "text/plain")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 415, "non-JSON content-type must be rejected");
    server.stop().await;
}

#[tokio::test]
async fn validator_rejects_bad_protocol_version() {
    let (mut server, url, _port) = spawn().await;
    let resp = client()
        .post(&url)
        .header("content-type", "application/json")
        .header("mcp-protocol-version", "1999-01-01")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400, "unsupported protocol version must be rejected");
    server.stop().await;
}

#[tokio::test]
async fn validator_allows_missing_origin_and_protocol_version() {
    // Non-browser clients omit Origin and may omit the protocol header.
    let (mut server, url, _port) = spawn().await;
    let resp = client()
        .post(&url)
        .header("content-type", "application/json")
        .body(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    server.stop().await;
}

// ── .well-known ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn well_known_endpoint_returns_loopback_resource() {
    let (mut server, _url, port) = spawn().await;
    let resp = client()
        .get(format!(
            "http://127.0.0.1:{port}/.well-known/oauth-protected-resource"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();
    assert_eq!(body, format!(r#"{{"resource":"http://127.0.0.1:{port}"}}"#));
    server.stop().await;
}
