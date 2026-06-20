---
kind: doc
domain: [build-orchestration]
type: retrospective
status: adopted
links: [[FOUNDATION]] [[PRD]] [[phase0-reconciliation]] [[build-orchestration]]
---

# M1 retrospective — the hand-edit MVP

**M1 is complete.** Every story across Epics 1–6 is merged and green on `main`
(`github.com/Wrenbjor/palmier-pro-win`), built autonomously from an empty template through
spec → BMAD plan → 135 stories → wave-by-wave worktree-isolated delegation.

## What shipped
A native Windows (Tauri 2 / Rust / React) re-implementation of Palmier Pro's editing core:
- **Epic 1 — App shell:** real Tauri 2.11 runtime + boot sequence, full menu + shortcuts, windows
  (Home/Project/Settings/Help/Feedback), 5-tab settings, updater, telemetry (Sentry+tracing), Clerk/Convex
  auth + keyring, registry + autosave + sample materialization.
- **Epic 2 — Model + Project I/O:** Timeline/Track/Clip/keyframes/MediaAsset/dates; crash-safe atomic
  `.palmier` save/load (reference filenames); registry; autosave; 3 golden bundles (SM-7/SM-1b gates).
  The `f64::round` ties-away frame-math parity is locked.
- **Epic 3 — Editing:** pure engines (ripple/overwrite/split/snap) + orchestration (atomic apply, one-undo-
  per-edit) + drag-state + an interactive timeline canvas.
- **Epic 4 — Media:** cache, pure-Rust metadata, ffmpeg sprite thumbnails + waveform + image thumbnails;
  the full media panel UI (folders/sort/filter/search/generation) + folder model + import orchestration.
- **Epic 5 — Preview:** ffmpeg decode/frame-source (HW+CPU), the wgpu composition graph + per-frame sampling,
  audio mixer (envelope parity), transport (play/seek/step), **the wgpu compositor present** (composited under
  a transparent webview via WRY `build_as_child`), GPU text rendering (cosmic-text), the preview viewport UI.
- **Epic 6 (export side):** the byte-exact FCP7 XMEML emitter + goldens, self-contained bundle export, and
  the video export pipeline (HW encoders + ProRes).

**Scale:** ~47 stories + 5 spikes merged, 18 crates, hundreds of unit/integration/golden tests, all green.

## The two headline results
- **wgpu→WebView (the #1 architecture risk) is solved and hardware-proven:** the compositor renders decoded
  frames to a native GPU surface composited under the transparent webview, zero-copy (S-1 + the s2 WRY sub-spike
  put real pixels on screen on D3D12/AMD).
- **Performance crushes SM-2:** measured on an AMD RX 6600 XT (≈ the §10 reference GPU) — **1080p60 = 602 fps,
  4K30 = 529 fps** (floors 60/30). The GPU path ships; no CPU fallback or interpolation waiver needed.
- **Video export proven:** a real ProRes 422 encode ran end-to-end (render→readback→prores_ks→mux, re-decoded OK).

## Obstacles cleared autonomously (no human in the loop)
- **MSVC linker** — `vswhere` didn't register the VS install → `scripts/with-msvc.ps1` calls `vcvars64.bat`.
- **FFmpeg on Windows** — provisioned FFmpeg 7.1 LGPL + a no-admin libclang wheel; env auto-sourced; the LGPL
  build excludes x264/x265 → HW encoders for H.264/H.265, ProRes via prores_ks.
- **wgpu→WebView** — proven via spikes (S-1 + s2-wry-integration), pinned wgpu 27.0.1 set.
- **gpu-allocator/windows 0.56-vs-0.58 conflict** — a feature-gated `windows = "0.58"` dep on palmier-engine
  keeps the pin through lock regeneration (verified across merges).
- **Transient burst rate-limit** — re-architected the doc-generation workflow to throttled batches + retries.
- **A stalled worker** (0 tool-uses) — diagnosed, confirmed no stray branch, re-dispatched fresh.
- **A 1-day arithmetic slip** in the S-1b Date spike — caught by a downstream worker, corrected.

## Decisions parked for Wren (both reversible, proceeding on defaults)
- **ProRes 422 for v1** (not 4444+alpha) — `[[phase0-reconciliation]]` #17.
- **Accept GPLv3** for the port (verbatim agent prompt + reference fonts make it a derivative) —
  `[[gpl-cleanroom-contradiction]]`.

## Honest caveats / QA follow-ups (need a real display or specific hardware)
- **Live-window visual confirmation** of the wgpu-under-transparent-webview composite (no `clip_children(false)`
  in tao 0.35) and the **§11.3 driven e2e** (tauri-driver + Playwright) weren't runnable headless — they're a
  QA pass on a real desktop. All underlying logic is unit/integration/golden/perf-verified.
- **H.264/H.265 HW encode** selection is correct but unexercised (this box is AMD, no NVIDIA driver; ProRes proven).
- Golden frames pinned on AMD/Vulkan — first CI run on NVIDIA/Intel should be watched (tolerances are loose).

## What remains
- **M2 (Epics 7–8) — IN PROGRESS:** MCP server (30-tool registry + executor + read/edit tool bodies + agent
  undo all in; remaining: generate/library tool bodies, the axum/rmcp transport on 127.0.0.1:19789, the verbatim
  AgentInstructions, .mcpb) + the in-app agent (message model + request/SSE + the real AnthropicClient in;
  remaining: the tool-execution loop, mentions, PalmierClient, the agent panel UI). This is the product's
  strategic centerpiece.
- **M3** generation + transcription · **M4** visual search · **M5** polish + release — per `docs/PRD.md` §12.

## Timeline
2026-06-20 | M1 COMPLETE — Epics 1–6 merged + green (cargo default + wgpu-compositor + gpu-export builds, SM-2 GPU tests, goldens, pnpm). M2 already underway.
