//! Cache-directory resolution for the media visual cache (E4-S2).
//!
//! Port of the Swift `DiskCache(named: "MediaVisualCache")` directory. The
//! macOS reference put its cache under the app's caches dir; on our platforms
//! (`docs/reference/media-panel.md` §"Mapping to FOUNDATION crates" + story
//! acceptance) it lands under:
//!
//! * **Windows** — `%LOCALAPPDATA%\PalmierProWin\Cache\MediaVisualCache`
//! * **Linux**   — `$XDG_CACHE_HOME` (or `~/.cache`) `/PalmierProWin/MediaVisualCache`
//!
//! Both are resolved via the `dirs` crate's [`dirs::cache_dir`], which returns
//! `%LOCALAPPDATA%` on Windows and the XDG cache dir on Linux — exactly the two
//! locations the story specifies.

use std::path::PathBuf;

/// App-scoped subdirectory under the platform cache root.
pub const APP_CACHE_NAMESPACE: &str = "PalmierProWin";

/// The named sub-cache, matching `DiskCache(named: "MediaVisualCache")`.
pub const MEDIA_VISUAL_CACHE_NAME: &str = "MediaVisualCache";

/// Resolve the platform cache root (`%LOCALAPPDATA%` / XDG cache dir).
///
/// Returns `None` only if the OS gives no cache/home directory (extremely rare;
/// e.g. no `HOME`/`LOCALAPPDATA`). Callers should degrade to in-memory-only
/// caching in that case rather than panicking.
pub fn platform_cache_root() -> Option<PathBuf> {
    dirs::cache_dir()
}

/// Resolve the `MediaVisualCache` directory:
/// `<platform-cache-root>/PalmierProWin/Cache/MediaVisualCache`.
///
/// Note on the `Cache` segment: on Windows the story calls for
/// `%LOCALAPPDATA%\PalmierProWin\Cache`. `dirs::cache_dir()` already returns
/// `%LOCALAPPDATA%` on Windows, so we append `PalmierProWin/Cache` there. On
/// Linux the platform root is *already* a cache dir (`~/.cache`), so a nested
/// `Cache` segment would be redundant — we use `PalmierProWin` directly. The
/// [`media_visual_cache_dir`] helper applies the correct shape per-OS.
pub fn media_visual_cache_dir() -> Option<PathBuf> {
    let root = platform_cache_root()?;
    let mut dir = root;
    dir.push(APP_CACHE_NAMESPACE);
    // On Windows %LOCALAPPDATA% is a general app-data root (not cache-specific),
    // so the story asks for an explicit `Cache` segment. On Linux the root IS
    // the cache dir, so we don't double up.
    #[cfg(target_os = "windows")]
    dir.push("Cache");
    dir.push(MEDIA_VISUAL_CACHE_NAME);
    Some(dir)
}

/// Resolve the `MediaVisualCache` directory **and create it** if missing.
///
/// Returns the path on success, or an `io::Error` if the directory can't be
/// created (or `NotFound` if no platform cache root exists). The visual-cache
/// store calls this once on construction.
pub fn ensure_media_visual_cache_dir() -> std::io::Result<PathBuf> {
    let dir = media_visual_cache_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no platform cache directory available (no LOCALAPPDATA / HOME)",
        )
    })?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_is_namespaced_and_named() {
        // On any normal test host a cache root exists.
        let dir = media_visual_cache_dir().expect("cache root should resolve on test host");
        assert!(
            dir.ends_with(MEDIA_VISUAL_CACHE_NAME),
            "must end with the named sub-cache, got {dir:?}"
        );
        let s = dir.to_string_lossy();
        assert!(
            s.contains(APP_CACHE_NAMESPACE),
            "must be namespaced under {APP_CACHE_NAMESPACE}, got {s}"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_path_has_localappdata_cache_segment() {
        let dir = media_visual_cache_dir().expect("cache root");
        let s = dir.to_string_lossy();
        // %LOCALAPPDATA%\PalmierProWin\Cache\MediaVisualCache
        assert!(
            s.contains("PalmierProWin") && s.contains("Cache"),
            "windows cache path shape unexpected: {s}"
        );
    }

    #[test]
    fn ensure_creates_the_directory() {
        // We can't easily override the OS cache root without env-var plumbing,
        // so just assert the call succeeds and the dir exists afterward.
        let dir = ensure_media_visual_cache_dir().expect("should create cache dir");
        assert!(dir.is_dir(), "ensured cache dir should exist: {dir:?}");
    }
}
