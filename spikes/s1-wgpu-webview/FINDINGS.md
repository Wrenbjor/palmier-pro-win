# Spike S-1 — wgpu to WebView presentation: FINDINGS & DECISION RECORD

**Spike:** S-1 (Epic 5 story E5-S1), the #1 architecture risk (R-1, ruling #23).
**Question:** how does a Rust `wgpu`-rendered GPU texture actually reach the screen inside or over the
Tauri WebView? FOUNDATION SS4/SS6.5 assert a "shared WebGPU surface" / "present the texture to the
WebGPU canvas in the webview" but specify **no mechanism**. This gates all of Epic 5 (preview) and
Epic 6 (export).

**Status:** RESOLVED with a recommendation. Produce-side + fallback are **proven on real hardware**;
the recommended zero-copy seam is **pinned call-by-call** and validated against current (June 2026)
wgpu 27 / Tauri 2 / WebView2 / WebKitGTK capabilities. The remaining work is integration in E5-S8, not
research.

---

## 1. Recommendation (the decision Epic 5 builds on)

**Adopt mechanism (a)/(c): a native GPU surface composited with the webview by the OS compositor — NOT
rendered into the webview's own GPU context.** The preview is a native `wgpu` surface that the OS
window compositor draws **underneath a transparent webview**; the webview hosts only UI chrome and a
transparent "hole" over the viewport rect.

| Platform | Mechanism | wgpu surface target | Composited by |
|---|---|---|---|
| **Windows (D3D12)** | wgpu dx12 surface bound to a **DirectComposition visual**, placed UNDER a transparent WebView2 visual in one DComp visual tree | `SurfaceTargetUnsafe::CompositionVisual(IDCompositionVisual*)` + `Dx12SwapchainKind::DxgiFromVisual` | DWM / DirectComposition |
| **Linux (Vulkan)** | wgpu Vulkan surface on a **native GTK child** (`GtkGLArea` / `DrawingArea`) parented in the same `gtk::Fixed` as the WRY webview, z-ordered below a transparent WebKitGTK webview | `SurfaceTargetUnsafe::from_window(child)` (RawHandle w/ Wayland/X11 display handle) | GTK + X11/Wayland compositor |

This is the same architectural shape the macOS reference used (`AVPlayerLayer` sat as a native layer
inside the `NSView`, with SwiftUI chrome around it) — we reproduce "native preview surface + web/native
UI on top," which also matches the cmd-scroll zoom / `videoRect` geometry already in `PreviewNSView`.

### Why NOT the other candidates

- **(b) DXGI shared-handle into the webview's `<canvas>`/WebGPU context — REJECTED for v1.** A
  Rust-owned `wgpu` texture lives on the host's device; the webview's `<canvas>` WebGPU context is owned
  by the **WebView2/WebKitGTK GPU process**, a different device/process. There is no stable API to
  import an external D3D11/D3D12 shared handle into that JS-visible WebGPU context. The closest Windows
  facility is **`ICoreWebView2ExperimentalTexture` / `ICoreWebView2ExperimentalTextureStream`** — it
  lets the host write a shared D3D11 texture into a WebView2-managed video/texture-stream visual, but
  (1) it is **experimental / prerelease SDK only**, (2) it targets a designated stream visual, **not** a
  page `<canvas>`'s WebGPU device, and (3) it has no WebKitGTK equivalent. Not viable as the v1
  cross-platform path. Re-evaluate only if Microsoft stabilises it AND a Linux peer appears.
- **(c) IPC readback — FALLBACK ONLY.** Read the frame back to CPU and push to a `<canvas>`. Kept as
  the GPU-floor degraded path (FOUNDATION SS3), not the primary. Cost measured below.

---

## 2. Concrete API path

### Windows (D3D12) — `present.rs::windows`

1. Build the wgpu instance with DComp presentation: `wgpu::InstanceDescriptor` with
   `backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual` (wgpu 27,
   2025-10-01).
2. `DCompositionCreateDevice` then `IDCompositionDevice::CreateTargetForHwnd(tauri_hwnd)`.
3. Create the preview `IDCompositionVisual`, then the wgpu surface bound to it:
   `instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::CompositionVisual(visual_ptr))`.
