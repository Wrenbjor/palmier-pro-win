//! The recommended presentation seam — how the Rust-owned wgpu frame reaches the screen
//! under/over the Tauri WebView, per platform. These are the *concrete API paths* Epic 5
//! (E5-S8) wires up.
//!
//! Why a documented seam and not a live window here: a real proof needs a running Tauri
//! window hosting WebView2 (Win) / WebKitGTK (Linux) with a sibling native GPU surface,
//! which is an app-shell concern (palmier-tauri) outside a headless spike. What the spike
//! CAN do — and does — is (1) prove the wgpu producer side runs on the right backend, (2)
//! measure the readback fallback, and (3) pin down, call-by-call, the zero-copy seam so
//! there is no ambiguity left for E5-S8. Every wgpu surface-target variant and OS interop
//! call below is named precisely.

use crate::{PresentationMechanism, TargetOs};

/// Geometry of the preview viewport rectangle inside the webview, in physical pixels.
/// The native surface is sized/positioned to this rect; on zoom/scroll the webview reports
/// a new rect (mirrors the reference `PreviewNSView` `videoRect` + cmd-scroll zoom).
#[derive(Debug, Clone, Copy)]
pub struct ViewportRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// The decision record this spike produces, in code form, for E5-S8 to consume.
#[derive(Debug, Clone)]
pub struct PresentationPlan {
    pub target_os: TargetOs,
    pub mechanism: PresentationMechanism,
    /// Whether the timeline canvas shares this surface or stays a webview-side WebGPU
    /// canvas (preview-engine.md open question). Spike answer: preview = native surface;
    /// timeline canvas stays in the webview (it's cheap 2D, not the perf-critical path).
    pub timeline_shares_surface: bool,
    pub notes: &'static str,
}

pub fn plan_for(target_os: TargetOs) -> PresentationPlan {
    PresentationPlan {
        target_os,
        mechanism: PresentationMechanism::recommended_for(target_os),
        timeline_shares_surface: false,
        notes: match target_os {
            TargetOs::Windows => windows::SEAM_NOTES,
            TargetOs::Linux => linux::SEAM_NOTES,
        },
    }
}

/// Windows seam: DirectComposition visual fed by a wgpu dx12 `DxgiFromVisual` surface,
/// composited UNDER a transparent WebView2 visual.
pub mod windows {
    use super::ViewportRect;

    pub const SEAM_NOTES: &str = "\
Windows (D3D12): wgpu dx12 surface -> IDCompositionVisual UNDER transparent WebView2.";

    /// The exact call path. This is documentation-as-code: the steps E5-S8 implements in
    /// `palmier-tauri`. Each line maps to a real `windows`-crate / wgpu call.
    ///
    /// 1. Build the wgpu instance with DComp presentation enabled:
    ///    ```ignore
    ///    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
    ///        backends: wgpu::Backends::DX12,
    ///        backend_options: wgpu::BackendOptions {
    ///            dx12: wgpu::Dx12BackendOptions {
    ///                // first-class DComp support landed in wgpu 27 (2025-10-01)
    ///                presentation_system: wgpu::Dx12SwapchainKind::DxgiFromVisual,
    ///                ..Default::default()
    ///            },
    ///            ..Default::default()
    ///        },
    ///        ..Default::default()
    ///    });
    ///    ```
    /// 2. Create a DirectComposition device + target on the Tauri window HWND
    ///    (`DCompositionCreateDevice` -> `IDCompositionDevice::CreateTargetForHwnd`).
    /// 3. Create the wgpu surface bound to a composition visual:
    ///    ```ignore
    ///    // create an IDCompositionVisual for the preview, then:
    ///    let surface = unsafe {
    ///        instance.create_surface_unsafe(
    ///            wgpu::SurfaceTargetUnsafe::CompositionVisual(visual_ptr),
    ///        )?
    ///    };
    ///    ```
    ///    (wgpu 27 `SurfaceTargetUnsafe::CompositionVisual` — pass the `IDCompositionVisual`).
    /// 4. Build the DComp visual tree: root <- [preview_visual (wgpu surface, z=0),
    ///    webview_visual (z=1, ABOVE)]. `IDCompositionVisual::AddVisual`.
    /// 5. Host WebView2 as a composition visual via
    ///    `ICoreWebView2Environment::CreateCoreWebView2CompositionController` and connect
    ///    its `RootVisualTarget` into the webview_visual. WRY already creates WebView2 in
    ///    composition-hosted (windowless/visual) mode on Tauri 2, so this visual exists.
    /// 6. Make the webview transparent: WebView2 in composition mode renders no opaque
    ///    background when the page background is transparent; the preview visual shows
    ///    through the viewport rect. (NOTE: `DefaultBackgroundColor` does NOT apply to
    ///    composition controllers — transparency comes from the page + the visual having
    ///    no opaque fill. Confirmed via WebView2 docs.)
    /// 7. Per frame: render into the surface's `get_current_texture()`, `submit`,
    ///    `SurfaceTexture::present()`, then `IDCompositionDevice::Commit()`. The OS DWM
    ///    compositor merges preview + webview — zero CPU copy, no swapchain fighting.
    /// 8. On viewport move/resize/zoom: set the preview visual's transform/clip to the new
    ///    `ViewportRect` (`IDCompositionVisual::SetTransform` / `SetClip`) and reconfigure
    ///    the surface size. No webview round-trip needed for geometry.
    pub fn seam_call_path(_viewport: ViewportRect) -> &'static str {
        // Body intentionally a no-op: the value is the documented call path above, which
        // E5-S8 implements against a live HWND. Keeping it compile-checked guards the
        // module from bit-rot.
        SEAM_NOTES
    }
}

