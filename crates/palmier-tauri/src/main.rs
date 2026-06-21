//! # palmier-tauri
//!
//! The Tauri 2 binary that wires every core crate together (FOUNDATION §4, §6.1).
//! Owns the boot sequence, window/menu/lifecycle plumbing, and Tauri commands/events.
//!
//! E1-S1 lands the **real Tauri 2 runtime**: `tauri::Builder` with a `setup` hook
//! that runs the FOUNDATION §6.1 boot sequence and shows the Home window
//! (1200×1200 default, 760×480 min — declared in `tauri.conf.json`). The macOS
//! reference `App/main.swift` + `App/AppDelegate.swift` boot order is ported in
//! `boot.rs`; settings persistence (`settings.json`, absent⇒ON booleans) in
//! `settings.rs`.
//!
//! Boot steps that are **real** here: telemetry-owned crash hook + tracing/Sentry
//! (steps 1–2, via `palmier_telemetry::init`), settings read (step 3), Clerk +
//! Convex auth config (step 4, via `palmier_auth::Auth::init`), the full main menu
//! with shortcuts (E1-S3), and Home window show (step 7). Stubbed for later
//! stories: the `/v1/models` async fetch (step 5) and the MCP start (step 6, Epic 7).
//!
//! ## Managed state
//! The long-lived boot handles are moved into Tauri managed state so they live
//! for the process and commands can reach them:
//! - [`palmier_telemetry::TelemetryHandle`] — **must** outlive the process
//!   (dropping it flushes logging + stops Sentry).
//! - [`palmier_auth::Auth`] — single source of truth for account/credit/key state.
//! - [`AppSettings`] — the launch-time settings snapshot.

// On Windows release builds, suppress the extra console window. Harmless in debug.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod boot;
mod commands;
mod media;
mod menu;
// E5-S8 — the wgpu preview present seam (plan A1: wgpu swapchain on the Tauri window
// HWND under a transparent WebView2 child). Self-contained to stay parallel-safe with
// concurrent panel/overlay work (E5-S10).
mod preview;
// Robust preview path: composite the active timeline OFFSCREEN → GPU readback → base64
// → a <canvas> in the webview. Replaces the fragile/unused on-window present (plan A1)
// as the actual preview surface. Reuses the export crate's offscreen render+readback.
mod preview_render;
mod project;
mod settings;
mod update;
mod window;

use std::sync::Mutex;

use tauri::Manager;

use commands::SettingsState;

/// The launch-time settings snapshot, stored in Tauri managed state so commands
/// (E1-S9 Settings UI) can read the booted prefs without re-reading the file.
pub struct AppSettings(pub settings::Settings);

