//! The concrete BYOK [`AnthropicClient`] transport (`reqwest` + SSE).
//!
//! Ports `Clients/AnthropicClient.swift` — the direct-to-`api.anthropic.com`
//! transport a BYOK user runs without signing in. Replaces the [`MockAgentClient`]
//! scaffold from E8-S1 with the real streaming implementation (E8-S3).
//!
//! ## What this story lands
//! - [`AnthropicClient`] — implements [`AgentClient`]: build the request body via
//!   [`request::build_bytes`], `POST https://api.anthropic.com/v1/messages` with
//!   [`request::anthropic_headers`] over **async `reqwest`** (rustls — no system
//!   OpenSSL), stream the response bytes into [`SseParser::feed`], and yield the
//!   [`StreamEvent`]s as a [`BoxStream`].
//! - **HTTP ≥ 400 handling.** The status is checked before streaming; on `>= 400`
//!   the body is drained and a terminal [`StreamEvent::Error`] (carrying the typed
//!   [`AgentClientError::HttpError { status, body }`]) is the only event yielded.
//! - **Per-chunk cancellation.** A [`tokio_util::sync::CancellationToken`] is
//!   checked before every received chunk; cancelling it (or simply dropping the
//!   returned stream) stops the request — the in-flight `reqwest` future is dropped,
//!   which aborts the connection (parity with the reference
//!   `continuation.onTermination { task.cancel() }`).
//! - **Key-load seam.** The API key is taken as a constructor parameter — the
//!   keyring load (`palmier-auth::AnthropicKeyStore`, account `anthropic-api-key`,
//!   ruling #5) and the key-changed reload event live in `palmier-auth`, so this
//!   client stays a pure transport and is testable with no keyring and no network.
//!
//! ## Byte-source seam (testable without the live API)
//! The HTTP send is factored behind [`ByteSource`]: [`HttpByteSource`] is the real
//! `reqwest` transport; tests inject a recorded SSE byte stream (or a tiny local
//! `wiremock` server) so the four behaviors — text stream, tool_use stream, HTTP
//! 400 → `Error`, and cancellation — are exercised with zero live-network calls.
//! The one `#[ignore]`d test hits the real API only when `ANTHROPIC_API_KEY` is set.
//!
//! ## Deferred (later E8 stories)
//! - **E8-S4** — the agentic run loop that drives `stream` and dispatches tool calls.
//! - **E8-S6** — the `PalmierClient` (Convex proxy) sharing the same parser/body.

use crate::client::{AgentClient, AgentClientError, AgentRequest};
use crate::event::StreamEvent;
use crate::request;
use crate::sse::SseParser;
use futures_core::stream::BoxStream;
use futures_util::stream::StreamExt;
use std::pin::Pin;
use tokio_util::sync::CancellationToken;

/// The production Anthropic Messages endpoint (`agent-panel.md` line 70).
pub const ANTHROPIC_MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";

/// How much of an HTTP error body to retain in [`AgentClientError::HttpError`]
/// (the full body can be large; the loop only needs the message for display/logs).
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;

/// The outcome of opening the HTTP request: either a live byte stream to drive the
/// SSE parser over, or a terminal error (HTTP ≥ 400 / transport failure).
///
/// Splitting "open" from "stream" lets the byte-source seam surface the
/// status-check-and-drain step (reference: "if HTTP status >= 400, drain the body
/// and throw") before any SSE byte is read.
pub enum ByteStreamOpen {
    /// The request returned a 2xx/3xx streaming response; drive the SSE parser over
    /// these chunks. The boxed stream yields `Ok(chunk)` per received frame, or
    /// `Err(message)` if the underlying transport errors mid-stream.
    Stream(Pin<Box<dyn futures_core::Stream<Item = Result<Vec<u8>, String>> + Send>>),
    /// The request failed before streaming began — yield this as the sole terminal
    /// [`StreamEvent::Error`].
    Error(AgentClientError),
}

