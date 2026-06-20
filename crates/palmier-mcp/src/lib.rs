//! # palmier-mcp
//!
//! Local MCP server — HTTP on `127.0.0.1:19789`, the strategic centerpiece
//! (FOUNDATION §4, §6.14). Wires `rmcp`'s tool registry to the `palmier-tools`
//! dispatcher; no protocol re-implementation. `rmcp`/`axum`/`tokio` are added
//! per-story, not in this skeleton.

/// Default loopback bind address for the MCP server (FOUNDATION §6.14).
pub const DEFAULT_BIND: &str = "127.0.0.1:19789";

#[cfg(test)]
mod tests {
    #[test]
    fn default_bind_is_loopback() {
        assert_eq!(super::DEFAULT_BIND, "127.0.0.1:19789");
    }
}
