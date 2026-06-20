//! The wgpu compositor — E5-S8 (FOUNDATION §6.5 step 3, reconciliation #22/#23).
//!
//! Turns a [`RenderFrame`](crate::RenderFrame) (the transport's
//! `TransportEvent::Render` payload) into composited pixels: clear to black, then
//! draw each [`LayerRender`](crate::LayerRender) bottom→top as a **textured quad**
//! with its [`Mat3`](crate::Mat3) affine, [`CropRect`](crate::CropRect), opacity, and
//! **premultiplied-alpha blend**. Color is a single BT.709 working space (risk #5);
//! straight-alpha sources are premultiplied on upload so edges don't fringe (risk #3).
//!
//! ## Module split (pure vs. GPU)
//! The pure, headless-testable math is **always compiled** so it unit-tests without a
//! GPU; only the wgpu device/pipeline lands behind the `wgpu-compositor` feature:
//! - [`quad`] — [`Mat3`](crate::Mat3) → clip-space `mat4`, [`CropRect`](crate::CropRect)
//!   → UV rect, quad corners. (pure)
//! - [`pixels`] — decoded planes → premultiplied `Rgba8` (YUV→RGB BT.709 + premul). (pure)
//! - [`texture_cache`] — the 1.5 GB VRAM LRU texture cache, generic over the texture
//!   handle so eviction is tested with a fake. (pure)
//! - [`gpu`] — the `Compositor`: wgpu device + textured-quad pipeline + present,
//!   on-screen (surface on a window handle, the S-2 plan-A1 seam) or headless
//!   (offscreen texture). **(feature `wgpu-compositor`)**
//!
//! ## What S-2 proved (the present seam)
//! `spikes/s2-wry-integration/FINDINGS.md`: a wgpu swapchain on a window's HWND
//! composites UNDER a transparent WebView2 child (`build_as_child`,
//! `with_clip_children(false)`) by DWM — zero readback, zero IPC. The on-screen
//! [`Compositor`](gpu::Compositor) reproduces the wgpu side of that; the present
//! integration into the real Tauri window lives in `palmier-tauri`'s preview module.

pub mod pixels;
pub mod provider;
pub mod quad;
pub mod texture_cache;

pub use pixels::{decoded_to_rgba, RgbaImage};
pub use provider::FrameProvider;
pub use quad::{crop_corners, crop_uv_rect, layer_clip_matrix, CanvasSize, UvRect};
pub use texture_cache::{TexCacheStats, TexKey, TextureCache, DEFAULT_VRAM_CEILING_BYTES};

#[cfg(feature = "wgpu-compositor")]
pub mod gpu;
#[cfg(feature = "wgpu-compositor")]
pub mod text_pass;
#[cfg(feature = "wgpu-compositor")]
pub use gpu::{Compositor, CompositorError};
#[cfg(feature = "wgpu-compositor")]
pub use text_pass::TextPass;
