---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# preview-engine — reference port notes

## Purpose
Document the macOS reference preview/playback pipeline under `Sources/PalmierPro/Preview/` so the
Windows+Linux port can rebuild it as a wgpu+FFmpeg composition graph (`palmier-engine` + `palmier-media`).
The reference builds an `AVMutableComposition` from the `Timeline`, attaches an `AVVideoComposition`
(layer instructions for transform/opacity/crop) + `AVMutableAudioMix` (volume envelopes), plays it
through one `AVPlayer`/`AVPlayerLayer`, overlays text via a live `CALayer` tree, and exports via
`AVAssetExportSession` (text baked in with `AVVideoCompositionCoreAnimationTool`). Every Apple API here
must be replaced; the single biggest risk is how rendered GPU frames reach the screen (`AVPlayerLayer`
draws itself; wgpu must instead present a texture into the WebView — see Port risks).

## Key types & files (under Sources/PalmierPro/Preview/...)
- `VideoEngine.swift` — `@MainActor` transport. Owns one `AVPlayer`, drives rebuild/seek/playback,
  periodic time observer, interactive-scrub throttle. Maps `palmier-engine` transport API.
- `CompositionBuilder.swift` (38 KB, the core) — `static build(...) -> CompositionResult`: assembles
  `AVMutableComposition` tracks; `buildVisuals(...)`: emits `AVVideoComposition` + `AVMutableAudioMix`
  from keyframes. Pure algorithm; reimplement verbatim in `palmier-engine`.
- `TimelineRenderer.swift` — frame-range export to temp mp4 via `AVAssetExportSession` (used for
  selection renders; full export lives in `palmier-export`).
- `ImageVideoGenerator.swift` — bakes a still image (and a black background) to a 1800 s `.mov`
  (ProRes4444 if alpha, else H.264) so `AVPlayer` can treat images as video.
- `LottieVideoGenerator.swift` — renders Lottie (`lottie-ios`, mainThread engine) to a ProRes4444 alpha
  `.mov`; freeze-frame held to 1800 s.
- `AlphaVideoNormalizer.swift` — transcodes straight-alpha video to premultiplied-alpha ProRes4444 via
  `AVAssetReader`/`AVAssetWriter` + `vImagePremultiplyData_RGBA8888`.
- `TextLayerController.swift` — live `CATextLayer` tree (preview) + one-shot tree for export
  (`AVVideoCompositionCoreAnimationTool`); 30-frame preroll.
- `PreviewView.swift` / `PreviewNSView` — `NSViewRepresentable` hosting `AVPlayerLayer` + text `CALayer`;
  cmd+scroll zoom; `videoRect` → text rescale. **This is the screen-presentation surface to replace.**
- `PreviewTab.swift` — `.timeline` | `.mediaAsset(id,name,type)` tab enum.

## Core behaviors & algorithms (downstream implements from this)
Time base: `timescale = CMTimeScale(timeline.fps)`; frame N → `CMTime(value: N, timescale: fps)`.
`frameDuration = 1/fps`. Color tags forced to BT.709 (`ITU_R_709_2` primaries/transfer/matrix).

**Composition build (`CompositionBuilder.build`)** — per track, bottom→top (track order = z-order):
1. Sort clips by `startFrame`; drop `.text` clips (text never becomes a composition track).
2. Video tracks: one `AVMutableCompositionTrack`. Per clip, skip if `durationFrames<=0` or
   `startFrame < previousEndFrame` (no overlap on a single AV track — overlaps need separate tracks;
   here same-track clips are serialized). Insert gap as `insertEmptyTimeRange` when `clipStart>cursor`.
   `sourceFrames = speed==1 ? durationFrames : max(1, round(durationFrames*speed))`; insert
   `[trimStart, trimStart+sourceFrames)` of source at `clipStart`; if `speed!=1` then
   `scaleTimeRange(...toDuration: durationFrames)` (retime). Images use `trimStart = max(0, trimStartFrame)`.
3. Capture per-clip `naturalSize` and `preferredTransform`: store display size as bbox of
   `natSize.applying(preferredTransform)` and a normalized transform that re-origins bbox to (0,0).
4. Audio tracks: clips with `speed==1` share one track; `speed!=1` clips each get their own (so
   `scaleTimeRange` retime is isolated).
5. After all tracks, insert a **black background** video track (bottommost) spanning
   `max(totalFrames, lastVideoEnd)` via `ImageVideoGenerator.blackVideo`. This is the opaque floor —
   in wgpu just clear the target to black; do NOT need a real layer.

**Visual instructions (`buildVisuals`)** produce, per video track mapping, an
`AVVideoCompositionLayerInstruction`:
- Initial opacity 0 at t=0 (layer hidden until its clip is active); set 0 again at clip `end` and at
  `mapping.endTime` if before composition end. The black bg goes opacity 1 at range.start, 0 at range.end.