/// The transport seam: open a streaming `POST` for `body` with `headers` and return
/// either a byte stream or a terminal error. [`HttpByteSource`] is the real
/// `reqwest` implementation; tests substitute a recorded-stream source so the SSE
/// pipeline is exercised without the live API.
#[async_trait::async_trait]
pub trait ByteSource: Send + Sync {
    /// Issue the streaming request. `headers` are `(name, value)` pairs from
    /// [`request::anthropic_headers`]; `body` is the canonical JSON from
    /// [`request::build_bytes`]. Implementations MUST perform the HTTP-status check
    /// (≥ 400 → drain body → [`ByteStreamOpen::Error`]) before returning a stream.
    async fn open(&self, headers: Vec<(&'static str, String)>, body: String) -> ByteStreamOpen;
}

/// Real `reqwest` byte source: async client, rustls TLS (no system OpenSSL — the
/// workspace already uses rustls in `palmier-auth`).
pub struct HttpByteSource {
    client: reqwest::Client,
    url: String,
}

impl HttpByteSource {
    /// A source posting to `url` with a fresh rustls `reqwest::Client`.
    ///
    /// # Errors
    /// Returns [`AgentClientError::Upstream`] if the `reqwest` client cannot be
    /// built (TLS backend init failure).
    pub fn new(url: impl Into<String>) -> Result<Self, AgentClientError> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(|e| AgentClientError::Upstream(e.to_string()))?;
        Ok(Self {
            client,
            url: url.into(),
        })
    }
}

#[async_trait::async_trait]
impl ByteSource for HttpByteSource {
    async fn open(&self, headers: Vec<(&'static str, String)>, body: String) -> ByteStreamOpen {
        let mut req = self.client.post(&self.url).body(body);
        for (name, value) in headers {
            req = req.header(name, value);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => return ByteStreamOpen::Error(AgentClientError::Upstream(e.to_string())),
        };

        let status = resp.status();
        if status.as_u16() >= 400 {
            // Drain the body for the error message (bounded).
            let body = resp.text().await.unwrap_or_default();
            let body = truncate_utf8(&body, MAX_ERROR_BODY_BYTES);
            return ByteStreamOpen::Error(AgentClientError::HttpError {
                status: status.as_u16(),
                body,
            });
        }

        // 2xx/3xx → stream the bytes. Map reqwest's frame errors to strings so the
        // boxed stream is transport-agnostic.
        let byte_stream = resp
            .bytes_stream()
            .map(|frame| frame.map(|b| b.to_vec()).map_err(|e| e.to_string()));
        ByteStreamOpen::Stream(Box::pin(byte_stream))
    }
}

/// Truncate `s` to at most `max` bytes without splitting a UTF-8 codepoint.
fn truncate_utf8(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// The BYOK Anthropic transport (reference `AnthropicClient`).
///
/// Holds the API key and the byte source; [`AgentClient::stream`] builds the body,
/// opens the request, and drives the SSE parser. Construct with [`Self::new`] for
/// the live endpoint or [`Self::with_source`] to inject a test byte source.
pub struct AnthropicClient {
    api_key: String,
    source: Box<dyn ByteSource>,
}

impl AnthropicClient {
    /// A live client posting to [`ANTHROPIC_MESSAGES_URL`] with `api_key`.
    ///
    /// `api_key` comes from `palmier-auth::AnthropicKeyStore::load` (account
    /// `anthropic-api-key`) — this constructor does not touch the keyring, keeping
    /// the transport a pure function of its inputs.
    ///
    /// # Errors
    /// Returns [`AgentClientError::Upstream`] if the rustls `reqwest::Client` cannot
    /// be built.
    pub fn new(api_key: impl Into<String>) -> Result<Self, AgentClientError> {
        let source = HttpByteSource::new(ANTHROPIC_MESSAGES_URL)?;
        Ok(Self {
            api_key: api_key.into(),
            source: Box::new(source),
        })
    }

    /// A client pointed at `base_url` (e.g. a local mock server) with `api_key`.
    ///
    /// # Errors
    /// Returns [`AgentClientError::Upstream`] if the `reqwest::Client` cannot be built.
    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, AgentClientError> {
        let source = HttpByteSource::new(base_url)?;
        Ok(Self {
            api_key: api_key.into(),
            source: Box::new(source),
        })
    }

    /// A client over an injected [`ByteSource`] — the seam tests use to drive the SSE
    /// pipeline from a recorded byte stream with no network.
    #[must_use]
    pub fn with_source(api_key: impl Into<String>, source: Box<dyn ByteSource>) -> Self {
        Self {
            api_key: api_key.into(),
            source,
        }
    }

    /// Stream with an explicit [`CancellationToken`]. Cancelling the token stops the
    /// request before the next chunk is processed (in addition to drop-cancellation,
    /// which applies to the [`AgentClient::stream`] path too). The loop (E8-S4) uses
    /// this to drop an in-flight assistant turn cleanly.
    #[must_use]
    pub fn stream_with_cancel<'a>(
        &'a self,
        request: AgentRequest,
        cancel: CancellationToken,
    ) -> BoxStream<'a, StreamEvent> {
        let headers = request::anthropic_headers(&self.api_key);
        let body = match request::build_bytes(&request) {
            Ok(b) => b,
            // A non-serializable request can't happen for valid Values, but surface
            // it as a terminal error rather than panicking.
            Err(e) => {
                let err = StreamEvent::Error(format!("failed to build request body: {e}"));
                return Box::pin(futures_util::stream::once(async move { err }));
            }
        };

        let source = &*self.source;
        Box::pin(async_stream::stream! {
            // Open the request (status check + drain happens inside the source).
            let opened = source.open(headers, body).await;
            let mut byte_stream = match opened {
                ByteStreamOpen::Stream(s) => s,
                ByteStreamOpen::Error(err) => {
                    yield StreamEvent::Error(err.to_string());
                    return;
                }
            };

            let mut parser = SseParser::new();
            loop {
                // Per-chunk cancellation: check BEFORE awaiting the next frame so a
                // cancel mid-stream drops the in-flight turn promptly.
                if cancel.is_cancelled() {
                    return;
                }
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => {
                        return;
                    }
                    frame = byte_stream.next() => {
                        match frame {
                            Some(Ok(chunk)) => {
                                for ev in parser.feed(&chunk) {
                                    yield ev;
                                }
                            }
                            Some(Err(msg)) => {
                                yield StreamEvent::Error(msg);
                                return;
                            }
                            None => {
                                // End of stream: flush any trailing buffered line.
                                for ev in parser.finish() {
                                    yield ev;
                                }
                                return;
                            }
                        }
                    }
                }
            }
        })
    }
}

