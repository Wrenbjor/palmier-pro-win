//! The [`AgentClient`] trait + request/response types + client-selection logic.
//!
//! Ports the `AgentClient` protocol and `AgentService.selectClient`
//! (`agent-panel.md` lines 47-50, 63-69 of `AgentClientTypes.swift`).
//!
//! ## What this story lands (scaffold)
//! - [`AgentClient`] — the transport abstraction over `AnthropicClient` (BYOK,
//!   direct `api.anthropic.com`) and `PalmierClient` (Convex-proxied). `stream`
//!   returns a boxed [`StreamEvent`] stream so the trait is **dyn-compatible**
//!   (selection returns `Box<dyn AgentClient>`).
//! - [`AgentRequest`] / [`AnthropicToolSchema`] / [`WireMessage`] — the request
//!   shape `stream` takes (the real body builder + SSE wiring are E8-S2/S3).
//! - [`select_client`] — the BYOK-vs-proxied-vs-none decision (`agent-panel.md`
//!   lines 47-50).
//! - [`MockAgentClient`] — a scripted stub so the loop/UI can be built and tested
//!   before the live HTTP transports land. **NOT** a real network client.
//!
//! ## Deferred (later E8 stories)
//! The concrete `AnthropicClient` (E8-S3: `reqwest` byte-stream + keyring) and
//! `PalmierClient` (E8-S6: Convex proxy + Clerk JWT) replace [`MockAgentClient`].
//! The shared `AnthropicRequestBody::build` (2 cache breakpoints, sorted keys)
//! and `AnthropicSSE::parse` are E8-S2. This module defines only the seam.

use crate::event::{AnthropicModel, StreamEvent};
use futures_core::stream::BoxStream;
use serde::{Deserialize, Serialize};

/// A tool definition advertised to the model (reference `AnthropicToolSchema`).
///
/// `input_schema` is a raw JSON value (the JSON-Schema object) so it round-trips
/// without the agent crate having to know each tool's shape — the catalogue is
/// owned by `palmier-tools` (Epic 7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnthropicToolSchema {
    /// Tool name (e.g. `get_timeline`).
    pub name: String,
    /// Contract description text (verbatim from `palmier-tools`).
    pub description: String,
    /// The JSON-Schema `input_schema` object.
    #[serde(rename = "input_schema")]
    pub input_schema: serde_json::Value,
}

/// A wire-projected message (reference `AnthropicMessage`).
///
/// `content` is the already-projected array of content blocks (objects). The
/// projection from [`crate::model::AgentMessage`] (`api_messages()`, mentions,
/// image inlining) is E8-S5; this is just the carrier the request body builder
/// consumes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireMessage {
    /// `"user"` or `"assistant"`.
    pub role: String,
    /// The content blocks for this message (already wire-shaped).
    pub content: Vec<serde_json::Value>,
}

/// Everything [`AgentClient::stream`] needs to issue one streaming request.
///
/// Identical for both transports — the body builder (E8-S2) turns this into the
/// exact wire bytes with the 2 cache breakpoints. `max_tokens` defaults to the
/// reference 8192.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRequest {
    /// The target model (its `wire_id` goes in the body's `model`).
    pub model: AnthropicModel,
    /// `max_tokens` (reference default 8192).
    pub max_tokens: u32,
    /// The verbatim shared agent system prompt (Epic 7 owns the constant).
    pub system: String,
    /// The tool catalogue advertised this turn.
    pub tools: Vec<AnthropicToolSchema>,
    /// The wire-projected conversation.
    pub messages: Vec<WireMessage>,
}

/// The reference `max_tokens` for every agent request (`agent-panel.md` line 60).
pub const DEFAULT_MAX_TOKENS: u32 = 8192;

impl AgentRequest {
    /// A request with the reference `max_tokens` (8192) and no tools/messages —
    /// callers fill in `tools` / `messages` (the body builder lands in E8-S2).
    #[must_use]
    pub fn new(model: AnthropicModel, system: impl Into<String>) -> Self {
        Self {
            model,
            max_tokens: DEFAULT_MAX_TOKENS,
            system: system.into(),
            tools: Vec::new(),
            messages: Vec::new(),
        }
    }
}

