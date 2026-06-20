//! Preview present integration — E5-S8 (the S-2 plan-A1 seam in the real Tauri window).
//!
//! This is the `palmier-tauri` half of E5-S8: it reproduces the proven S-2 mechanism
//! (`spikes/s2-wry-integration/FINDINGS.md`) inside the **real Tauri 2.11 window**.
//! The composition graph + wgpu compositor live in `palmier-engine` (E5-S3..S8); this
//! module owns the *present seam* and the per-session wiring:
//!
//! 1. **Surface on the Tauri window (plan A1).** A Tauri [`WebviewWindow`] implements
//!    `raw_window_handle::HasWindowHandle` + `HasDisplayHandle` via **raw-window-handle
//!    0.6.2** — the SAME handle currency wgpu 27 consumes. So
//!    [`palmier_engine::Compositor::new_for_surface`] puts the wgpu swapchain directly
//!    on the window's HWND, and Tauri's existing **transparent** WebView2 child
//!    composites OVER it (DWM merges the two HWNDs — zero readback, zero IPC, SM-2
//!    zero-copy holds). This is exactly what S-2 proved on raw winit+wry; the only
//!    Tauri-specific step is getting the handle (done here) — and the
//!    `clip_children(false)` caveat (see [`PreviewSession::init`] notes + the module
//!    "Residual risk" comment).
//!
//! 2. **Transport → compositor.** The engine [`Transport`] (E5-S7) emits
//!    [`TransportEvent`]s. [`PreviewSession::apply_events`] turns each
//!    `TransportEvent::Render(frame)` into a `compositor.render(frame, frame_source)`
//!    (draw + present), each `SeekDecode` into the decode being served by the
//!    [`FrameSource`] inside `render`, and each `CurrentFrameChanged` into a
//!    `current_frame` **Tauri event** to the frontend (FR-19).
//!
//! ## Why a separate module (parallel-safe)
//! E5-S10 (panels / overlays) and other M1 work touch the editor surface concurrently.
//! Keeping the present seam in its own module + a single managed-state slot + a small
//! command set avoids colliding with panel work. The module is registered from
//! `main.rs` with three lines (managed state + the `preview_*` command handlers).
//!
//! ## Residual risk carried from S-2 (the one Tauri-specific gap)
//! tao 0.35 (which Tauri owns) does **not** expose `with_clip_children(false)` — the
//! load-bearing anti-flicker flag winit 0.30 gave the S-2 spike. Without it, the parent
//! window may clip the wgpu swapchain against the WebView2 child HWND (tauri#9220
//! "fighting for the surface" black/flicker). Mitigations, in order: (a) set the window
//! transparent + the webview transparent (done via `tauri.conf.json` for the project
//! window) which on many drivers composites cleanly anyway; (b) if flicker appears,
//! fall to **plan A2** (the S-1 `present.rs` hand-wired DirectComposition visual tree,
//! which does NOT depend on tao's child-clip behavior). This must be **verified in a
//! live window run** before merge — see the story result's "check before merge".

// The present-seam API (`apply_events`, `render`, `evict_asset`, the
// `current_frame` event payload/name) is the contract the editor's transport
// commands (E5-S10 / the playback-wiring follow-up) call. It is fully built + tested
// here but not yet invoked from a command this story, so allow the dead-code lint at
// the module level rather than scattering attributes; removing it is a one-line change
// when the transport commands land.
#![allow(dead_code)]

use std::sync::Mutex;

use palmier_engine::{Compositor, RenderFrame, TransportEvent};
use palmier_media::FrameSource;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, WebviewWindow};

/// The payload streamed to the frontend on every `current_frame` change (FR-19).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentFramePayload {
    /// The new playhead frame.
    pub frame: i32,
    /// Whether this is the timeline tab's `current_frame` (vs. an asset tab's
    /// `source_playhead_frame`).
    pub is_timeline: bool,
}

/// The Tauri event name the frontend subscribes to for the reactive playhead.
pub const CURRENT_FRAME_EVENT: &str = "preview://current-frame";

/// One live preview present session: the wgpu compositor bound to a project window's
/// surface + the decode owner ([`FrameSource`]) the compositor resolves layers
/// through. Created by [`PreviewSession::init`] when a project window opens; dropped
/// when it closes (releasing the GPU device + the surface).
pub struct PreviewSession {
    /// The wgpu compositor presenting onto the window's swapchain (plan A1).
    compositor: Compositor,
    /// The one-decode-owner the compositor resolves each layer's `FrameRef` through
    /// (`palmier-media`; the engine never opens FFmpeg).
    frame_source: FrameSource,
    /// The window label this session presents into (so events route to the right UI).
    window_label: String,
}

