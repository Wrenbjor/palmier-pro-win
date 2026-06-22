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
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use palmier_tools::ToolDispatch;

use crate::agent::AgentState;
use crate::settings::{self, Settings};
use crate::window::{self, WindowKind};
use crate::AppSettings;

/// The Tauri event emitted to every window after the shared `EditorState` changes
/// (an in-app/MCP edit or an `editor_edit` mutation). The Project surface listens
/// for this and refetches `editor_get_timeline` / `editor_get_media` so the panels
/// reflect the new state — the UI never polls. Emitted both here (UI edits) and
/// from the agent's tool-dispatch path (`agent.rs`) so AGENT edits update the UI.
pub const TIMELINE_CHANGED_EVENT: &str = "timeline://changed";

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

/// MCP server liveness for the Agent-tab status row. Now reads the **live** server
/// state (M2 boot integration): `running` is whether the real loopback server is
/// bound, NOT just the pref — so a failed bind shows enabled-but-not-running,
/// matching the reference `mcpService?.isRunning` (settings-account-app.md gotcha).
#[tauri::command]
pub fn get_mcp_status(app: AppHandle) -> McpStatus {
    let enabled = live_settings(&app).mcp_enabled;
    // Live liveness from the running server handle (M2 boot integration). Falls back
    // to the pref echo only if the AgentState isn't managed (tests / early boot).
    let (running, bind) = match app.try_state::<crate::agent::AgentState>() {
        Some(agent) => (
            agent.mcp_running(),
            agent
                .mcp_bind()
                .unwrap_or_else(|| palmier_mcp::DEFAULT_BIND.to_string()),
        ),
        None => (enabled, palmier_mcp::DEFAULT_BIND.to_string()),
    };
    McpStatus {
        enabled,
        running,
        bind,
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

// `open_project` + `show_home` moved to `project.rs` (E1-S7): they now run the real
// registry-backed open + autosave-on-switch lifecycle instead of a window-only stub.

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

// ─── editor read / edit bridge (Project window) ──────────────────────────────────
//
// The Project surface reads the timeline + media library and dispatches mutating
// tools through the ONE shared `Arc<ToolExecutor>` (the same owner the MCP server +
// in-app agent drive — `agent.rs`). Reads run the real `palmier_tools::read` bodies;
// edits run any of the 30 tools via `executor.execute`. After a successful mutation
// we emit `timeline://changed` so every window refetches (the UI never polls).

/// Pull the single JSON text block out of a [`palmier_tools::ToolResult`]. The READ
/// bodies (`get_timeline` / `get_media`) return exactly one `Block::Text` carrying a
/// JSON string; this parses it back to a `serde_json::Value` for the frontend. An
/// error result (or a non-text / unparseable block) surfaces as an `Err(String)`.
fn tool_result_to_json(result: palmier_tools::ToolResult) -> Result<Value, String> {
    let text = result
        .content
        .into_iter()
        .find_map(|block| match block {
            palmier_tools::Block::Text(s) => Some(s),
            palmier_tools::Block::Image { .. } => None,
        })
        .ok_or_else(|| "tool returned no text content".to_string())?;
    if result.is_error {
        return Err(text);
    }
    serde_json::from_str(&text).map_err(|e| format!("failed to parse tool JSON: {e}"))
}

/// `editor_get_timeline` — the shaped timeline JSON the Project window renders
/// (reference `get_timeline`). Reads the shared `EditorState` through the executor;
/// `args` carries the optional `{ startFrame?, endFrame? }` window (default: whole
/// timeline). Returns the parsed JSON object (`adapt.ts` maps it to a `TimelineView`).
///
/// After serializing the full-fidelity timeline, this injects each AUDIO clip's
/// per-source `waveform` peak array (cached per asset; cold misses generate in the
/// background and emit `timeline://changed` so the UI refetches — see
/// [`crate::waveform_cache`]). The renderer slices the full-source peaks to each
/// clip's trimmed window. The compact MCP `get_timeline` tool the LLM uses is
/// UNCHANGED and carries NO waveform (`read::get_timeline`).
#[tauri::command]
pub fn editor_get_timeline<R: Runtime>(
    app: AppHandle<R>,
    agent: State<'_, AgentState>,
    waveforms: State<'_, crate::waveform_cache::WaveformState>,
    args: Option<Value>,
) -> Result<Value, String> {
    use std::collections::HashMap;
    // The UI needs the FULL clip model (real volume / trim / speed / opacity /
    // fades / keyframes), not the defaults-stripped MCP `get_timeline` summary.
    // `args` (the optional `{startFrame,endFrame}` window) is intentionally ignored:
    // the editor renders the whole timeline and windows in the canvas.
    let _ = args;

    // Serialize the timeline + resolve the audio-bearing assets' paths/durations in
    // ONE state borrow, so the lock-free waveform lookup/inject runs after.
    let (value, assets) = agent.executor.with_state_ref(|state| {
        let value = palmier_tools::read::full_timeline_json(state);
        let mut assets: HashMap<String, crate::waveform_cache::AudioAsset> = HashMap::new();
        for asset in &state.library.assets {
            // Only AUDIO assets get a waveform (the renderer draws waveforms for
            // audio clips only; video clips show a thumbnail).
            if asset.asset_type != palmier_model::ClipType::Audio {
                continue;
            }
            if let Some(path) = crate::audio_build::asset_path(&asset.source) {
                assets.insert(
                    asset.id.clone(),
                    crate::waveform_cache::AudioAsset {
                        path,
                        duration: asset.duration_seconds,
                    },
                );
            }
        }
        (value, assets)
    });

    // Inject cached peaks; cold misses spawn one background decode per asset (deduped),
    // which emits `timeline://changed` on completion so the UI refetches.
    let value = crate::waveform_cache::inject_waveforms(&waveforms, &assets, value, |_, asset| {
        waveforms.spawn_generate(&app, asset.path.clone(), asset.duration);
    });
    Ok(value)
}

/// `editor_get_media` — the media-library JSON the Media panel renders (reference
/// `get_media`). Reads the shared `EditorState` through the executor and returns the
/// parsed `{ assets: [...] }` object (`media-panel/adapt.ts` maps it to a snapshot).
#[tauri::command]
pub fn editor_get_media(agent: State<'_, AgentState>) -> Result<Value, String> {
    let result = agent
        .executor
        .with_state_ref(palmier_tools::read::get_media);
    tool_result_to_json(result)
}

/// `editor_edit` — dispatch any mutating tool (`add_clips`, `move_clips`,
/// `split_clip`, `remove_clips`, `set_clip_properties`, `ripple_delete_ranges`,
/// `undo`, `import_media`, `create_folder`, …) through the SHARED executor — the
/// SAME `Arc<ToolExecutor>` the MCP server + in-app agent use, so UI edits land on
/// one timeline / one undo timeline (FOUNDATION §4, PRD §10).
///
/// `name` is the tool wire name, `args` its inputSchema-shaped arguments. Returns the
/// tool's result JSON (the READ-shaped payload some tools echo, else `{}`), or the
/// tool error string. On success emits `timeline://changed` to every window so the UI
/// refetches the new state.
#[tauri::command]
pub fn editor_edit<R: Runtime>(
    app: AppHandle<R>,
    agent: State<'_, AgentState>,
    name: String,
    args: Option<Value>,
) -> Result<Value, String> {
    let args = args.unwrap_or_else(|| Value::Object(Default::default()));
    // `execute` ignores its ctx arg (it re-snapshots its own IdUniverse), so reuse the
    // agent module's trivial adapter context.
    let result = agent
        .executor
        .execute(&name, args, &crate::agent::adapter_context());

    // Parse the result for the caller. A tool-error result becomes an `Err(String)`;
    // a non-JSON/empty text block (some mutators return prose) becomes `Value::Null`,
    // which the frontend treats as "no echo, refetch".
    let parsed = if result.is_error {
        let msg = result
            .content
            .into_iter()
            .find_map(|b| match b {
                palmier_tools::Block::Text(s) => Some(s),
                palmier_tools::Block::Image { .. } => None,
            })
            .unwrap_or_else(|| format!("tool {name} failed"));
        return Err(msg);
    } else {
        let text: Option<String> = result.content.into_iter().find_map(|b| match b {
            palmier_tools::Block::Text(s) => Some(s),
            palmier_tools::Block::Image { .. } => None,
        });
        match text {
            Some(s) => serde_json::from_str::<Value>(&s).unwrap_or(Value::Null),
            None => Value::Null,
        }
    };

    // A successful mutation changed the shared EditorState → mark the active document
    // dirty so autosave/flush persists the LIVE executor state (timeline-persistence
    // fix; without this, edits were dropped on save→reopen) and notify every window so
    // the panels refetch. Logged-but-non-fatal on emit failure (the edit already landed).
    crate::project::mark_timeline_dirty(&app);
    if let Err(err) = app.emit(TIMELINE_CHANGED_EVENT, ()) {
        tracing::warn!(target: "app", error = %err, "failed to emit timeline://changed");
    }
    tracing::info!(target: "app", tool = %name, "editor_edit dispatched (shared executor)");
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    use palmier_tools::{ToolExecutor, ToolResult};
    use std::sync::Arc;

    #[test]
    fn tool_result_to_json_parses_text_block() {
        let result = ToolResult::ok("{\"fps\":30,\"tracks\":[]}".to_string());
        let json = tool_result_to_json(result).expect("should parse");
        assert_eq!(json.get("fps").and_then(Value::as_i64), Some(30));
    }

    #[test]
    fn tool_result_to_json_surfaces_error() {
        let result = ToolResult::error("bad args");
        let err = tool_result_to_json(result).unwrap_err();
        assert_eq!(err, "bad args");
    }

    /// The editor read/edit bridge runs over the SAME shared executor the MCP server +
    /// in-app agent use: a `get_timeline` read, an `add_clips`-style edit, then a read
    /// that observes it — proving `editor_get_timeline` / `editor_edit` operate on one
    /// `EditorState`. (Exercises the executor flow the command wrappers delegate to;
    /// the `#[tauri::command]` fns themselves need a live `State`, covered by smoke.)
    #[test]
    fn editor_bridge_reads_and_edits_one_shared_state() {
        let executor = Arc::new(ToolExecutor::new());

        // Seed a media asset so add_clips has something to place (mirrors a loaded lib).
        executor.with_state_mut(|state| {
            state.library.assets.push(palmier_model::MediaAsset::new(
                "asset-1",
                "clip.mp4",
                palmier_model::ClipType::Video,
                palmier_model::MediaSource::External {
                    absolute_path: "/clip.mp4".to_string(),
                },
                1.0,
            ));
        });

        // get_timeline read → parses to a JSON object with the injected fields.
        let read = executor.with_state_ref(|s| {
            palmier_tools::read::get_timeline(s, &Value::Object(Default::default()))
        });
        let before = tool_result_to_json(read).expect("read parses");
        assert!(before.get("totalFrames").is_some());
        let tracks_before = before
            .get("tracks")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);

        // add_clips via execute (the same path editor_edit uses) — auto-creates a track.
        let args = serde_json::json!({
            "entries": [{ "mediaRef": "asset-1", "startFrame": 0, "durationFrames": 30 }]
        });
        let edit = executor.execute("add_clips", args, &crate::agent::adapter_context());
        assert!(!edit.is_error, "add_clips should succeed: {:?}", edit.content);

        // A subsequent read observes the new track/clip on the SAME state.
        let read2 = executor.with_state_ref(|s| {
            palmier_tools::read::get_timeline(s, &Value::Object(Default::default()))
        });
        let after = tool_result_to_json(read2).expect("read parses");
        let tracks_after = after
            .get("tracks")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        assert!(tracks_after > tracks_before, "add_clips created a track");
    }

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
