//! Main menu + keyboard shortcuts (E1-S3).
//!
//! Ports the reference `App/MainMenu.swift` (`MainMenuBuilder.buildMenu()` +
//! the `EditorActions` responder-chain protocol) onto the Tauri 2 `Menu` API.
//! Per FOUNDATION §6.1 and `docs/reference/settings-account-app.md` the bindings
//! are identical to the Mac app with two platform substitutions:
//!
//! - **Cmd → Ctrl** for every accelerator (`CmdOrCtrl` resolves to Ctrl on
//!   Windows/Linux); `Cmd+Option` → `Ctrl+Alt`.
//! - **Enter Full Screen** is **F11** (Windows convention), not `Cmd+F`.
//!
//! ## Dispatch model
//! macOS sent menu actions up the `NSApp.sendAction` responder chain. Tauri has
//! no responder chain, so each menu item carries a stable **command id** (the
//! `MenuId`) and the single [`on_menu_event`] handler routes it:
//!
//! - **App/window items** (Settings, About, Check for Updates, Quit, fullscreen)
//!   invoke their handler **directly** in Rust.
//! - **Editor-action items** (Undo/Redo, Cut/Copy/Paste/Select-All, Split, Trim,
//!   Delete, panel toggles, layout presets) **emit a Tauri event**
//!   (`menu://<command-id>`) the editor frontend (`src-ui`) consumes. Until the
//!   owning epic lands its real handler, the frontend listener is a logged stub —
//!   the binding is still provably invokable (the event fires).
//! - **Help/Feedback items** emit the same event family so `src-ui` can open the
//!   Help/Feedback surfaces (E1-S4/E1-S9 own those windows).
//!
//! Items whose target subsystem is a **later story/epic** (Save/Save-As/Export,
//! clip edits) dispatch to that same event seam — a registered, no-op-on-the-
//! frontend handler — so every §6.1 row is wired and testable now.
//!
//! The [`MENU_TABLE`] is the single source of truth: the builder is generated
//! from it and the parity test (`menu_table_covers_every_reference_row`) asserts
//! each §6.1 row resolves to a registered command id with the expected
//! accelerator.

use tauri::menu::{Menu, MenuItemBuilder, Submenu, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, Runtime, Wry};

/// How a menu item is dispatched when invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// Handled directly in Rust (app/window scope).
    Native,
    /// Emitted as a `menu://<id>` Tauri event for the frontend to consume.
    Event,
}

/// One row of the reference §6.1 menu. The builder and the parity test are both
/// generated from this table, so the menu can never drift from the spec silently.
#[derive(Debug, Clone, Copy)]
pub struct MenuRow {
    /// Top-level menu this row belongs to ("app"/"file"/"edit"/"view"/"help").
    pub menu: &'static str,
    /// Stable command id (also the Tauri event name suffix). Kebab-case.
    pub id: &'static str,
    /// User-visible label.
    pub label: &'static str,
    /// Tauri accelerator string (`None` ⇒ no shortcut). `CmdOrCtrl` resolves to
    /// Ctrl on Windows/Linux. `F11` is the fullscreen substitution.
    pub accelerator: Option<&'static str>,
    /// How the item is dispatched.
    pub dispatch: Dispatch,
}

