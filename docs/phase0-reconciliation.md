---
kind: doc
domain: [build-orchestration]
type: decision
status: adopted
links: [[FOUNDATION]] [[build-orchestration]] [[orchestrator-protocol]]
---

# Phase 0 reconciliation — binding amendments to FOUNDATION

Phase 0 documented the macOS reference (`../palmier-pro/`) subsystem-by-subsystem (see
`docs/reference/*.md`). The agents found 24 points where `docs/FOUNDATION.md` contradicts the actual
reference. **Rule (per the orchestrator mandate): the reference codebase is the behavior-parity
authority.** Where FOUNDATION is factually wrong about the reference, the reference wins. Where
FOUNDATION makes a deliberate, better cross-platform improvement, FOUNDATION wins and we note it.

These rulings are **binding** for the PRD, architecture, epics, stories, and dev. They amend
FOUNDATION without rewriting it; cite this doc when FOUNDATION and a `docs/reference/*.md` disagree.

## Rulings

| # | Topic | FOUNDATION says | Reference truth | **Ruling** | Why |
|---|---|---|---|---|---|
| 1 | MCP tool count | 36 (§6.14, §13.12 "find missing 6") | **30** (ToolName enum=30, ToolDefinitions.all=30, exhaustive switch=30) | **30 tools.** §13.12 is void — there is no missing-6 set. | Verified 3 ways; the 30-row catalogue IS complete. |
| 2 | Agent prompt | substitute platform refs (§7) | one shared string, both MCP + in-app, no macOS phrasing | **Port verbatim, no substitution.** Single shared constant, both injection sites. | Prompt has zero Apple/macOS references. |
| 3 | Bundle filenames | `timeline.json`/`manifest.json`/`generation_log.json`/`chatsessions/`/`registry.json` (§5.7) | `project.json`/`media.json`/`generation-log.json`/`chat/`/`project-registry.json` | **Reference filenames.** | The Convex sample server emits the reference names — FOUNDATION names break sample import. Interop-critical. |
| 4 | Chat session writes | on tab-close + new-session (§6.13) | on document save (onSessionsChanged marks dirty) | **On save.** | Matches NSDocument lifecycle / our Tauri save. |
| 5 | Keychain account | `palmier-pro-anthropic-api-key` (§6.13) | `anthropic-api-key` | **`anthropic-api-key`.** | Parity; wrong name silently loses saved keys. |
| 6 | Pref keys | `palmier.notifications.enabled` etc. (§6.16) | `io.palmier.pro.{notifications,telemetry,mcp}.enabled` | **`io.palmier.pro.*.enabled`.** Absent ⇒ ON; telemetry/privacy snapshot at launch (restart-required). | Parity. |
| 7 | Clip Transform storage | `top_left` (x,y) (§5.4) | **center-based** (centerX/centerY/width/height) + legacy x/y migration (centerX=oldX+w−0.5) | **Center-based** with legacy migration. | Top-left breaks project-file round-trip. |
| 8 | Keyframe default interp | linear (§5.5) | **smooth** (smoothstep) | **Smooth default.** | Wrong serde default silently changes every animation/fade curve. |
| 9 | Volume dB range | −120…0 (§5.3/§5.5) | VolumeScale **−60…+15** (amplification allowed); rubber-band draw axis +6…−60; keyframe storage floor unverified | **−60…+15** for the field/scale. Verify keyframe-storage floor in code before locking (3 distinct dB constants exist). | Parity; allows >0 dB gain. |
| 10 | Snap sticky multiplier | 2.5× (§6.3) | **1.5×** | **1.5×.** (Playhead multiplier 1.5× matches; base threshold 8px; trim handle 4px.) | Parity. |
| 11 | Slip / Slide edit gestures | listed (§6.3/§6.4) | **neither exists** in the reference | **Defer both** (out of parity scope; revisit post-v1). | They'd be net-new design, not a port. |
| 12 | Cross-track move compat | text/lottie own-type only (§6.3) | `ClipType.isCompatible` makes **all visual types interchangeable** (video/image/text/lottie) | **All visual types interchangeable.** | Parity. |
| 13 | Visual search model | "CLIP" (§2.2/§6.10) | **SigLIP2** base patch16-256, 768-dim (CoreML) | **SigLIP2 base patch16-256, 768-dim.** Source ONNX/candle weights; reproduce `.embed` format (magic `PALMEMB1`) or pick a new magic + re-index. | Embeddings not interchangeable with OpenAI CLIP. |
| 14 | Media "Music" tab | built-in library from Convex `/v1/music` (§6.2) | a video/text→**music generation** form | **Generation form.** | Parity; no `/v1/music` library exists. |
| 15 | Media sort modes | 3 (dateAdded/name/duration) (§6.2) | **4** (+ type); dateAdded = insertion order | **4 modes**, dateAdded = insertion order. | Parity. |
| 16 | Waveform/thumb format | ~2000 samples/min; frame sequence (§6.2) | **150 samples/s capped 20000**; video thumb = single JPEG **sprite-sheet + JSON sidecar**; cache key `sha256(path\|size\|mtime).prefix16` | **Reference constants.** Gates: waveform=2, image-thumb=4, video-thumb **ungated**. | Parity; mtime key may false-hit on coarse Windows FS — watch. |
| 17 | ProRes export | "ProRes 4444 (alpha)" (§6.12) | ProRes **422 LPCM** (no alpha) | **ProRes 422 LPCM for v1.** 4444+alpha = future enhancement. ⚑ Wren-visible. | Avoids threading alpha through the whole pipeline; matches reference. |
| 18 | Caption text case | upper/lower/title (§6.9) | **auto/upper/lower** (rejects "title") | **auto/upper/lower.** | Parity; no title-case in reference. |
| 19 | Transcription cache key | `sha256(content)+model+language` (§6.9) | `sha256(path\|mtime\|size)` (no model/lang) | **Adopt FOUNDATION's key** (content+model+language). | One of the few places FOUNDATION is *better* — mtime false-hits; model/lang must invalidate. Hash first N MB if 25-min hashing is slow. |
| 20 | Paid-tier model | Opus 4.8 via Convex catalog (§6.13) | hard-coded `[sonnet46]` | **Catalog-driven**, default Sonnet 4.6; Convex catalog may enable Opus. | Reference hard-code is a limitation, not a spec; keep it flexible. |
| 21 | Design token hexes | accent-timecode `#F2994A`, accent-primary `#F5F0E4` (§9) | computed from AppTheme: **`#F29933`**, **`#F5EFE4`** | **Use computed values** (`#F29933`, `#F5EFE4`). Add §9-omitted tokens (spotlight/ai/ai-dark/shimmer gradients, all component/window/caption/media/layout sizing). `track-text == track-image` (#B72DD2) — keep, flag as possible upstream bug. | AppTheme.swift is ground truth for visuals. |
| 22 | Preview stills/Lottie | first-class GpuTexture LayerRender (§6.5) | reference **bakes** stills/Lottie into 1800s `.mov` (AVPlayer limitation) | **First-class GpuTexture — drop the .mov bake.** | FOUNDATION wins: genuine improvement enabled by the per-frame wgpu model. |
| 23 | wgpu→WebView presentation | "present the texture to the WebGPU canvas" / "shared WebGPU surface" (§4/§6.5) | **no mechanism specified anywhere** | **RESOLVED by Spike S-1** (`spikes/s1-wgpu-webview/FINDINGS.md`): render wgpu to a **native GPU surface composited UNDER a transparent webview by the OS compositor** — NOT into the webview's GPU context. **Windows:** wgpu DX12 surface on a DirectComposition visual (`SurfaceTargetUnsafe::CompositionVisual` + `Dx12SwapchainKind::DxgiFromVisual`, wgpu **27.x**) under a transparent WebView2 visual → DWM merges, zero-copy. **Linux:** wgpu/Vulkan on a `GtkGLArea` child in the WRY `gtk::Fixed`, z-ordered under transparent WebKitGTK. Shared-handle-into-canvas (b) rejected (no stable path). **CPU readback (c)** = sub-floor-GPU fallback only (SM-C1 waiver). **SM-2 FPS floors met zero-copy** (measured fallback on AMD RX6600XT: 1080p60 3.54ms / 4K30 13.32ms). **Pin wgpu 27.x.** Residual risk → short WRY visual-tree integration sub-spike at the START of E5-S8 (plan B: second transparent topmost window; plan C: readback). Validate D3D12 on an NVIDIA box + a Linux driver matrix during E5. **SUB-SPIKE CONFIRMED (spike/s2-wry-integration, `spikes/s2-wry-integration/FINDINGS.md`):** a composited frame **actually appeared** (screenshot-proven) — wgpu 27 frame behind a transparent WRY WebView2 child, **D3D12/AMD path**, zero-copy via DWM, SM-2 holds, no CPU fallback needed. **Mechanism for E5-S8 = Plan A1: WRY `WebViewBuilder::build_as_child(&window)`** (one HWND backs both surfaces; no WRY patch — `build_as_child` takes the same `raw-window-handle 0.6` handle wgpu uses). **Pinned set: wgpu 27.0.1 / winit 0.30.13 / wry 0.55.1 / raw-window-handle 0.6.2**; HARD requirement `with_clip_children(false)` (anti-flicker, tauri#9220) + regression test. A2 (hand-wired DirectComposition visual) held in reserve for viewport-zoom precision (decide at E5-S10). **E5-S8's first task: reproduce A1 inside the real `palmier-tauri` tao/Tauri window** (get its `HasWindowHandle` + clip_children(false)) — the one Medium-risk Windows gap. Linux/WebKitGTK separately gated. | Was the largest unresolved risk; now a binding architecture decision for Epic 5. |
| 24 | Generation cancel | implied cancel | client-side subscription teardown only; job keeps running/billing; `rendering` status never set; `can_generate` is advisory (server mutation is the real gate) | **Client teardown only for v1**; add a Convex cancel mutation later if needed (backend out of our repo). | Parity; real gate is server-side. |

## Ruling 25 — Convex Rust SDK exists (corrects FOUNDATION §2.2/§8.1)
**Spike S-2** (`spikes/s2-convex-ws/FINDINGS.md`) found that FOUNDATION §2.2/§8.1 are WRONG to say "no native
Rust Convex SDK; use reqwest". The **official `convex` crate** (`get-convex/convex-rs`, v0.10.4, **Apache-2.0** —
GPLv3-compatible) implements the full Convex WS sync protocol over tokio-tungstenite: `subscribe` (→ Stream),
`query`/`mutation`/`action`, and dynamic JWT auth via `set_auth_callback` (re-minted on reconnect) — a 1:1 match
for the reference's ConvexMobile usage. **Ruling: ADOPT the `convex` crate** for all Convex access (generation
subscriptions, models, samples, billing). Do NOT hand-roll the protocol. **E9 contract** (FINDINGS §3): WS
live-query for `generations:byId` behind a `GenerationTransport` trait (HTTP `/api/query` polling fallback);
`/v1/models` HTTP snapshot cached 24h; Clerk JWT (the `convex` template) via palmier-auth; `completedAt` =
apple-ref-epoch double (S-1b codec); cancel = client teardown only (#24). **Gating for E9 (Wren / §13.9):** a
live Convex deployment URL + a test Clerk account are still secret/unreachable — E9 builds against the crate +
recorded shapes, but the live round-trip + the R-6 Date capture need that access.

## Carry-forward port-critical details (not contradictions, but must-preserve)
From `docs/reference/*.md` — the load-bearing specifics downstream stories must honor:
- **Tool descriptions are contract text** (all-or-none trackIndex, ripple_delete units, source-vs-timeline
  frame math, ShortId ≥8-char unique-prefix rules, get_timeline default-omission + 200-row captionGroup
  cap, pagination caps 400 segments/10000 words/12 frames). Port verbatim.
- **Frame rounding parity:** all source↔timeline conversions are `round(x*speed)` / `round(x/speed)`
  ties-away-from-zero → use `f64::round` (NOT `round_ties_even`).
- **Agent undo** refuses unless the editor's current `undoActionName` equals the pushed name — every
  mutation's undo-group name must match the reference exactly. Separate user/agent stacks.
- **Anthropic request:** exactly 2 ephemeral cache breakpoints (system+tools, conversation tail);
  `.sortedKeys` canonical JSON; orphan-tool_use repair injects synthetic Cancelled tool_results;
  image inline limits longest-edge 1568px / 3,500,000 bytes / JPEG q[0.85,0.7,0.55,0.4].
- **Preview constants:** interactive-scrub tolerance `min(0.75, 0.15*activeLayerCount)s` @ ts 600,
  throttle 1/30s; text preroll 30 frames; smoothSegments=8; encoder dim clamp 4096 + even dims; BT.709.
- **CaptionBuilder** has 14 unit tests — port verbatim (grapheme-aware counts; enforceMinDuration may
  push final phrase end past segment end; breakOn delimiter+space so "U.S."/"3.14" don't split).
- **Export/XMEML golden fidelity:** 2-space indent, `\n` joins, self-closing tags, escape order,
  TRUE/FALSE literals, exact float formats, drop-frame `round(fps*0.066666)` approximation copied
  exactly, `file://localhost//` pathurl rewrite, rotation negated, center as normalized offset-from-0.5.
- **Search/SigLIP:** 256×256 squash (no crop, black fill, sRGB BGRA), tokenizer pad-to-64 id 0 no mask,
  raw dot-product ranking (model must output L2-normalized), cosine floor 0.05, relative cutoff 0.85.
- **Project I/O Date encoding** — **RESOLVED by Spike S-1b** (`spikes/s1b-convex-date/FINDINGS.md`):
  per-field codec. `media.json` (`cachedRemoteURLExpiresAt`, `GenerationInput.createdAt`) +
  `generation-log.json` (`createdAt`) = Apple reference-epoch **doubles** (`unix = apple_ref +
  978_307_200`), Optional + `skip_serializing_if`. `chat/*.json` (`updatedAt`) = **ISO-8601** string,
  pretty 2-space + sorted keys. `project.json` has **no** Date field. media/log written **compact**.
  E2-S8 implements `crates/palmier-model/src/serde_date.rs` (modules `apple_ref_epoch` + `iso8601`) per
  FINDINGS. Confirmed from the reference decoder (Convex URL is a build secret, unreachable) — **R-6
  carry-forward:** treat as provisional-from-code until a real `/v1/samples/resolve` payload is captured
  (during the S-2 window) and diffed.

## Open items for Wren (recorded, proceeding with the ruling above)
1. **ProRes 422 vs 4444+alpha** (#17) — shipping 422 for v1; confirm if alpha export is needed sooner.
2. **GPLv3 clean-room contradiction** — porting the agent prompt verbatim + bundling reference fonts is
   *not* clean-room; the result inherits GPLv3. Recorded as `signals/gpl-cleanroom-contradiction`.
3. ~~**wgpu→WebView spike** (#23)~~ **RESOLVED** — native-composited-surface mechanism chosen, SM-2 met zero-copy, wgpu 27.x pinned; one WRY-integration sub-spike deferred to E5-S8 start. See #23.

## Spike & build-time resolutions (binding, recorded post-Phase-0)

**S-3 — SigLIP2 visual-search runtime (resolves the runtime half of #13).** RESOLVED
(`spikes/s3-siglip2/FINDINGS.md`). Runtime = **`ort` (ONNX Runtime 2.0)** with DirectML (DX12, GPU on
this AMD box) + automatic CPU fallback — candle is fallback-only (ships SigLIP**1**; SigLIP2 is an
unmerged candle PR). Weights = **`onnx-community/siglip2-base-patch16-256-ONNX`** (Apache-2.0, same
`google/siglip2-base-patch16-256` base the reference CoreML derives from): split `vision_model.onnx`
(pixel `[1,3,256,256]`→`[1,768]`) + `text_model.onnx` (ids `[1,64]`→`[1,768]`), Gemma `tokenizer.json`.
**Load-bearing finding:** ONNX `pooler_output` is **NOT L2-normalized** (the reference CoreML output is)
→ the port **must add an explicit L2-normalize** after encode so raw dot==cosine and the 0.05/0.85 cutoffs
hold (expect parity, no pre-emptive re-tune). `.embed` keeps the byte-exact **PALMEMB1** format but bumps
`modelVersion`→**2** to force a clean re-index (ONNX vectors ≠ CoreML vectors bit-wise). Proven: compiles +
19 tests pass; `ort` encode path type-checks vs `ort 2.0.0-rc.10`. **NOT yet proven:** a real encode +
measured cosine (blocked on ~750 MB weights + `onnxruntime.dll` download). **Orchestrator must decide
before Epic 11:** (a) fp16 (recommended default) vs fp32; (b) re-host ONNX under `palmier-io` (recommended)
vs pin onnx-community + SHA-verify; (c) ship `onnxruntime.dll`+DirectML as a Tauri resource (parallels the
FFmpeg DLLs); (d) confirm `.embed` modelVersion=2 re-index; (e) run the live encode to lock `COSINE_FLOOR`.

**E9 — `convex` crate enables serde_json `preserve_order` (workspace-contagious).** BINDING FOLLOW-UP.
The official `convex` crate (adopted per #25) turns on serde_json's `preserve_order` feature, which is
workspace-wide once any crate depends on it. That makes serde_json emit map keys in insertion order, which
**breaks palmier-agent's Anthropic-request goldens** (the reference requires `.sortedKeys` canonical JSON —
see the carry-forward note above). **Mitigation in place:** the `convex-transport` feature on `palmier-gen`
is **defaulted OFF**, so default builds keep sorted-key behavior and the goldens pass (verified: 52 default
suites green post-merge). **Before `convex-transport` is ever enabled by default, palmier-agent MUST
canonicalize its request JSON explicitly** (serialize via a `BTreeMap`/sorted-keys pass, not rely on
serde_json's default ordering) so the 2-ephemeral-breakpoint cache and golden bytes survive `preserve_order`.

## Timeline
2026-06-20 | Phase 0 complete — 15 reference docs + this reconciliation. Reference = parity authority; 24 discrepancies ruled. Advancing to Phase 1 (PRD).
2026-06-20 | M2 build: S-3 SigLIP2 runtime resolved (ort+ONNX, explicit L2-normalize, modelVersion=2); E9 convex-transport preserve_order hazard recorded — convex-transport OFF by default until palmier-agent sorts request JSON.
