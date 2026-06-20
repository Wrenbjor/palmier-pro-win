//! Tauri commands backing the Home / Settings / Help / Feedback UI (E1-S4 + E1-S9).
//!
//! These are the seam the React surfaces (`src-ui/{home,settings}`) call via
//! `@tauri-apps/api`'s `invoke`. They read the **booted prefs from managed state** (the
//! launch-time `Settings` snapshot + the `palmier_auth::Auth` handle), toggle the
//! General-tab prefs (persisting atomically), read the account/credit state, manage the
//! Anthropic key, and report the MCP liveness stub.
//!
//! Reference mapping (settings-account-app.md "Settings tabs"):
//! - General → Privacy toggle ⇒ `io.palmier.pro.telemetry.enabled` + "Restart to apply"
//!   when it differs from the launch snapshot (telemetry is launch-snapshotted).
//! - General → Notifications toggle ⇒ `io.palmier.pro.notifications.enabled`.
//! - Agent tab ⇒ Anthropic key save/delete (keyring `anthropic-api-key`) + MCP status.
//! - Account tab ⇒ signed-in/out + tier + credits (hidden when `is_misconfigured`).
//!
//! All state mutation goes through [`SettingsState`] (a `Mutex<Settings>` in managed
//! state) so the live, in-session pref values stay consistent across windows.

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::settings::{self, Settings};
use crate::window::{self, WindowKind};
use crate::AppSettings;

/// Live (mutable) settings, behind a `Mutex`, in Tauri managed state. Seeded from the
/// boot-time snapshot; the General-tab toggles mutate + persist it.
pub struct SettingsState(pub Mutex<Settings>);

/// The bundle of booted prefs + telemetry-launch state the Settings UI reads on mount.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    /// `io.palmier.pro.notifications.enabled` (absent ⇒ ON).
    pub notifications_enabled: bool,
    /// `io.palmier.pro.telemetry.enabled` (absent ⇒ ON) — the **live** pref value.
    pub telemetry_enabled: bool,
    /// `io.palmier.pro.mcp.enabled` (absent ⇒ ON).
    pub mcp_enabled: bool,
    /// Welcome-overlay dismissal flag.
    pub has_seen_welcome: bool,
    /// Telemetry value snapshotted at launch (restart required to change effect). The
    /// Privacy pane shows "Restart Palmier Pro to apply" when `telemetry_enabled` differs
    /// from this.
    pub telemetry_enabled_for_launch: bool,
}

/// Account/credit state for the Account tab (reference `AccountService` derived getters).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSnapshot {
    /// Account tab hidden when true (Clerk/Convex not configured).
    pub is_misconfigured: bool,
    pub is_loading: bool,
    pub is_signed_in: bool,
    pub ai_allowed: bool,
    /// "none" / "pro" / "max".
    pub tier: String,
    pub plan_label: String,
    pub remaining_credits: i64,
    /// `None` until an account snapshot exists (reference returns nil pre-account).
    pub budget_credits: Option<i64>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub last_error: Option<String>,
    /// Top-off (buy-more-credits) limits for the `TopOffField` ($5–$1000, default $20).
    pub top_off_min: i64,
    pub top_off_max: i64,
    pub top_off_default: i64,
}

/// MCP server liveness for the Agent-tab status row. The real server is Epic 7; this is
/// the liveness **stub** (settings-account-app.md gotcha: the toggle reflects *liveness*,
/// not just the pref — here liveness == the booted start-result while the real server
/// doesn't exist yet).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    /// Whether MCP is enabled by pref.
    pub enabled: bool,
    /// Whether the (stub) server is "running" — green dot vs grey.
    pub running: bool,
    /// `127.0.0.1:19789` (reference endpoint).
    pub bind: String,
}

/// Resolve the live `Settings`, applying any in-session toggles.
fn live_settings<R: Runtime>(app: &AppHandle<R>) -> Settings {
    if let Some(state) = app.try_state::<SettingsState>() {
        return state.0.lock().expect("settings mutex").clone();
    }
    // Fall back to the boot snapshot if the live state isn't registered (tests).
    app.try_state::<AppSettings>()
        .map(|s| s.0.clone())
        .unwrap_or_default()
}

/// Persist the live settings to `settings.json` (atomic). Logged-but-non-fatal on error
/// so a read-only disk never breaks the toggle UX.
fn persist(settings: &Settings) {
    if let Some(path) = settings::settings_path() {
        if let Err(err) = settings.write_to(&path) {
            tracing::warn!(target: "app", error = %err, "failed to persist settings.json");
        }
    }
}

// ─── settings reads ────────────────────────────────────────────────────────────────

