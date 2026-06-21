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
//! ([`AGENT_INSTRUCTIONS`]). As of E7-S13 the single source of that text lives in the
//! shared [`palmier_prompt`] crate, which both `palmier-mcp` and `palmier-agent`
//! import (ruling #2 — one constant, no drift between the two injection sites). This
//! crate re-exports it for back-compat; the byte-fidelity gate lives in
//! `palmier-prompt`.

pub mod jsonrpc;
pub mod server;
pub mod validators;
pub mod well_known;

pub use server::{McpServer, MutationCallback, ServerConfig};
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

/// The **verbatim** agent system prompt — the `instructions` field of the MCP
/// `initialize` result.
///
/// Re-exported from the shared [`palmier_prompt`] crate (ruling #2 — single constant,
/// no drift between the MCP `instructions` field and the in-app agent `system` field
/// in `palmier-agent`). The byte-for-byte fidelity gate (length, opening/closing
/// lines, section headers, Unicode-glyph counts) lives in `palmier-prompt`'s tests.
pub use palmier_prompt::AGENT_INSTRUCTIONS;

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

    /// The re-exported prompt is wired (the byte-fidelity gate itself lives in
    /// `palmier-prompt`). This just proves the `initialize` `instructions` field is
    /// non-empty and is the shared constant — a smoke test for the re-export seam.
    #[test]
    fn agent_instructions_reexport_is_wired() {
        assert!(!AGENT_INSTRUCTIONS.is_empty());
        assert!(std::ptr::eq(AGENT_INSTRUCTIONS, palmier_prompt::AGENT_INSTRUCTIONS));
    }
}