impl PreviewSession {
    /// Stand up the present session for `window`: create the wgpu surface on the
    /// window's handle (plan A1) sized to the window's inner size, and take ownership
    /// of the [`FrameSource`] the compositor decodes through.
    ///
    /// `force_dx12` pins the Windows production backend (the S-2-proven DX12 path);
    /// pass `false` to let wgpu choose (e.g. Vulkan on Linux).
    ///
    /// ## clip_children caveat (residual risk)
    /// Tauri/tao does not expose `with_clip_children(false)`. The project window +
    /// its webview should be declared **transparent** in `tauri.conf.json` so DWM
    /// composites the swapchain under the transparent WebView2 child. If
    /// surface-fighting flicker appears on a target driver, switch this session to
    /// plan A2 (own DirectComposition visual). See the module-level note.
    pub fn init<R: Runtime>(
        window: &WebviewWindow<R>,
        frame_source: FrameSource,
        force_dx12: bool,
    ) -> Result<Self, String> {
        let size = window.inner_size().map_err(|e| format!("inner_size: {e}"))?;
        let (w, h) = (size.width.max(1), size.height.max(1));

        // The Tauri WebviewWindow implements HasWindowHandle + HasDisplayHandle via
        // raw-window-handle 0.6.2 — the SAME handle wgpu 27 consumes (the S-2 seam).
        // We hand the compositor a cheap clonable wrapper so it owns a 'static handle.
        let handle = std::sync::Arc::new(WindowHandleProxy::new(window)?);

        let compositor = Compositor::new_for_surface(handle, w, h, force_dx12)
            .map_err(|e| format!("compositor surface init failed: {e}"))?;

        tracing::info!(
            target: "preview",
            label = %window.label(),
            adapter = %compositor.adapter_summary(),
            "E5-S8 preview surface up (plan A1: wgpu swapchain on the Tauri window HWND)"
        );

        Ok(PreviewSession {
            compositor,
            frame_source,
            window_label: window.label().to_string(),
        })
    }

    /// The window label this session presents into.
    pub fn window_label(&self) -> &str {
        &self.window_label
    }

    /// Resize the present surface (window resize / quality-scale change).
    pub fn resize(&mut self, width: u32, height: u32) {
        self.compositor.resize(width, height);
    }

    /// Render + present a single finalized frame (the transport's
    /// `TransportEvent::Render` payload). Resolves each layer's pixels through the
    /// [`FrameSource`] and presents under the transparent webview.
    pub fn render(&mut self, frame: &RenderFrame) -> Result<(), String> {
        self.compositor
            .render(frame, &self.frame_source)
            .map_err(|e| format!("compositor render failed: {e}"))
    }

    /// Drop cached GPU textures for a media ref (asset removed / source edited).
    pub fn evict_asset(&mut self, media_ref: &str) {
        self.compositor.evict_asset(media_ref);
    }

    /// Apply a batch of [`TransportEvent`]s (the engine transport's output): present
    /// each `Render` frame and emit each `CurrentFrameChanged` to the frontend. Decode
    /// (`SeekDecode`) is served inside `render` by the `FrameSource`; `PlaybackState`
    /// is forwarded for UI sync. Render errors are logged, not propagated, so one bad
    /// frame doesn't tear down playback.
    pub fn apply_events<R: Runtime>(&mut self, app: &AppHandle<R>, events: &[TransportEvent]) {
        for ev in events {
            match ev {
                TransportEvent::Render(frame) => {
                    if let Err(e) = self.render(frame) {
                        tracing::warn!(target: "preview", error = %e, "render frame failed");
                    }
                }
                TransportEvent::CurrentFrameChanged { frame, is_timeline } => {
                    let payload = CurrentFramePayload { frame: *frame, is_timeline: *is_timeline };
                    if let Err(e) = app.emit_to(self.window_label.as_str(), CURRENT_FRAME_EVENT, payload) {
                        tracing::warn!(target: "preview", error = %e, "emit current-frame failed");
                    }
                }
                TransportEvent::PlaybackStateChanged(playing) => {
                    let _ = app.emit_to(
                        self.window_label.as_str(),
                        "preview://playback-state",
                        *playing,
                    );
                }
                // Decode is performed by the FrameSource inside `render`; nothing to do
                // here for the SeekDecode hint (the transport already chose tolerance).
                TransportEvent::SeekDecode { .. } => {}
            }
        }
    }
}