/// Read the booted prefs + telemetry-launch state (Settings UI on mount).
#[tauri::command]
pub fn get_settings(app: AppHandle, telemetry: State<'_, palmier_telemetry::TelemetryHandle>) -> SettingsSnapshot {
    let s = live_settings(&app);
    SettingsSnapshot {
        notifications_enabled: s.notifications_enabled,
        telemetry_enabled: s.telemetry_enabled,
        mcp_enabled: s.mcp_enabled,
        has_seen_welcome: s.has_seen_welcome,
        telemetry_enabled_for_launch: telemetry.enabled_for_current_launch(),
    }
}

/// Toggle `io.palmier.pro.notifications.enabled` (General → Notifications).
#[tauri::command]
pub fn set_notifications_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let state = app
        .try_state::<SettingsState>()
        .ok_or("settings state unavailable")?;
    let snapshot = {
        let mut s = state.0.lock().map_err(|e| e.to_string())?;
        s.notifications_enabled = enabled;
        s.clone()
    };
    persist(&snapshot);
    tracing::info!(target: "app", enabled, "notifications pref toggled");
    Ok(())
}

/// Toggle `io.palmier.pro.telemetry.enabled` (General → Privacy). Restart-required: the
/// effect is snapshotted at launch, so this only updates the stored pref (the UI shows
/// "Restart to apply" by comparing against `telemetry_enabled_for_launch`).
#[tauri::command]
pub fn set_telemetry_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let state = app
        .try_state::<SettingsState>()
        .ok_or("settings state unavailable")?;
    let snapshot = {
        let mut s = state.0.lock().map_err(|e| e.to_string())?;
        s.telemetry_enabled = enabled;
        s.clone()
    };
    persist(&snapshot);
    tracing::info!(target: "app", enabled, "telemetry pref toggled (restart required to apply)");
    Ok(())
}

/// Toggle `io.palmier.pro.mcp.enabled` (Agent tab MCP toggle → `set_mcp_enabled`).
#[tauri::command]
pub fn set_mcp_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    let state = app
        .try_state::<SettingsState>()
        .ok_or("settings state unavailable")?;
    let snapshot = {
        let mut s = state.0.lock().map_err(|e| e.to_string())?;
        s.mcp_enabled = enabled;
        s.clone()
    };
    persist(&snapshot);
    tracing::info!(target: "app", enabled, "mcp pref toggled");
    Ok(())
}

/// Persist `has_seen_welcome = true` (Home welcome-overlay dismissal, FR-1).
#[tauri::command]
pub fn dismiss_welcome(app: AppHandle) -> Result<(), String> {
    let state = app
        .try_state::<SettingsState>()
        .ok_or("settings state unavailable")?;
    let snapshot = {
        let mut s = state.0.lock().map_err(|e| e.to_string())?;
        s.has_seen_welcome = true;
        s.clone()
    };
    persist(&snapshot);
    Ok(())
}

// ─── account / agent reads ─────────────────────────────────────────────────────────

/// Read the account/credit snapshot for the Account tab.
#[tauri::command]
pub fn get_account(auth: State<'_, palmier_auth::Auth>) -> AccountSnapshot {
    let acct = auth.account();
    let user = acct.account().map(|a| &a.user);
    let tier = acct.tier();
    AccountSnapshot {
        is_misconfigured: acct.is_misconfigured(),
        is_loading: acct.is_loading(),
        is_signed_in: acct.is_signed_in(),
        ai_allowed: acct.ai_allowed(),
        tier: match tier {
            palmier_auth::AccountTier::None => "none",
            palmier_auth::AccountTier::Pro => "pro",
            palmier_auth::AccountTier::Max => "max",
        }
        .to_string(),
        plan_label: tier.plan_label().to_string(),
        remaining_credits: acct.remaining_credits(),
        budget_credits: acct.budget_credits(),
        email: user.and_then(|u| u.email.clone()),
        name: user.and_then(|u| u.name.clone()),
        last_error: acct.last_error().map(str::to_owned),
        top_off_min: palmier_auth::top_off_limits::MIN_DOLLARS,
        top_off_max: palmier_auth::top_off_limits::MAX_DOLLARS,
        top_off_default: palmier_auth::top_off_limits::DEFAULT_DOLLARS,
    }
}

/// Whether a non-empty Anthropic key is stored (Agent tab: show masked vs placeholder).
#[tauri::command]
pub fn has_anthropic_key(auth: State<'_, palmier_auth::Auth>) -> bool {
    auth.anthropic_key().has_key().unwrap_or(false)
}

/// Save the Anthropic key (Agent tab Save). Persists via the keyring (account
/// `anthropic-api-key`, ruling #5) and emits `anthropic-api-key-changed`.
#[tauri::command]
pub fn save_anthropic_key(
    app: AppHandle,
    auth: State<'_, palmier_auth::Auth>,
    key: String,
) -> Result<(), String> {
    auth.anthropic_key().save(&key).map_err(|e| e.to_string())?;
    let _ = app.emit("anthropic-api-key-changed", ());
    tracing::info!(target: "app", "anthropic key saved");
    Ok(())
}