- **Transform**: base = `preferredTransform.concatenating(affineTransform(for: clip.transform))`.
  `affineTransform` maps normalized 0–1 canvas Transform → render-pixel affine:
  `sx = (renderW/natW)*t.width*(flipH?-1:1)`, `sy = (renderH/natH)*t.height*(flipV?-1:1)`,
  `tx = (flipH? tl.x+t.width : tl.x)*renderW`, `ty` analogous; rotation applied about
  `(centerX*renderW, centerY*renderH)` via translate(-c)·rotate(deg·π/180)·translate(c). Static clips:
  one `setTransform`. Animated: union of position/scale/rotation keyframe offsets, each segment
  subdivided into `smoothSegments=8` ramps using fractional CMTimes (integer-frame rounding would
  collapse short spans → must replicate in `palmier-engine` sampler).
- **Crop**: rect in source pixels `(left*natW, top*natH, visibleWidthFraction*natW, visibleHeightFraction*natH)`
  then `.applying(preferredTransform.inverted())`; static `setCropRectangle` or `addCropRectangleRamp`.
- **Opacity**: if no fade, ramp/hold from `opacityTrack` (else static `clip.opacity`). If fade present,
  build a piecewise-linear envelope over offset set `{0, dur} ∪ keyframes ∪ fadeIn/Out edges ∪ smooth
  subdivisions`; sample `opacityAt(frame)`. Clamp opacity to [0,1], drop non-numeric/negative times.

**Audio mix (`buildVisuals`)**: per audio mapping, `AVMutableAudioMixInputParameters`; muted track →
`setVolume(0)`. Else per clip emit a volume envelope: if no keyframes and no fade, one flat
`setVolumeRamp(v,v,[start,end])`; else piecewise-linear ramps via the same offset-set algorithm
sampling `clip.volumeAt(frame)` (folds static × keyframe × fade). Keyframe interpolation modes:
`.hold` (step), `.linear`, `.smooth` (8-segment `smoothstep`).

**Transport (`VideoEngine`)**:
- `seek(frame, mode)`: `.exact` → tolerance 0, cancel pending; `.interactiveScrub` → tolerance
  `min(0.75, 0.15*activeVideoLayerCount)` s (timescale 600), throttled to one dispatch per `1/30` s via
  a coalescing pending-seek; always `item.cancelPendingSeeks()` first. `textController.tick(frame)` runs
  synchronously every seek.
- `play`/`pause`/`toggle`/`resume` drive `AVPlayer` + `editor.isPlaying`.
- `rebuild()` runs `CompositionBuilder.build` in a cancellable `Task`, swaps `AVPlayerItem`
  (`asset`+`audioMix`+`videoComposition`), re-syncs text, re-seeks. `refreshVisuals()` rebuilds only
  audioMix+videoComposition on the existing item when track mappings already exist (cheap edit path —
  port must keep this fast-path: re-sample instructions without re-decoding).
- Periodic time observer at `1/fps` updates `editor.currentFrame` (timeline tab) or
  `sourcePlayheadFrame` (asset tab) when playing and not scrubbing.

**Text (`TextLayerController`)**: preview keeps a long-lived `CALayer` root (`isGeometryFlipped=true`)
sized to `playerLayer.videoRect`; per `tick`, materialize a `CATextLayer` only when
`currentFrame >= clip.startFrame - 30 && < endFrame` (30-frame preroll), set opacity from
`opacityAt(frame)`, evict others. Style: frame from normalized `transform`, fontSize scaled by
`containerH/1080`, attributed string, alignment, optional background/border/shadow. Export builds a
one-shot tree with a discrete `CAKeyframeAnimation` on opacity (one value per frame,
`values.count == keyTimes.count-1`) fed to `AVVideoCompositionCoreAnimationTool`.

**Image/Lottie/alpha generators** (cached on disk, keyed by mediaRef + size + file mtime/size):
- Image: clamp to even dims ≤4096; premultipliedFirst BGRA `CGContext`; 2-frame `.mov` (t=0 and
  t=ceil(1800)-1) so a still is freely resizable/extendable. Alpha → ProRes4444, opaque → H.264.
- Black: same writer, cleared to black, H.264.
- Lottie: `lottie-ios` mainThread engine, render each frame into a CVPixelBuffer via `layer.render(in:)`
  with a top-left flip, ProRes4444; appends a final freeze frame at 1800 s.
