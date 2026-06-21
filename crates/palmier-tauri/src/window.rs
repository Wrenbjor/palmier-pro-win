//! Window set + lifecycle (E1-S4).
//!
//! The reference app has five windows — Home (project browser), Project (editor),
//! Settings, Help, and Feedback — each created lazily by an `NSWindowController` with a
//! fixed size/min-size and (for Settings/Help) a frame-autosave name. Ports them onto
//! Tauri `WebviewWindow`s (settings-account-app.md "Window config"; FOUNDATION §6.1
//! "Windows").
//!
//! ## Sizes (replicate exactly)
//! | window   | default     | min       | notes                                    |
//! |----------|-------------|-----------|------------------------------------------|
//! | Home     | 1200×1200   | 760×480   | project browser + welcome overlay        |
//! | Project  | 1600×1000   | 960×600   | one window **per project**               |
//! | Settings | 980×640     | 760×480   | dark, transparent titlebar, full content |
//! | Help     | 900×560     | 820×520   | 2 tabs (Shortcuts / MCP)                 |
//! | Feedback | 480×480     | 480×420   | not-released-on-close                    |
//!
//! Home is declared statically in `tauri.conf.json` (created visible-false, shown in
//! boot step 7). The others are created **on demand** here, mirroring the reference's
//! lazy `*WindowController.shared.showWindow`. Re-invoking focuses the existing window
//! rather than spawning a duplicate (Settings/Help/Feedback are singletons; Project is
//! keyed per project id — one window per project, FR-2).
//!
//! ## Routing
//! Each window loads the same `index.html`; `src-ui` reads its window label
//! (`getCurrentWindow().label`) and mounts the matching surface (`App.tsx` router). The
//! Project window's label is `project/<id>` so the frontend can read the id from the
//! label suffix.
//!
//! ## Position/size persistence
//! `tauri-plugin-window-state` persists size+position per window label, replacing the
//! reference autosave names (`PalmierProSettings-v2` / `PalmierProHelp-v1`). The label
//! is the state key, so they cannot collide (settings-account-app.md autosave gotcha).

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// A non-Home window kind with its reference geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// Settings (980×640 / min 760×480), dark + transparent titlebar.
    Settings,
    /// Help (900×560 / min 820×520).
    Help,
    /// Feedback (480×480 / min 480×420).
    Feedback,
}

impl WindowKind {
    /// Stable window label / state key.
    pub fn label(self) -> &'static str {
        match self {
            WindowKind::Settings => "settings",
            WindowKind::Help => "help",
            WindowKind::Feedback => "feedback",
        }
    }

    /// Window title.
    fn title(self) -> &'static str {
        match self {
            WindowKind::Settings => "Settings",
            WindowKind::Help => "Palmier Pro Help",
            WindowKind::Feedback => "Send Feedback",
        }
    }

    /// `(default_w, default_h, min_w, min_h)` — the reference geometry.
    fn geometry(self) -> (f64, f64, f64, f64) {
        match self {
            WindowKind::Settings => (980.0, 640.0, 760.0, 480.0),
            WindowKind::Help => (900.0, 560.0, 820.0, 520.0),
            WindowKind::Feedback => (480.0, 480.0, 480.0, 420.0),
        }
    }
}

/// Build a webview window **on a background thread**.
///
/// WHY: `WebviewWindowBuilder::build()` blocks the calling thread until the OS webview
/// finishes initializing. Tauri commands and menu handlers run on the **main
/// (event-loop) thread**; calling `build()` there deadlocks on Windows — the native
/// window appears (chrome + menu) but WebView2 init needs the very main thread that
/// `build()` is blocking, so the app hangs (blank-white content, can't open DevTools,
/// closing freezes). Creating the window from a spawned thread with a cloned
/// `AppHandle` (a supported Tauri pattern) frees the main loop to service the webview
/// init. Build errors are logged rather than returned (the caller already left the
/// command), which is acceptable for window opening.
fn spawn_window_build<R: Runtime>(
    app: &AppHandle<R>,
    label: String,
    url: WebviewUrl,
    geometry: (f64, f64, f64, f64),
    title: String,
    decorations: bool,
    what: &'static str,
) {
    let app = app.clone();
    let (w, h, min_w, min_h) = geometry;
    std::thread::spawn(move || {
        let res = WebviewWindowBuilder::new(&app, &label, url)
            .title(title)
            .inner_size(w, h)
            .min_inner_size(min_w, min_h)
            .resizable(true)
            .focused(true)
            .decorations(decorations)
            .build();
        match res {
            Ok(win) => {
                let _ = win.show();
                let _ = win.set_focus();
                tracing::info!(target: "app", window = %label, "{what}");
            }
            Err(e) => {
                tracing::error!(target: "app", window = %label, error = %e, "{what} failed");
            }
        }
    });
}

