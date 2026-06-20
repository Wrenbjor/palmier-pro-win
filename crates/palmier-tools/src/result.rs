//! `ToolResult` — the tool output type and the wire error shape (E7-S1 scaffold
//! seam; reference `ToolResult.swift`).
//!
//! Every tool returns a [`ToolResult`]: a list of content [`Block`]s plus an
//! `is_error` flag. The wire JSON ([`ToolResult::to_mcp_json`]) matches the
//! reference's `toMCPResult()` / FOUNDATION §6.14 exactly. The **error shape** is
//! the load-bearing contract clients depend on:
//!
//! ```json
//! { "isError": true, "content": [{ "type": "text", "text": "<msg>" }] }
//! ```
//!
//! `ToolResult` carries Rust-side `Block`s (text or image); the MCP/rmcp adapter
//! in `palmier-mcp` (E7-S11) consumes [`ToolResult::to_mcp_json`]. Keeping the
//! wire mapping here means both the MCP server and the in-app agent (E8) serialize
//! identically.

use serde_json::{json, Value};

/// A single content block in a tool result (reference `ToolResult.Block`).
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Plain text content (the dominant case).
    Text(String),
    /// A base64-encoded image with its MIME media type
    /// (reference `.image(base64:mediaType:)`).
    Image { base64: String, media_type: String },
}

impl Block {
    /// The MCP wire object for this block (`{"type":"text","text":…}` or
    /// `{"type":"image","data":…,"mimeType":…}`), matching `toMCPResult()`.
    pub fn to_mcp_json(&self) -> Value {
        match self {
            Block::Text(s) => json!({ "type": "text", "text": s }),
            Block::Image { base64, media_type } => {
                json!({ "type": "image", "data": base64, "mimeType": media_type })
            }
        }
    }
}

/// A tool's result: content blocks + error flag (reference `ToolResult`).
///
/// Constructed via [`ToolResult::ok`] / [`ToolResult::error`] for the common
/// single-text-block cases, matching the reference `ToolResult.ok/.error`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub content: Vec<Block>,
    pub is_error: bool,
}

impl ToolResult {
    /// A successful single-text result (reference `ToolResult.ok`).
    pub fn ok(text: impl Into<String>) -> ToolResult {
        ToolResult { content: vec![Block::Text(text.into())], is_error: false }
    }

    /// An error single-text result (reference `ToolResult.error`). Serializes to
    /// the contract error shape `{ "isError": true, "content": [{…text…}] }`.
    pub fn error(message: impl Into<String>) -> ToolResult {
        ToolResult { content: vec![Block::Text(message.into())], is_error: true }
    }

    /// Map to the MCP `CallTool.Result` JSON shape (reference `toMCPResult()`).
    ///
    /// `isError` is emitted as `true` only when set (the reference passes `nil`
    /// for the success case, which serializes the key as absent — we mirror that
    /// by omitting the key when `false`).
    pub fn to_mcp_json(&self) -> Value {
        let content: Vec<Value> = self.content.iter().map(Block::to_mcp_json).collect();
        if self.is_error {
            json!({ "content": content, "isError": true })
        } else {
            json!({ "content": content })
        }
    }
}