- AlphaNormalizer: only when codec format ext `ContainsAlphaChannel==true` and preferredTransform is
  identity; reads 32BGRA, `vImagePremultiplyData_RGBA8888` in place, writes ProRes4444.

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `AVMutableComposition` / `AVMutableCompositionTrack` (insertTimeRange, insertEmptyTimeRange,
  scaleTimeRange) → in-memory composition graph in `palmier-engine`; no real track muxing — resolve
  per-frame which clips are active and request decoded frames from `palmier-media`.
- `AVPlayer` / `AVPlayerItem` / addPeriodicTimeObserver / seek(toleranceBefore/After) → custom transport
  loop in `palmier-engine` (play/pause/seek/step + reactive `current_frame` via Tauri events).
- `AVPlayerLayer` (`videoGravity`, `videoRect`) → **wgpu render target presented into WebView**
  (FOUNDATION SS4 "shared WebGPU surface", SS6.5 step 4). Biggest risk — see below.
- `AVVideoComposition` + `AVVideoCompositionInstruction` + `AVVideoCompositionLayerInstruction`
  (setTransform/addTransformRamp, setOpacity/addOpacityRamp, setCropRectangle/addCropRectangleRamp) →
  per-frame `CompositionFrame { layers: Vec<LayerRender> }` sampled in Rust; ramps replaced by sampling
  the keyframe value at the exact frame (no pre-baked ramp list needed when sampling per frame).
- `AVMutableAudioMix` / `AVMutableAudioMixInputParameters.setVolumeRamp` → per-sample volume envelope in
  the `palmier-engine`/audio mixer (cpal output; symphonia decode; rubato/signalsmith for speed).
- `AVAssetExportSession` (`export(to:as:)`, presets, timeRange) → FFmpeg muxer+encoder loop
  (`palmier-export`, SS9 video export); selection render = same path over a frame sub-range.
- `AVVideoCompositionCoreAnimationTool` (text overlay bake) → rasterize text track to overlay textures
  per output frame and composite in the wgpu pass (FOUNDATION SS6.6 export-mode).
- `AVAssetWriter`/`AVAssetReader`/`AVAssetWriterInputPixelBufferAdaptor` (generators, normalizer) →
  FFmpeg encode/decode in `palmier-media`. **Note:** the still/Lottie "bake to .mov" trick exists only
  because `AVPlayer` can't play images/JSON. The wgpu port should decode images/Lotties **directly to a
  GpuTexture** (Image/Lottie are first-class `LayerRender` variants in SS6.5) — drop the .mov bake
  entirely. Keep the disk cache concept but key textures, not movies.
- `CALayer`/`CATextLayer`/`CAKeyframeAnimation`/`CATransaction` → `palmier-text` (cosmic-text + fontdb)
  glyph runs rendered as textured quads/SDF; preroll concept kept (SS6.6).
- `CVPixelBuffer`/`CGContext`/`CGImageSource`/`NSImage` (pixel buffers, image probing, alpha detection)
  → FFmpeg/`image` crate decode; alpha detection from codec/pixfmt; premultiply in a shader or CPU pass.
- `vImagePremultiplyData_RGBA8888` (Accelerate) → straightforward shader/SIMD premultiply, or premultiply
  in the wgpu blend (use premultiplied-alpha blending so straight-alpha sources work without a pre-pass).
- `CMFormatDescription` alpha-channel extension → FFmpeg pixfmt has-alpha check.
- `CMTime`/`CMTimeRange`/`CMTimeScale` → integer frame indices + `(fps numerator/denominator)`; avoid
  float seconds where the reference used fractional CMTimes only to dodge integer-frame collapse.

## Mapping to FOUNDATION crates (palmier-engine, palmier-media)
- `palmier-media`: replaces all generators + normalizer + decode. One `DecoderThread` per source URL
  (FFmpeg `AVFormatContext`+`AVCodecContext`, HW decode when available), LRU `FrameCache` keyed by
  `(media_ref, source_frame)`, 1.5 GB VRAM / 512 MB RAM ceilings (SS6.5). Image/Lottie decode straight to
  GpuTexture. Thumbnails/waveforms also here.