/// Open (or focus, if already open) a singleton auxiliary window.
///
/// Returns the existing window when present (reference: `*WindowController.shared` is a
/// singleton — re-invoking just brings it forward), else builds it with the reference
/// geometry. The hash route (`#/<label>`) tells `src-ui` which surface to mount even when
/// `withGlobalTauri`'s label lookup is unavailable.
pub fn open_or_focus<R: Runtime>(
    app: &AppHandle<R>,
    kind: WindowKind,
) -> tauri::Result<()> {
    let label = kind.label();
    if let Some(win) = app.get_webview_window(label) {
        let _ = win.show();
        let _ = win.set_focus();
        tracing::debug!(target: "app", window = label, "focused existing window");
        return Ok(());
    }

    let url = WebviewUrl::App(format!("index.html#/{label}").into());
    // Build off the main thread to avoid the WebView2 creation deadlock (see
    // `spawn_window_build`). Decorations stay on for every kind (Settings is dark with
    // a transparent titlebar + full-size content; decorations default on regardless).
    spawn_window_build(
        app,
        label.to_string(),
        url,
        kind.geometry(),
        kind.title().to_string(),
        true,
        "opened window",
    );
    Ok(())
}

/// One project window per project (FR-2). Label is `project/<id>`; re-opening the same
/// project id focuses the existing window instead of spawning a duplicate.
pub fn open_project_window<R: Runtime>(
    app: &AppHandle<R>,
    project_id: &str,
) -> tauri::Result<()> {
    let label = format!("project/{project_id}");
    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.show();
        let _ = win.set_focus();
        tracing::debug!(target: "app", window = %label, "focused existing project window");
        return Ok(());
    }

    let url = WebviewUrl::App(format!("index.html#/project/{project_id}").into());
    spawn_window_build(
        app,
        label,
        url,
        (1600.0, 1000.0, 960.0, 600.0),
        "Palmier Pro".to_string(),
        true,
        "opened project window",
    );
    Ok(())
}

/// Show the Home window (reference "Reopen with no windows → showHome()"). Creates it if
/// it somehow no longer exists.
pub fn show_home<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(home) = app.get_webview_window("home") {
        let _ = home.show();
        let _ = home.set_focus();
        return Ok(());
    }
    // Home is normally static in tauri.conf.json; recreate defensively — off the main
    // thread to avoid the WebView2 creation deadlock (see `spawn_window_build`).
    spawn_window_build(
        app,
        "home".to_string(),
        WebviewUrl::App("index.html".into()),
        (1200.0, 1200.0, 760.0, 480.0),
        "Palmier Pro".to_string(),
        true,
        "recreated home window",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_distinct_and_stable() {
        assert_eq!(WindowKind::Settings.label(), "settings");
        assert_eq!(WindowKind::Help.label(), "help");
        assert_eq!(WindowKind::Feedback.label(), "feedback");
    }

    #[test]
    fn geometry_matches_reference_window_config() {
        assert_eq!(WindowKind::Settings.geometry(), (980.0, 640.0, 760.0, 480.0));
        assert_eq!(WindowKind::Help.geometry(), (900.0, 560.0, 820.0, 520.0));
        assert_eq!(WindowKind::Feedback.geometry(), (480.0, 480.0, 480.0, 420.0));
    }
}
