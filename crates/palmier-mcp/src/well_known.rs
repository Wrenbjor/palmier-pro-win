//! `GET /.well-known/oauth-protected-resource` — the OAuth protected-resource
//! metadata for the Claude Desktop one-click handshake (reference
//! `MCPHTTPServer.swift:72-77`).
//!
//! The body is the literal `{"resource":"http://127.0.0.1:<port>"}` — note the
//! resource value has **no trailing path** (not `/mcp`). The reference builds the
//! string by hand; we mirror that exactly so the bytes match.

/// Build the `.well-known/oauth-protected-resource` JSON body for `port`. The
/// `resource` value is the loopback origin with no trailing path (reference
/// `"{\"resource\":\"http://127.0.0.1:\(port)\"}"`).
pub fn oauth_protected_resource_body(port: u16) -> String {
    format!("{{\"resource\":\"http://127.0.0.1:{port}\"}}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn body_is_the_literal_reference_shape() {
        assert_eq!(
            oauth_protected_resource_body(19789),
            r#"{"resource":"http://127.0.0.1:19789"}"#
        );
    }

    #[test]
    fn body_has_no_trailing_path() {
        let body = oauth_protected_resource_body(19789);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["resource"], "http://127.0.0.1:19789");
        assert!(!v["resource"].as_str().unwrap().ends_with("/mcp"));
    }

    #[test]
    fn body_tracks_the_configured_port() {
        assert_eq!(
            oauth_protected_resource_body(28000),
            r#"{"resource":"http://127.0.0.1:28000"}"#
        );
    }
}
