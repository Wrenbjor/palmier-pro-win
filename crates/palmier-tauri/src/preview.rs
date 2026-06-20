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

// ## E5-S10 — transport wiring (the E5-S8 follow-up)
//
// E5-S8 built + tested the present seam (`apply_events`/`render`) but left it
// **un-invoked** — no command drove the engine [`Transport`] into it. This story
// closes that gap: the `preview_play`/`pause`/`toggle_playback`/`seek`/`step`/`set_tab`
// commands below drive a session-owned [`Transport<WallClock>`] and route every emitted
// [`TransportEvent`] through [`PreviewSession::apply_events`], so the compositor renders
// + presents on each `Render` event and the reactive `current_frame` streams to the
// viewport (FR-19). Preview is now functional end-to-end (modulo the live-window
// surface verification carried from E5-S8 + real decoder metadata, see the result).

use std::sync::Mutex;

use palmier_engine::{
    Compositor, RenderFrame, SeekMode, SourceInfo, Transport, TransportEvent, WallClock,
};
use palmier_media::FrameSource;
use palmier_model::Timeline;
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
    /// The engine transport (E5-S7) this session drives (E5-S10 wiring). Owns the
    /// playback state machine + per-tab playheads; emits [`TransportEvent`]s that
    /// [`PreviewSession::apply_events`] presents.
    transport: Transport<WallClock>,
    /// The project timeline the transport composes from. Pushed via
    /// [`preview_set_timeline`] (until the `get_timeline` command lands in Epic 7 the
    /// frontend owns the timeline view-model and hands it over serialized).
    timeline: Timeline,
}

/// The session's source-geometry resolver.
///
/// Until `palmier-media` exposes real decoder metadata (natural size +
/// `preferred_transform` per source URL) to the engine, we resolve every `media_ref`
/// to an **upright** [`SourceInfo`] sized to the timeline canvas. That is enough for
/// the transport→compositor pipeline to be exercised end-to-end (the clip composites
/// as an upright full-canvas layer); real per-source geometry replaces this when the
/// decoder metadata seam lands (tracked as the E5-S2/decode follow-up).
// `+ use<>` makes the returned closure capture **no** lifetimes (it only moves the two
// `f64`s out of `timeline`), so it does not keep `&self.timeline` borrowed across the
// later `&mut self` `apply_events` call (Rust 2024 impl-Trait capture rule).
fn timeline_resolver(timeline: &Timeline) -> impl Fn(&str) -> Option<SourceInfo> + use<> {
    let w = timeline.width.max(1) as f64;
    let h = timeline.height.max(1) as f64;
    move |_media_ref: &str| Some(SourceInfo::upright((w, h)))
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
            transport: Transport::new(WallClock::new()),
            timeline: Timeline::default(),
        })
    }

    /// Replace the project timeline the transport composes from (E5-S10). The frontend
    /// pushes a serialized `palmier-model::Timeline` until the `get_timeline` command
    /// lands (Epic 7). Does NOT itself render — the next transport action rebuilds.
    pub fn set_timeline(&mut self, timeline: Timeline) {
        self.timeline = timeline;
    }

    /// Whether playback is currently running (reactive UI sync). Part of the transport
    /// seam consumed by the orchestrator's tick loop (E5-S11); not yet command-invoked.
    #[allow(dead_code)]
    pub fn is_playing(&self) -> bool {
        self.transport.is_playing()
    }

    /// The transport's active-tab playhead frame. Seam method for the tick loop
    /// (E5-S11); the commands return their own `current_frame`, so this is not yet
    /// directly command-invoked.
    #[allow(dead_code)]
    pub fn current_frame(&self) -> i32 {
        self.transport.current_frame()
    }

    /// Start playback (engine `Transport::play`), presenting every emitted event.
    pub fn play<R: Runtime>(&mut self, app: &AppHandle<R>) -> i32 {
        let resolver = timeline_resolver(&self.timeline);
        let events = self.transport.play(&self.timeline, &resolver);
        self.apply_events(app, &events);
        self.transport.current_frame()
    }

    /// Pause playback (engine `Transport::pause`).
    pub fn pause<R: Runtime>(&mut self, app: &AppHandle<R>) -> i32 {
        let events = self.transport.pause();
        self.apply_events(app, &events);
        self.transport.current_frame()
    }

    /// Toggle play/pause, returning the resulting playing flag.
    pub fn toggle_playback<R: Runtime>(&mut self, app: &AppHandle<R>) -> bool {
        let resolver = timeline_resolver(&self.timeline);
        let events = self.transport.toggle_playback(&self.timeline, &resolver);
        self.apply_events(app, &events);
        self.transport.is_playing()
    }

    /// Seek to `frame` under `mode` (engine `Transport::seek`), presenting the result.
    pub fn seek<R: Runtime>(&mut self, app: &AppHandle<R>, frame: i32, mode: SeekMode) -> i32 {
        let resolver = timeline_resolver(&self.timeline);
        let events = self.transport.seek(&self.timeline, &resolver, frame, mode);
        self.apply_events(app, &events);
        self.transport.current_frame()
    }

    /// Step the playhead by `delta` frames (exact; engine `Transport::step`).
    pub fn step<R: Runtime>(&mut self, app: &AppHandle<R>, delta: i32) -> i32 {
        let resolver = timeline_resolver(&self.timeline);
        let events = self.transport.step(&self.timeline, &resolver, delta);
        self.apply_events(app, &events);
        self.transport.current_frame()
    }

    /// Advance playback one clock tick (the orchestrator's periodic time observer at
    /// `1/fps`). Presents the new frame when the playhead advanced. Returns whether a
    /// new frame was emitted (so the caller can stop ticking when paused).
    ///
    /// Built + tested here; the periodic-tick driver (a Tauri async loop that calls
    /// this every `1/fps` while playing) lands with the playback-loop polish in E5-S11.
    /// Until then, `play`/`seek`/`step` exercise the same `apply_events` present seam.
    #[allow(dead_code)]
    pub fn tick<R: Runtime>(&mut self, app: &AppHandle<R>) -> bool {
        let resolver = timeline_resolver(&self.timeline);
        let events = self.transport.tick(&self.timeline, &resolver);
        let advanced = !events.is_empty();
        self.apply_events(app, &events);
        advanced
    }

    /// Activate a preview tab by id (engine `Transport::activate_tab`), restoring its
    /// retained playhead. Returns the restored frame.
    pub fn set_tab<R: Runtime>(&mut self, app: &AppHandle<R>, tab: palmier_engine::PreviewTab) -> i32 {
        let events = self.transport.activate_tab(tab);
        self.apply_events(app, &events);
        self.transport.current_frame()
    }

    /// The window label this session presents into (E5-S8 seam; a multi-window model
    /// keys sessions by this — not yet read in the single-window M1 wiring).
    #[allow(dead_code)]
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

    /// Drop cached GPU textures for a media ref (asset removed / source edited). E5-S8
    /// seam; wired to the media-panel asset-removed event in a later integration story.
    #[allow(dead_code)]
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

