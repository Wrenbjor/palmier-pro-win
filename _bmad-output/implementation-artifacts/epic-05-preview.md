---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
title: "Epic 5 — Preview Composition & Playback (story decomposition)"
created: 2026-06-20
governing_reference: docs/reference/preview-engine.md
foundation_section: "§6.5 / §6.6"
prd_features: [FR-17, FR-18, FR-19, FR-20]
milestone: M1
spike_gated_by: S-1
---

# Epic 5 — Preview Composition & Playback (spike-gated)

## Epic goal

Replace the macOS reference's entire AVFoundation preview/playback pipeline
(`AVMutableComposition` + `AVVideoComposition` + `AVMutableAudioMix` + one `AVPlayer`/`AVPlayerLayer`
+ a live `CALayer` text tree) with a **Rust per-frame composition graph rendered via wgpu from
FFmpeg-decoded frames**, plus a Rust audio mixer, a transport loop, the preview-tab model, and the
viewport transform/crop overlays. This is "the editor's eyes" — every other epic that shows a frame
depends on it.

Crate boundaries are **pinned by the Glossary one-decode-owner contract** (PRD §3, FOUNDATION §4):
- **`palmier-media`** owns the `DecoderThread` (one FFmpeg `AVFormatContext`+`AVCodecContext` per
  source URL, HW decode when available) and the LRU `FrameCache` keyed by `(media_ref, source_frame)`
  with the **1.5 GB VRAM texture / 512 MB system-RAM YUV ceilings** (FOUNDATION §6.5). It decodes
  images/Lottie straight to a `GpuTexture`. **`palmier-engine` NEVER opens an `AVFormatContext`** — it
  consumes decoded frames via a handle from `palmier-media`.
- **`palmier-engine`** owns the `CompositionFrame` builder (port of `CompositionBuilder.build` +
  `buildVisuals`), the wgpu compositor (textured quads, `Mat3` affine, premultiplied-alpha blend,
  crop), the transport loop (`VideoEngine` port: `SeekMode` Exact/InteractiveScrub), and the audio
  mixer (symphonia decode → rubato resample 48 kHz → speed time-stretch → volume/fade envelope →
  cpal out).
- **`palmier-text`** owns text layout: `TextLayerController` → cosmic-text glyph runs as
  `LayerRender::Text`, with the **30-frame preroll** retained.
- **`src-ui/editor` (preview)** owns the viewport, preview tabs, and transform/crop overlays.

Realizes **all five UJs** (it is the only way any of them sees a frame). Milestone **M1 — Hand-Edit
MVP** (PRD §12).

### PRD §4.5 / §10 acceptance this epic must satisfy

From the §10 Epic 5 row and §4.5 (FR-17..FR-20):