/// Errors a transport can surface (reference `AnthropicClientError` /
/// `PalmierClientError`).
///
/// The proxied-specific mapping (`unauthenticated` / `insufficient_credits`) is
/// E8-S6; the scaffold carries the shared shapes so the loop can match on them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentClientError {
    /// No Anthropic API key is set (reference `.missingAPIKey`).
    #[error("No Anthropic API key is set.")]
    MissingApiKey,
    /// Transport HTTP error (reference `.httpError(status, body)`).
    #[error("Anthropic API error ({status}): {body}")]
    HttpError {
        /// HTTP status code (≥ 400).
        status: u16,
        /// The drained response body (truncated for display by the caller).
        body: String,
    },
    /// A `data: {"type":"error"}` SSE event (reference `.streamError`).
    #[error("Stream error: {0}")]
    StreamError(String),
    /// Proxied: the Clerk session is not authenticated (reference
    /// `.unauthenticated`; status 401 / `code == "unauthenticated"`).
    #[error("Not signed in.")]
    Unauthenticated,
    /// Proxied: the account is out of credits (reference `.insufficientCredits`;
    /// status 402 / `code == "insufficient_credits"`).
    #[error("Insufficient credits.")]
    InsufficientCredits,
    /// Any other upstream error (reference `.upstream`).
    #[error("{0}")]
    Upstream(String),
}

/// The streaming transport abstraction (reference `AgentClient` protocol).
///
/// Both `AnthropicClient` (BYOK) and `PalmierClient` (proxied) implement this.
/// `stream` returns a **boxed** [`StreamEvent`] stream so the trait stays
/// dyn-compatible — [`select_client`] hands back a `Box<dyn AgentClient>` and the
/// loop drives it uniformly. A transport-level failure (bad key, HTTP ≥ 400) is
/// delivered as a terminal [`StreamEvent::Error`] rather than a `Result`, so the
/// loop has one event channel to consume (mirrors the reference, where errors
/// arrive via `continuation.finish(throwing:)`).
pub trait AgentClient: Send + Sync {
    /// Open a streaming request and yield [`StreamEvent`]s until the turn ends or
    /// the stream errors. Dropping the returned stream cancels the in-flight
    /// request (reference `continuation.onTermination { task.cancel() }`).
    fn stream<'a>(&'a self, request: AgentRequest) -> BoxStream<'a, StreamEvent>;
}

/// The agent backend chosen by [`select_client`] (reference `selectClient`
/// branches). Carries enough to construct the concrete client downstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectedBackend {
    /// BYOK direct to `api.anthropic.com` with this key (reference
    /// `AnthropicClient(apiKey, …)`).
    Anthropic {
        /// The Anthropic API key from the OS keyring.
        api_key: String,
    },
    /// Convex-proxied for a signed-in user (reference `PalmierClient(…)`).
    Palmier,
    /// No backend: neither a key nor a signed-in session (reference `nil` →
    /// `streamError = .upstream("No backend available.")`).
    None,
}

/// The "no backend available" message (reference `selectClient` nil branch).
pub const NO_BACKEND_MESSAGE: &str = "No backend available.";

/// The send-gate prompt when the user can neither stream nor add a key
/// (reference `send()` `canStream` guard; `agent-panel.md` lines 51-52).
pub const SIGN_IN_OR_ADD_KEY_MESSAGE: &str =
    "Sign in to a paid plan or add an Anthropic API key to start.";

/// Decide which backend the agent should use (reference `selectClient`):
/// 1. A non-empty keyring Anthropic key → [`SelectedBackend::Anthropic`].
/// 2. Else a signed-in account → [`SelectedBackend::Palmier`].
/// 3. Else → [`SelectedBackend::None`].
///
/// `api_key` should already be trimmed-empty-⇒-`None` (the keyring façade does
/// this — see `palmier-auth::AnthropicKeyStore::load`). `is_signed_in` is the
/// account state machine's `is_signed_in` (`palmier-auth::AccountState`).
#[must_use]
pub fn select_client(api_key: Option<&str>, is_signed_in: bool) -> SelectedBackend {
    match api_key {
        Some(key) if !key.trim().is_empty() => SelectedBackend::Anthropic {
            api_key: key.to_string(),
        },
        _ if is_signed_in => SelectedBackend::Palmier,
        _ => SelectedBackend::None,
    }
}

