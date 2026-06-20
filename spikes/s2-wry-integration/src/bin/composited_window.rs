//! S-2 — LIVE composited window: a wgpu frame under a transparent WRY WebView2.
//!
//! Layout it produces (the thing E5-S8 needs to be true):
//!   - A transparent winit window.
//!   - A wgpu swapchain ON that window's HWND, drawing a pulsing magenta clear + a
//!     green triangle (the stand-in for the `palmier-engine` preview compositor output).
//!   - A WRY webview parented as a CHILD over the window via `build_as_child`, with a
//!     SEMI-TRANSPARENT top chrome bar and a fully TRANSPARENT "viewport hole" in the
//!     middle. Through that hole the wgpu magenta+triangle shows; the chrome sits on top.
//!   - The OS compositor (DWM) merges the two HWNDs — zero readback, zero IPC.
//!
//! This is the WRY-realized form of S-1's recommended mechanism. See FINDINGS.md for how
//! it maps to plan A (DComp visual) / plan B (separate child window) / plan C (readback).
//!
//! Run modes:
//!   (default)                      open the window; render until closed.
//!   S2_MAX_FRAMES=<n>              render n frames then exit 0 (drives screenshot/CI).
//!   S2_FORCE_DX12=1                force the wgpu DX12 backend (Windows production path).
//!   S2_SMOKE=1                     construct GPU + webview, render a few frames headless-
//!                                  friendly, print diagnostics, exit — for build-verify.

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{Rect, WebViewBuilder};

use s2_wry_integration::GfxState;