/// The complete reference main menu (FOUNDATION §6.1 / settings-account-app.md),
/// with Cmd→Ctrl and Cmd+F→F11 already applied. Order matches the reference.
pub const MENU_TABLE: &[MenuRow] = &[
    // ── Palmier Pro ───────────────────────────────────────────────────────────
    MenuRow { menu: "app", id: "about",             label: "About Palmier Pro",  accelerator: None,                 dispatch: Dispatch::Event },
    MenuRow { menu: "app", id: "check-for-updates", label: "Check for Updates…", accelerator: None,                 dispatch: Dispatch::Native },
    MenuRow { menu: "app", id: "settings",          label: "Settings…",          accelerator: Some("CmdOrCtrl+,"),  dispatch: Dispatch::Native },
    MenuRow { menu: "app", id: "quit",              label: "Quit Palmier Pro",   accelerator: Some("CmdOrCtrl+Q"),  dispatch: Dispatch::Native },
    // ── File ──────────────────────────────────────────────────────────────────
    MenuRow { menu: "file", id: "new",          label: "New",          accelerator: Some("CmdOrCtrl+N"),       dispatch: Dispatch::Event },
    MenuRow { menu: "file", id: "open",         label: "Open…",        accelerator: Some("CmdOrCtrl+O"),       dispatch: Dispatch::Event },
    MenuRow { menu: "file", id: "save",         label: "Save",         accelerator: Some("CmdOrCtrl+S"),       dispatch: Dispatch::Event },
    MenuRow { menu: "file", id: "save-as",      label: "Save As…",     accelerator: Some("CmdOrCtrl+Shift+S"), dispatch: Dispatch::Event },
    MenuRow { menu: "file", id: "import-media", label: "Import Media…",accelerator: Some("CmdOrCtrl+I"),       dispatch: Dispatch::Event },
    MenuRow { menu: "file", id: "export",       label: "Export…",      accelerator: Some("CmdOrCtrl+E"),       dispatch: Dispatch::Event },
    // ── Edit ──────────────────────────────────────────────────────────────────
    MenuRow { menu: "edit", id: "undo",        label: "Undo",             accelerator: Some("CmdOrCtrl+Z"),       dispatch: Dispatch::Event },
    MenuRow { menu: "edit", id: "redo",        label: "Redo",             accelerator: Some("CmdOrCtrl+Shift+Z"), dispatch: Dispatch::Event },
    MenuRow { menu: "edit", id: "cut",         label: "Cut",              accelerator: Some("CmdOrCtrl+X"),       dispatch: Dispatch::Event },
    MenuRow { menu: "edit", id: "copy",        label: "Copy",             accelerator: Some("CmdOrCtrl+C"),       dispatch: Dispatch::Event },
    MenuRow { menu: "edit", id: "paste",       label: "Paste",            accelerator: Some("CmdOrCtrl+V"),       dispatch: Dispatch::Event },
    MenuRow { menu: "edit", id: "select-all",  label: "Select All",       accelerator: Some("CmdOrCtrl+A"),       dispatch: Dispatch::Event },
    MenuRow { menu: "edit", id: "split",       label: "Split at Playhead",accelerator: Some("CmdOrCtrl+K"),       dispatch: Dispatch::Event },
    MenuRow { menu: "edit", id: "trim-start",  label: "Trim Start to Playhead", accelerator: Some("Q"),           dispatch: Dispatch::Event },
    MenuRow { menu: "edit", id: "trim-end",    label: "Trim End to Playhead",   accelerator: Some("W"),           dispatch: Dispatch::Event },
    MenuRow { menu: "edit", id: "delete",      label: "Delete",           accelerator: Some("Backspace"),         dispatch: Dispatch::Event },
    // ── View ──────────────────────────────────────────────────────────────────
    MenuRow { menu: "view", id: "toggle-media-panel",     label: "Media Panel",            accelerator: Some("CmdOrCtrl+0"),     dispatch: Dispatch::Event },
    MenuRow { menu: "view", id: "toggle-inspector",       label: "Inspector",              accelerator: Some("CmdOrCtrl+Alt+0"), dispatch: Dispatch::Event },
    MenuRow { menu: "view", id: "toggle-agent-panel",     label: "Agent Panel",            accelerator: Some("CmdOrCtrl+Alt+A"), dispatch: Dispatch::Event },
    MenuRow { menu: "view", id: "maximize-panel",         label: "Maximize Focused Panel", accelerator: Some("`"),               dispatch: Dispatch::Event },
    MenuRow { menu: "view", id: "layout-default",         label: "Default",                accelerator: Some("CmdOrCtrl+1"),     dispatch: Dispatch::Event },
    MenuRow { menu: "view", id: "layout-media",           label: "Media",                  accelerator: Some("CmdOrCtrl+2"),     dispatch: Dispatch::Event },
    MenuRow { menu: "view", id: "layout-vertical",        label: "Vertical",               accelerator: Some("CmdOrCtrl+3"),     dispatch: Dispatch::Event },
    MenuRow { menu: "view", id: "toggle-fullscreen",      label: "Enter Full Screen",      accelerator: Some("F11"),             dispatch: Dispatch::Native },
    // ── Help ──────────────────────────────────────────────────────────────────
    MenuRow { menu: "help", id: "tutorial",           label: "Tutorial",           accelerator: None,      dispatch: Dispatch::Event },
    MenuRow { menu: "help", id: "keyboard-shortcuts", label: "Keyboard Shortcuts", accelerator: Some("?"), dispatch: Dispatch::Native },
    MenuRow { menu: "help", id: "mcp-instructions",   label: "MCP Instructions",   accelerator: None,      dispatch: Dispatch::Native },
    MenuRow { menu: "help", id: "send-feedback",      label: "Send Feedback…",     accelerator: None,      dispatch: Dispatch::Native },
];

