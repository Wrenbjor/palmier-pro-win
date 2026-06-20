//! # palmier-tauri
//!
//! The Tauri 2 binary that wires every core crate together (FOUNDATION §4, §6.1).
//! Owns the boot sequence, window/menu/lifecycle plumbing, and Tauri commands/events.
//!
//! Skeleton stub: this `main` only sketches the FOUNDATION §6.1 boot ORDER against
//! the placeholder crate hooks so the workspace compiles end-to-end. The real
//! `tauri::Builder` setup + window show lands in Epic 1 / E1-S1; the `tauri`
//! runtime dependency is added there (kept out of the skeleton for fast builds).

fn main() {
    // FOUNDATION §6.1 boot order (placeholder hooks; real impl per E1-S1):
    // 1. crash handler + tracing subscriber
    palmier_telemetry::start(false, None);
    // 5. fonts register before window build (E1-S5)
    palmier_text::register_bundled_fonts();

    println!(
        "palmier-tauri skeleton — boot order stub. \
         model={} auth_keyring={} update_channel={} mcp_bind={}",
        palmier_model::CRATE_NAME,
        palmier_auth::KEYRING_ACCOUNT,
        palmier_update::CHANNEL,
        palmier_mcp::DEFAULT_BIND,
    );

    // Touch palmier-project so the dependency edge is exercised in the skeleton.
    let _ = (palmier_project::PROJECT_FILE, palmier_project::MEDIA_FILE);
}
