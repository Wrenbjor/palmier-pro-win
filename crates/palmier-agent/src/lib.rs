//! # palmier-agent
//!
//! In-app agent clients — Anthropic Messages API (BYOK) and the Convex-proxied
//! Palmier client — with SSE streaming and the tool-execution loop
//! (FOUNDATION §4, §6.13). Invokes the SAME `palmier-tools` dispatcher the MCP
//! server uses.
//!
//! ## Story E8-S1 — scaffold (this slice)
//! This story lands the **data model + client trait + event types + model
//! availability + session-persistence seam** — the stable foundation every later
//! E8 story builds on. Per `docs/reference/agent-panel.md`:
//!
//! - [`model`] — the message/content/session value types ([`AgentMessage`],
//!   [`AgentContentBlock`], [`ToolResultBlock`], [`ChatSession`], [`Role`],
//!   mentions) with serde matching the reference `Codable` wire JSON (the `kind`
//!   discriminator; `input_json` stored + forwarded **verbatim**).
//! - [`event`] — the streaming types ([`StreamEvent`], [`AnthropicModel`],
//!   [`AnthropicStopReason`], [`Usage`]) the (later) SSE parser emits.
//! - [`client`] — the [`AgentClient`] trait + request/response types +
//!   [`select_client`] selection logic, plus a [`MockAgentClient`] stub.
//! - [`availability`] — model lists + tier gating ([`available_models`],
//!   [`effective_model`]).
//! - [`session_store`] — read/write [`ChatSession`] to `<project>/chat/<uuid>.json`.
//!
//! ## E8-S2 — request body builder + shared SSE parser (this slice)
//! - [`request`] — `AnthropicRequestBody::build`: serialize an [`AgentRequest`]
//!   into the exact Anthropic Messages JSON (model wire-id, `max_tokens` 8192,
//!   `stream`, the system block + tools array + messages with the 2 ephemeral cache
//!   breakpoints / 3 wire markers), in canonical sorted-key form. Plus the headers
//!   contract ([`request::anthropic_headers`]) as data for the transport.
//! - [`sse`] — `AnthropicSSE::parse`: the line-oriented `text/event-stream` parser
//!   (`message_start`→Usage, `text_delta`→TextDelta, chunked `input_json_delta` +
//!   `content_block_start`/`stop`→ToolUseComplete, `message_delta`→MessageStop,
//!   `error`→Error), with partial-line buffering ([`sse::SseParser::feed`]) for the
//!   live byte stream. The reqwest HTTP transport that drives it is E8-S3.
//!
//! ## E8-S3 — concrete `AnthropicClient` (BYOK transport) (this slice)
//! - [`anthropic_client`] — the real [`AnthropicClient`]: builds the body via
//!   [`request::build_bytes`], `POST`s to `api.anthropic.com/v1/messages` with
//!   [`request::anthropic_headers`] over **async `reqwest`** (rustls, no system
//!   OpenSSL), streams the response bytes through [`sse::SseParser`], and yields
//!   [`StreamEvent`]s as a `BoxStream`. HTTP ≥ 400 → a terminal
//!   [`StreamEvent::Error`] carrying [`AgentClientError::HttpError`]; per-chunk
//!   cancellation via a `CancellationToken` (and drop-cancellation). The HTTP send
//!   is behind a [`ByteSource`] seam so the SSE pipeline is unit-tested off a
//!   recorded stream; `tests/anthropic_http.rs` drives the real reqwest path over a
//!   local `wiremock` server. The API key is a constructor parameter — the keyring
//!   load + reload event live in `palmier-auth`. [`client::build_client`] /
//!   [`client::select_and_build_client`] wire [`select_client`] to construct it.
//!
//! ## Deferred to later E8 stories (NOT in this slice)
//! - **E8-S4** — the agentic run loop + `palmier-tools::execute` dispatch +
//!   orphan-tool_use repair.
//! - **E8-S5** — `api_messages()` wire projection + mentions/context-hints +
//!   image inlining.
//! - **E8-S6** — the `PalmierClient` (Convex-proxied) transport + live model
//!   catalog.
//! - **E8-S7** — the tab/session orchestration + save-on-document-save trigger.
//!
//! Keyring storage (account `anthropic-api-key`, ruling #5) and the signed-in /
//! credit state both live in `palmier-auth`; this crate consumes them via
//! [`client::select_client`] / [`client::can_stream`].

pub mod anthropic_client;
pub mod availability;
pub mod client;
pub mod event;
pub mod model;
pub mod request;
pub mod session_store;
pub mod sse;

pub use availability::{
    available_models, effective_model, Tier, AGENT_MODEL_CONFIG_KEY, DEFAULT_MODEL,
};
pub use anthropic_client::{
    AnthropicClient, ByteSource, ByteStreamOpen, HttpByteSource, ANTHROPIC_MESSAGES_URL,
};
pub use client::{
    build_client, can_stream, select_and_build_client, select_client, AgentClient,
    AgentClientError, AgentRequest, AnthropicToolSchema, MockAgentClient, SelectedBackend,
    WireMessage, DEFAULT_MAX_TOKENS, NO_BACKEND_MESSAGE, SIGN_IN_OR_ADD_KEY_MESSAGE,
};
pub use event::{AnthropicModel, AnthropicStopReason, StreamEvent, Usage};
pub use model::{
    to_canonical_json, AgentContentBlock, AgentMention, AgentMessage, AgentTimelineRangeMention,
    ChatSession, Role, ToolResultBlock, DEFAULT_SESSION_TITLE,
};
pub use request::{
    anthropic_headers, build as build_request_body, build_bytes as build_request_bytes,
    cache_control_marker_count, ANTHROPIC_VERSION,
};
pub use sse::{parse_lines as parse_sse_lines, parse_str as parse_sse, SseParser};
pub use session_store::{
    chat_dir, encode_session, load_sessions, session_path, write_session, write_sessions,
    CHAT_DIR_NAME,
};
