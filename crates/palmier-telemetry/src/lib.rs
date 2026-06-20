//! # palmier-telemetry
//!
//! Sentry crash reporting + `tracing-subscriber` categorized logging with daily
//! rotation (FOUNDATION §4, §6.16; E1-S2). Categories: app/editor/export/preview/
//! mcp/generation/project/transcription/search. Sentry/tracing deps are added
//! per-story, not in this skeleton.

/// Reference log category targets (FOUNDATION §6.16).
pub const CATEGORIES: &[&str] = &[
    "app",
    "editor",
    "export",
    "preview",
    "mcp",
    "generation",
    "project",
    "transcription",
    "search",
];

/// Start telemetry. Skeleton no-op; real impl lands per E1-S2.
pub fn start(_enabled: bool, _dsn: Option<&str>) {
    // no-op placeholder
}

#[cfg(test)]
mod tests {
    #[test]
    fn categories_present() {
        assert_eq!(super::CATEGORIES.len(), 9);
        super::start(false, None);
    }
}
