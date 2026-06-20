//! End-to-end transport tests for [`AnthropicClient`] over a **local mock HTTP
//! server** (`wiremock`) — exercising the real `reqwest`/rustls byte-stream path
//! (`HttpByteSource`), not just the injected byte-source seam.
//!
//! These never touch the live Anthropic API: `wiremock` binds a loopback server,
//! serves a recorded SSE body (or a 4xx error), and `AnthropicClient::with_base_url`
//! points at it. Covers: a text stream, a tool_use stream, an HTTP 400 → terminal
//! `Error`, and the request shape (headers + body) the client actually sends.

use futures_util::StreamExt;
use palmier_agent::{
    AgentClient, AgentRequest, AnthropicClient, AnthropicModel, AnthropicStopReason, StreamEvent,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const TEXT_STREAM: &str = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":4,\"output_tokens\":1}}}\n\
\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\
\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n";

const TOOL_STREAM: &str = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_9\",\"name\":\"split_clip\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"frame\\\":120}\"}}\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n";

fn user_request(text: &str) -> AgentRequest {
    let mut req = AgentRequest::new(AnthropicModel::Haiku45, "You are the editor agent.");
    req.messages.push(palmier_agent::WireMessage {
        role: "user".to_string(),
        content: vec![serde_json::json!({ "type": "text", "text": text })],
    });
    req
}

#[tokio::test]
async fn http_text_stream_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-mock"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(header("accept", "text/event-stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(TEXT_STREAM),
        )
        .mount(&server)
        .await;

    let url = format!("{}/v1/messages", server.uri());
    let client = AnthropicClient::with_base_url("sk-ant-mock", url).unwrap();
    let events: Vec<StreamEvent> = client.stream(user_request("hi")).collect().await;

    assert!(events
        .iter()
        .any(|e| matches!(e, StreamEvent::TextDelta(t) if t == "Hello")));
    assert_eq!(
        events.last(),
        Some(&StreamEvent::MessageStop {
            reason: AnthropicStopReason::EndTurn
        })
    );
}

#[tokio::test]
async fn http_tool_use_stream_round_trip() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(TOOL_STREAM),
        )
        .mount(&server)
        .await;

    let url = format!("{}/v1/messages", server.uri());
    let client = AnthropicClient::with_base_url("sk-ant-mock", url).unwrap();
    let events: Vec<StreamEvent> = client.stream(user_request("cut it")).collect().await;

    let tool = events.iter().find_map(|e| match e {
        StreamEvent::ToolUseComplete { id, name, json } => {
            Some((id.clone(), name.clone(), json.clone()))
        }
        _ => None,
    });
    assert_eq!(
        tool,
        Some((
            "toolu_9".to_string(),
            "split_clip".to_string(),
            "{\"frame\":120}".to_string()
        ))
    );
    assert_eq!(
        events.last(),
        Some(&StreamEvent::MessageStop {
            reason: AnthropicStopReason::ToolUse
        })
    );
}

#[tokio::test]
async fn http_400_yields_terminal_error_with_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(400).set_body_string(
            "{\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"oops\"}}",
        ))
        .mount(&server)
        .await;

    let url = format!("{}/v1/messages", server.uri());
    let client = AnthropicClient::with_base_url("sk-ant-mock", url).unwrap();
    let events: Vec<StreamEvent> = client.stream(user_request("x")).collect().await;

    assert_eq!(events.len(), 1, "exactly one terminal Error event");
    match &events[0] {
        StreamEvent::Error(msg) => {
            assert!(msg.contains("400"), "carries status: {msg}");
            assert!(msg.contains("oops"), "carries drained body: {msg}");
        }
        other => panic!("expected terminal Error, got {other:?}"),
    }
}

#[tokio::test]
async fn http_429_maps_to_typed_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
        .mount(&server)
        .await;

    let url = format!("{}/v1/messages", server.uri());
    let client = AnthropicClient::with_base_url("sk-ant-mock", url).unwrap();
    let events: Vec<StreamEvent> = client.stream(user_request("x")).collect().await;
    assert_eq!(events.len(), 1);
    let StreamEvent::Error(msg) = &events[0] else {
        panic!("expected Error");
    };
    assert!(msg.contains("429"));
    assert!(msg.contains("rate limited"));
}

#[tokio::test]
async fn request_body_is_the_canonical_built_body() {
    // Capture the body the client actually POSTed and assert it equals the
    // canonical request builder's output (2-cache-breakpoint, sorted-key bytes).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(TEXT_STREAM),
        )
        .mount(&server)
        .await;

    let url = format!("{}/v1/messages", server.uri());
    let request = user_request("hello there");
    let expected_body = palmier_agent::build_request_bytes(&request).unwrap();

    let client = AnthropicClient::with_base_url("sk-ant-mock", url).unwrap();
    let _events: Vec<StreamEvent> = client.stream(request).collect().await;

    let received: Vec<Request> = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let sent_body = String::from_utf8(received[0].body.clone()).unwrap();
    assert_eq!(sent_body, expected_body, "client sent the canonical built body");
}