/// Construct a live [`AgentClient`] for the chosen backend (reference
/// `selectClient` → a concrete client). This is the bridge from the pure
/// [`select_client`] decision to a usable transport:
/// - [`SelectedBackend::Anthropic`] → a real
///   [`AnthropicClient`](crate::anthropic_client::AnthropicClient) (BYOK, direct
///   `api.anthropic.com`, E8-S3).
/// - [`SelectedBackend::Palmier`] → **deferred to E8-S6** (the Convex-proxied
///   transport); until then this returns [`AgentClientError::Upstream`] so the
///   caller surfaces the proxied path as not-yet-wired rather than silently
///   selecting nothing.
/// - [`SelectedBackend::None`] → [`AgentClientError::Upstream`] carrying
///   [`NO_BACKEND_MESSAGE`].
///
/// # Errors
/// - [`AgentClientError::Upstream`] when no backend is available, the proxied path
///   is selected (E8-S6 not yet landed), or the `reqwest`/rustls client fails to
///   build.
pub fn build_client(backend: SelectedBackend) -> Result<Box<dyn AgentClient>, AgentClientError> {
    match backend {
        SelectedBackend::Anthropic { api_key } => {
            let client = crate::anthropic_client::AnthropicClient::new(api_key)?;
            Ok(Box::new(client))
        }
        SelectedBackend::Palmier => Err(AgentClientError::Upstream(
            "Convex-proxied transport is not yet available (E8-S6).".to_string(),
        )),
        SelectedBackend::None => Err(AgentClientError::Upstream(NO_BACKEND_MESSAGE.to_string())),
    }
}

/// Select and construct the live [`AgentClient`] in one step (reference
/// `selectClient`): runs [`select_client`] then [`build_client`]. The BYOK path
/// (key present) yields a real
/// [`AnthropicClient`](crate::anthropic_client::AnthropicClient).
///
/// # Errors
/// Propagates [`build_client`]'s errors (no backend / proxied-not-wired / client
/// build failure).
pub fn select_and_build_client(
    api_key: Option<&str>,
    is_signed_in: bool,
) -> Result<Box<dyn AgentClient>, AgentClientError> {
    build_client(select_client(api_key, is_signed_in))
}

/// Whether a turn may be streamed at all (reference `canStream =
/// has_api_key || (is_signed_in && has_credits)`; `agent-panel.md` lines 51-52).
///
/// The send gate (E8-S4/UI) consults this before allowing a send.
#[must_use]
pub fn can_stream(has_api_key: bool, is_signed_in: bool, has_credits: bool) -> bool {
    has_api_key || (is_signed_in && has_credits)
}

/// A scripted [`AgentClient`] stub for building/testing the loop + UI before the
/// live HTTP transports land (E8-S3/S6). Replays a fixed event script on every
/// [`AgentClient::stream`] call — no network, no keyring, no SSE parsing.
///
/// This is **scaffold only**; it lets E8-S4 develop the run loop and E8-S8 the
/// panel against deterministic streams.
#[derive(Debug, Clone, Default)]
pub struct MockAgentClient {
    script: Vec<StreamEvent>,
}

impl MockAgentClient {
    /// A mock that replays `script` verbatim on each `stream` call.
    #[must_use]
    pub fn new(script: Vec<StreamEvent>) -> Self {
        Self { script }
    }

    /// A mock that streams `text` as a single delta then ends the turn
    /// (`end_turn`). Handy default for "say hi"-style tests.
    #[must_use]
    pub fn say(text: impl Into<String>) -> Self {
        Self {
            script: vec![
                StreamEvent::TextDelta(text.into()),
                StreamEvent::MessageStop {
                    reason: crate::event::AnthropicStopReason::EndTurn,
                },
            ],
        }
    }
}