- `palmier-engine`: replaces `CompositionBuilder` + `VideoEngine` + `AVVideoComposition`/`AudioMix`.
  Build `CompositionFrame` per visible frame (SS6.5 frame-composition loop), wgpu compositor (textured
  quads, affine `Mat3`, opacity blend, crop), transport (`SeekMode` Exact/InteractiveScrub mirroring the
  reference's tolerance/throttle), audio mixer. `affineTransform` math + smooth-keyframe sampling port
  here verbatim. `palmier-export` reuses the same composition path for encode (SS9).
- `palmier-text`: `TextLayerController` → cosmic-text glyph runs; 30-frame preroll retained.
- Timeline→source-frame mapping (`start_frame`/`trim_start_frame`/`speed`) is SS6.5 step 1; matches the
  reference `insertClip` retime math exactly (`sourceFrames = round(durationFrames*speed)`).

## Port risks & gotchas
1. **BIGGEST: GPU frame → screen.** Reference uses `AVPlayerLayer`, which self-draws a decoded surface
   in an `NSView`. The port has no such layer: wgpu renders to a texture that must appear in the WebView.
   FOUNDATION SS4 says "shared WebGPU surface"; SS6.5 step 4 says "present the rendered texture to the
   WebGPU canvas in the webview." There is **no spec'd mechanism** for sharing a Rust-owned wgpu texture
   into WebView2 (Win) / WebKitGTK (Linux) — `<canvas>` WebGPU contexts are owned by the WebView's GPU
   process. Plausible approaches, all need validation: (a) render in Rust to a shared D3D11/D3D12 swap-
   chain or DXGI shared handle composited under/over the WebView (DirectComposition on Win; on Linux a
   GTK GL/Vulkan area or dmabuf); (b) read back the texture and push frames over IPC to a JS-side WebGPU
   canvas (simplest, but a full-res RGBA readback per frame at 30–60 fps is the perf cliff); (c) a
   transparent native child surface positioned over the viewport rect (mirrors how `AVPlayerLayer` sat in
   the NSView, and matches the cmd-scroll zoom geometry already in `PreviewNSView`). Recommend (a)/(c);
   prototype before committing. This unknown gates the whole preview crate.
2. **Overlap semantics differ.** The reference serializes single-track clips and forbids on-track overlap
   (`startFrame >= previousEndFrame`), pushing real overlaps onto separate tracks/z-order. The wgpu
   per-frame model can composite arbitrary overlaps directly — make sure track→z-order and clip
   precedence match the reference (track order = render order, bottom→top).
2.5. **Drop the still/Lottie .mov bake.** Reimplementing it 1:1 (1800 s movies, 2-frame stills, freeze
   tails) would be wasted work — it only exists to satisfy `AVPlayer`. SS6.5 makes Image/Lottie texture
   layers. But preserve the *cache keying* (mediaRef + size + mtime) and the 4096 px / even-dimension
   clamps if you keep an encode path for export.
3. **Premultiplied alpha.** Reference normalizes straight-alpha video to premultiplied (ProRes4444) and
   bakes images premultipliedFirst. In wgpu, use premultiplied-alpha blend state and premultiply on
   texture upload; otherwise alpha edges fringe. Only trust the codec alpha flag (reference does), not
   container capability.
4. **Smooth-keyframe subdivision (8 segments, smoothstep).** The reference pre-bakes ramps because
   AVFoundation interpolates linearly between instruction times; a per-frame sampler in `palmier-engine`
   can sample the true curve each frame and skip subdivision — but must use the **same** smoothstep so
   exported frames match. Unit-test parity (FOUNDATION SS "palmier-engine" tests).
5. **Color management.** Everything forced to BT.709; image/Lottie bakes use sRGB transfer. Keep a single
   working color space in the compositor and tag exports BT.709 to match reference output.
6. **Interactive-scrub throttle + tolerance** (`1/30` s coalesced dispatch; tolerance scaled by active
   layer count, capped 0.75 s) is a UX-critical heuristic — port the exact constants.
7. **Text geometry flip.** Reference text uses `isGeometryFlipped=true` + container-height/1080 scale and
   normalized transform frames. cosmic-text uses top-left origin already; verify Y math matches so text
   doesn't mirror vertically vs. video.
8. **`refreshVisuals` fast path.** Editing transform/opacity/volume must NOT trigger a full
   decode/rebuild — only re-sample instructions. Preserve this two-tier (build vs. visuals-only) split.

## Open questions
- Exact mechanism for presenting the wgpu texture into WebView2 / WebKitGTK (risk #1) — undecided in
  FOUNDATION; needs a spike (DXGI shared-handle + DirectComposition vs. IPC readback vs. native child
  surface). Does the timeline canvas share the same surface or is preview a separate native overlay?
- Is the 1.5 GB VRAM / 512 MB RAM cache ceiling per-asset or global? (SS6.5 reads global.)
- Does the port keep any disk-cached encoded intermediates, or decode-on-demand only? (Reference caches
  baked .mov per mediaRef+size.)
- Audio time-stretch engine choice (rubato vs. signalsmith-stretch) for `speed != 1.0` pitch-preserving —
  FOUNDATION lists both; reference relies on AVFoundation `scaleTimeRange` (no pitch preservation).
- How are multiple `PreviewTab`s' transports isolated — one engine instance per tab or shared with
  per-tab state? Reference shares one `AVPlayer` and swaps the item on tab activation.