/// Linux seam: a native GL/Vulkan child surface parented in the same GTK container as the
/// WRY webview, z-ordered below a transparent WebKitGTK webview.
pub mod linux {
    use super::ViewportRect;

    pub const SEAM_NOTES: &str = "\
Linux (Vulkan/GL): native GtkGLArea child UNDER transparent WebKitGTK webview in a gtk::Fixed.";

    /// The exact call path E5-S8 implements in `palmier-tauri` on Linux:
    ///
    /// 1. Get the Tauri window's `gtk::ApplicationWindow` (`window.gtk_window()` via
    ///    tao/Tauri) and its content `gtk::Fixed`/overlay container. WRY builds its
    ///    WebKitGTK webview into this container (`WebViewBuilder::build_gtk(&fixed)`).
    /// 2. Add a sibling native render widget in the SAME container, positioned UNDER the
    ///    webview: a `GtkGLArea` (GL) — or a `gtk::DrawingArea` whose native surface is
    ///    used for a Vulkan swapchain. Z-order: native child below, webview above
    ///    (`gtk::Fixed::put` order / `gtk_widget_set_child_above_sibling`).
    /// 3. Create the wgpu surface from the native child's window handle:
    ///    ```ignore
    ///    let surface = unsafe {
    ///        instance.create_surface_unsafe(
    ///            wgpu::SurfaceTargetUnsafe::from_window(&gl_area_raw_handle)?
    ///        )?  // Vulkan (Wayland/X11) raw_window_handle path
    ///    };
    ///    ```
    ///    wgpu picks the Vulkan backend; `SurfaceTargetUnsafe::RawHandle { raw_display_handle,
    ///    raw_window_handle }` carries the Wayland/X11 surface (display handle REQUIRED on Linux).
    /// 4. Make WebKitGTK transparent: `webkit_web_view_set_background_color(rgba a=0)` +
    ///    page `background: transparent`. The native child shows through the viewport rect.
    ///    KNOWN RISK: WebKitGTK's DMABUF renderer + some NVIDIA drivers mishandle
    ///    transparent surfaces (tauri#14924). Mitigation: set
    ///    `WEBKIT_DISABLE_DMABUF_RENDERER=1` (also the documented workaround for tauri#9220
    ///    flicker) — costs some webview perf but stabilises compositing. Validate per-driver.
    /// 5. Per frame: render into `surface.get_current_texture()`, `submit`, `present()`.
    ///    GTK/the X11/Wayland compositor merges the child surface with the webview above it.
    /// 6. On viewport move/zoom: reposition/resize the native child in the `gtk::Fixed`
    ///    and reconfigure the surface — mirrors the Windows visual transform.
    ///
    /// FALLBACK NOTE: where transparent-webview compositing is unstable on a given Linux
    /// GPU/driver, drop to candidate (c) IPC readback (see `crate::readback`) for that
    /// machine — the GPU-floor (FOUNDATION §3) CPU-compositing branch already accepts a
    /// degraded preview.
    pub fn seam_call_path(_viewport: ViewportRect) -> &'static str {
        SEAM_NOTES
    }
}