impl AgentClient for MockAgentClient {
    fn stream<'a>(&'a self, _request: AgentRequest) -> BoxStream<'a, StreamEvent> {
        let script = self.script.clone();
        Box::pin(futures_util::stream::iter(script))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::AnthropicStopReason;
    use futures_util::StreamExt;

    #[test]
    fn select_prefers_key_over_signed_in() {
        // A present key wins even when signed in (reference order).
        let b = select_client(Some("sk-ant-xyz"), true);
        assert_eq!(
            b,
            SelectedBackend::Anthropic {
                api_key: "sk-ant-xyz".to_string()
            }
        );
    }

    #[test]
    fn select_blank_key_falls_through_to_signed_in() {
        // Whitespace-only key is treated as absent → proxied.
        assert_eq!(select_client(Some("   "), true), SelectedBackend::Palmier);
        assert_eq!(select_client(None, true), SelectedBackend::Palmier);
    }

    #[test]
    fn select_signed_in_without_key_is_palmier() {
        assert_eq!(select_client(None, true), SelectedBackend::Palmier);
    }

    #[test]
    fn select_no_key_not_signed_in_is_none() {
        assert_eq!(select_client(None, false), SelectedBackend::None);
        assert_eq!(select_client(Some(""), false), SelectedBackend::None);
    }

    #[test]
    fn can_stream_matrix() {
        // BYOK key alone is enough.
        assert!(can_stream(true, false, false));
        // Signed in + credits.
        assert!(can_stream(false, true, true));
        // Signed in but no credits → cannot stream.
        assert!(!can_stream(false, true, false));
        // Nothing.
        assert!(!can_stream(false, false, true));
    }

    #[test]
    fn agent_request_defaults_max_tokens() {
        let r = AgentRequest::new(AnthropicModel::Sonnet46, "you are an editor");
        assert_eq!(r.max_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(r.max_tokens, 8192);
        assert!(r.tools.is_empty());
    }

    #[tokio::test]
    async fn mock_client_replays_script() {
        let client = MockAgentClient::say("hi there");
        let req = AgentRequest::new(AnthropicModel::Haiku45, "sys");
        let events: Vec<StreamEvent> = client.stream(req).collect().await;
        assert_eq!(
            events,
            vec![
                StreamEvent::TextDelta("hi there".to_string()),
                StreamEvent::MessageStop {
                    reason: AnthropicStopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn build_client_byok_key_yields_a_real_client() {
        // A present key builds a concrete AnthropicClient (no network at build time).
        let client = build_client(SelectedBackend::Anthropic {
            api_key: "sk-ant-xyz".to_string(),
        });
        assert!(client.is_ok(), "BYOK key must build a live client");
    }

    #[test]
    fn build_client_palmier_is_not_yet_wired() {
        // E8-S6 lands the proxied transport; until then this is an explicit error,
        // not a silent no-op. (Box<dyn AgentClient> isn't Debug, so match rather
        // than unwrap_err.)
        match build_client(SelectedBackend::Palmier) {
            Err(AgentClientError::Upstream(_)) => {}
            Ok(_) => panic!("proxied path must not build a client yet (E8-S6)"),
            Err(other) => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn build_client_none_carries_no_backend_message() {
        match build_client(SelectedBackend::None) {
            Err(e) => assert_eq!(e, AgentClientError::Upstream(NO_BACKEND_MESSAGE.to_string())),
            Ok(_) => panic!("no backend must not build a client"),
        }
    }

    #[test]
    fn select_and_build_prefers_byok() {
        // A key present + signed in → the BYOK client is built (key wins).
        assert!(select_and_build_client(Some("sk-ant-xyz"), true).is_ok());
        // No key, not signed in → no-backend error.
        match select_and_build_client(None, false) {
            Err(e) => assert_eq!(e, AgentClientError::Upstream(NO_BACKEND_MESSAGE.to_string())),
            Ok(_) => panic!("no backend must not build a client"),
        }
    }

    #[tokio::test]
    async fn mock_client_is_dyn_compatible() {
        // Proves the trait is object-safe — selection returns Box<dyn AgentClient>.
        let client: Box<dyn AgentClient> = Box::new(MockAgentClient::say("ok"));
        let req = AgentRequest::new(AnthropicModel::Sonnet46, "sys");
        let n = client.stream(req).count().await;
        assert_eq!(n, 2);
    }

    #[test]
    fn tool_schema_round_trips_with_input_schema_key() {
        let schema = AnthropicToolSchema {
            name: "get_timeline".to_string(),
            description: "Read the timeline.".to_string(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        };
        let v = serde_json::to_value(&schema).unwrap();
        assert_eq!(v["name"], "get_timeline");
        assert!(v.get("input_schema").is_some());
        let back: AnthropicToolSchema = serde_json::from_value(v).unwrap();
        assert_eq!(back, schema);
    }
}
