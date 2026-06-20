//! # palmier-mcp
//!
//! The local **loopback HTTP JSON-RPC** MCP server (E7-S11) — the network surface of
//! the product's strategic centerpiece. It binds **`127.0.0.1:19789`** (loopback
//! only), validates every request through three middleware validators, and
//! dispatches JSON-RPC into the shared [`palmier_tools`] dispatcher so external MCP
//! clients (Claude Desktop / Claude Code / Cursor / Codex) reach the same 30 tools
//! and 2 resources the in-app agent uses.
//!
//! ## Why axum + hand-rolled JSON-RPC (not rmcp)
//!
//! The spec allowed either. We chose **axum + a thin JSON-RPC layer** deliberately:
//!
//! - `palmier-tools` already emits the exact MCP `CallTool.Result` JSON via
//!   [`ToolResult::to_mcp_json`](palmier_tools::ToolResult::to_mcp_json). rmcp would
//!   force re-wrapping into its own result types and re-deriving the contract error
//!   shape, fighting the existing seam.
//! - The **three validators** (Origin allowlist, content-type, protocol version) and
//!   the **loopback-only bind** are a security boundary (SM-C3) we want to control
//!   exactly. axum middleware expresses them directly; rmcp's transport abstraction
//!   would bury them.
//! - This mirrors the reference's own split: `MCPHTTPServer.swift` hand-rolls the
//!   HTTP + validators + routing and only uses the SDK for request handling. Here
//!   axum = HTTP/validators/routing, this crate = JSON-RPC protocol, `palmier-tools`
//!   = tool/resource logic.
//!
//! ## Surface
//!
//! - `POST /mcp` — JSON-RPC, **single OR batched**. Methods: `initialize`,
//!   `tools/list`, `tools/call`, `resources/list`, `resources/read`, plus the
//!   `notifications/initialized` notification.
//! - `GET /.well-known/oauth-protected-resource` — the literal body
//!   `{"resource":"http://127.0.0.1:<port>"}` for the Claude Desktop handshake.
//!
//! ## Boot seam (NOT wired here)
//!
//! [`McpServer::start`] / [`McpServer::stop`] are the API the Tauri boot sequence
//! (Epic 1, step 6) calls behind the `io.palmier.pro.mcp.enabled` pref (ruling #6,
//! absent ⇒ ON). This story exposes the API but does **not** wire it into
//! `palmier-tauri` — that is the boot-integration story.
//!
//! ## Agent instructions constant
//!
//! The `initialize` response embeds the **verbatim** agent instructions
//! ([`AGENT_INSTRUCTIONS`]) via `include_str!`. E7-S13 hoists this to a shared
//! `palmier-prompt` module that both `palmier-mcp` and `palmier-agent` import
//! (ruling #2 — one constant, no drift). Until that lands, the constant lives here
//! so the `initialize` identity is correct and parity-tested today; E7-S13 should
//! replace this `include_str!` with the shared import and delete the local copy.

pub mod jsonrpc;
pub mod server;
pub mod validators;
pub mod well_known;

pub use server::{McpServer, ServerConfig};
pub use validators::{ValidatorError, MCP_PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS};

use std::net::Ipv4Addr;

/// The server identity `name` (reference `MCPService.swift:38`, `Server(name:)`).
/// The literal product token stays `palmier-pro` (ruling #2/#3) — model-facing
/// identity is unchanged even though the Windows app is branded "Palmier Pro
/// Windows".
pub const SERVER_NAME: &str = "palmier-pro";

/// The server identity `version` (reference `MCPService.swift:39`).
pub const SERVER_VERSION: &str = "1.0.0";

/// The default loopback port (reference `MCPService.port = 19789`). Configurable via
/// [`ServerConfig::port`]; the **bind host is always loopback** (SM-C3 — never made
/// bindable to a non-localhost interface).
pub const DEFAULT_PORT: u16 = 19789;

/// The only interface the server ever binds to: IPv4 loopback. The server is never
/// reachable from the LAN (reference `requiredLocalEndpoint host:127.0.0.1`,
/// FR-25/SM-C3).
pub const BIND_HOST: Ipv4Addr = Ipv4Addr::LOCALHOST;

/// The default loopback bind address string (back-compat with the scaffold).
pub const DEFAULT_BIND: &str = "127.0.0.1:19789";

/// The **verbatim** agent system prompt (E7-S13 constant, ported byte-for-byte from
/// `docs/reference/agent-instructions.md` VERBATIM block / reference
/// `AgentInstructions.serverInstructions`). Embedded as `include_str!` so the bytes
/// are reviewable as plain text and never re-authored. Preserves Unicode
/// (`×` U+00D7, `–` U+2013, `•` U+2022, `…`) as UTF-8 — do **not** ASCII-fold.
///
/// This is the `instructions` field of the MCP `initialize` result. E7-S13 will move
/// this to a shared `palmier-prompt` module imported by both this crate and
/// `palmier-agent` (ruling #2 — single constant, no drift between injection sites).
pub const AGENT_INSTRUCTIONS: &str = include_str!("agent_instructions.txt");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_bind_is_loopback() {
        assert_eq!(DEFAULT_BIND, "127.0.0.1:19789");
        assert_eq!(BIND_HOST, Ipv4Addr::LOCALHOST);
        assert_eq!(DEFAULT_PORT, 19789);
    }

    #[test]
    fn server_identity_matches_reference() {
        assert_eq!(SERVER_NAME, "palmier-pro");
        assert_eq!(SERVER_VERSION, "1.0.0");
    }

    /// Verbatim-fidelity gate on the embedded agent instructions. These assertions
    /// live in-crate (the source `.md` block is outside the crate). E7-S13 adds the
    /// byte-diff-vs-md gate when it hoists the constant to the shared module.
    #[test]
    fn agent_instructions_are_verbatim() {
        // Exact opening line (reference first line).
        assert!(AGENT_INSTRUCTIONS.starts_with(
            "You are a creative AI assistant connected to palmier-pro, an AI-native video editor."
        ));
        // The literal product token stays as written.
        assert!(AGENT_INSTRUCTIONS.contains("palmier-pro"));
        // Every section header in order (the prompt's behavioral contract).
        for header in [
            "# Core model",
            "# Always do",
            "# Editing",
            "# Generation",
            "# Audio generation",
            "# Prompt craft",
            "# Communication",
        ] {
            assert!(AGENT_INSTRUCTIONS.contains(header), "missing section {header}");
        }
        // Unicode glyphs survive as UTF-8 (do NOT ASCII-fold).
        assert!(AGENT_INSTRUCTIONS.contains('×'), "× U+00D7 preserved");
        assert!(AGENT_INSTRUCTIONS.contains('–'), "– U+2013 preserved");
        assert!(AGENT_INSTRUCTIONS.contains('•'), "• U+2022 preserved");
        // Exact byte length of the ported block (drift tripwire).
        assert_eq!(AGENT_INSTRUCTIONS.len(), 8694, "instructions byte length drifted");
    }
}