- **FR-17 Per-frame composition.** For each visible frame: sample animated props, convert
  timeline→source frame, **fetch the decoded frame from `palmier-media`'s `FrameCache` via a handle**,
  build `LayerRender`s bottom→top (track order = z-order), render textured quads + text via wgpu.
  **4K scrub ≥ 30 fps (5 clips / 2 layers, no keyframe motion) and 1080p60 ≥ 60 fps on §10 GPU
  (SM-2)**. Stills/Lottie are first-class GPU textures, **no `.mov` bake** (ruling #22).
- **FR-18 wgpu→WebView presentation.** The rendered texture is presented into the webview viewport.
  **This mechanism is unspecified in ALL grounding docs and is a MANDATORY SPIKE (S-1, ruling #23,
  R-1) before any Epic 5 architecture commit.** The chosen mechanism hits the SM-2 FPS targets on
  Windows (D3D12) and Linux (Vulkan), or the documented CPU-compositing fallback is used.
- **FR-19 Audio mixing & transport.** symphonia decode → rubato 48 kHz resample → speed
  time-stretch → volume envelope + fades → sum → cpal. Transport play/pause/toggle/seek(mode)/step;
  `current_frame` reactive via Tauri events. **InteractiveScrub tolerance
  `min(0.75, 0.15*activeLayerCount)s`** (carry-forward note) tested at layer counts 1/3/6; Exact mode
  lands on the exact frame.
- **FR-20 Preview tabs & overlays.** Closable per-asset tabs + the always-present `.timeline` tab;
  transform overlay (corner/edge/rotation handles, center snap guides) and crop overlay
  (rule-of-thirds, aspect lock), counter-rotated to clip-local axes.

**Golden / benchmark gates that exit on this epic (PRD §10 Epic 5 + §11.4/§11.5 → M1):**
- **§11.4 composition-graph Criterion benchmark at 50 / 200 / 1000 clips** (the 1000-clip case is an
  explicit Epic 5 acceptance item, previously unmapped).
- **SM-C1 golden rendered-frame comparison** (SSIM ≥ threshold or exact-within-tolerance) at known
  frames on `golden_project_keyframes` + `golden_project_text`, **run on BOTH the wgpu and CPU-fallback
  paths**. A fidelity regression (interpolation / BT.709 color / layer drift) fails the gate.

---

## Spike / risk note (THIS EPIC IS GATED)

**R-1 [Critical] — wgpu→WebView texture presentation is unspecified.** No grounding doc defines how a
wgpu-composited frame reaches the webview viewport (ruling #23, preview-engine.md "Port risks" #1).
`<canvas>` WebGPU contexts are owned by the WebView GPU process; a Rust-owned wgpu texture has no
specified path into WebView2 (Win) / WebKitGTK (Linux).

**Consequence for decomposition:** **Story E5-S1 IS the spike (S-1).** It MUST land — with a chosen
mechanism + a measured per-platform FPS number, OR an explicit CPU-fallback decision — **before any
other E5 story makes a presentation-mechanism assumption.** Every downstream presentation/viewport
story (E5-S8, E5-S9, E5-S10) lists **E5-S1 as a hard dependency** and must consume whatever
presentation surface S-1 selects. The **pure-Rust** stories (composition math, audio mixer, transport
state machine, decode pipeline, text glyph runs, overlay geometry) are deliberately written to be
**presentation-agnostic** so they can proceed in parallel with the spike — they produce/consume a
`CompositionFrame` and a rendered `GpuTexture`/readback buffer regardless of how it is later presented.

**Pass bar for S-1 (PRD §11):** measured per-platform FPS ≥ SM-2 floors (4K ≥ 30 / 1080p60 ≥ 60) on
the §10 reference HW, OR an explicit, documented decision to use the FOUNDATION §3 CPU-compositing
fallback. **Sanctioned SM-C1 waiver:** if the CPU fallback is chosen, live keyframe interpolation
degrades to frame-stepped per FOUNDATION §3 — the **one** waiver of SM-C1's interpolation clause; it
binds only the GPU path. Color/layer accuracy still bind on both paths.

Other carried risks:
- **preview-engine.md risk #2 — overlap semantics.** The reference serializes single-track clips and
  forbids on-track overlap (`startFrame >= previousEndFrame`); the wgpu per-frame model composites
  arbitrary overlaps directly. Track order = render order bottom→top must match the reference.
- **risk #3 — premultiplied alpha.** Use premultiplied-alpha blend + premultiply on upload; trust the
  **codec** alpha flag only, not container capability.
- **risk #4 — smooth-keyframe parity.** Per-frame sampler must use the **same smoothstep** (8-segment
  `smoothSegments` math) the reference pre-bakes, or exported/preview frames drift (SM-C1).
- **risk #5 — color.** Single working color space, everything BT.709; sRGB transfer for image/Lottie.
- **risk #7 — text geometry flip.** Reference uses `isGeometryFlipped=true` + `containerH/1080` scale;
  cosmic-text is top-left origin — verify Y math so text doesn't mirror vs. video.
- **risk #8 — `refreshVisuals` fast path.** Editing transform/opacity/volume must re-sample
  instructions only, NOT trigger a full decode/rebuild. Preserve the two-tier build-vs-visuals split.

---

## Story map (dependency order)

| id | title | depends on | parallel-safe |
|---|---|---|---|
| **E5-S1** | Spike S-1 — wgpu→WebView presentation (BLOCKER) | — | gating (run first) |
| **E5-S2** | `palmier-media` decode pipeline: DecoderThread + LRU FrameCache | E2(model) | yes |
| **E5-S3** | Image / Lottie → GpuTexture (drop the .mov bake) | E5-S2 | yes |
| **E5-S4** | Composition graph: `CompositionFrame` builder + clip→source mapping | E2(model), E5-S2 | yes |
| **E5-S5** | Animated-property sampler (transform/opacity/crop, smoothstep parity) | E5-S4 | yes |
| **E5-S6** | Audio mixer: decode→resample→stretch→envelope→cpal | E2(model) | yes |
| **E5-S7** | Transport loop: SeekMode + scrub throttle/tolerance + Tauri events | E5-S4, E5-S6 | yes |
| **E5-S8** | wgpu compositor pass + present via the S-1 surface | E5-S1, E5-S4, E5-S5 | no |
| **E5-S9** | `palmier-text` glyph runs + 30-frame preroll text layer | E5-S5, E5-S8 | partial |
| **E5-S10** | Preview tabs + transform/crop overlays (frontend) | E5-S1, E5-S7, E5-S8 | no |
| **E5-S11** | Criterion 50/200/1000-clip bench + SM-C1 golden-frame gate (both paths) | E5-S8, E5-S9 | no |

---

## E5-S1 — Spike S-1: wgpu→WebView presentation (BLOCKER, run first)

**As** the Epic 5 architect, **I want** a proven mechanism for getting a Rust-owned wgpu texture into
the Tauri webview viewport at the SM-2 FPS floors on both platforms, **so that** the rest of the
preview crate can be built against a real presentation surface instead of an unspecified one.

**Acceptance criteria:**
- **Given** the three R-1 candidates, **when** the spike runs, **then** it delivers a **working
  prototype** that composites a moving wgpu-rendered texture under/over the webview viewport rect, on
  **both** Windows (D3D12) and Linux (Vulkan).
- Candidates evaluated, in priority order (preview-engine.md risk #1, ruling #23): **(a)** native
  transparent child surface — D3D11/DXGI swapchain via DirectComposition on Windows; GTK native
  GL/Vulkan child (or dmabuf) on Linux; **(c)** transparent native child surface positioned over the
  viewport rect (mirrors how `AVPlayerLayer` sat in the `NSView`, matches the cmd-scroll zoom geometry
  in `PreviewNSView`); **(b)** DXGI shared-handle into a `<canvas>`; **fallback** IPC readback (full-res
  RGBA readback per frame — prove the perf cliff, do not adopt unless a/c fail).
- **Pass bar:** a **measured per-platform FPS number** ≥ SM-2 floors (4K scrub ≥ 30 fps, 1080p60 ≥ 60
  fps) on §10 reference HW (RTX 4060 / Radeon 7600, NVMe), **OR** an explicit documented decision to
  use the FOUNDATION §3 CPU-compositing fallback (frame-stepped, interpolation off — the sanctioned
  SM-C1 waiver).
- Output is a **decision record** answering: chosen mechanism; does the timeline canvas share the same
  surface or is preview a separate native overlay (preview-engine.md open question); per-platform FPS;
  fallback trigger. This record becomes the binding input to E5-S8 and E5-S10.
- **No Epic 5 architecture commit (no E5-S8/S9/S10) until this lands** (ruling #23).

**Implementation context:**
- Reference surface being replaced: `PreviewView.swift` / `PreviewNSView` (the `AVPlayerLayer` host —
  "This is the screen-presentation surface to replace"), `PreviewContainerView.swift`.
- Docs: `docs/reference/preview-engine.md` "Port risks & gotchas" #1; FOUNDATION §4 "shared WebGPU
  surface", §6.5 step 4 ("present the rendered texture to the WebGPU canvas in the webview"), §3 (GPU
  floor D3D12 12_0 / Vulkan 1.2 / 4 GB VRAM and the CPU-compositing fallback). PRD §11 S-1, §9 R-1.
- Crates touched: `palmier-engine` (presentation seam), `palmier-tauri` (window/surface wiring),
  `src-ui/editor` (viewport host). Tauri 2 over WebView2 (Win) / WebKitGTK (Linux).

**Dependencies:** none (must run first; gates the rest of Epic 5).
**Parallel-safe?** Gating — it precedes the presentation stories. The pure-Rust stories (S2–S7) may
start in parallel since they are presentation-agnostic, but S8/S9/S10 are blocked on its outcome.

---

## E5-S2 — palmier-media decode pipeline: DecoderThread + LRU FrameCache

**As** the preview engine, **I want** one decode owner that hands decoded frames to the compositor via
a handle, **so that** `palmier-engine` never opens an `AVFormatContext` and frames are cached/evicted
near the playhead.

**Acceptance criteria:**
- **Given** a source asset URL, **when** the engine requests `(media_ref, source_frame)`, **then**
  `palmier-media` returns the decoded frame from an **LRU `FrameCache` keyed by `(media_ref,
  source_frame)`**, decoding on miss via **one `DecoderThread` per source URL** (FFmpeg
  `AVFormatContext` + `AVCodecContext`, **HW decoder when available**, CPU otherwise).
- **Cache ceilings (FOUNDATION §6.5): 1.5 GB VRAM for textures + 512 MB system RAM for decoded YUV
  planes; eviction by distance from the current playhead.** (Whether the ceiling is global vs per-asset
  reads **global** per FOUNDATION §6.5 — implement global.)
- The crate exposes a **handle/API** (`request_frame`, `prefetch`, cache-stats) consumed by
  `palmier-engine`; **`palmier-engine` opens no FFmpeg context itself** (Glossary one-decode-owner
  contract — assert at the API boundary / in review).
- Color: decoded frames carried in a single working space; BT.709 enforced downstream (risk #5).
  Premultiplied-alpha handling on upload (risk #3): trust the **codec/pixfmt** alpha flag only.
- **Unit tests (FOUNDATION §11.1):** cache hit/miss/eviction-order under the ceilings; one decoder
  thread per distinct URL (no duplicate contexts); HW→CPU decode fallback path.

**Implementation context:**
- Reference: `Sources/PalmierPro/Preview/VideoEngine.swift` (transport's decode use),
  `AlphaVideoNormalizer.swift` (alpha detection from codec format ext, not container);
  preview-engine.md "Mapping to FOUNDATION crates → palmier-media" and "macOS/Apple APIs to replace"
  (`AVAssetReader`/`CVPixelBuffer`/`CMFormatDescription` alpha ext → FFmpeg pixfmt has-alpha).
- Types: `FrameCache { (media_ref, source_frame) → GpuTexture | YUV planes }`, `DecoderThread`.
- Crate: `palmier-media`. Docs: FOUNDATION §6.5 "Decode pipeline"; preview-engine.md risk #3.

**Dependencies:** Epic 2 (`palmier-model` media-ref types).
**Parallel-safe?** Yes — own crate (`palmier-media`), no shared files with sibling stories.

---

## E5-S3 — Image / Lottie → GpuTexture (drop the .mov bake)

**As** the preview engine, **I want** stills and Lottie decoded directly to first-class GPU textures,
**so that** they composite as `LayerRender::Image` / `LayerRender::Lottie` without the reference's
1800-second `.mov` bake (ruling #22).

**Acceptance criteria:**
- **Given** a still image or a Lottie JSON asset, **when** the engine needs it as a layer, **then**
  `palmier-media` decodes it **straight to a `GpuTexture`** — **no `.mov` bake, no 2-frame still movie,
  no freeze-frame tail** (ruling #22, preview-engine.md risk #2.5).
- **Preserve the cache keying and clamps** from the reference generators: cache key
  `mediaRef + size + file mtime/size`; **clamp to even dimensions ≤ 4096 px**; sRGB transfer for
  image/Lottie (risk #5). Lottie pre-rendered to a texture (FOUNDATION §6.5 `Lottie { texture }`).
- Alpha: premultiply on upload (premultipliedFirst BGRA equivalent), premultiplied-alpha blend
  downstream (risk #3).
- **Unit tests:** image decode → texture dims/alpha correct; Lottie frame → texture; cache key
  round-trips and is invalidated on size/mtime change.

**Implementation context:**
- Reference: `ImageVideoGenerator.swift`, `LottieVideoGenerator.swift` (port the **cache keying +
  4096/even-dim clamps + sRGB** only — drop the AVPlayer-driven .mov machinery). preview-engine.md
  "Image/Lottie/alpha generators" + risk #2.5; ruling #22.
- Crate: `palmier-media`. Lottie engine choice is a Rust equivalent (e.g. `rlottie`/`velato`) — render
  frame → texture; FOUNDATION §13.5 keeps Lottie in v1.

**Dependencies:** E5-S2 (texture upload + cache infra).
**Parallel-safe?** Yes — `palmier-media`, distinct files from S2's video path.

---

## E5-S4 — Composition graph: CompositionFrame builder + clip→source mapping

**As** the preview engine, **I want** a per-frame `CompositionFrame` of bottom-to-top `LayerRender`s
built from the timeline, **so that** the compositor has an exact, reference-parity layer stack for any
visible frame.

**Acceptance criteria:**
- **Given** a `Timeline` and a frame index, **when** the builder runs, **then** it emits
  `CompositionFrame { frame_index, layers: Vec<LayerRender> }` with **layers bottom→top, track order =
  render order** (FOUNDATION §6.5), `.text` clips excluded from video layering (handled by E5-S9).
- **Clip → source frame mapping (FOUNDATION §6.5 step 1, preview-engine.md "insertClip" retime):**
  `sourceFrames = speed == 1 ? durationFrames : max(1, round(durationFrames * speed))`; source range
  `[trimStart, trimStart + sourceFrames)` placed at `clipStart`; images use
  `trimStart = max(0, trimStartFrame)`. **All source↔timeline rounding uses `f64::round`
  ties-AWAY-from-zero, NEVER `round_ties_even`** (carry-forward note).
- **Overlap semantics (risk #2):** the wgpu model composites arbitrary overlaps directly (no
  separate-track serialization), but **track order = z-order bottom→top** and clip precedence must
  match the reference. Black background = clear the wgpu target to black (no real layer — §6.5).
- Skip clips with `durationFrames <= 0`. Capture per-clip `natural_size` + `preferred_transform` as the
  reference does (bbox of `natSize.applying(preferredTransform)`, re-origined to (0,0)).
- For each layer, the builder requests the decoded frame from **E5-S2's FrameCache handle** (it does
  not decode).
- **Unit + Criterion (FOUNDATION §11.1/§11.4):** composition-build correctness on a known timeline;
  **the 50 / 200 / 1000-clip composition benchmark is gated here** (final assertion lands in E5-S11).

**Implementation context:**
- Reference: `CompositionBuilder.swift` (`static build(...) -> CompositionResult`) — the 38 KB core;
  port the build algorithm verbatim minus the AV-track muxing. preview-engine.md "Composition build"
  steps 1–5 + "Mapping → palmier-engine".
- Types: `CompositionFrame`, `LayerRender` (FOUNDATION §6.5 enum: Video/Image/Text/Lottie with
  `transform: Mat3`, `opacity`, `crop`).
- Crate: `palmier-engine`. Docs: FOUNDATION §6.5 frame-composition; ruling #22, #12 (visual-type
  interchange already enforced in Epic 3).

**Dependencies:** Epic 2 (`palmier-model` Timeline/Clip/Transform), E5-S2 (FrameCache handle).
**Parallel-safe?** Yes — `palmier-engine` build module; presentation-agnostic, runs alongside the spike.

---

## E5-S5 — Animated-property sampler (transform / opacity / crop, smoothstep parity)

**As** the composition builder, **I want** per-frame sampling of animated transform / opacity / crop /
volume that matches the reference curves exactly, **so that** preview and export frames are
byte-for-byte fidelity-equivalent to the macOS reference (SM-C1).

**Acceptance criteria:**
- **Given** a clip's keyframe tracks, **when** sampled at a frame, **then** the sampler returns the
  value using the reference interpolation modes: **`Hold` (step), `Linear`, `Smooth` (smoothstep) —
  default `Smooth`** (ruling #8). The **per-frame sampler samples the true smoothstep curve** rather
  than pre-baking ramps, but **must use the same `smoothSegments = 8` smoothstep math** so it matches
  the reference's pre-baked ramps (risk #4, carry-forward note).
- **Transform (preview-engine.md "Transform"):** base =
  `preferred_transform.concatenating(affineTransform(for: clip.transform))`. `affineTransform` maps
  normalized 0–1 canvas Transform → render-pixel affine: `sx = (renderW/natW)*t.width*(flipH?-1:1)`,
  `sy` analogous with `flipV`, `tx = (flipH? tl.x+t.width : tl.x)*renderW`, `ty` analogous; **rotation
  about `(centerX*renderW, centerY*renderH)`** via `translate(-c)·rotate(deg·π/180)·translate(c)`.
  Transform is **center-based** (ruling #7) — consume the center-based model from Epic 2.
- **Opacity:** layer hidden (opacity 0) until its clip is active; ramp/hold from `opacityTrack`; if a
  fade is present, build the piecewise-linear envelope over offset set
  `{0, dur} ∪ keyframes ∪ fadeIn/Out edges ∪ smooth subdivisions`; **clamp to [0,1]**, drop
  non-numeric/negative times.
- **Crop:** rect in source pixels `(left*natW, top*natH, visibleWidthFraction*natW,
  visibleHeightFraction*natH)` then `.applying(preferred_transform.inverted())`.
- **`refreshVisuals` fast path (risk #8):** re-sampling transform/opacity/volume must NOT trigger
  decode/rebuild — expose a visuals-only re-sample entry point distinct from full build.
- **Unit tests (FOUNDATION §11.1, §6.9 "palmier-engine"):** keyframe sampling at `t=0`, `t=end`,
  exact-on-key, and between-keys for **Smooth / Linear / Hold**, asserting parity with reference values
  — mirrors the Epic 2 keyframe-boundary test, here for the render sampler. Smoothstep parity test vs.
  the 8-segment reference subdivision.

**Implementation context:**
- Reference: `CompositionBuilder.swift` `buildVisuals(...)` (transform/opacity/crop ramps + smooth
  subdivision); preview-engine.md "Visual instructions (`buildVisuals`)" + risk #4. The volume side of
  the same offset-set algorithm is consumed by E5-S6.
- Crate: `palmier-engine` (sampler module feeding S4's builder). `affineTransform` math + smooth
  sampling port **verbatim** (preview-engine.md "Mapping → palmier-engine").

**Dependencies:** E5-S4 (builder consumes the sampled props; same crate).
**Parallel-safe?** Yes within `palmier-engine` if S4/S5 split builder vs. sampler modules cleanly;
otherwise sequence S5 right after S4.

---

## E5-S6 — Audio mixer: decode → resample → stretch → volume/fade envelope → cpal

**As** the playback engine, **I want** a Rust audio mixer that sums per-clip envelopes to the output
device, **so that** preview audio matches the reference's `AVMutableAudioMix` behavior.

**Acceptance criteria:**
- **Given** the active audio clips for a played frame range, **when** the mixer runs, **then** per
  clip: **symphonia decode → rubato resample to 48 kHz → time-stretch for `speed != 1.0`
  (rubato/signalsmith-stretch, pitch-preserving) → per-frame volume envelope (static × keyframe ×
  fade) → sum all clips → cpal output** (FOUNDATION §6.5 "Audio mixing").
- **Volume envelope** uses the **same offset-set / piecewise-linear ramp algorithm** as E5-S5's
  opacity (preview-engine.md "Audio mix"): no-keyframe-no-fade → one flat ramp; else piecewise ramps
  sampling `clip.volumeAt(frame)`. Interpolation modes `Hold`/`Linear`/`Smooth` (8-segment smoothstep).
  Muted track → volume 0.
- **Volume range −60…+15 dB** (ruling #9 — amplification allowed; the Inspector field/scale lives in
  Epic 12, but the mixer must accept >0 dB gain). Verify keyframe-storage dB floor against code before
  locking (ruling #9 open item).
- **Unit tests (FOUNDATION §11.1, §6.9):** volume + fade envelope correctness; speed-retime sample
  count; flat-vs-ramped envelope selection; muted-track silence.

**Implementation context:**
- Reference: `CompositionBuilder.swift` `buildVisuals` audio-mix branch
  (`AVMutableAudioMixInputParameters.setVolumeRamp`); preview-engine.md "Audio mix (`buildVisuals`)" +
  "Mapping → palmier-engine" (cpal/symphonia/rubato/signalsmith). Audio time-stretch engine choice is a
  preview-engine.md open question — rubato for resample, signalsmith-stretch for pitch-preserving speed.
- Crate: `palmier-engine` (audio mixer module). Reference relies on AVFoundation `scaleTimeRange` (no
  pitch preservation) — we improve to pitch-preserving stretch.

**Dependencies:** Epic 2 (`palmier-model` clip volume/fade/speed).
**Parallel-safe?** Yes — distinct audio module; presentation-agnostic.

---

## E5-S7 — Transport loop: SeekMode + scrub throttle/tolerance + reactive current_frame

**As** the editor, **I want** a transport that plays/pauses/seeks/steps and pushes `current_frame`
reactively, **so that** the timeline and asset tabs scrub and play with the reference's exact timing
feel.

**Acceptance criteria:**
- **Given** the transport API `play() / pause() / toggle_playback() / seek(frame, mode) /
  step(delta_frames)` (FOUNDATION §6.5), **when** invoked, **then** it drives the composition + audio
  and updates `current_frame` as a **reactive value over the Tauri event stream**.
- **SeekMode (preview-engine.md "Transport", FOUNDATION §6.5):**
  - **`Exact`** → tolerance 0, cancel pending seeks, land on the **exact** frame (playback start,
    frame stepping).
  - **`InteractiveScrub`** → tolerance **`min(0.75, 0.15 * activeLayerCount)` s** (timescale 600
    equivalent), **throttled to one dispatch per `1/30` s** via a coalescing pending-seek; always
    cancel pending seeks first.
- **FR-19 testable gate (PRD §4.5):** an **InteractiveScrub-tolerance test** asserts the displayed
  frame is within `min(0.75, 0.15*activeLayerCount)s` of the requested target for
  **activeLayerCount ∈ {1, 3, 6}**, and an **Exact-mode test** asserts the displayed frame equals the
  exact target.
- Periodic time observer at `1/fps` updates `current_frame` (timeline tab) or `source_playhead_frame`
  (asset tab) when playing and not scrubbing.
- **Two-tier rebuild (risk #8):** `rebuild()` = full `CompositionFrame` rebuild on structural change;
  `refresh_visuals()` = re-sample visuals only (transform/opacity/volume) on the existing frame graph —
  must NOT re-decode. Preserve this split (preview-engine.md "Transport" + risk #8).

**Implementation context:**
- Reference: `VideoEngine.swift` (`@MainActor` transport — `seek(frame, mode)`, `rebuild()`,
  `refreshVisuals()`, periodic time observer, interactive-scrub throttle). preview-engine.md
  "Transport (`VideoEngine`)" + risk #6 (port the exact throttle/tolerance constants).
- Crate: `palmier-engine` (transport). Reactive state to frontend via Tauri events (FOUNDATION §4
  strict layering — frontend never touches the engine directly).

**Dependencies:** E5-S4 (composition to drive), E5-S6 (audio to drive).
**Parallel-safe?** Yes — transport state machine is presentation-agnostic (it drives composition +
emits events; actual pixels are presented by E5-S8).

---

## E5-S8 — wgpu compositor pass + present via the S-1 surface

**As** the preview viewport, **I want** the `CompositionFrame` rendered by wgpu and presented into the
webview, **so that** the user sees the composited frame at the SM-2 FPS targets.

**Acceptance criteria:**
- **Given** a `CompositionFrame`, **when** the compositor runs, **then** it renders **bottom→top
  textured quads with `Mat3` affine transforms, premultiplied-alpha opacity blend, and crop**, clearing
  the target to **black** as the opaque floor (FOUNDATION §6.5 step 3, risk #2/#3).
- **Color:** single working color space, **everything BT.709** (risk #5); premultiplied-alpha blend
  state so straight-alpha sources don't fringe (risk #3).
- **Presentation:** present the rendered texture into the webview **using the mechanism chosen by
  E5-S1** — this story consumes S-1's decision record; it does NOT pick a mechanism. If S-1 selected a
  native child surface, position it over the viewport rect (matching cmd-scroll zoom geometry); if
  shared-handle/IPC, wire that path.
- **SM-2 acceptance:** 4K scrub ≥ 30 fps (5 clips / 2 layers, no keyframe motion) and 1080p60 ≥ 60 fps
  on §10 GPU. **If S-1 chose the CPU fallback**, the path degrades to **frame-stepped, live keyframe
  interpolation off** (FOUNDATION §3 — sanctioned SM-C1 waiver), and SM-2 is re-scoped per the S-1
  decision (decided at M1, not deferred).
- GPU floor: D3D12 12_0 / Vulkan 1.2 / 4 GB VRAM; below floor → CPU compositing via FFmpeg libavfilter
  (FOUNDATION §3).

**Implementation context:**
- Reference: the per-frame `AVVideoComposition` → `CompositionFrame` render (preview-engine.md
  "Mapping → palmier-engine" wgpu compositor: textured quads, affine `Mat3`, opacity blend, crop).
- Crate: `palmier-engine` (wgpu compositor) + the presentation seam established by E5-S1 in
  `palmier-tauri` / `src-ui/editor`.
- Docs: FOUNDATION §6.5 step 3–4, §3 GPU floor/fallback; SM-2, SM-C1.

**Dependencies:** **E5-S1 (hard — presentation mechanism)**, E5-S4 (CompositionFrame), E5-S5 (sampled
transforms).
**Parallel-safe?** **No** — it owns the wgpu render + present seam shared with E5-S1's output and with
the frontend viewport; gate it behind S-1.

---

## E5-S9 — palmier-text glyph runs + 30-frame preroll text layer

**As** the compositor, **I want** text clips rendered as cosmic-text glyph runs with the reference's
preroll, **so that** captions and titles appear with correct geometry and timing.

**Acceptance criteria:**
- **Given** a `.text` clip, **when** composited, **then** `palmier-text` produces a `LayerRender::Text {
  glyphs: Vec<GlyphRun>, transform, opacity }` via **cosmic-text** layout/shaping (+ `fontdb` for
  system + bundled fonts), rendered as textured quads / glyph atlas in the wgpu text pass.
- **30-frame preroll (carry-forward, FOUNDATION §6.6):** a text clip is materialized when
  `currentFrame >= clip.startFrame - 30 && < endFrame`; opacity from `opacityAt(frame)`; others evicted.
- **Geometry-flip parity (risk #7):** reference uses `isGeometryFlipped=true` + `containerH/1080`
  font-scale + normalized transform frames; cosmic-text is top-left origin — **verify Y math so text
  does not mirror vertically vs. video.**
- Style as shader uniforms: color, background fill, border, shadow, alignment (FOUNDATION §6.6).
- Bundle **the same reference fonts** (`Sources/PalmierPro/Resources/Fonts/`) — note this is part of the
  GPLv3 boundary (R-2), acceptable per OQ-11 working decision.
- **Unit tests:** glyph-run layout for a known string/transform; preroll window boundary
  (start−30 .. end); fade opacity sampling.

**Implementation context:**
- Reference: `TextLayerController.swift` (live `CATextLayer` tree, 30-frame preroll; export one-shot
  tree handled in Epic 6). preview-engine.md "Text (`TextLayerController`)" + risk #7; FOUNDATION §6.6.
- Crate: `palmier-text` (glyph runs) consumed by `palmier-engine`'s text pass (E5-S8).

**Dependencies:** E5-S5 (opacity/transform sampling), E5-S8 (wgpu text pass to render into).
**Parallel-safe?** Partial — glyph-run layout (`palmier-text`) is independent and can proceed early;
the render-into-pass wiring waits on E5-S8.

---

## E5-S10 — Preview tabs + transform / crop overlays (frontend)

**As** the editor user, **I want** preview tabs and direct-manipulation transform/crop overlays, **so
that** I can preview any asset or the timeline and adjust clip geometry in the viewport.

**Acceptance criteria:**
- **Given** the viewport, **when** rendered, **then** a horizontally-scrollable tab bar shows the
  **always-present, non-closable `.timeline` tab** plus **closable `.media_asset { id, name, type }`
  tabs**, each with **per-tab playback state** (FOUNDATION §6.5, `PreviewTab.swift`).
- **Transform overlay:** 4 corner handles, edge handles, rotation handle, center drag, **pink
  center-to-center snap guides**; **counter-rotated to clip-local axes** for accurate manipulation
  (FOUNDATION §6.5, `TransformOverlayView.swift`). Geometry operates on the **center-based** Transform
  (ruling #7).
- **Crop overlay:** rule-of-thirds guides, pan-inside + resize-edges, **aspect-lock toggle**, also
  counter-rotated (`CropOverlayView.swift`).
- Overlays are positioned against the presentation surface from **E5-S1** (native child vs.
  shared-canvas changes hit-testing/geometry); cmd/ctrl-scroll zoom + `videoRect` → overlay rescale
  (preview-engine.md `PreviewNSView`).
- Strict layering (FOUNDATION §4): overlay edits flow through Tauri commands into `palmier-engine` /
  the edit engines; no direct engine access from the webview.

**Implementation context:**
- Reference: `PreviewView.swift`/`PreviewNSView`, `PreviewContainerView.swift`, `PreviewTab.swift`,
  `TransformOverlayView.swift`, `CropOverlayView.swift`. preview-engine.md key types/files + open
  question (one engine per tab vs. shared with per-tab state — reference shares one `AVPlayer`, swaps
  item on tab activation; mirror with per-tab transport state).
- Crate: `src-ui/editor` (preview) + Tauri commands into `palmier-engine`.

**Dependencies:** **E5-S1 (presentation surface geometry)**, E5-S7 (transport per tab), E5-S8 (rendered
viewport).
**Parallel-safe?** **No** — frontend viewport shares the presentation seam with S-1/S8.

---

## E5-S11 — Criterion 50/200/1000-clip bench + SM-C1 golden-frame gate (both paths)

**As** the build, **I want** the composition benchmark and the golden rendered-frame fidelity gate
wired into CI, **so that** Epic 5 exits M1 with measured perf and proven fidelity on both the wgpu and
CPU-fallback paths.

**Acceptance criteria:**
- **§11.4 Criterion composition-graph benchmark** runs at **50 / 200 / 1000 clips** (the 1000-clip case
  is the explicit Epic 5 acceptance item) and is recorded as a perf baseline. Per-frame eval included.
- **SM-2 FPS gates** asserted on §10 HW: 4K scrub ≥ 30 fps (5 clips / 2 layers, no keyframe motion);
  1080p60 ≥ 60 fps — **or** the S-1 CPU-fallback re-scope is recorded (decided at M1).
- **SM-C1 golden rendered-frame comparison (PRD §10 Epic 5, §11.5):** per-frame **SSIM ≥ threshold or
  exact-within-tolerance** at known frames on **`golden_project_keyframes` + `golden_project_text`**,
  **run on BOTH the wgpu path AND the CPU fallback.** A fidelity regression (interpolation / BT.709
  color / layer drift) **fails the gate**. Golden regeneration gated behind `--update-golden` review;
  any diff in CI blocks merge (mirrors SM-7/SM-13 treatment).
- **Sanctioned SM-C1 waiver (S-1, R-1):** on the CPU-fallback path only, the **interpolation clause is
  waived** (frame-stepped, live interpolation off per FOUNDATION §3); **color + layer accuracy still
  bind on both paths.** Apply the waiver only to that branch.
- **FR-19 transport tests** (InteractiveScrub tolerance at layer counts 1/3/6 + Exact-mode exact-frame,
  from E5-S7) are part of the Epic 5 acceptance suite gated here.

**Implementation context:**
- Crate: `palmier-engine` (benches + golden harness). Golden fixtures: `golden_project_keyframes`,
  `golden_project_text` (FOUNDATION §11.5; shared golden-asset convention with Epics 2 and 6).
- Docs: PRD §7 (SM-2, SM-C1), §10 Epic 5 acceptance, §11.4/§11.5; FOUNDATION §3 (fallback), §11.

**Dependencies:** E5-S8 (rendered frames to compare), E5-S9 (text rendering for `golden_project_text`).
**Parallel-safe?** **No** — it is the integrating gate; lands last in Epic 5.

---

## Cross-epic dependency summary

- **Upstream (must land first):** Epic 2 — `palmier-model` (Timeline/Clip/**center-based** Transform,
  ruling #7; Keyframe modes + **Smooth default**, ruling #8; serde) — blocks E5-S2/S4/S5/S6. Epic 2's
  **keyframe-boundary** and **`f64::round` ties-away** parity tests are the foundation E5-S5 builds on.
- **Spike gate:** **E5-S1 (= Spike S-1)** blocks E5-S8 / E5-S9 / E5-S10 (every presentation/viewport
  story). Pure-Rust S2–S7 run in parallel with the spike.
- **Downstream consumers:** Epic 6 (Export) **reuses the same composition path** (E5-S4/S5 + the wgpu
  compositor E5-S8) for encode, and the text export-tree (E5-S9 sibling); Spike S-4 confirms the
  readback/NVENC boundary for export. Epic 10 (CaptionBuilder) emits `TextClipSpec`s rendered by the
  E5-S9 text path. Epic 12 owns the Inspector volume field (−60…+15 dB, ruling #9) consuming E5-S6's
  mixer range.

## Parallelization note (for the orchestrator)

Run **E5-S1 first and alone** (it gates the architecture commit). Concurrently with S-1, the
presentation-agnostic Rust stories form two independent lanes that can each run in their own worktree:
**media lane** (E5-S2 → E5-S3) and **engine lane** (E5-S4 → E5-S5 → E5-S7, with E5-S6 parallel). Once
S-1 lands, **E5-S8** unblocks, then **E5-S9** and **E5-S10**, and finally **E5-S11** integrates and
gates. S4/S5 and S5/S7 share the `palmier-engine` crate — split by module (build / sampler / transport
/ audio) to keep worktrees non-colliding, else sequence within the engine lane.