fn main() {
    // Steps 1–6 of the FOUNDATION §6.1 boot sequence run synchronously, before
    // the Tauri event loop starts. Step 5 (model catalog) is spawned inside the
    // `setup` hook (it needs the async runtime handle); step 7 (show Home) also
    // happens in `setup` once the window exists.
    let boot_ctx = boot::run_sync_boot();

    tauri::Builder::default()
        // E1-S4 — persist each window's size+position per window label, replacing the
        // reference frame-autosave names (the label is the state key, so Settings/Help
        // cannot collide — settings-account-app.md autosave gotcha).
        .plugin(tauri_plugin_window_state::Builder::default().build())
        // E1-S7 — native Save-As / Open dialogs for New/Open project. The Home UI
        // drives the picker via `@tauri-apps/plugin-dialog`, then hands the chosen
        // `.palmier` path to `create_project` / `open_project`.
        .plugin(tauri_plugin_dialog::init())
        // E4-S12 — media-panel OS actions: Reveal in Explorer (opener) + Copy Path /
        // clipboard paste (clipboard-manager). The Media panel drives these via
        // `src-ui/media-panel/media-actions.ts`.
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // E1-S4/E1-S9/E1-S10 — the command seam the Home/Settings/Help/Feedback surfaces
        // and the menu router call via `invoke`.
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::set_notifications_enabled,
            commands::set_telemetry_enabled,
            commands::set_mcp_enabled,
            commands::dismiss_welcome,
            commands::get_account,
            commands::has_anthropic_key,
            commands::save_anthropic_key,
            commands::delete_anthropic_key,
            commands::get_mcp_status,
            commands::open_settings,
            commands::open_help,
            commands::open_feedback,
            commands::check_for_updates,
            commands::send_feedback,
            // E1-S7 — project lifecycle (Recent / create / open / delete / autosave-on-home).
            project::list_recent,
            project::create_project,
            project::open_project,
            project::open_project_dialog,
            project::delete_project,
            project::show_home,
            project::default_storage_dir,
            // E1-S8 — sample carousel (list / resolve+materialize+open).
            project::list_samples,
            project::open_sample,
            // E4-S12 — media-panel OS actions (Reveal / Copy Path / Relink / paste)
            // + the E4-S3 moment-thumbnail seam (stub until palmier-media/Epic 11).
            media::reveal_in_explorer,
            media::copy_paths_to_clipboard,
            media::pick_relink_path,
            media::read_clipboard_importable_paths,
            media::thumbnail,
            // E5-S8 — preview present surface lifecycle (plan A1 wgpu-under-webview).
            preview::preview_init,
            preview::preview_resize,
            preview::preview_teardown,
            // E5-S10 — preview transport (drives the engine Transport into the present
            // seam so preview plays/seeks/steps end-to-end; FR-19 current_frame events).
            preview::preview_set_timeline,
            preview::preview_play,
            preview::preview_pause,
            preview::preview_toggle_playback,
            preview::preview_seek,
            preview::preview_step,
            preview::preview_set_tab,
            // Robust preview: composite the active timeline offscreen + read it back as
            // base64 RGBA for the <canvas> (the actual preview path the viewport uses).
            preview_render::preview_render_frame,
            // Project editor bridge — read the shared timeline / media library and
            // dispatch mutating tools through the ONE shared executor (the same owner
            // the MCP server + in-app agent drive). `editor_edit` emits
            // `timeline://changed` so the UI refetches without polling.
            commands::editor_get_timeline,
            commands::editor_get_media,
            commands::editor_edit,
            // M2 boot integration — the in-app agent command surface (the panel's
            // agent_send/agent_cancel + agent_status/agent_set_pref seam). Tool
            // dispatch routes into the SAME shared executor the MCP server uses.
            agent::agent_send,
            agent::agent_cancel,
            agent::agent_status,
            agent::agent_set_pref,
            // E8-S7 — agent chat session persistence + tab orchestration. The panel's
            // tab bar + history dropdown call these; sessions load on project open and
            // persist into the bundle's chat/ dir on document save (ruling #4).
            agent::agent_list_sessions,
            agent::agent_get_session,
            agent::agent_new_session,
            agent::agent_open_session,
            agent::agent_close_session,
            agent::agent_delete_session,
        ])
        .setup(move |app| {
            // E1-S7/E1-S8 — build the project lifecycle state BEFORE `auth` is moved
            // into managed state: the sample service needs the Convex HTTP URL from
            // the auth config (None ⇒ empty carousel, offline-safe). The registry
            // loads from `project-registry.json` (missing ⇒ empty, lenient).
            let project_state = project::build_state(&boot_ctx.auth);

            // Move the long-lived boot handles into Tauri managed state. The
            // telemetry handle MUST live for the whole process (dropping it stops
            // Sentry + flushes the rotated-file log writer); managed state holds
            // it for the app lifetime. Auth + settings are likewise process-wide.
            app.manage(boot_ctx.telemetry);
            app.manage(boot_ctx.auth);
            app.manage(project_state);
            app.manage(AppSettings(boot_ctx.settings.clone()));
            // E1-S9 — the live (mutable) settings the General-tab toggles mutate +
            // persist, seeded from the boot snapshot.
            app.manage(SettingsState(Mutex::new(boot_ctx.settings.clone())));
            // E5-S8 — the preview present session slot (None until a project window
            // calls `preview_init`). Holds the wgpu compositor + decode owner.
            app.manage(preview::PreviewState::default());
            // The cached headless compositor for the robust offscreen preview path
            // (None until the first `preview_render_frame`; rebuilt on a size change).
            app.manage(preview_render::PreviewRenderState::default());

            // M2 boot integration — the agent state owns the ONE shared
            // `Arc<ToolExecutor>` (single `EditorState`) that BOTH the loopback MCP
            // server and the in-app agent drive. Managed BEFORE the MCP start so
            // step 6 can clone the shared executor into the server.
            app.manage(agent::AgentState::new());

            // E1-S3 — build + install the full main menu (Palmier Pro / File /
            // Edit / View / Help) with the reference Windows/Linux accelerators
            // and wire the single menu-event router.
            menu::install(&app.handle().clone())?;

            // Step 5 — kick the non-blocking model-catalog load. Spawned onto the
            // Tauri async runtime and never awaited: offline/slow Convex cannot
            // delay reaching Home (FR-1 / SM-1).
            let handle = tauri::async_runtime::handle();
            boot::spawn_model_catalog_load(&handle);

            // Step 6 (real) — start the loopback MCP server over the SHARED executor
            // if `io.palmier.pro.mcp.enabled` (ruling #6, absent ⇒ ON). The bind
            // returns as soon as the listener is up (serving runs on a background
            // task), and a bind FAILURE is logged-not-fatal — so this cannot stall
            // cold start or break boot. `get_mcp_status` reads the live state.
            let mcp_running =
                agent::start_mcp(&app.handle().clone(), boot_ctx.settings.mcp_enabled);
            tracing::info!(target: "mcp", mcp_running, "boot 6/7: MCP start (live)");

            // Step 7 — show the Home window. The window is declared in
            // tauri.conf.json (label "home", 1200×1200 / min 760×480) and is
            // created by Tauri before `setup`; we surface + focus it here, which
            // mirrors the reference `AppDelegate.showWindow`.
            if let Some(home) = app.get_webview_window("home") {
                let _ = home.show();
                let _ = home.set_focus();
                tracing::info!(target: "app", "boot 7/7: Home window shown");
            } else {
                // Should not happen given tauri.conf.json, but never hard-fail boot.
                tracing::error!(
                    target: "app",
                    "boot 7/7: Home window 'home' not found in config"
                );
            }

            // Autonomous-test affordance (no-op unless PALMIER_OPEN_* is set): open a
            // project window at boot so the editor can be driven/screenshotted without a
            // click. See domains/build-orchestration/autonomous-finish.md.
            if let Some(id) = app.state::<project::ProjectState>().boot_open_id() {
                tracing::info!(target: "app", id = %id, "boot: auto-opening project window (PALMIER_OPEN_*)");
                let _ = window::open_project_window(&app.handle().clone(), &id);
            }

            // E1-S10 — touch the updater (reference `AppDelegate` touches
            // `Updater.shared`). Runs a real check only when a signed feed is
            // configured; a dev build with no feed stays completely silent.
            update::check_on_boot(&app.handle().clone());

            tracing::info!(
                target: "app",
                mcp_started = boot_ctx.mcp_started,
                has_seen_welcome = boot_ctx.settings.has_seen_welcome,
                "palmier-tauri boot complete"
            );

            // `applicationShouldOpenUntitledFile`-equivalent is implicitly false:
            // boot does NOT auto-create a project (no New on launch). Reopen-with-
            // no-windows → show Home is handled by E1-S4's window lifecycle.

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // M2 boot integration — stop the loopback MCP server gracefully on exit
            // so its background serving task + bound port are released cleanly.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                agent::stop_mcp(app_handle);
            }
        });
}