/// Look up a row by its command id.
#[must_use]
pub fn row(id: &str) -> Option<&'static MenuRow> {
    MENU_TABLE.iter().find(|r| r.id == id)
}

/// Build a single leaf menu item from a [`MenuRow`].
fn item<R: Runtime>(
    app: &AppHandle<R>,
    r: &MenuRow,
) -> tauri::Result<tauri::menu::MenuItem<R>> {
    let mut b = MenuItemBuilder::with_id(r.id, r.label);
    if let Some(acc) = r.accelerator {
        b = b.accelerator(acc);
    }
    b.build(app)
}

/// Build the submenu named `title` from every [`MENU_TABLE`] row whose `menu`
/// matches `menu_key` (in table order), inserting a separator after any id in
/// `separators_after` — matching the reference grouping.
fn build_submenu<R: Runtime>(
    app: &AppHandle<R>,
    title: &str,
    menu_key: &str,
    separators_after: &[&str],
) -> tauri::Result<Submenu<R>> {
    let mut sb = SubmenuBuilder::new(app, title);
    for r in MENU_TABLE.iter().filter(|r| r.menu == menu_key) {
        sb = sb.item(&item(app, r)?);
        if separators_after.contains(&r.id) {
            sb = sb.separator();
        }
    }
    sb.build()
}

/// Build the full application menu from [`MENU_TABLE`].
///
/// The reference nests Layout (Default/Media/Vertical) under a "Layout" submenu
/// of View; we keep that structure. All other items are flat under their menu.
pub fn build_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    // Palmier Pro — separators after Updates and Settings.
    let app_menu = build_submenu(app, "Palmier Pro", "app", &["check-for-updates", "settings"])?;

    // File
    let file_menu = build_submenu(app, "File", "file", &["open", "save-as", "import-media"])?;

    // Edit
    let edit_menu = build_submenu(app, "Edit", "edit", &["redo", "select-all", "trim-end"])?;

    // View — Layout is a nested submenu; build it separately, then assemble View.
    let layout_menu = {
        let mut sb = SubmenuBuilder::new(app, "Layout");
        for r in MENU_TABLE
            .iter()
            .filter(|r| r.menu == "view" && r.id.starts_with("layout-"))
        {
            sb = sb.item(&item(app, r)?);
        }
        sb.build()?
    };

    let view_menu = {
        let mut sb = SubmenuBuilder::new(app, "View");
        // Panel toggles
        sb = sb
            .item(&item(app, row("toggle-media-panel").unwrap())?)
            .item(&item(app, row("toggle-inspector").unwrap())?)
            .item(&item(app, row("toggle-agent-panel").unwrap())?)
            .separator()
            .item(&item(app, row("maximize-panel").unwrap())?)
            .separator()
            .item(&layout_menu)
            .separator()
            .item(&item(app, row("toggle-fullscreen").unwrap())?);
        sb.build()?
    };

    // Help
    let help_menu = build_submenu(app, "Help", "help", &["tutorial", "mcp-instructions"])?;

    Menu::with_items(
        app,
        &[&app_menu, &file_menu, &edit_menu, &view_menu, &help_menu],
    )
}

/// The single menu-event router. Wire via `.on_menu_event(menu::on_menu_event)`.
///
/// Routes each item by its [`Dispatch`]:
/// - [`Dispatch::Native`] items are handled in Rust here.
/// - [`Dispatch::Event`] items emit `menu://<id>` for the frontend to consume.
pub fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: tauri::menu::MenuEvent) {
    let id = event.id().0.as_str();
    let Some(r) = row(id) else {
        tracing::warn!(target: "app", menu_id = id, "menu event with unknown id");
        return;
    };

    match r.dispatch {
        Dispatch::Native => handle_native(app, r),
        Dispatch::Event => {
            // Emit `menu://<id>`. The editor/Help frontend listens and runs the
            // real (or, for later-epic items, a logged no-op) handler.
            let event_name = format!("menu://{id}");
            if let Err(err) = app.emit(&event_name, ()) {
                tracing::error!(
                    target: "app",
                    menu_id = id,
                    error = %err,
                    "failed to emit menu event"
                );
            } else {
                tracing::debug!(target: "app", menu_id = id, event = %event_name, "menu event emitted");
            }
        }
    }
}

