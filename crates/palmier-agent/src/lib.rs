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
//! ## Deferred to later E8 stories (NOT in this scaffold)
//! - **E8-S2** — `AnthropicRequestBody::build` (the 2-cache-breakpoint body) +
//!   `AnthropicSSE::parse` (the real line-oriented SSE parser feeding
//!   [`StreamEvent`]).
//! - **E8-S3** — the concrete `AnthropicClient` (BYOK `reqwest` byte-stream +
//!   keyring reload event) replacing [`MockAgentClient`].
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

pub mod availability;
pub mod client;
pub mod event;
pub mod model;
pub mod session_store;

pub use availability::{
    available_models, effective_model, Tier, AGENT_MODEL_CONFIG_KEY, DEFAULT_MODEL,
};
pub use client::{
    can_stream, select_client, AgentClient, AgentClientError, AgentRequest, AnthropicToolSchema,
    MockAgentClient, SelectedBackend, WireMessage, DEFAULT_MAX_TOKENS, NO_BACKEND_MESSAGE,
    SIGN_IN_OR_ADD_KEY_MESSAGE,
};
pub use event::{AnthropicModel, AnthropicStopReason, StreamEvent, Usage};
pub use model::{
    to_canonical_json, AgentContentBlock, AgentMention, AgentMessage, AgentTimelineRangeMention,
    ChatSession, Role, ToolResultBlock, DEFAULT_SESSION_TITLE,
};
pub use session_store::{
    chat_dir, encode_session, load_sessions, session_path, write_session, write_sessions,
    CHAT_DIR_NAME,
};
