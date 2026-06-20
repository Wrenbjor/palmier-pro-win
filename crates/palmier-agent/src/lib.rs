//! # palmier-agent
//!
//! In-app agent clients — Anthropic Messages API (BYOK) and the Convex-proxied
//! Palmier client — with SSE streaming and the tool-execution loop
//! (FOUNDATION §4, §6.13). Invokes the SAME `palmier-tools` dispatcher the MCP
//! server uses. HTTP/SSE deps are added per-story, not in this skeleton.

/// Placeholder for the agent subsystem.
pub fn placeholder() -> &'static str {
    "palmier-agent"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "palmier-agent");
    }
}