/// Delete the Anthropic key (Agent tab trash). Emits `anthropic-api-key-changed`.
#[tauri::command]
pub fn delete_anthropic_key(
    app: AppHandle,
    auth: State<'_, palmier_auth::Auth>,
) -> Result<(), String> {
    auth.anthropic_key().delete().map_err(|e| e.to_string())?;
    let _ = app.emit("anthropic-api-key-changed", ());
    tracing::info!(target: "app", "anthropic key deleted");
    Ok(())
}

/// MCP server liveness for the Agent-tab status row (stub; real server is Epic 7).
#[tauri::command]
pub fn get_mcp_status(app: AppHandle) -> McpStatus {
    let enabled = live_settings(&app).mcp_enabled;
    McpStatus {
        enabled,
        // Liveness stub: with no real server yet, "running" tracks the enabled pref. When
        // Epic 7 lands palmier-mcp this reads the actual server state (a failed bind ⇒
        // enabled-but-not-running), matching the reference `mcpService?.isRunning`.
        running: enabled,
        bind: palmier_mcp::DEFAULT_BIND.to_string(),
    }
}

// ─── window opens (E1-S4) — invoked by the menu router + Home/Settings UI ───────────

/// Open (or focus) the Settings window.
#[tauri::command]
pub fn open_settings(app: AppHandle) -> Result<(), String> {
    window::open_or_focus(&app, WindowKind::Settings).map_err(|e| e.to_string())
}

/// Open (or focus) the Help window. `tab` optionally selects "shortcuts" or "mcp".
#[tauri::command]
pub fn open_help(app: AppHandle) -> Result<(), String> {
    window::open_or_focus(&app, WindowKind::Help).map_err(|e| e.to_string())
}

/// Open (or focus) the Feedback window.
#[tauri::command]
pub fn open_feedback(app: AppHandle) -> Result<(), String> {
    window::open_or_focus(&app, WindowKind::Feedback).map_err(|e| e.to_string())
}

/// Open a Project window for `project_id` (one per project, FR-2).
#[tauri::command]
pub fn open_project(app: AppHandle, project_id: String) -> Result<(), String> {
    window::open_project_window(&app, &project_id).map_err(|e| e.to_string())
}

/// Return to Home (reference `showHome`). The autosave-on-home of the active project is
/// E1-S7's lifecycle; here we just surface Home.
#[tauri::command]
pub fn show_home(app: AppHandle) -> Result<(), String> {
    window::show_home(&app).map_err(|e| e.to_string())
}

/// Re-check for updates now (Settings/menu "Check for Updates"). Delegates to the
/// E1-S10 updater glue.
#[tauri::command]
pub fn check_for_updates(app: AppHandle) {
    crate::update::check_now(&app);
}

/// Send feedback (Feedback dialog → reference `feedback:send`). Routes through the
/// configured Convex backend (E1-S6). Degrades gracefully: when Convex is unreachable /
/// unconfigured it returns an error string the dialog surfaces, never panicking
/// (OQ-9 / R-4).
#[tauri::command]
pub fn send_feedback(
    auth: State<'_, palmier_auth::Auth>,
    message: String,
    may_contact: bool,
    email: Option<String>,
    screenshot_png_base64: Option<String>,
) -> Result<(), String> {
    let Some(convex) = auth.convex() else {
        return Err("Feedback is unavailable: backend not configured.".to_string());
    };
    let req = palmier_auth::FeedbackRequest {
        message,
        may_contact,
        email,
        screenshot_png_base64,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os_version: std::env::consts::OS.to_string(),
    };
    let jwt = auth.token().jwt();
    convex.send_feedback(jwt, &req).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_snapshot_serializes_camel_case() {
        let snap = SettingsSnapshot {
            notifications_enabled: true,
            telemetry_enabled: false,
            mcp_enabled: true,
            has_seen_welcome: false,
            telemetry_enabled_for_launch: true,
        };
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"notificationsEnabled\":true"));
        assert!(json.contains("\"telemetryEnabled\":false"));
        assert!(json.contains("\"telemetryEnabledForLaunch\":true"));
    }

    #[test]
    fn mcp_status_reports_reference_bind() {
        let s = McpStatus {
            enabled: true,
            running: true,
            bind: palmier_mcp::DEFAULT_BIND.to_string(),
        };
        assert_eq!(s.bind, "127.0.0.1:19789");
    }

    #[test]
    fn account_snapshot_default_tier_serializes() {
        let snap = AccountSnapshot {
            is_misconfigured: true,
            is_loading: false,
            is_signed_in: false,
            ai_allowed: false,
            tier: "none".to_string(),
            plan_label: "Free".to_string(),
            remaining_credits: 0,
            budget_credits: None,
            email: None,
            name: None,
            last_error: None,
            top_off_min: 5,
            top_off_max: 1000,
            top_off_default: 20,
        };
        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains("\"isMisconfigured\":true"));
        assert!(json.contains("\"topOffDefault\":20"));
    }
}
