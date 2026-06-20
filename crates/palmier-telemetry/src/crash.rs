//! Panic / crash capture.
//!
//! Reference (settings-account-app.md "Telemetry/logging" + crash-log-path
//! gotcha): macOS installed `NSSetUncaughtExceptionHandler` + POSIX signal
//! handlers writing `~/Library/Logs/PalmierPro/crash.log`. On Windows/Linux the
//! Rust analog is a **panic hook** that writes a per-crash file
//! `crashes/<timestamp>.log` under the log dir (FOUNDATION §6.16) and forwards
//! the panic to Sentry.
//!
//! ## On "async-signal-safe"
//! A panic hook runs in normal (unwinding) context, **not** an async-signal
//! handler, so it may safely allocate and do buffered I/O. Hard native crashes
//! (SIGSEGV/illegal instruction / access violation) are not Rust panics — those
//! are captured by **Sentry's native crash handler** (the `sentry` SDK installs
//! a `crashpad`/`breakpad`-class handler when its DSN is configured), which is
//! the async-signal-safe path. The per-crash file here keeps the panic record
//! local even when Sentry is disabled. The file write is deliberately minimal
//! (open + write + flush) so it stays robust mid-unwind.

use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Where crash files are written; captured at install time so the hook does not
/// re-resolve dirs while unwinding. `None` ⇒ filesystem capture is skipped
/// (Sentry/stderr still receive the panic).
static CRASH_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Install the panic hook. Chains: write the crash file, then delegate to the
/// previously-installed hook (Sentry's, when [`crate::init`] installed Sentry
/// first, so the panic is also forwarded upstream).
///
/// `crash_dir` is the resolved `crashes/` directory; pass `None` to skip the
/// filesystem record (e.g. when no app data dir is resolvable).
pub fn install(crash_dir: Option<PathBuf>) {
    // Best-effort directory creation up front (outside the panic path).
    if let Some(dir) = crash_dir.as_ref() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = CRASH_DIR.set(crash_dir);

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // 1. Local crash file (best-effort; never itself panics).
        if let Some(Some(dir)) = CRASH_DIR.get() {
            let _ = write_crash_file(dir, info);
        }
        // 2. Mirror to stderr for live debugging.
        eprintln!("[palmier-telemetry] panic: {}", render_panic(info));
        // 3. Forward to the prior hook (Sentry's panic integration, if installed).
        previous(info);
    }));
}

/// Compose the per-crash filename `<timestamp>.log` (UTC, filesystem-safe).
#[must_use]
pub fn crash_file_name(now: chrono::DateTime<chrono::Utc>) -> String {
    // Colons are illegal in Windows filenames → use a `-` separated stamp.
    format!("{}.log", now.format("%Y-%m-%dT%H-%M-%S%.3fZ"))
}

/// Render a panic into the text body written to the crash file / stderr.
#[must_use]
pub fn render_panic(info: &PanicHookInfo<'_>) -> String {
    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());

    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown location>".to_string());

    let backtrace = std::backtrace::Backtrace::force_capture();

    format!(
        "Palmier Pro panic\n\
         time: {}\n\
         location: {location}\n\
         message: {payload}\n\
         \nbacktrace:\n{backtrace}\n",
        chrono::Utc::now().to_rfc3339(),
    )
}

/// Write one crash file. Separated for testability (no panic context needed).
fn write_crash_file(dir: &Path, info: &PanicHookInfo<'_>) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(crash_file_name(chrono::Utc::now()));
    let mut f = std::fs::File::create(&path)?;
    f.write_all(render_panic(info).as_bytes())?;
    f.flush()?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn crash_filename_is_filesystem_safe() {
        let ts = chrono::Utc.with_ymd_and_hms(2026, 6, 20, 13, 5, 9).unwrap();
        let name = crash_file_name(ts);
        assert!(name.ends_with(".log"));
        // No characters illegal on Windows (`: \ / *` etc.).
        for bad in [':', '\\', '/', '*', '?', '"', '<', '>', '|'] {
            assert!(!name.contains(bad), "{name} contains {bad}");
        }
        assert!(name.starts_with("2026-06-20T13-05-09"));
    }

    #[test]
    fn write_crash_file_creates_file_with_message() {
        let tmp = std::env::temp_dir().join(format!("pt-crash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);

        // Drive write_crash_file through a real (caught) panic so we have a
        // genuine PanicHookInfo, set up via a scoped hook.
        let dir = tmp.join("crashes");
        let captured: std::sync::Arc<std::sync::Mutex<Option<PathBuf>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let cap2 = captured.clone();
        let dir2 = dir.clone();

        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let p = write_crash_file(&dir2, info).expect("write crash file");
            *cap2.lock().unwrap() = Some(p);
        }));
        let _ = std::panic::catch_unwind(|| panic!("boom-test-message"));
        std::panic::set_hook(prev);

        let path = captured.lock().unwrap().clone().expect("crash path captured");
        let body = std::fs::read_to_string(&path).expect("read crash file");
        assert!(body.contains("boom-test-message"), "body: {body}");
        assert!(body.contains("location:"));
        assert!(body.contains("backtrace:"));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