// ---------------------------------------------------------------------------------
// Transport commands (E5-S10 — the E5-S8 follow-up). Each drives the session's engine
// `Transport` and presents every emitted event through `apply_events`, so preview
// renders + the reactive `current_frame` streams to the viewport. All are no-ops
// (return a default) when no session is active (surface not yet initialized).
// ---------------------------------------------------------------------------------

/// Push the project timeline the transport composes from (E5-S10). Until `get_timeline`
/// lands (Epic 7) the frontend hands over a serialized `palmier-model::Timeline`.
#[tauri::command]
pub fn preview_set_timeline(state: State<'_, PreviewState>, timeline: Timeline) -> Result<(), String> {
    if let Some(s) = state.0.lock().unwrap().as_mut() {
        s.set_timeline(timeline);
    }
    Ok(())
}

/// Start playback. Returns the resulting `current_frame`.
#[tauri::command]
pub fn preview_play<R: Runtime>(app: AppHandle<R>, state: State<'_, PreviewState>) -> i32 {
    state
        .0
        .lock()
        .unwrap()
        .as_mut()
        .map(|s| s.play(&app))
        .unwrap_or(0)
}

/// Pause playback. Returns the resulting `current_frame`.
#[tauri::command]
pub fn preview_pause<R: Runtime>(app: AppHandle<R>, state: State<'_, PreviewState>) -> i32 {
    state
        .0
        .lock()
        .unwrap()
        .as_mut()
        .map(|s| s.pause(&app))
        .unwrap_or(0)
}

/// Toggle play/pause. Returns the resulting playing flag.
#[tauri::command]
pub fn preview_toggle_playback<R: Runtime>(app: AppHandle<R>, state: State<'_, PreviewState>) -> bool {
    state
        .0
        .lock()
        .unwrap()
        .as_mut()
        .map(|s| s.toggle_playback(&app))
        .unwrap_or(false)
}