/// The webview page: a translucent top chrome bar (so we can SEE the webview is real and
/// on top) + a transparent body so the wgpu layer shows through everywhere else. The
/// label text reports what S-2 is proving.
const PAGE: &str = r#"<!doctype html><html><head><meta charset="utf-8"><style>
  html,body { margin:0; height:100%; background:transparent; overflow:hidden;
              font-family: Segoe UI, system-ui, sans-serif; color:#fff; }
  .chrome { position:fixed; top:0; left:0; right:0; height:64px;
            background:rgba(20,22,40,0.78); backdrop-filter:blur(6px);
            display:flex; align-items:center; padding:0 18px; gap:14px;
            box-shadow:0 2px 12px rgba(0,0,0,0.5); }
  .dot { width:12px; height:12px; border-radius:50%; background:#39d98a; }
  .title { font-weight:600; font-size:15px; }
  .sub { font-size:12px; opacity:0.7; }
  .hole-label { position:fixed; top:80px; left:18px; font-size:13px;
                background:rgba(0,0,0,0.45); padding:6px 10px; border-radius:6px; }
</style></head><body>
  <div class="chrome">
    <div class="dot"></div>
    <div>
      <div class="title">S-2 WebView chrome (WRY / WebView2)</div>
      <div class="sub">This bar is the webview, ON TOP. The magenta + green triangle behind it is wgpu.</div>
    </div>
  </div>
  <div class="hole-label">transparent viewport &mdash; GPU frame shows through &darr;</div>
</body></html>"#;

struct App {
    window: Option<Arc<Window>>,
    webview: Option<wry::WebView>,
    gfx: Option<GfxState>,
    frame: u32,
    max_frames: Option<u32>,
    force_dx12: bool,
    smoke: bool,
    start: Instant,
    printed_summary: bool,
}

impl App {
    fn new() -> Self {
        let max_frames = std::env::var("S2_MAX_FRAMES")
            .ok()
            .and_then(|v| v.parse::<u32>().ok());
        let force_dx12 = std::env::var("S2_FORCE_DX12").map(|v| v == "1").unwrap_or(false);
        let smoke = std::env::var("S2_SMOKE").map(|v| v == "1").unwrap_or(false);
        App {
            window: None,
            webview: None,
            gfx: None,
            frame: 0,
            // smoke mode auto-caps frames so it always terminates.
            max_frames: if smoke { Some(max_frames.unwrap_or(8)) } else { max_frames },
            force_dx12,
            smoke,
            start: Instant::now(),
            printed_summary: false,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // already initialized (resumed can fire more than once)
        }

        let mut attrs = Window::default_attributes()
            .with_title("S-2 wgpu-under-WebView composite")
            .with_inner_size(LogicalSize::new(1100.0, 700.0))
            .with_transparent(true);

        #[cfg(windows)]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            // CRITICAL on Windows: without disabling child clipping, the parent window
            // clips the wgpu swapchain against the child webview HWND and you get the
            // tauri#9220 "fighting for the surface" black/flicker. This is the documented
            // fix from the wry wgpu example.
            attrs = attrs.with_clip_children(false);
        }

        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("[S2] create_window failed: {e:?}");
                event_loop.exit();
                return;
            }
        };

        // 1) wgpu producer surface ON the window.
        let gfx = match GfxState::new(Arc::clone(&window), self.force_dx12) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[S2] FATAL GfxState::new: {e}");
                event_loop.exit();
                return;
            }
        };

        // 2) transparent WRY webview parented as a CHILD over the same window. This is
        // the load-bearing call: WRY consumes the winit window's raw-window-handle 0.6
        // HasWindowHandle (the SAME handle wgpu used) and creates the WebView2 child.
        let size = window.inner_size();
        let webview = WebViewBuilder::new()
            .with_transparent(true)
            .with_bounds(Rect {
                position: LogicalPosition::new(0, 0).into(),
                size: LogicalSize::new(size.width, size.height).into(),
            })
            .with_html(PAGE)
            .build_as_child(&window);

        let webview = match webview {
            Ok(wv) => wv,
            Err(e) => {
                eprintln!("[S2] FATAL build_as_child (WRY did NOT expose a usable child seam): {e:?}");
                eprintln!("[S2] => plan A via build_as_child is BLOCKED on this stack; see FINDINGS.md plan B.");
                event_loop.exit();
                return;
            }
        };

        println!("[S2] adapter: {}", gfx.adapter_summary());
        println!("[S2] webview child built over window (build_as_child OK).");
        println!(
            "[S2] mode: max_frames={:?} force_dx12={} smoke={}",
            self.max_frames, self.force_dx12, self.smoke
        );

        self.window = Some(window.clone());
        self.gfx = Some(gfx);
        self.webview = Some(webview);
        self.printed_summary = true;

        window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.resize(size.width, size.height);
                }
                if let (Some(wv), _) = (self.webview.as_ref(), ()) {
                    let _ = wv.set_bounds(Rect {
                        position: LogicalPosition::new(0, 0).into(),
                        size: LogicalSize::new(size.width, size.height).into(),
                    });
                }
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(gfx) = self.gfx.as_mut() {
                    match gfx.render(self.frame) {
                        Ok(()) => {}
                        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                            let (w, h) = (gfx.config.width, gfx.config.height);
                            gfx.resize(w, h);
                        }
                        Err(e) => eprintln!("[S2] render error: {e:?}"),
                    }
                }
                self.frame = self.frame.saturating_add(1);

                if let Some(maxf) = self.max_frames {
                    if self.frame >= maxf {
                        println!(
                            "[S2] rendered {} frames in {:?} — exiting (max_frames).",
                            self.frame,
                            self.start.elapsed()
                        );
                        println!("[S2] VERDICT: window stood up; wgpu surface + transparent WRY child coexisted; frames presented without panic.");
                        event_loop.exit();
                        return;
                    }
                }

                // keep the loop alive (continuous animation)
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // On Linux the WebKitGTK webview needs the GTK loop pumped (mirrors the wry
        // example). No-op on Windows.
        #[cfg(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd",
            target_os = "openbsd",
        ))]
        {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
        }
    }
}

fn main() {
    let event_loop = EventLoop::new().expect("event loop");
    // Poll so the animation runs continuously and max_frames terminates promptly.
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::new();
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("[S2] event loop error: {e:?}");
        std::process::exit(1);
    }
    if !app.printed_summary {
        eprintln!("[S2] WARNING: app never fully initialized (resumed did not complete).");
        std::process::exit(2);
    }
}