/// Managed-state slot for the (at most one, for now) live preview session. `None`
/// until a project window initializes the preview; a future multi-window/multi-tab
/// model keys this by window label.
#[derive(Default)]
pub struct PreviewState(pub Mutex<Option<PreviewSession>>);

/// A `'static`, `Send + Sync` proxy that re-borrows a Tauri window's raw handles.
///
/// `Compositor::new_for_surface` needs `Arc<W: HasWindowHandle + HasDisplayHandle +
/// 'static>`. A Tauri `WebviewWindow<R>` is `'static` and implements both traits, but
/// is generic over `R`; we capture its **raw** handles once (an HWND on Windows is a
/// stable pointer for the window's lifetime) so the compositor holds a concrete,
/// runtime-agnostic handle. The window must outlive the session (it does — the session
/// is dropped on window close).
struct WindowHandleProxy {
    window: raw_window_handle::RawWindowHandle,
    display: raw_window_handle::RawDisplayHandle,
}

// SAFETY: the captured raw handles are plain pointers/ids (HWND on Windows, X11/Wayland
// ids on Linux). They are valid for the window's lifetime, which outlives the session.
// wgpu only reads them on the calling thread during surface creation/configure.
unsafe impl Send for WindowHandleProxy {}
unsafe impl Sync for WindowHandleProxy {}

impl WindowHandleProxy {
    fn new<R: Runtime>(window: &WebviewWindow<R>) -> Result<Self, String> {
        use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
        let window_handle = window
            .window_handle()
            .map_err(|e| format!("window_handle: {e}"))?
            .as_raw();
        let display = window
            .display_handle()
            .map_err(|e| format!("display_handle: {e}"))?
            .as_raw();
        Ok(WindowHandleProxy { window: window_handle, display })
    }
}

impl raw_window_handle::HasWindowHandle for WindowHandleProxy {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: the handle is valid for the window's lifetime (> session lifetime).
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(self.window) })
    }
}

impl raw_window_handle::HasDisplayHandle for WindowHandleProxy {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: as above.
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(self.display) })
    }
}

// ---------------------------------------------------------------------------------
// Command seam (registered from main.rs). Kept minimal: init the surface for a window,
// and a render/resize/evict trigger. The transport state machine itself is driven by
// the editor commands (E5-S7 / E5-S10); those route their emitted events through
// `PreviewSession::apply_events`. Here we expose the surface lifecycle.
// ---------------------------------------------------------------------------------

/// Initialize the preview present surface for a project `window` (by label). Builds
/// the wgpu compositor on the window handle (plan A1) and stores the session in
/// managed state. `force_dx12` pins the Windows production backend.
///
/// The `FrameSource` is constructed with a resolver that maps a `media_ref` to its
/// source URL; here the resolver is a placeholder that the editor wires to the
/// project's asset table (E2). Returns the adapter summary on success.
#[tauri::command]
pub fn preview_init<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, PreviewState>,
    window_label: String,
    force_dx12: Option<bool>,
) -> Result<String, String> {
    let window = app
        .get_webview_window(&window_label)
        .ok_or_else(|| format!("window '{window_label}' not found"))?;

    // The decode owner. The URL resolver is a placeholder until the editor passes the
    // project's asset table (E2 asset path resolution) — an offline ref yields None,
    // so the compositor simply skips that layer (no crash).
    let frame_source = FrameSource::new(std::sync::Arc::new(|_media_ref: &str| None));

    let session = PreviewSession::init(&window, frame_source, force_dx12.unwrap_or(cfg!(windows)))?;
    let summary = session.compositor.adapter_summary();
    *state.0.lock().unwrap() = Some(session);
    Ok(summary)
}

/// Resize the active preview surface (called on window resize from the frontend).
#[tauri::command]
pub fn preview_resize(state: State<'_, PreviewState>, width: u32, height: u32) -> Result<(), String> {
    if let Some(s) = state.0.lock().unwrap().as_mut() {
        s.resize(width, height);
    }
    Ok(())
}

/// Tear down the active preview surface (window closing). Releases the GPU device.
#[tauri::command]
pub fn preview_teardown(state: State<'_, PreviewState>) {
    *state.0.lock().unwrap() = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_frame_event_name_is_stable() {
        assert_eq!(CURRENT_FRAME_EVENT, "preview://current-frame");
    }

    #[test]
    fn current_frame_payload_serializes_camel_case() {
        let p = CurrentFramePayload { frame: 42, is_timeline: true };
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"frame\":42"));
        assert!(json.contains("\"isTimeline\":true"));
    }

    #[test]
    fn preview_state_starts_empty() {
        let s = PreviewState::default();
        assert!(s.0.lock().unwrap().is_none());
    }
}
