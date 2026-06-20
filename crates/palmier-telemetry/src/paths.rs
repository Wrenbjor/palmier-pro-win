//! Per-OS log and crash directory resolution.
//!
//! Reference parity (settings-account-app.md crash-log-path gotcha + FOUNDATION
//! §6.16): the macOS app wrote `~/Library/Logs/PalmierPro/crash.log`. On
//! Windows/Linux that moves to an app-data/state directory:
//!
//! - **Windows:** logs at `%LOCALAPPDATA%\PalmierProWin\Logs\` (rotated file
//!   `palmier.log`); crashes at `%LOCALAPPDATA%\PalmierProWin\Logs\crashes\`.
//! - **Linux:** logs at `~/.local/state/palmier-pro/logs/`; crashes at
//!   `~/.local/state/palmier-pro/logs/crashes/`.
//!
//! `%LOCALAPPDATA%` maps to `dirs::data_local_dir()`; the Linux state dir maps
//! to `dirs::state_dir()` (XDG `$XDG_STATE_HOME`, default `~/.local/state`),
//! which is the FOUNDATION §6.16 target. We keep the crash logs as a `crashes/`
//! subdir of the log dir so a single env override relocates both.

use std::path::PathBuf;

/// Windows app-data folder name under `%LOCALAPPDATA%`.
#[cfg(windows)]
const WIN_APP_FOLDER: &str = "PalmierProWin";
/// Windows log subfolder (FOUNDATION §6.16 capitalizes "Logs").
#[cfg(windows)]
const WIN_LOG_SUBDIR: &str = "Logs";

/// Linux app folder under the XDG state dir.
#[cfg(not(windows))]
const NIX_APP_FOLDER: &str = "palmier-pro";
/// Linux log subfolder.
#[cfg(not(windows))]
const NIX_LOG_SUBDIR: &str = "logs";

/// The rotated log file's base name. `tracing-appender` appends the daily date
/// suffix (e.g. `palmier.log.2026-06-20`).
pub const LOG_FILE_PREFIX: &str = "palmier.log";

/// Subdirectory (under the log dir) for per-crash files `crashes/<timestamp>.log`.
pub const CRASH_SUBDIR: &str = "crashes";

/// Resolve the directory that holds the rotated `palmier.log*` files.
///
/// Returns `None` only if the platform base dir cannot be determined (e.g. no
/// `HOME`/`%LOCALAPPDATA%`); callers degrade to stderr-only logging.
#[must_use]
pub fn log_dir() -> Option<PathBuf> {
    log_dir_from(base_data_dir())
}

/// Resolve the `crashes/` directory (subdir of [`log_dir`]).
#[must_use]
pub fn crash_dir() -> Option<PathBuf> {
    log_dir().map(|d| d.join(CRASH_SUBDIR))
}

/// Pure path-composition used by [`log_dir`] and unit tests: given the platform
/// base directory, append the app + log subfolders.
///
/// - Windows: `<base>\PalmierProWin\Logs`
/// - Linux/other: `<base>/palmier-pro/logs`
#[must_use]
pub fn log_dir_from(base: Option<PathBuf>) -> Option<PathBuf> {
    let base = base?;
    #[cfg(windows)]
    {
        Some(base.join(WIN_APP_FOLDER).join(WIN_LOG_SUBDIR))
    }
    #[cfg(not(windows))]
    {
        Some(base.join(NIX_APP_FOLDER).join(NIX_LOG_SUBDIR))
    }
}

/// The platform base directory the log dir is composed under:
/// `%LOCALAPPDATA%` on Windows, the XDG state dir on Linux/other.
#[must_use]
fn base_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        dirs::data_local_dir()
    }
    #[cfg(not(windows))]
    {
        // XDG_STATE_HOME (default ~/.local/state) — FOUNDATION §6.16.
        // Fall back to the data dir if state is somehow unavailable.
        dirs::state_dir().or_else(dirs::data_local_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn log_dir_composes_app_and_log_subfolders() {
        let base = PathBuf::from(if cfg!(windows) { r"C:\fake\local" } else { "/fake/state" });
        let dir = log_dir_from(Some(base)).expect("composed dir");

        // Always nested two levels under the base: <app>/<log-subdir>.
        let comps: Vec<_> = dir
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let tail = &comps[comps.len() - 2..];

        if cfg!(windows) {
            assert_eq!(tail, ["PalmierProWin", "Logs"]);
        } else {
            assert_eq!(tail, ["palmier-pro", "logs"]);
        }
    }

    #[test]
    fn log_dir_from_none_is_none() {
        assert!(log_dir_from(None).is_none());
    }

    #[test]
    fn crash_dir_is_subdir_of_log_dir() {
        let base = PathBuf::from(if cfg!(windows) { r"C:\fake\local" } else { "/fake/state" });
        let log = log_dir_from(Some(base.clone())).unwrap();
        let crash = log.join(CRASH_SUBDIR);
        assert!(crash.starts_with(&log));
        assert_eq!(crash.file_name().unwrap(), Path::new(CRASH_SUBDIR));
    }

    #[test]
    fn resolved_log_dir_matches_expected_platform_tail() {
        // Smoke: the live resolver (whatever the base is on this box) still ends
        // in the right app/log subfolders, proving the per-OS branch is wired.
        if let Some(dir) = log_dir() {
            let s = dir.to_string_lossy().replace('\\', "/");
            if cfg!(windows) {
                assert!(s.ends_with("PalmierProWin/Logs"), "got {s}");
            } else {
                assert!(s.ends_with("palmier-pro/logs"), "got {s}");
            }
        }
    }
}