/// Handle the app/window-scoped items that resolve in Rust (no frontend round-trip).
fn handle_native<R: Runtime>(app: &AppHandle<R>, r: &MenuRow) {
    match r.id {
        "quit" => {
            tracing::info!(target: "app", "menu: Quit");
            app.exit(0);
        }
        "toggle-fullscreen" => {
            // F11 toggles fullscreen on the focused (or Home) window.
            if let Some(win) = app
                .get_webview_window("home")
                .or_else(|| app.webview_windows().into_values().next())
            {
                let now = win.is_fullscreen().unwrap_or(false);
                let _ = win.set_fullscreen(!now);
                tracing::debug!(target: "app", fullscreen = !now, "menu: toggle fullscreen");
            }
        }
        "check-for-updates" => {
            // E1-S10 — run the real Tauri updater check (silently disabled in dev /
            // unsigned builds). Replaces E1-S3's placeholder event emit.
            tracing::info!(target: "app", "menu: Check for Updates");
            crate::update::check_now(app);
        }
        "settings" => {
            tracing::info!(target: "app", "menu: Settings");
            if let Err(err) = crate::window::open_or_focus(app, crate::window::WindowKind::Settings) {
                tracing::error!(target: "app", error = %err, "failed to open Settings window");
            }
        }
        "keyboard-shortcuts" | "mcp-instructions" => {
            // Both Help-menu items open the Help window (it has the Shortcuts + MCP tabs);
            // the frontend reads the requested tab from the focus event / its own state.
            tracing::info!(target: "app", menu_id = r.id, "menu: Help");
            if let Err(err) = crate::window::open_or_focus(app, crate::window::WindowKind::Help) {
                tracing::error!(target: "app", error = %err, "failed to open Help window");
            }
            // Tell the Help webview which tab to select.
            let tab = if r.id == "mcp-instructions" { "mcp" } else { "shortcuts" };
            let _ = app.emit("help://select-tab", tab);
        }
        "send-feedback" => {
            tracing::info!(target: "app", "menu: Send Feedback");
            if let Err(err) = crate::window::open_or_focus(app, crate::window::WindowKind::Feedback) {
                tracing::error!(target: "app", error = %err, "failed to open Feedback window");
            }
        }
        other => {
            tracing::warn!(target: "app", menu_id = other, "native menu item with no handler");
        }
    }
}

