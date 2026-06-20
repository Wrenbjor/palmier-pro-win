# Spike S-2 — WRY integration (de-risks E5-S8): FINDINGS & DECISION RECORD

**Spike:** S-2, the WRY-integration sub-spike authorized at the start of E5-S8 by S-1 §7 item 2.
**Question (the one un-executed S-1 risk):** can a Rust-owned `wgpu` frame be wired into WRY's real
WebView2 window in one process and ACTUALLY APPEAR composited under a transparent webview — or does WRY
fail to expose the handle, forcing plan B (a second topmost child window) or plan C (readback)?

**Status:** RESOLVED. **A composited frame appeared on real hardware (screenshot captured).** **Plan A
works** — realized through WRY's own `WebViewBuilder::build_as_child(&window)` child-webview path. The
**D3D12 (Windows production) backend cooperated** on this box (S-1 had only validated Vulkan/AMD).
**SM-2 zero-copy holds** — the OS compositor (DWM) merges the two HWNDs with no readback and no IPC.

---

## 1. Verdict (the answer E5-S8 builds on)

> **A live wgpu frame composited UNDER a transparent WebView2 was produced and visually confirmed.**
> Plan A is viable. WRY does NOT need a patch: it consumes the **same `raw-window-handle` 0.6
> `HasWindowHandle`** the winit window already gives wgpu, so one window backs both the wgpu swapchain
> and the WebView2 child, and **DWM composites them**. The D3D12 backend works. SM-2 zero-copy holds.