impl AgentClient for AnthropicClient {
    fn stream<'a>(&'a self, request: AgentRequest) -> BoxStream<'a, StreamEvent> {
        // Drop-cancellation only: dropping the returned stream drops the async_stream
        // future and the in-flight reqwest request. The explicit-token path is
        // `stream_with_cancel`.
        self.stream_with_cancel(request, CancellationToken::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AnthropicModel, AnthropicStopReason};
    use futures_util::stream;

    /// A [`ByteSource`] that replays a recorded SSE body (optionally chunked) with no
    /// network — the seam the unit tests drive the SSE pipeline through.
    struct RecordedSource {
        chunks: Vec<Vec<u8>>,
    }

    impl RecordedSource {
        fn whole(body: &str) -> Self {
            Self {
                chunks: vec![body.as_bytes().to_vec()],
            }
        }
        fn chunked(body: &str, size: usize) -> Self {
            let bytes = body.as_bytes();
            let chunks = bytes.chunks(size).map(<[u8]>::to_vec).collect();
            Self { chunks }
        }
    }

    #[async_trait::async_trait]
    impl ByteSource for RecordedSource {
        async fn open(
            &self,
            _headers: Vec<(&'static str, String)>,
            _body: String,
        ) -> ByteStreamOpen {
            let frames: Vec<Result<Vec<u8>, String>> =
                self.chunks.iter().cloned().map(Ok).collect();
            ByteStreamOpen::Stream(Box::pin(stream::iter(frames)))
        }
    }

    /// A source that simulates an HTTP ≥ 400 response.
    struct ErrorSource {
        status: u16,
        body: String,
    }

    #[async_trait::async_trait]
    impl ByteSource for ErrorSource {
        async fn open(
            &self,
            _headers: Vec<(&'static str, String)>,
            _body: String,
        ) -> ByteStreamOpen {
            ByteStreamOpen::Error(AgentClientError::HttpError {
                status: self.status,
                body: self.body.clone(),
            })
        }
    }

    /// A source that never yields and never completes — used to test cancellation
    /// (the stream must terminate via the cancel token, not the byte stream).
    struct PendingSource;

    #[async_trait::async_trait]
    impl ByteSource for PendingSource {
        async fn open(
            &self,
            _headers: Vec<(&'static str, String)>,
            _body: String,
        ) -> ByteStreamOpen {
            ByteStreamOpen::Stream(Box::pin(stream::pending()))
        }
    }

    fn req() -> AgentRequest {
        AgentRequest::new(AnthropicModel::Haiku45, "you are the editor agent")
    }

    const TEXT_STREAM: &str = "event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":4,\"output_tokens\":1}}}\n\
\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\
\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" there\"}}\n\
\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n";

    const TOOL_STREAM: &str = "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_7\",\"name\":\"get_timeline\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"pa\"}}\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"ge\\\":2}\"}}\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"}}\n";

    #[tokio::test]
    async fn text_stream_yields_deltas_then_message_stop() {
        let client = AnthropicClient::with_source(
            "sk-ant-test",
            Box::new(RecordedSource::whole(TEXT_STREAM)),
        );
        let events: Vec<StreamEvent> = client.stream(req()).collect().await;
        assert_eq!(
            events,
            vec![
                StreamEvent::MessageStart {
                    usage: crate::event::Usage {
                        input_tokens: 4,
                        output_tokens: 1,
                        ..Default::default()
                    }
                },
                StreamEvent::TextDelta("Hi".to_string()),
                StreamEvent::TextDelta(" there".to_string()),
                StreamEvent::MessageStop {
                    reason: AnthropicStopReason::EndTurn
                },
            ]
        );
    }

    #[tokio::test]
    async fn text_stream_chunked_across_byte_boundaries() {
        // Drive the same stream split into tiny chunks — proves the parser's
        // partial-line buffering survives the reqwest frame boundaries.
        let client = AnthropicClient::with_source(
            "sk-ant-test",
            Box::new(RecordedSource::chunked(TEXT_STREAM, 7)),
        );
        let events: Vec<StreamEvent> = client.stream(req()).collect().await;
        assert_eq!(
            events,
            vec![
                StreamEvent::MessageStart {
                    usage: crate::event::Usage {
                        input_tokens: 4,
                        output_tokens: 1,
                        ..Default::default()
                    }
                },
                StreamEvent::TextDelta("Hi".to_string()),
                StreamEvent::TextDelta(" there".to_string()),
                StreamEvent::MessageStop {
                    reason: AnthropicStopReason::EndTurn
                },
            ]
        );
    }

    #[tokio::test]
    async fn tool_use_stream_accumulates_chunked_input_json() {
        let client = AnthropicClient::with_source(
            "sk-ant-test",
            Box::new(RecordedSource::chunked(TOOL_STREAM, 13)),
        );
        let events: Vec<StreamEvent> = client.stream(req()).collect().await;
        // Find the ToolUseComplete and the terminal MessageStop(tool_use).
        let tool = events.iter().find_map(|e| match e {
            StreamEvent::ToolUseComplete { id, name, json } => {
                Some((id.clone(), name.clone(), json.clone()))
            }
            _ => None,
        });
        assert_eq!(
            tool,
            Some((
                "toolu_7".to_string(),
                "get_timeline".to_string(),
                "{\"page\":2}".to_string()
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
    async fn http_400_yields_single_terminal_error() {
        let client = AnthropicClient::with_source(
            "sk-ant-bad",
            Box::new(ErrorSource {
                status: 400,
                body: "{\"type\":\"error\",\"error\":{\"message\":\"bad request\"}}".to_string(),
            }),
        );
        let events: Vec<StreamEvent> = client.stream(req()).collect().await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::Error(msg) => {
                assert!(msg.contains("400"), "error carries status: {msg}");
                assert!(msg.contains("bad request"), "error carries body: {msg}");
            }
            other => panic!("expected terminal Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn http_401_and_429_map_to_typed_http_error() {
        for status in [401u16, 429] {
            let client = AnthropicClient::with_source(
                "sk-ant-x",
                Box::new(ErrorSource {
                    status,
                    body: format!("body for {status}"),
                }),
            );
            let events: Vec<StreamEvent> = client.stream(req()).collect().await;
            assert_eq!(events.len(), 1);
            let StreamEvent::Error(msg) = &events[0] else {
                panic!("expected Error");
            };
            // The HttpError Display carries the status + body verbatim.
            let typed = AgentClientError::HttpError {
                status,
                body: format!("body for {status}"),
            };
            assert_eq!(msg, &typed.to_string());
        }
    }

    #[tokio::test]
    async fn cancellation_token_stops_the_stream() {
        let client =
            AnthropicClient::with_source("sk-ant-x", Box::new(PendingSource));
        let cancel = CancellationToken::new();
        let mut s = client.stream_with_cancel(req(), cancel.clone());

        // Cancel, then the stream must terminate (no events) rather than hang.
        cancel.cancel();
        let next = tokio::time::timeout(std::time::Duration::from_secs(2), s.next()).await;
        assert_eq!(
            next.expect("stream must terminate on cancel, not hang"),
            None
        );
    }

    #[tokio::test]
    async fn dropping_the_stream_is_clean() {
        // Dropping the returned stream must not panic / leak — drop-cancellation.
        let client =
            AnthropicClient::with_source("sk-ant-x", Box::new(PendingSource));
        let s = client.stream(req());
        drop(s);
    }

    #[test]
    fn truncate_utf8_respects_codepoint_boundaries() {
        // "é" is 2 bytes; truncating to 1 must not split it.
        let s = "aé";
        assert_eq!(truncate_utf8(s, 1), "a");
        assert_eq!(truncate_utf8(s, 3), "aé");
        assert_eq!(truncate_utf8(s, 100), "aé");
    }

    /// Live smoke test against the real Anthropic API. Gated behind `#[ignore]` and
    /// only meaningful with a real key in `ANTHROPIC_API_KEY`. Run with:
    /// `cargo test -p palmier-agent -- --ignored live_say_hi`.
    #[tokio::test]
    #[ignore = "hits the real Anthropic API; needs ANTHROPIC_API_KEY"]
    async fn live_say_hi_streams_text_then_end_turn() {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .expect("set ANTHROPIC_API_KEY to run the live test");
        let client = AnthropicClient::new(key).expect("build client");
        let mut request = AgentRequest::new(AnthropicModel::Haiku45, "You are a terse assistant.");
        request.messages.push(crate::client::WireMessage {
            role: "user".to_string(),
            content: vec![serde_json::json!({ "type": "text", "text": "Say hi in one word." })],
        });
        let events: Vec<StreamEvent> = client.stream(request).collect().await;
        assert!(
            events
                .iter()
                .any(|e| matches!(e, StreamEvent::TextDelta(_))),
            "expected at least one TextDelta, got {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                StreamEvent::MessageStop {
                    reason: AnthropicStopReason::EndTurn
                }
            )),
            "expected MessageStop(end_turn), got {events:?}"
        );
    }
}
