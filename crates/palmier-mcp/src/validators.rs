//! The three request validators (reference `MCPHTTPServer.swift:46-50`
//! `StandardValidationPipeline`: `OriginValidator.localhost`, `ContentTypeValidator`,
//! `ProtocolVersionValidator`).
//!
//! These run as `POST /mcp` request middleware. They are a **security boundary**
//! (SM-C3): do not loosen them to ease client setup. A failing validator rejects the
//! request before it reaches the JSON-RPC dispatch.
//!
//! 1. **Origin** — DNS-rebinding defense. The browser-set `Origin` header must be
//!    **absent** (non-browser clients), `null`, or the exact loopback origin
//!    `http://127.0.0.1:<port>`. Anything else (e.g. `http://evil.com`) is rejected.
//! 2. **Content-Type** — `POST /mcp` bodies must be `application/json` (a `charset`
//!    suffix is tolerated, matching lenient MCP clients).
//! 3. **Protocol version** — the `MCP-Protocol-Version` header, when present, must be
//!    a version this server supports. Absent ⇒ allowed (the MCP spec lets the server
//!    assume a default for backward compatibility); an *unknown* version ⇒ rejected.

use axum::http::{HeaderMap, StatusCode};

/// The MCP protocol version this server advertises in the `initialize` result.
/// Date-based per the MCP spec (`2025-06-18` is the current stable revision the
/// reference clients negotiate).
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Protocol versions the server accepts in the `MCP-Protocol-Version` header. Newer
/// first. The reference clients (Claude Desktop/Code, Cursor, Codex) negotiate one of
/// these; an unknown version is rejected by [`validate_protocol_version`].
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// A validator rejection: an HTTP status + a human-readable reason. Mapped to a
/// JSON-RPC-friendly error response by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorError {
    pub status: StatusCode,
    pub reason: String,
}

impl ValidatorError {
    fn forbidden(reason: impl Into<String>) -> ValidatorError {
        ValidatorError { status: StatusCode::FORBIDDEN, reason: reason.into() }
    }
    fn unsupported_media(reason: impl Into<String>) -> ValidatorError {
        ValidatorError { status: StatusCode::UNSUPPORTED_MEDIA_TYPE, reason: reason.into() }
    }
    fn bad_request(reason: impl Into<String>) -> ValidatorError {
        ValidatorError { status: StatusCode::BAD_REQUEST, reason: reason.into() }
    }
}

/// Run all three validators against an inbound `POST /mcp` request's headers. `port`
/// is the loopback port the server bound, so the allowed Origin is computed exactly.
/// Returns `Ok(())` if every validator passes, else the first failure.
pub fn validate_request(headers: &HeaderMap, port: u16) -> Result<(), ValidatorError> {
    validate_origin(headers, port)?;
    validate_content_type(headers)?;
    validate_protocol_version(headers)?;
    Ok(())
}

/// (1) Origin allowlist. Allowed: header **missing**, `Origin: null`, or
/// `http://127.0.0.1:<port>`. Everything else is rejected (DNS-rebinding defense,
/// reference `OriginValidator.localhost(port:)`).
pub fn validate_origin(headers: &HeaderMap, port: u16) -> Result<(), ValidatorError> {
    let Some(origin) = headers.get("origin") else {
        // Missing Origin: non-browser MCP clients omit it. Allowed.
        return Ok(());
    };
    let origin = origin
        .to_str()
        .map_err(|_| ValidatorError::forbidden("Origin header is not valid UTF-8"))?;

    if origin == "null" {
        return Ok(());
    }
    let expected = format!("http://127.0.0.1:{port}");
    if origin == expected {
        return Ok(());
    }
    Err(ValidatorError::forbidden(format!(
        "Origin '{origin}' is not allowed (loopback only)"
    )))
}

/// (2) Content-Type must be `application/json` (a `; charset=…` suffix is tolerated).
pub fn validate_content_type(headers: &HeaderMap) -> Result<(), ValidatorError> {
    let Some(ct) = headers.get("content-type") else {
        return Err(ValidatorError::unsupported_media(
            "missing Content-Type (expected application/json)",
        ));
    };
    let ct = ct
        .to_str()
        .map_err(|_| ValidatorError::unsupported_media("Content-Type is not valid UTF-8"))?;
    // Tolerate `application/json; charset=utf-8` and surrounding whitespace.
    let essence = ct.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if essence == "application/json" {
        Ok(())
    } else {
        Err(ValidatorError::unsupported_media(format!(
            "Content-Type '{ct}' is not application/json"
        )))
    }
}