/// Convenience for `main.rs`: build + attach the menu and its event handler in
/// the Tauri `setup` hook. Generic kept to [`Wry`] (the binary's runtime).
pub fn install(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    app.set_menu(menu)?;
    app.on_menu_event(on_menu_event);
    tracing::info!(
        target: "app",
        items = MENU_TABLE.len(),
        "main menu installed (E1-S3)"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// FR-3 parity: every reference §6.1 row is present with the exact label,
    /// the expected Windows accelerator (Cmd→Ctrl, fullscreen→F11), and a
    /// registered (unique) command id. This is the menu-shortcut table test the
    /// story's third acceptance criterion requires.
    #[test]
    fn menu_table_covers_every_reference_row() {
        // (menu, label, accelerator) — the reference §6.1 table, ported.
        let expected: &[(&str, &str, Option<&str>)] = &[
            // Palmier Pro
            ("app", "About Palmier Pro", None),
            ("app", "Check for Updates…", None),
            ("app", "Settings…", Some("CmdOrCtrl+,")),
            ("app", "Quit Palmier Pro", Some("CmdOrCtrl+Q")),
            // File
            ("file", "New", Some("CmdOrCtrl+N")),
            ("file", "Open…", Some("CmdOrCtrl+O")),
            ("file", "Save", Some("CmdOrCtrl+S")),
            ("file", "Save As…", Some("CmdOrCtrl+Shift+S")),
            ("file", "Import Media…", Some("CmdOrCtrl+I")),
            ("file", "Export…", Some("CmdOrCtrl+E")),
            // Edit
            ("edit", "Undo", Some("CmdOrCtrl+Z")),
            ("edit", "Redo", Some("CmdOrCtrl+Shift+Z")),
            ("edit", "Cut", Some("CmdOrCtrl+X")),
            ("edit", "Copy", Some("CmdOrCtrl+C")),
            ("edit", "Paste", Some("CmdOrCtrl+V")),
            ("edit", "Select All", Some("CmdOrCtrl+A")),
            ("edit", "Split at Playhead", Some("CmdOrCtrl+K")),
            ("edit", "Trim Start to Playhead", Some("Q")),
            ("edit", "Trim End to Playhead", Some("W")),
            ("edit", "Delete", Some("Backspace")),
            // View
            ("view", "Media Panel", Some("CmdOrCtrl+0")),
            ("view", "Inspector", Some("CmdOrCtrl+Alt+0")),
            ("view", "Agent Panel", Some("CmdOrCtrl+Alt+A")),
            ("view", "Maximize Focused Panel", Some("`")),
            ("view", "Default", Some("CmdOrCtrl+1")),
            ("view", "Media", Some("CmdOrCtrl+2")),
            ("view", "Vertical", Some("CmdOrCtrl+3")),
            ("view", "Enter Full Screen", Some("F11")),
            // Help
            ("help", "Tutorial", None),
            ("help", "Keyboard Shortcuts", Some("?")),
            ("help", "MCP Instructions", None),
            ("help", "Send Feedback…", None),
        ];

        for (menu, label, acc) in expected {
            let found = MENU_TABLE
                .iter()
                .find(|r| r.menu == *menu && r.label == *label)
                .unwrap_or_else(|| panic!("missing menu row: {menu}/{label}"));
            assert_eq!(
                found.accelerator, *acc,
                "accelerator mismatch for {menu}/{label}"
            );
        }

        // No extra rows beyond the reference set.
        assert_eq!(
            MENU_TABLE.len(),
            expected.len(),
            "MENU_TABLE has a row not in the reference §6.1 table (or vice versa)"
        );
    }

    /// Every command id is unique (ids double as Tauri event names).
    #[test]
    fn command_ids_are_unique() {
        let mut seen = HashSet::new();
        for r in MENU_TABLE {
            assert!(seen.insert(r.id), "duplicate menu command id: {}", r.id);
        }
    }

    /// `row()` resolves every id, and the window/app items are Native while the
    /// editor-action items are Event (the dispatch contract the frontend relies on).
    #[test]
    fn dispatch_routing_matches_contract() {
        // App/window-scoped items handled in Rust. With E1-S4's windows landed, the
        // window-opening Help/Settings/Feedback items are now Native too (they open a
        // real WebviewWindow directly), matching the reference responder-chain handlers.
        for id in [
            "quit",
            "toggle-fullscreen",
            "check-for-updates",
            "settings",
            "keyboard-shortcuts",
            "mcp-instructions",
            "send-feedback",
        ] {
            assert_eq!(row(id).unwrap().dispatch, Dispatch::Native, "{id} should be Native");
        }
        // Editor-action items still go to the frontend via events (their owning epic
        // lands the real handler later).
        for id in ["undo", "split", "trim-start", "delete", "about", "tutorial"] {
            assert_eq!(row(id).unwrap().dispatch, Dispatch::Event, "{id} should be Event");
        }
    }

    /// The F11 substitution (not Cmd+F) is in place for fullscreen, and no
    /// accelerator still contains a bare `Cmd` that would not resolve on Win/Linux
    /// (we use the portable `CmdOrCtrl` form everywhere).
    #[test]
    fn fullscreen_is_f11_and_accelerators_are_portable() {
        assert_eq!(row("toggle-fullscreen").unwrap().accelerator, Some("F11"));
        for r in MENU_TABLE {
            if let Some(acc) = r.accelerator {
                assert!(
                    !acc.contains("Cmd+") || acc.contains("CmdOrCtrl"),
                    "{} uses a bare Cmd accelerator ({acc}); use CmdOrCtrl",
                    r.id
                );
            }
        }
    }
}