Proof artifact: `proof-screenshot.png` (committed). It shows the wgpu DX12 layer (pulsing **magenta**
clear + an opaque **green triangle**) filling the window, with the **WebView2 chrome bar ON TOP** (its
translucent dark background lets the magenta bleed through) and a floating webview label over the GPU
content. No black corners, no flicker, no surface-fighting (tauri#9220 avoided — see §4).

```
[ S-2 wgpu-under-WebView composite ]            <- winit title bar
+-----------------------------------------------+
|  o S-2 WebView chrome (WRY / WebView2) ......  |  <- WebView2 child, ON TOP, translucent
|  This bar is the webview ... behind it is wgpu |     (magenta shows THROUGH it)
|  [ transparent viewport -- GPU shows through ] |  <- webview label floating over GPU
|                  ^                             |
|                 / \     green wgpu triangle    |  <- wgpu DX12 frame, BEHIND the webview
|                /   \    on magenta clear       |
|               /_____\                          |
+-----------------------------------------------+
```

---

## 2. Which plan worked: **A** (and how it maps to S-1's A/B/C)

S-1 framed three plans. This spike landed **plan A**, in its *simplest viable form*:

| Plan | S-1 description | S-2 result |
|---|---|---|
| **A** | native GPU surface composited UNDER a transparent webview by the OS compositor | **PROVEN LIVE.** Two forms of plan A exist (below); the spike proved form A1. |
| B | a second transparent topmost child window aligned over the webview (tauri#11944) | not needed — A1 worked. Documented as fallback in §6. |
| C | GPU->CPU readback to a canvas | not needed — remains the GPU-floor fallback (S-1 §5 measured it). |

**Two forms of plan A — both composite under a transparent webview by the OS; they differ only in HOW
the two surfaces become sibling HWNDs/visuals:**

- **A1 — WRY `build_as_child` (what S-2 proved).** Create ONE winit window. Put the wgpu swapchain
  directly on that window's HWND. Call `WebViewBuilder::...build_as_child(&window)` to parent a
  **transparent WebView2 child HWND** over it. DWM composites parent-HWND (wgpu) + child-HWND (webview).
  **Zero windows-rs / DirectComposition code.** Lowest-risk path; the spike's recommendation for
  E5-S8's first cut.
- **A2 — hand-wired DirectComposition visual tree (S-1's `present.rs` call path).** Build the wgpu
  instance with `Dx12SwapchainKind::DxgiFromVisual`, create an `IDCompositionVisual`, bind the surface
  via `SurfaceTargetUnsafe::CompositionVisual(visual_ptr)`, and place it as a sibling UNDER WRY's
  WebView2 composition visual in one DComp tree. More control (explicit z-order, transform/clip on the
  visual for viewport zoom), more windows-rs surface area. **Only needed if A1's child-HWND geometry/
  z-order proves insufficient** for the viewport-zoom + overlay requirements (E5-S10).

**Recommendation: start E5-S8 on A1; keep A2 (present.rs call path) in reserve for fine-grained visual
control.** Both are "plan A"; A1 removes the entire windows-rs DComp surface from the critical path.

---

## 3. Concrete API path for E5-S8 (crates + key calls — pinned, proven)

**Pinned versions that resolve together (Cargo.lock):** `wgpu 27.0.1`, `winit 0.30.13`, `wry 0.55.1`,
`webview2-com 0.38.2`, `raw-window-handle 0.6.2`, `pollster 0.4.0`. They build clean through
`scripts/with-msvc.ps1` on Rust 1.94.1.

> NOTE on Tauri: this spike used **winit + wry directly** (not the full `tauri` crate) to isolate the
> integration seam. In `palmier-tauri`, Tauri 2 owns the window (tao, not winit) and creates the
> WebView2 itself. The seam is identical in shape — Tauri's `WebviewWindow` exposes the window handle
> and Tauri already builds WebView2 in composition-hosted mode. E5-S8's first task is to get the Tauri
> window's `HWND`/`HasWindowHandle` and put the wgpu surface on it (see §6 residual risk #1).

### The seam, call by call (A1 — proven this spike)

1. **One window, transparent, child-clipping OFF.** winit:
   ```rust
   let mut attrs = Window::default_attributes().with_transparent(true);
   #[cfg(windows)] { attrs = attrs.with_clip_children(false); } // CRITICAL — see §4
   let window = Arc::new(event_loop.create_window(attrs)?);
   ```
2. **wgpu surface on the window's HWND.** `wgpu::Instance::create_surface(window.clone())` —
   takes the `Arc<Window>` (it impls `HasWindowHandle`+`HasDisplayHandle`), yields `Surface<'static>`.
   Configure with a BGRA8 format. (Force the backend with `Backends::DX12` for the Windows path.)
3. **Transparent WebView2 child over the same window.** wry:
   ```rust
   let webview = WebViewBuilder::new()
       .with_transparent(true)
       .with_bounds(Rect { position: (0,0).into(), size: (w,h).into() })
       .with_html(ui_html) // page body background:transparent
       .build_as_child(&window)?;   // consumes the SAME HasWindowHandle wgpu used
   ```
   `build_as_child<W: HasWindowHandle>(&self, &W)` is the load-bearing call. It returned `Ok` — **WRY
   exposes the seam cleanly; no patch needed.**
4. **Per frame:** `surface.get_current_texture()` -> render pass (clear + draw layers) -> `queue.submit`
   -> `frame.present()`. **DWM composites.** No `Commit()` call needed in A1 (that's A2's DComp path).
5. **Viewport geometry:** on resize/zoom, `webview.set_bounds(Rect{..})` + `surface.configure(..)` to
   the new size. (A1 moves the child HWND; A2 would `IDCompositionVisual::SetTransform/SetClip` instead.)
6. **Transparency truth:** the webview page must use `background: transparent` / `rgba(...,a<1)`. WebView2
   in composition mode renders no opaque fill when the page is transparent (S-1 §2 step 6 — confirmed:
   the magenta shows through the translucent chrome bar in the screenshot).

### The A2 path (if A1's control is insufficient)
Unchanged from S-1: `spikes/s1-wgpu-webview/src/present.rs::windows` documents it call-by-call
(`Dx12SwapchainKind::DxgiFromVisual`, `DCompositionCreateDevice` -> `CreateTargetForHwnd`,
`SurfaceTargetUnsafe::CompositionVisual`, `AddVisual`, `Commit`). S-2 does not re-prove A2 live, but
confirms its precondition: the wgpu 27 dx12 backend is present and functional on this box (§4).

---

## 4. What was PROVEN vs what remains uncertain

### Proven (executed on real hardware — `composited_window` bin)
- **The integration dependency triangle resolves and builds:** `wgpu 27` + `winit 0.30` + `wry 0.55`
  in one crate, clean compile through `with-msvc.ps1` (exit 0). This is itself a result — it was the
  open question whether a wgpu-27-era crate and wry-0.55 share a compatible `raw-window-handle`. **They
  share `raw-window-handle 0.6.2`** — the handle currency works, no patch/fork.
- **A live window stood up** with a wgpu swapchain AND a transparent WebView2 child on the same window;
  frames presented continuously without panic, on BOTH backends (smoke runs exit 0).
- **A composited frame visually appeared** (`proof-screenshot.png`): wgpu content behind, webview chrome
  on top, transparency working in both directions (translucent chrome shows GPU through it).
- **D3D12 cooperated.** `S2_FORCE_DX12=1` -> `Dx12 | AMD Radeon RX 6600 XT | DiscreteGpu`, frames
  presented, composite correct. **This closes S-1's "D3D12 path unproven on this box" residual risk**
  for the *window-surface* form. (DX12 smoke even ran faster than Vulkan: ~0.9 s vs ~2.4 s for 8 frames
  debug — DX12 is the native Windows path.)
- **No surface-fighting / flicker** (tauri#9220) — because `with_clip_children(false)` is set, the
  parent does NOT clip the wgpu swapchain against the child webview HWND. With clipping left on, this is
  the documented black/flicker failure mode; OFF, it composites cleanly. **This is the concrete
  mitigation E5-S8 must carry.**

### Uncertain / not executed (carried into E5-S8)
- **Tauri-owned window vs. raw winit window.** S-2 used winit+wry directly. Tauri 2 uses **tao** and
  owns window + WebView2 creation. The seam shape is the same (get the Tauri window handle, put wgpu on
  it), but the exact Tauri 2 API to (a) obtain the `HasWindowHandle`/HWND and (b) ensure
  `clip_children(false)` on a Tauri window is **not exercised here**. **Medium risk — first E5-S8 task.**
- **A2 (hand-wired DComp visual) not live-proven.** Only its precondition (dx12 backend works) is
  confirmed. If E5-S8 needs A2 for viewport-zoom precision, the `present.rs` call path is the spec but
  is unproven end-to-end.
- **Viewport "hole" geometry under zoom/scroll** (cmd-scroll zoom from the reference `PreviewNSView`):
  S-2 used a full-window child with a static layout. Moving/clipping the GPU region to a sub-rect that
  tracks the webview's viewport element under zoom is **not exercised**. (A2's `SetClip`/`SetTransform`
  is the cleaner tool here than A1's child-bounds — a reason to keep A2 in reach.)
- **Linux (WebKitGTK) path** — out of scope for this Windows-first spike; the wry example's GTK child +
  `WEBKIT_DISABLE_DMABUF_RENDERER=1` mitigation (S-1 §3) still stands unvalidated on Linux hardware.
- **Input hit-testing** across the transparent GPU region vs. the webview (clicks landing on the right
  surface) is not tested — relevant for E5-S10 overlays.

---

## 5. Does SM-2 zero-copy (SM-2 FPS floors) hold, or is a fallback needed?

**SM-2 zero-copy HOLDS — confirmed by construction.** A1 does **no per-frame CPU copy and no IPC**: wgpu
presents its swapchain on the window HWND and **DWM merges it with the WebView2 child HWND** as part of
the normal desktop composite every window already pays. The frame never leaves the GPU. This is
strictly cheaper than S-1's measured plan-C readback (which already fit the budget with thin margins).
The per-frame cost is the wgpu compositor pass itself (textured quads — far under the 16.67 ms / 33.33
ms budgets on SS10-class GPUs, per S-1 §4).

**No fallback needed as the primary mechanism.** Plan C (readback, S-1 §5 trigger) remains the
GPU-floor degraded branch only (sub-D3D12-12_0 / sub-Vulkan-1.2 / <4 GB VRAM), carrying the sanctioned
SM-C1 interpolation waiver.

---

## 6. Residual risks carried into E5-S8

| Risk | Severity | Mitigation / owner |
|---|---|---|
| Tauri-2-owned window (tao) seam differs from raw winit: getting `HasWindowHandle`/HWND + `clip_children(false)` on a `WebviewWindow` | **Medium** | First E5-S8 task: reproduce A1 inside `palmier-tauri` against the Tauri window. Seam shape is proven; only the Tauri accessor is unverified. If Tauri blocks `clip_children(false)`, fall to A2 (own DComp visual tree, no reliance on Tauri's child-clip behavior). |
| Viewport "hole" must track a webview element under cmd-scroll zoom (E5-S10 geometry) | Medium | Prefer **A2** for the preview region: `IDCompositionVisual::SetClip/SetTransform` gives exact sub-rect control; A1 child-bounds is coarser. Decide per E5-S10 overlay needs. |
| `with_clip_children(false)` is the load-bearing anti-flicker flag; a future Tauri/winit/wry rev could change child-HWND clipping and reintroduce surface-fighting | Medium | Pin winit 0.30 / wry 0.55; assert the flag in E5-S8; regression-screenshot test. |
| A2 (hand-wired DComp) unproven end-to-end | Low-Med | Only matters if A1 control is insufficient; `present.rs` is the spec; dx12 precondition confirmed. |
| Input hit-testing across transparent GPU region vs webview | Medium | Validate in E5-S10 (overlays/handles); A2 visual ordering or webview `pointer-events:none` over the hole. |
| Linux WebKitGTK transparent compositing (driver-fragile, tauri#14924) | Medium-High | Unchanged from S-1; `WEBKIT_DISABLE_DMABUF_RENDERER=1`; per-driver matrix; out of S-2 scope. |

---

## 7. What the orchestrator must decide before E5-S8 build

1. **Accept plan A1 (WRY `build_as_child`) as E5-S8's first-cut mechanism**, with A2 (S-1 `present.rs`
   DComp visual) held in reserve for viewport-zoom precision. (This spike's recommendation.)
2. **Pin the integration set:** `wgpu 27.0.x` + `winit`-or-`tao`-as-Tauri-ships + `wry 0.55.x` +
   `raw-window-handle 0.6`. Carry `with_clip_children(false)` (or the Tauri equivalent) as a hard
   requirement and a regression test.
3. **Scope the first E5-S8 task as "reproduce A1 inside the real `palmier-tauri` window"** (Tauri/tao
   window handle + transparent WebView2 + wgpu surface), since S-2 proved it on raw winit+wry, not on a
   full Tauri window. This is the only Medium-risk gap left for the produce->present seam on Windows.
4. **Defer the A1-vs-A2 final choice to E5-S10 geometry needs** (does the viewport hole need
   visual-precise clip/transform under zoom? -> A2; else A1 suffices).
5. **Linux remains separately gated** (S-1 §7 item 4 driver matrix) — unchanged; not touched here.

---

## 8. How to reproduce

```pwsh
cd spikes/s2-wry-integration
pwsh -File ../../scripts/with-msvc.ps1 cargo build                              # compile-verify (exit 0)

# Smoke (constructs window + wgpu + transparent webview, renders N frames, exits):
$env:S2_SMOKE="1"; pwsh -File ../../scripts/with-msvc.ps1 cargo run --bin composited_window
$env:S2_FORCE_DX12="1"; pwsh -File ../../scripts/with-msvc.ps1 cargo run --bin composited_window  # DX12 path

# Live window (open until closed; or cap frames for a screenshot):
$env:S2_MAX_FRAMES="2000"; $env:S2_FORCE_DX12="1"
pwsh -File ../../scripts/with-msvc.ps1 cargo run --bin composited_window
```

Env flags: `S2_SMOKE=1` (auto-caps frames, prints diagnostics), `S2_FORCE_DX12=1` (force Windows
production backend), `S2_MAX_FRAMES=<n>` (render n frames then exit 0 — drives screenshot/CI).

Files:
- `src/lib.rs` — `GfxState`: the wgpu producer surface ON a real window (the `palmier-engine`
  compositor-output shape), backend-forceable, reports adapter + composite-alpha.
- `src/bin/composited_window.rs` — the live integration: transparent winit window + wgpu swapchain +
  transparent WRY WebView2 child via `build_as_child`; renders clear+triangle; A/B/C plan notes inline.
- `proof-screenshot.png` — the captured composite (DX12 magenta+triangle behind WebView2 chrome).

## 9. Sources (current as of June 2026)
- WRY `examples/wgpu.rs` (canonical single-window wgpu-under-transparent-webview pattern,
  `build_as_child`, `with_clip_children(false)`): github.com/tauri-apps/wry `dev` branch.
- `WebViewBuilder::build_as_child<W: HasWindowHandle>` signature: docs.rs/wry/0.55.1.
- wgpu 27 surface/instance API (`create_surface`, `CompositeAlphaMode`, `request_device` w/
  `experimental_features`): docs.rs/wgpu/27.0.1.
- wgpu/webview surface-fighting flicker + clip-children/DMABUF workarounds: tauri-apps/tauri #9220.
- Render wgpu frames as webview overlay (plan-B dual-window discussion): tauri-apps/tauri #11944;
  github.com/clearlysid/tauri-wgpu-cam.
- S-1 decision record + A2 DComp call path: `spikes/s1-wgpu-webview/FINDINGS.md`,
  `spikes/s1-wgpu-webview/src/present.rs`.
