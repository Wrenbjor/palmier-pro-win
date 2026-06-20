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

mod boot;
mod menu;
mod settings;

use tauri::Manager;

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
        .setup(move |app| {
            // Move the long-lived boot handles into Tauri managed state. The
            // telemetry handle MUST live for the whole process (dropping it stops
            // Sentry + flushes the rotated-file log writer); managed state holds
            // it for the app lifetime. Auth + settings are likewise process-wide.
            app.manage(boot_ctx.telemetry);
            app.manage(boot_ctx.auth);
            app.manage(AppSettings(boot_ctx.settings.clone()));

            // E1-S3 — build + install the full main menu (Palmier Pro / File /
            // Edit / View / Help) with the reference Windows/Linux accelerators
            // and wire the single menu-event router.
            menu::install(&app.handle().clone())?;

            // Step 5 — kick the non-blocking model-catalog load. Spawned onto the
            // Tauri async runtime and never awaited: offline/slow Convex cannot
            // delay reaching Home (FR-1 / SM-1).
            let handle = tauri::async_runtime::handle();
            boot::spawn_model_catalog_load(&handle);

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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
