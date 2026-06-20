//! Spike S-1 — how a Rust `wgpu`-rendered GPU texture reaches the screen inside/over
//! the Tauri WebView. This crate is the executable half of the spike; `FINDINGS.md`
//! is the analytical half.
//!
//! It contains three things:
//!   1. [`render`] — a real, runnable headless wgpu pipeline that composites a moving
//!      coloured quad to an offscreen texture. This is the exact shape of the preview
//!      compositor's output (`palmier-engine` E5-S8): a `wgpu::Texture` we own.
//!   2. [`readback`] — the candidate-(c) fallback: copy that texture to a CPU buffer
//!      (`copy_texture_to_buffer` + `map_async`). This is what we'd push over Tauri IPC
//!      to a `<canvas>` if no zero-copy path is available. The `readback_proof` bin runs
//!      it end-to-end and prints timing — a real measurement of the fallback's cost.
//!   3. [`present`] — the *recommended* zero-copy presentation seam, per platform:
//!      - Windows: [`present::windows`] — DirectComposition visual fed by a wgpu dx12
//!        surface created `DxgiFromVisual`, composited UNDER a transparent WebView2
//!        visual in the same DComp visual tree.
//!      - Linux: [`present::linux`] — a native GL/Vulkan child surface (e.g. a
//!        `GtkGLArea`) parented in the same `gtk::Fixed` as the WRY webview, z-ordered
//!        below a transparent WebKitGTK webview.
//!      Both are written as the concrete API path Epic 5 (E5-S8) wires up. They are
//!      documented seams, not live-window demos, because spinning a real Tauri window +
//!      WebView2/WebKitGTK is outside a headless spike's reach — but every wgpu call,
//!      surface-target variant, and OS interop call is named so the build is unambiguous.

pub mod render;
pub mod readback;
pub mod present;

/// Output of the headless render: the GPU texture we own + its dimensions/format.
/// In production this is the `palmier-engine` composited frame that must reach the
/// webview viewport — the whole subject of this spike.
pub struct RenderedFrame {
    pub texture: wgpu::Texture,
    pub width: u32,
    pub height: u32,
    pub format: wgpu::TextureFormat,
}

/// The presentation mechanism this spike recommends Epic 5 build on, per platform.
/// Mirrors the three R-1 candidates so the decision is explicit in code, not just prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationMechanism {
    /// (a)/(c) recommended: native GPU surface composited with the webview by the OS
    /// compositor — DirectComposition (Windows) / GTK child surface (Linux). Zero-copy.
    NativeCompositedSurface,
    /// (b) rejected for v1: hand the texture to the webview's own WebGPU/canvas context
    /// via a shared D3D handle. Not supported by WebView2's stable surface — see FINDINGS.
    SharedHandleIntoCanvas,
    /// (c) fallback only: read the frame back to CPU and push to a `<canvas>` over IPC.
    /// Triggered when a/c are unavailable (driver/GPU-floor failure). Perf cliff.
    IpcReadback,
}

impl PresentationMechanism {
    /// The spike's recommendation for the given target OS.
    pub const fn recommended_for(target_os: TargetOs) -> Self {
        match target_os {
            TargetOs::Windows | TargetOs::Linux => Self::NativeCompositedSurface,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    Windows,
    Linux,
}