/// (3) `MCP-Protocol-Version` header: when present, must be supported. Absent ⇒ OK
/// (server assumes a default version, per the MCP spec's backward-compat rule).
pub fn validate_protocol_version(headers: &HeaderMap) -> Result<(), ValidatorError> {
    let Some(v) = headers.get("mcp-protocol-version") else {
        return Ok(());
    };
    let v = v
        .to_str()
        .map_err(|_| ValidatorError::bad_request("MCP-Protocol-Version is not valid UTF-8"))?;
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) {
        Ok(())
    } else {
        Err(ValidatorError::bad_request(format!(
            "Unsupported MCP-Protocol-Version '{v}' (supported: {})",
            SUPPORTED_PROTOCOL_VERSIONS.join(", ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const PORT: u16 = 19789;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    // ── Origin (allowlist) ──────────────────────────────────────────────────

    #[test]
    fn origin_missing_is_allowed() {
        assert!(validate_origin(&headers(&[]), PORT).is_ok());
    }

    #[test]
    fn origin_null_is_allowed() {
        assert!(validate_origin(&headers(&[("origin", "null")]), PORT).is_ok());
    }

    #[test]
    fn origin_loopback_is_allowed() {
        let h = headers(&[("origin", "http://127.0.0.1:19789")]);
        assert!(validate_origin(&h, PORT).is_ok());
    }

    #[test]
    fn origin_other_is_rejected() {
        let h = headers(&[("origin", "http://evil.com")]);
        let err = validate_origin(&h, PORT).unwrap_err();
        assert_eq!(err.status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn origin_localhost_hostname_is_rejected() {
        // `localhost` is NOT `127.0.0.1` — only the numeric loopback origin matches.
        let h = headers(&[("origin", "http://localhost:19789")]);
        assert!(validate_origin(&h, PORT).is_err());
    }

    // ── Content-Type ────────────────────────────────────────────────────────

    #[test]
    fn content_type_json_is_allowed() {
        let h = headers(&[("content-type", "application/json")]);
        assert!(validate_content_type(&h).is_ok());
    }

    #[test]
    fn content_type_json_with_charset_is_allowed() {
        let h = headers(&[("content-type", "application/json; charset=utf-8")]);
        assert!(validate_content_type(&h).is_ok());
    }

    #[test]
    fn content_type_text_is_rejected() {
        let h = headers(&[("content-type", "text/plain")]);
        let err = validate_content_type(&h).unwrap_err();
        assert_eq!(err.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[test]
    fn content_type_missing_is_rejected() {
        let err = validate_content_type(&headers(&[])).unwrap_err();
        assert_eq!(err.status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    // ── Protocol version ────────────────────────────────────────────────────

    #[test]
    fn protocol_version_absent_is_allowed() {
        assert!(validate_protocol_version(&headers(&[])).is_ok());
    }

    #[test]
    fn protocol_version_supported_is_allowed() {
        for v in SUPPORTED_PROTOCOL_VERSIONS {
            let h = headers(&[("mcp-protocol-version", v)]);
            assert!(validate_protocol_version(&h).is_ok(), "{v} should be allowed");
        }
    }

    #[test]
    fn protocol_version_unknown_is_rejected() {
        let h = headers(&[("mcp-protocol-version", "1999-01-01")]);
        let err = validate_protocol_version(&h).unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    // ── full pipeline ───────────────────────────────────────────────────────

    #[test]
    fn full_pipeline_passes_a_clean_request() {
        let h = headers(&[
            ("content-type", "application/json"),
            ("mcp-protocol-version", "2025-06-18"),
        ]);
        assert!(validate_request(&h, PORT).is_ok());
    }

    #[test]
    fn full_pipeline_origin_failure_short_circuits() {
        let h = headers(&[
            ("origin", "http://evil.com"),
            ("content-type", "application/json"),
        ]);
        assert_eq!(validate_request(&h, PORT).unwrap_err().status, StatusCode::FORBIDDEN);
    }
}