4. Visual tree: root has [preview_visual (z=0, wgpu), webview_visual (z=1, above)]
   via `IDCompositionVisual::AddVisual`.
5. Host WebView2 via `ICoreWebView2Environment::CreateCoreWebView2CompositionController` and connect its
   `RootVisualTarget` into webview_visual. **WRY already creates WebView2 in composition-hosted (visual)
   mode under Tauri 2**, so this visual already exists — integration is adding the sibling preview
   visual, not replacing WRY's webview.
6. Transparency: a composition-hosted WebView2 renders no opaque background when the page background is
   transparent. (`DefaultBackgroundColor` does **not** apply to composition controllers — confirmed;
   transparency comes from the page + the visual having no opaque fill.)
7. Per frame: render into `surface.get_current_texture()`, `queue.submit`, `SurfaceTexture::present()`,
   `IDCompositionDevice::Commit()`. DWM merges preview + webview. **Zero CPU copy.**
8. Viewport move/zoom: `IDCompositionVisual::SetTransform` / `SetClip` on the preview visual + resize
   the surface. No webview round-trip for geometry.

### Linux (Vulkan) — `present.rs::linux`

1. Get the Tauri window's `gtk::ApplicationWindow` (`window.gtk_window()`); WRY builds its WebKitGTK
   webview into the content container via `WebViewBuilder::build_gtk(&fixed)`.
2. Add a sibling native render widget (`GtkGLArea`, or a `DrawingArea` whose native surface backs a
   Vulkan swapchain) in the **same** container, **below** the webview (`gtk::Fixed::put` order /
   `gtk_widget_set_child_above_sibling`).
3. `instance.create_surface_unsafe(SurfaceTargetUnsafe::from_window(&child))` — wgpu picks Vulkan; the
   RawHandle carries the Wayland/X11 surface (**display handle REQUIRED on Linux**).
4. Transparency: `webkit_web_view_set_background_color(rgba a=0)` + page `background: transparent`.
5. Per frame: `surface.get_current_texture()`, submit, `present()`; GTK + the X11/Wayland compositor
   merge child-under-webview.
6. Viewport move/zoom: reposition/resize the child in the `gtk::Fixed` + reconfigure the surface.

---

## 3. What was proven vs. what remains uncertain

### Proven (executed on real hardware — `readback_proof` bin, exit 0)
- **wgpu 27.0.x builds and runs** through the repo's MSVC wrapper (`scripts/with-msvc.ps1`), in an
  isolated non-workspace crate that touches no prod code.
- **The produce seam is real:** wgpu brings up a discrete-GPU adapter and renders a frame into a
  Rust-owned `wgpu::Texture` — exactly the `palmier-engine` compositor output that must reach the
  webview. On this box the adapter came up **Vulkan / AMD Radeon RX 6600 XT** (the Linux-style backend),
  so the produce path is concretely validated on Vulkan.
- **The fallback (c) cost is measured** (GPU to CPU readback, this box, debug build):

  | Case | Frame size | Readback avg | Worst | Per-frame budget | Verdict (GPU to CPU only) |
  |---|---|---|---|---|---|
  | 1080p60 | 7.9 MB | **3.54 ms** | 5.43 ms | 16.67 ms @ 60 fps | within budget |
  | 4K@30 | 31.6 MB | **13.32 ms** | 19.42 ms | 33.33 ms @ 30 fps | within budget |

  This is GPU to CPU **only**; production adds IPC serialize + `<canvas>` upload on top, and a debug
  build understates wgpu (release would be faster). The readback alone already consumes **~21%** of the
  1080p60 budget and **~40%** of the 4K@30 budget — thin margins, which is exactly why readback is the
  fallback, not the primary.

### Uncertain / not executed (integration risks for E5-S8, by design of a headless spike)
- **No live composited window was produced.** Standing up a real Tauri window with WebView2/WebKitGTK +
  a sibling native surface and visually confirming the wgpu frame shows through the transparent webview
  is an **app-shell task** (`palmier-tauri`), out of a headless spike's reach. The seam is pinned
  call-by-call but the end-to-end visual proof lands in E5-S8.
