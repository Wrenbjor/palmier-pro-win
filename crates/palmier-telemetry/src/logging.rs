//! `tracing-subscriber` setup: categorized targets, daily file rotation with
//! 7-day retention, stderr mirror, and (when Sentry is on) a Sentry layer that
//! maps events to breadcrumbs/captures.
//!
//! Reference (settings-account-app.md "Telemetry/logging" + FOUNDATION §6.16):
//! `os.Logger` categories `app/editor/export/preview/mcp/generation/project/
//! transcription/search`; `warning` ⇒ Sentry breadcrumb, `error`/`fault` ⇒
//! Sentry capture-message; all levels mirror to stderr. Logs are written to
//! `palmier.log` rotated **daily** and **7 days retained**.

use crate::CATEGORIES;
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

/// Number of rotated daily log files to retain (FOUNDATION §6.16).
pub const LOG_RETENTION_DAYS: usize = 7;

/// Outcome of [`init_logging`]: the worker guard (must be kept alive for the
/// process lifetime so the non-blocking writer flushes) and whether a file sink
/// was attached. The boot path stores the guard on a long-lived value.
#[must_use = "drop of the guard flushes and stops the log writer"]
pub struct LoggingGuard {
    _file_guard: Option<WorkerGuard>,
    pub file_logging: bool,
}

/// Build the `EnvFilter` for the subscriber.
///
/// Base level is `info` (`debug` when `debug` is true, reference "Level Info
/// default; Debug with --debug"), and every reference category target is
/// admitted at that level so category logs (`tracing::info!(target: "export", …)`)
/// are captured. `RUST_LOG`, if set, overrides everything.
#[must_use]
pub fn build_env_filter(debug: bool) -> EnvFilter {
    if let Ok(f) = EnvFilter::try_from_default_env() {
        return f;
    }
    let base = if debug { "debug" } else { "info" };
    let mut filter = EnvFilter::new(base);
    for cat in CATEGORIES {
        // e.g. `export=info` — explicit per-category directive so the target set
        // is provably wired even if the base default changes later.
        let directive = format!("{cat}={base}")
            .parse()
            .expect("valid tracing directive");
        filter = filter.add_directive(directive);
    }
    filter
}

/// Whether a target string is one of the reference categories.
#[must_use]
pub fn is_known_category(target: &str) -> bool {
    CATEGORIES.contains(&target)
}

/// Initialize the global tracing subscriber.
///
/// - `log_dir`: directory for the rotated `palmier.log`; `None` ⇒ stderr-only.
/// - `debug`: base level `debug` vs `info`.
/// - `with_sentry`: attach the Sentry tracing layer (breadcrumbs/captures).
///
/// Returns a [`LoggingGuard`] that must be kept alive for the process lifetime.
/// Safe to call once; a second call is a no-op returning a stderr-only guard
/// (the global default is already set).
pub fn init_logging(log_dir: Option<&Path>, debug: bool, with_sentry: bool) -> LoggingGuard {
    let fmt_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_ansi(false);

    // File layer with daily rotation + 7-day retention, if a dir is available.
    let (file_layer, file_guard, file_logging) = match log_dir {
        Some(dir) => match build_file_appender(dir) {
            Ok(appender) => {
                let (nb, guard) = tracing_appender::non_blocking(appender);
                let layer = fmt::layer()
                    .with_writer(nb)
                    .with_target(true)
                    .with_ansi(false);
                (Some(layer), Some(guard), true)
            }
            Err(e) => {
                eprintln!("[palmier-telemetry] file logging disabled: {e}");
                (None, None, false)
            }
        },
        None => (None, None, false),
    };

    let sentry_layer = if with_sentry {
        // Maps `warning` ⇒ breadcrumb, `error`+ ⇒ capture (reference mapping).
        Some(sentry::integrations::tracing::layer())
    } else {
        None
    };

    let registry = tracing_subscriber::registry()
        .with(build_env_filter(debug))
        .with(fmt_layer)
        .with(file_layer)
        .with(sentry_layer);

    // try_init: a second call (or a host that already set a subscriber) is a
    // non-fatal no-op rather than a panic.
    if registry.try_init().is_err() {
        return LoggingGuard {
            _file_guard: None,
            file_logging: false,
        };
    }

    LoggingGuard {
        _file_guard: file_guard,
        file_logging,
    }
}

/// Build the daily-rotating file appender with 7-day retention.
fn build_file_appender(dir: &Path) -> std::io::Result<RollingFileAppender> {
    std::fs::create_dir_all(dir)?;
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(crate::paths::LOG_FILE_PREFIX)
        .max_log_files(LOG_RETENTION_DAYS)
        .build(dir)
        .map_err(std::io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CATEGORIES;

    #[test]
    fn retention_is_seven_days() {
        assert_eq!(LOG_RETENTION_DAYS, 7);
    }

    #[test]
    fn env_filter_admits_every_category() {
        // Build succeeds (each directive parses) for both levels.
        let _ = build_env_filter(false);
        let _ = build_env_filter(true);
    }

    #[test]
    fn category_membership_is_exact() {
        for c in CATEGORIES {
            assert!(is_known_category(c), "{c} should be known");
        }
        assert!(!is_known_category("network"));
        assert!(!is_known_category("App")); // case-sensitive
        assert!(!is_known_category(""));
    }

    #[test]
    fn category_set_matches_reference() {
        // Reference target set (settings-account-app.md "Telemetry/logging").
        assert_eq!(
            CATEGORIES,
            &[
                "app",
                "editor",
                "export",
                "preview",
                "mcp",
                "generation",
                "project",
                "transcription",
                "search",
            ]
        );
    }

    #[test]
    fn file_appender_builds_and_writes() {
        let tmp = std::env::temp_dir().join(format!("pt-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let appender = build_file_appender(&tmp).expect("appender builds");
        // Writing through it creates today's rotated file.
        {
            use std::io::Write;
            let mut a = appender;
            a.write_all(b"hello\n").unwrap();
            a.flush().unwrap();
        }
        let count = std::fs::read_dir(&tmp).unwrap().count();
        assert!(count >= 1, "expected a rotated log file");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
