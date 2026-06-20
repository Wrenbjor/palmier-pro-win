//! # palmier-tools
//!
//! Shared tool dispatch — the single implementation of the 30 MCP tools, invoked
//! by BOTH the MCP server (`palmier-mcp`) and the in-app agent (`palmier-agent`),
//! exactly one impl per tool name (FOUNDATION §4, §6.14). Operates over the
//! `palmier-model` shapes; ID prefix shortening + agent undo stack live here.
//!
//! Skeleton stub: real tool registry + dispatcher land per Epic 7.

/// Dispatch a tool by name. Skeleton stub returning an error for any name.
pub fn execute(_name: &str, _args: serde_json::Value) -> Result<serde_json::Value, String> {
    Err("palmier-tools: not yet implemented".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn execute_is_stubbed() {
        let r = super::execute("get_timeline", serde_json::Value::Null);
        assert!(r.is_err());
    }
}
