//! App settings persistence (FOUNDATION §6.1 boot step 3; E1-S1).
//!
//! Settings live at `%APPDATA%\PalmierProWin\settings.json` (Windows) /
//! `~/.config/palmier-pro/settings.json` (Linux) — **not** a macOS preference
//! domain (settings-account-app.md "Settings persistence is UserDefaults-backed
//! booleans" gotcha). The three `*_enabled` booleans use **absent ⇒ ON**
//! semantics (ruling #6): when the key is missing from the JSON, the value
//! defaults to `true`. `has_seen_welcome` defaults to `false`.
//!
//! The JSON keys mirror the reference pref keys
//! `io.palmier.pro.{notifications,telemetry,mcp}.enabled` and `has_seen_welcome`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// User settings, persisted to `settings.json`.
///
/// Field-level `#[serde(default)]` gives the **absent ⇒ value** semantics so a
/// fresh install (no file, or a partial file) lands on the reference defaults:
/// the three `*_enabled` flags default ON, `has_seen_welcome` defaults off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// `io.palmier.pro.mcp.enabled` — absent ⇒ ON (ruling #6).
    #[serde(rename = "io.palmier.pro.mcp.enabled", default = "default_true")]
    pub mcp_enabled: bool,

    /// `io.palmier.pro.notifications.enabled` — absent ⇒ ON (ruling #6).
    #[serde(
        rename = "io.palmier.pro.notifications.enabled",
        default = "default_true"
    )]
    pub notifications_enabled: bool,

    /// `io.palmier.pro.telemetry.enabled` — absent ⇒ ON (ruling #6).
    #[serde(rename = "io.palmier.pro.telemetry.enabled", default = "default_true")]
    pub telemetry_enabled: bool,

    /// Welcome-overlay dismissal flag — absent ⇒ false.
    #[serde(rename = "has_seen_welcome", default)]
    pub has_seen_welcome: bool,
}

impl Default for Settings {
    fn default() -> Self {
        // A fresh install with no file == an empty JSON object, which under the
        // per-field defaults yields exactly this.
        Self {
            mcp_enabled: true,
            notifications_enabled: true,
            telemetry_enabled: true,
            has_seen_welcome: false,
        }
    }
}

impl Settings {
    /// Parse settings from a JSON string. An empty object (`{}`) — or any object
    /// missing some keys — fills the gaps with the reference defaults.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Serialize to pretty JSON for persistence.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Persist to `path` **atomically** (write a sibling temp file, then rename over the
    /// target) so a crash mid-write never leaves a truncated `settings.json` (E1-S9: the
    /// General-tab toggles call this). Creates the parent dir if missing.
    pub fn write_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = self
            .to_json()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // Temp file in the same dir so the rename is atomic (same filesystem).
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, path)?;
        tracing::debug!(target: "app", path = %path.display(), "settings.json written");
        Ok(())
    }

    /// Read settings from `path`. A missing file ⇒ defaults (fresh install). A
    /// malformed file is logged and also degrades to defaults rather than
    /// blocking boot (FR-1: boot must always reach Home).
    pub fn read_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => Self::from_json(&contents).unwrap_or_else(|err| {
                tracing::warn!(
                    target: "app",
                    error = %err,
                    path = %path.display(),
                    "settings.json malformed; using defaults"
                );
                Settings::default()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(
                    target: "app",
                    path = %path.display(),
                    "no settings.json; using defaults (fresh install)"
                );
                Settings::default()
            }
            Err(err) => {
                tracing::warn!(
                    target: "app",
                    error = %err,
                    path = %path.display(),
                    "could not read settings.json; using defaults"
                );
                Settings::default()
            }
        }
    }
}

/// Resolve the app settings directory:
/// `%APPDATA%\PalmierProWin\` (Windows) / `~/.config/palmier-pro/` (Linux).
///
/// Uses `dirs::config_dir()` which maps to `%APPDATA%` (Roaming) on Windows and
/// `$XDG_CONFIG_HOME` (`~/.config`) on Linux — matching FOUNDATION §6.1 exactly.
pub fn settings_dir() -> Option<PathBuf> {
    let base = dirs::config_dir()?;
    // Windows AppData → "PalmierProWin"; Linux ~/.config → "palmier-pro".
    #[cfg(windows)]
    let dir = base.join("PalmierProWin");
    #[cfg(not(windows))]
    let dir = base.join("palmier-pro");
    Some(dir)
}

/// Full path to `settings.json` within the app settings dir.
pub fn settings_path() -> Option<PathBuf> {
    settings_dir().map(|d| d.join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_install_defaults_are_on() {
        // No file ⇒ default(): the three *_enabled flags ON, welcome unseen.
        let s = Settings::default();
        assert!(s.mcp_enabled);
        assert!(s.notifications_enabled);
        assert!(s.telemetry_enabled);
        assert!(!s.has_seen_welcome);
    }

    #[test]
    fn empty_object_yields_defaults_absent_is_on() {
        // An empty settings.json (`{}`) must apply absent ⇒ ON (ruling #6).
        let s = Settings::from_json("{}").expect("empty object parses");
        assert_eq!(s, Settings::default());
        assert!(s.mcp_enabled && s.notifications_enabled && s.telemetry_enabled);
        assert!(!s.has_seen_welcome);
    }

    #[test]
    fn explicit_false_overrides_absent_on() {
        let json = r#"{ "io.palmier.pro.telemetry.enabled": false }"#;
        let s = Settings::from_json(json).expect("parses");
        // Explicit false respected...
        assert!(!s.telemetry_enabled);
        // ...while the absent siblings still default ON.
        assert!(s.mcp_enabled);
        assert!(s.notifications_enabled);
    }

    #[test]
    fn has_seen_welcome_round_trips() {
        let json = r#"{ "has_seen_welcome": true }"#;
        let s = Settings::from_json(json).expect("parses");
        assert!(s.has_seen_welcome);
        // Round-trip through JSON preserves all fields.
        let back = Settings::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn json_uses_reference_pref_keys() {
        let s = Settings::default();
        let json = s.to_json().unwrap();
        assert!(json.contains("io.palmier.pro.mcp.enabled"));
        assert!(json.contains("io.palmier.pro.notifications.enabled"));
        assert!(json.contains("io.palmier.pro.telemetry.enabled"));
        assert!(json.contains("has_seen_welcome"));
    }

    #[test]
    fn settings_path_ends_with_expected_segments() {
        if let Some(p) = settings_path() {
            let s = p.to_string_lossy().replace('\\', "/");
            assert!(s.ends_with("settings.json"));
            #[cfg(windows)]
            assert!(s.contains("PalmierProWin"));
            #[cfg(not(windows))]
            assert!(s.contains("palmier-pro"));
        }
    }

    #[test]
    fn read_from_missing_file_is_defaults() {
        let p = std::path::Path::new(
            "this/path/definitely/does/not/exist/settings.json",
        );
        assert_eq!(Settings::read_from(p), Settings::default());
    }

    #[test]
    fn write_then_read_round_trips_atomically() {
        // E1-S9: a General-tab toggle persists via write_to; read_from must see it.
        let dir = std::env::temp_dir().join(format!(
            "palmier-settings-test-{}",
            std::process::id()
        ));
        let path = dir.join("settings.json");
        let mut s = Settings::default();
        s.notifications_enabled = false;
        s.telemetry_enabled = false;
        s.has_seen_welcome = true;
        s.write_to(&path).expect("atomic write succeeds");

        let back = Settings::read_from(&path);
        assert_eq!(back, s);
        // No stale temp file left behind by the atomic rename.
        assert!(!path.with_extension("json.tmp").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