- **wgpu `SurfaceTargetUnsafe::CompositionVisual` + WRY's existing WebView2 visual tree** have not been
  wired together in one process. Risk: WRY may not expose the HWND/visual tree handle cleanly; may need
  a small `wry`/`tao` patch or `raw-window-handle` plumbing. **Medium risk.**
- **The flicker / "fighting for the surface" failure mode is real** (tauri#9220): it happens when wgpu
  and the webview are put on the **same window/swapchain** with naive transparency. The DComp-visual /
  separate-GTK-child approach specifically avoids it (they are separate compositor visuals, not two
  renderers on one surface) — but this must be **confirmed** in E5-S8, not assumed.
- **Linux transparent-webview compositing is driver-fragile** (tauri#14924: WebKitGTK DMABUF renderer +
  some NVIDIA drivers cause black corners / ghosting / GBM crashes). Mitigation:
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` (also the tauri#9220 flicker workaround) — costs some webview perf,
  stabilises compositing. **Per-driver validation required on the Linux target matrix.**
- **D3D12 backend** specifically was not exercised here (the box chose Vulkan/AMD). The DComp path
  depends on the dx12 backend; needs a run on a D3D12 adapter in E5-S8.

---

## 4. Can the SM-2 FPS floors (4K >= 30 / 1080p60 >= 60) be met?

**Yes — with high confidence on the recommended zero-copy path.** The native-composited-surface
mechanism does **no per-frame CPU copy**: wgpu presents its swapchain and the OS compositor (DWM /
GTK+X11/Wayland) merges it with the webview. The FPS ceiling is therefore the **compositor render
itself** (textured quads + blend over a handful of layers), which on the SS10 reference GPUs (RTX 4060 /
Radeon 7600 class) is far below the 16.67 ms / 33.33 ms budgets. The only per-frame "tax" is the
compositor merge, which DWM/Wayland do for every window anyway.

The measured fallback (c) numbers show even the **slow** path stays within budget for GPU to CPU on this
mid-class AMD card — but with thin margins once IPC is added, which is why it is the fallback.

**Verdict: the CPU fallback is NOT needed as the primary mechanism.** Adopt the native composited
surface; keep readback only for the GPU-floor (sub-D3D12-12_0 / sub-Vulkan-1.2 / <4 GB VRAM) branch.

---

## 5. Fallback trigger (when to drop to CPU compositing -> SM-C1 interpolation waiver)

Trigger candidate (c) IPC readback (and the FOUNDATION SS3 CPU-compositing degraded preview) when **any**
of:
1. The GPU is **below the floor** (no D3D12 feature level 12_0 / no Vulkan 1.2 / < 4 GB VRAM).
2. On Linux, the driver/compositor **cannot do stable transparent-webview compositing** even with
   `WEBKIT_DISABLE_DMABUF_RENDERER=1` (per-machine detection at startup).
3. `SurfaceTargetUnsafe::CompositionVisual` (Win) or the GTK-child surface (Linux) **fails to create**
   on the target (driver/WRY incompatibility).

When triggered, the **sanctioned SM-C1 waiver** (E5-S1 pass bar) applies: live keyframe interpolation
degrades to **frame-stepped** on that path only; **color (BT.709) and layer accuracy still bind** on
both paths. This waiver binds only the degraded branch, never the GPU path.

---

## 6. Residual risks (carried into E5-S8 / E5-S10)

| Risk | Severity | Mitigation / owner |
|---|---|---|
| WRY does not cleanly expose its WebView2 visual / window-handle for reparenting under one DComp tree | Medium | Spike a minimal `wry` integration early in E5-S8; may need a small upstream/patch or `raw-window-handle` plumbing |
| wgpu/webview surface "fighting" (flicker) if the separate-visual contract is violated | Medium | Enforce: preview = its OWN DComp visual / GTK child; never share one swapchain with the webview |
| Linux transparent compositing driver fragility (tauri#14924) | Medium-High | `WEBKIT_DISABLE_DMABUF_RENDERER=1`; per-driver validation matrix; readback fallback per-machine |
| D3D12 DComp path unproven on this box (came up Vulkan) | Low-Med | Validate on an NVIDIA/D3D12 adapter in E5-S8 |
| Viewport hit-testing/geometry across a native surface vs. webview overlay (overlays in E5-S10) | Medium | Overlays positioned against the native surface rect; transform/crop handles counter-rotated as in reference `PreviewNSView` |
| wgpu 27 API churn (`SurfaceTargetUnsafe::CompositionVisual` is new) | Low | Pin wgpu minor; the spike already absorbed two API drifts (`PollType::Wait{}`, `DeviceDescriptor::experimental_features`) |

---

## 7. Explicit decision for Epic 5

> **E5-S1 decision:** Preview is a **native wgpu surface composited under a transparent webview by the
> OS compositor** — DirectComposition visual on Windows (`SurfaceTargetUnsafe::CompositionVisual` +
> `Dx12SwapchainKind::DxgiFromVisual`, wgpu 27), native GTK child surface on Linux
> (`SurfaceTargetUnsafe::from_window`, Vulkan). The webview hosts UI chrome with a transparent hole over
> the viewport rect. **The timeline canvas does NOT share this surface** — it stays a cheap webview-side
> 2D canvas; only the perf-critical preview is native. The shared-handle-into-`<canvas>` approach (b) is
> rejected for v1. IPC readback (c) is the GPU-floor fallback only, carrying the sanctioned SM-C1
> interpolation waiver. SM-2 FPS floors are achievable on the GPU path with high confidence (zero
> per-frame copy).

### What the orchestrator must decide before Epic 5 build
1. **Accept the native-composited-surface mechanism** as the binding E5-S8/E5-S10 input (this record).
2. **Authorize a short WRY-integration sub-spike at the START of E5-S8** to confirm WRY exposes the
   WebView2 visual / GTK child seam in one process (the one un-executed integration risk) — before the
   pure-Rust S2–S7 lanes converge on it. If WRY blocks it, fall back to a **second transparent topmost
   window** (the tauri discussion-#11944 dual-window pattern) as plan B, or readback as plan C.
3. **Pin wgpu to 27.x** (DComp support floor) in the root workspace scaffold.
4. **Define the Linux driver validation matrix** (Wayland/X11 x NVIDIA/AMD/Intel) and the per-machine
   fallback detection for transparent compositing.
5. Confirm **D3D12 path** on an NVIDIA box during E5-S8 (this spike validated Vulkan/AMD).

---

## 8. How to reproduce

```pwsh
cd spikes/s1-wgpu-webview
pwsh -File ../../scripts/with-msvc.ps1 cargo build                        # compile-verify (exit 0)
pwsh -File ../../scripts/with-msvc.ps1 cargo run --bin readback_proof     # produce + fallback timing
```

Files:
- `src/render.rs` — headless wgpu produce seam (the `palmier-engine` output shape).
- `src/readback.rs` — candidate (c) GPU to CPU readback, timed.
- `src/present.rs` — the recommended Windows (DComp) + Linux (GTK child) seams, pinned call-by-call.
- `src/bin/readback_proof.rs` — runnable proof + measurements.

## 9. Sources (current as of June 2026)
- wgpu DirectComposition support (`Dx12SwapchainKind::DxgiFromVisual`, `SurfaceTargetUnsafe::CompositionVisual`, wgpu 27, 2025-10-01): gfx-rs/wgpu CHANGELOG + docs.rs/wgpu.
- Tauri 2 multi-surface in one window; `tauri-wgpu-cam` / FabianLars demo: tauri-apps/tauri discussion #11944, github.com/clearlysid/tauri-wgpu-cam.
- "Render WebView on top of native GPU content": tauri-apps/tauri issue #8246.
- wgpu/webview surface-fighting flicker + `WEBKIT_DISABLE_DMABUF_RENDERER` workaround: tauri-apps/tauri issue #9220.
- Linux transparent-window driver crashes/artifacts: tauri-apps/tauri issue #14924.
- WRY `build_gtk` / GTK child integration: tauri-apps/wry.
- WebView2 host-texture stream (experimental): `ICoreWebView2ExperimentalTexture(Stream)`, Microsoft Learn (prerelease).
- WebView2 composition-controller transparency (no `DefaultBackgroundColor`): Microsoft Learn / WebView2Feedback.