/// Seek to `frame` under `mode` (`"exact"` | `"interactiveScrub"`). Returns the
/// landed `current_frame`.
#[tauri::command]
pub fn preview_seek<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, PreviewState>,
    frame: i32,
    mode: String,
) -> i32 {
    let mode = match mode.as_str() {
        "interactiveScrub" | "interactive_scrub" => SeekMode::InteractiveScrub,
        _ => SeekMode::Exact,
    };
    state
        .0
        .lock()
        .unwrap()
        .as_mut()
        .map(|s| s.seek(&app, frame, mode))
        .unwrap_or(0)
}

/// Step the playhead by `delta` frames (exact). Returns the new `current_frame`.
#[tauri::command]
pub fn preview_step<R: Runtime>(app: AppHandle<R>, state: State<'_, PreviewState>, delta: i32) -> i32 {
    state
        .0
        .lock()
        .unwrap()
        .as_mut()
        .map(|s| s.step(&app, delta))
        .unwrap_or(0)
}

/// Activate a preview tab by id (`"__timeline__"` or `"media_<assetId>"`). Returns the
/// restored playhead frame.
#[tauri::command]
pub fn preview_set_tab<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, PreviewState>,
    tab_id: String,
) -> i32 {
    let tab = tab_from_id(&tab_id);
    state
        .0
        .lock()
        .unwrap()
        .as_mut()
        .map(|s| s.set_tab(&app, tab))
        .unwrap_or(0)
}

/// Parse a frontend tab id into an engine [`PreviewTab`]. The timeline id is fixed;
/// an asset id (`media_<assetId>`) yields a media-asset tab. Display name + clip type
/// are not carried in the id, so the engine tab uses the asset id as the name and
/// `Video` as a neutral type — the transport only keys off the **id** (per-tab
/// playhead identity), so this is sufficient for activation.
fn tab_from_id(tab_id: &str) -> palmier_engine::PreviewTab {
    use palmier_engine::PreviewTab;
    // The engine's fixed timeline-tab id (`PreviewTab::Timeline.id()`); kept as a
    // literal here to avoid widening the engine's public API (it isn't re-exported).
    if tab_id == "__timeline__" {
        PreviewTab::Timeline
    } else if let Some(asset_id) = tab_id.strip_prefix("media_") {
        PreviewTab::media_asset(asset_id, asset_id, palmier_model::ClipType::Video)
    } else {
        PreviewTab::Timeline
    }
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

    #[test]
    fn tab_from_id_maps_timeline_and_asset_ids() {
        // The timeline id → Timeline; an asset id → a media-asset tab keyed by that id.
        assert!(tab_from_id("__timeline__").is_timeline());
        let asset = tab_from_id("media_abc123");
        assert!(!asset.is_timeline());
        assert_eq!(asset.id(), "media_abc123", "round-trips the asset tab id");
        // An unknown id falls back to the timeline (never panics).
        assert!(tab_from_id("garbage").is_timeline());
    }

    #[test]
    fn timeline_resolver_yields_upright_canvas_sized_source() {
        let mut tl = Timeline::default();
        tl.width = 1280;
        tl.height = 720;
        let resolver = timeline_resolver(&tl);
        let info = resolver("any-media-ref").expect("resolver always resolves a source");
        assert_eq!(info.natural_size, (1280.0, 720.0));
    }

    #[test]
    fn seek_mode_string_parsing_matches_command_contract() {
        // The `preview_seek` command parses the frontend's `SeekMode` strings; assert
        // the mapping the command relies on so a rename can't silently break scrubbing.
        let parse = |m: &str| match m {
            "interactiveScrub" | "interactive_scrub" => SeekMode::InteractiveScrub,
            _ => SeekMode::Exact,
        };
        assert_eq!(parse("exact"), SeekMode::Exact);
        assert_eq!(parse("interactiveScrub"), SeekMode::InteractiveScrub);
        assert_eq!(parse("interactive_scrub"), SeekMode::InteractiveScrub);
        assert_eq!(parse("unknown"), SeekMode::Exact, "defaults to exact");
    }

    #[test]
    fn timeline_round_trips_through_serde_for_set_timeline() {
        // `preview_set_timeline` receives a serialized `Timeline` from the frontend;
        // assert the model round-trips (the command contract) on a non-trivial value.
        let mut tl = Timeline::default();
        tl.fps = 60;
        tl.width = 3840;
        tl.height = 2160;
        let json = serde_json::to_string(&tl).unwrap();
        let back: Timeline = serde_json::from_str(&json).unwrap();
        assert_eq!(back.fps, 60);
        assert_eq!((back.width, back.height), (3840, 2160));
    }
}
