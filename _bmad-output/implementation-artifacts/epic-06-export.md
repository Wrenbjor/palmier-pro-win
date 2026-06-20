---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
title: "Epic 6 — Export (video / XMEML / bundle)"
created: 2026-06-20
---

# Epic 6 — Export (video / XMEML / bundle)

## Epic goal

Port the three export modes of the macOS reference `Sources/PalmierPro/Export/` to the
Windows/Linux Rust stack: (1) rendered **video** (H.264 / H.265 / **ProRes 422 LPCM**) through the
wgpu+FFmpeg pipeline that shares the composition graph with preview; (2) the **FCP7 XMEML 4** XML
emitter (Premiere/DaVinci/FCP interchange — byte-exact golden-test-critical); (3) the self-contained
**`.palmier` bundle** export; plus (4) the **social-platform sidecar** (`<export>.palmier-meta.json`)
handoff. It is a behavior-parity port: the XMEML byte structure must match committed golden fixtures
exactly, the video path may swap engines (AVFoundation → wgpu+FFmpeg per FOUNDATION §6.12) but must
preserve the same compositing semantics, and the bundle export must reuse the reference filenames and
`MediaManifest`/`MediaSource` serde types.

## PRD acceptance this epic must satisfy (PRD §4.6 / §10 Epic 6)

- **FR-21 Video export.** Per output frame, build composition (same path as preview) → render to wgpu
  texture → read back (or NVENC zero-copy) → FFmpeg encode; mix audio → AAC; mux. Codecs H.264, H.265,
  **ProRes 422 LPCM** (ruling #17 — 4444+alpha deferred, OQ-1). **SM-5:** 1 min 1080p H.264 exports
  faster than real time on an RTX 4060 (NVENC, balanced). Progress + cancellation via Tauri events.
- **FR-22 XMEML export (golden fidelity).** Emit FCP7 XMEML 4 byte-for-byte against committed golden
  fixtures: 2-space indent, `\n` joins, self-closing tags, exact escape order, `TRUE`/`FALSE` literals,
  exact float formats, drop-frame `round(fps*0.066666)`, `file://localhost//` rewrite, rotation negated,
  center as normalized offset-from-0.5. **SM-7:** CI diff against goldens is exact. Text overlays / flips
  / custom easing are documented as XML-unsupported.
- **FR-23 Self-contained bundle export.** Rewrite `External` media refs → `Project { relative_path }`,
  copy into `media/`, copy log/chat/thumbnail; report `collected, copied_internal, missing, total_bytes`.
- **FR-24 Social-platform sidecar.** "Export to Social" emits MP4 + `<export>.palmier-meta.json`
  (transcript, chapter markers, AI-suggested captions, source project hash). **Schema is OQ-7** — M1
  ships a best-effort sidecar under a provisional schema; the frozen schema + full UJ-5 land at M5.

**Milestone (PRD §12):** **M1 — Hand-Edit MVP** (Epics 1–6). Export realizes **UJ-5 partial** (MP4 +
best-effort sidecar; frozen sidecar schema at M5). M1 validates **SM-5** (export speed) and **SM-7**
(XMEML golden + round-trip fidelity). XMEML/bundle land fully in M1; the sidecar's *frozen* schema is
explicitly deferred to **M5** (do not block M1 on OQ-7).

**Crates:** `palmier-export` (owner), `palmier-engine` (shared composition graph, consumed via handle),
`palmier-media` (decode + FrameCache + FFmpeg mux/encode), `palmier-text` (text rasterization),
`palmier-tauri` + `src-ui/export` (export panel, progress/cancel wiring — UI ported separately).

---

## Spike / risk gate

This epic has **two gating concerns**, both narrower than Epic 5's blocker:

- **Depends on Epic 5's composition graph (S-1-gated upstream).** The video path (FR-21) reuses the
  **same per-frame composition** the preview builds (`palmier-engine`). Epic 5 is gated by **Spike S-1**
  (wgpu→WebView presentation). **Export does NOT need the WebView presentation mechanism** — it renders
  to an offscreen wgpu texture and reads it back / NVENC-encodes; it never presents to the webview. So
  the **XMEML path (E6-S1..S4), the bundle path (E6-S7), and the sidecar (E6-S8) are fully independent of
  S-1** and can proceed in M1 in parallel with Epic 5. The **video-render path (E6-S5, E6-S6) depends on
  the Epic 5 composition-graph API** (`palmier-engine` frame builder + `palmier-media` FrameCache handle),
  not on S-1's presentation outcome.
- **Spike S-4 [before E6-S5, low-risk confirm] — wgpu texture readback / NVENC zero-copy.** PRD §11 S-4:
  confirm the export readback path (or NVENC zero-copy) feeds FFmpeg at **faster-than-real-time** for SM-5.
  **Pass bar:** the readback+encode path sustains ≥ SM-5 throughput (1 min 1080p H.264 < real time, NVENC
  balanced) on the §10 reference GPU (RTX 4060), measured end-to-end. **E6-S5 is gated on S-4** — do not
  commit the video-encoder architecture until S-4 lands. S-4 is small and can run in parallel with the
  XMEML/bundle work. **S-4 is the first video-path story below (E6-S0).**

The **XMEML emitter is a pure `timeline -> String` function with no I/O and no GPU** — it is the highest-
value, lowest-risk, most-parallel work in this epic and should start immediately, independent of every
spike.

---

## Stories

### E6-S0 — Spike S-4: wgpu readback / NVENC zero-copy throughput confirm

*As the build team, I want to confirm the export readback+encode path hits SM-5 throughput before
committing the video-encoder architecture, so the FR-21 pipeline isn't designed on an unproven boundary.*

**Acceptance criteria:**
- Given a synthetic 1-minute 1080p sequence of wgpu-rendered RGBA textures, when fed through the chosen
  readback path (CPU readback **or** NVENC zero-copy via `ffmpeg-next`/`gstreamer-rs` fallback per
  FOUNDATION §2 stack) into the FFmpeg `libx264`/NVENC `h264_nvenc` encoder at the **balanced** preset,
  then end-to-end wall-clock encode time is **< 60 s on the §10 reference GPU (RTX 4060)** — i.e. faster
  than real time (**SM-5**).
- Records, in this epic doc's Timeline and in a `docs/` artifact, the chosen mechanism (CPU readback vs
  NVENC zero-copy), the measured throughput, and the FFmpeg encoder/muxer config per format×resolution
  (there is **no** 1:1 AVFoundation preset enum — build bitrate/profile config per `docs/reference/export.md`
  §"macOS/Apple APIs to replace"). Documents the CPU-fallback encode path (libavfilter/libx264) for the
  sub-GPU-floor case (FOUNDATION §3, R-8).
- If neither path hits SM-5, records an explicit re-scope decision (do not silently miss SM-5).

**Implementation context:** Crates `palmier-media` (FFmpeg encode/mux owner per FOUNDATION §4), `palmier-export`.
Reference: `docs/reference/export.md` §A + §"macOS/Apple APIs to replace" (AVAssetExportSession → FFmpeg
muxer/encoder libx264/libx265/NVENC/prores_ks; AVVideoComposition color tags BT.709). PRD §11 Spike S-4.
**Dependencies:** none (synthetic textures; does not need real composition). **Parallel-safe?** Yes —
spike-only, isolated `palmier-media` encoder probe; no shared production files.

---

### E6-S1 — XMEML `XMLNode` tree + `render`/`escapeXML` whitespace-exact core

*As a dev, I want the structure-vs-whitespace split (`XMLNode` tree + `render`) ported literally, so the
XMEML emitter produces byte-stable output that golden-diffs trivially.*

**Acceptance criteria:**
- Implements an `XMLNode` tree (element name, ordered attrs, optional text, ordered children) and a pure
  `render(node, indent) -> String` that owns **all** whitespace, exactly per `docs/reference/export.md` §B:
  leaf → `<name attrs>escaped-text</name>`; element-with-children → open tag, `\n`, children joined by
  `\n` at **indent+2**, `\n`, close; empty+no-text → `<name/>` self-closing. Indent = **2 spaces per
  level**, starts at 0.
- `escapeXML` escapes in **exact order** `& < > " '` → `&amp; &lt; &gt; &quot; &apos;`.
- Bools render literal `TRUE`/`FALSE`.
- Float formatting uses a **fixed-locale (C-locale) formatter** matching Swift `String(format:)`: the spec
  strings `%.4f`, `%.5f`, `%.1f`, `%.2f` produce identical bytes to the reference (Rust `format!` is
  locale-independent — verify against a fixture of edge values incl. negative, `0.0`, ties).
- Document prolog is emitted exactly: `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE xmeml>\n` then the
  rendered tree.
- **Unit tests:** a `render`-golden test over a hand-built node tree covering all four node shapes (leaf,
  children, empty self-close, attrs), the escape table, and each float spec; bytes asserted exact.

**Implementation context:** Crate `palmier-export`. Pure functions, **no I/O, no AV, no GPU** (FOUNDATION
§4: "FCP7 XML emitter"; `docs/reference/export.md` §"Mapping to FOUNDATION crates" — keep the
`XMLNode`/`render` split, "it is why goldens are stable"). Reference file
`Sources/PalmierPro/Export/XMLExporter.swift` (the `render(node,indent:)` + `escapeXML` internals).
docs/reference/export.md §B opening paragraph. **Dependencies:** Epic 2 (`palmier-model` Timeline/Clip/Track
serde types must exist) — but the renderer itself only needs primitive node types, so it can be built
against stub model types and wired later. **Parallel-safe?** Yes — new files in `palmier-export`, no shared
state.

---

### E6-S2 — XMEML document shell + track/clipitem/file emission

*As a dev, I want the full XMEML document tree (sequence shell, tracks, clipitems, file dedup) built from a
timeline, so a real project emits a structurally-correct FCP7 sequence.*

**Acceptance criteria:**
- Emits the **document shell** exactly per `docs/reference/export.md` §B "Document shell":
  `<xmeml version="4"><sequence id="sequence-1">` with `name`=`Timeline Export`, `duration`=
  `timeline.totalFrames`, `<rate><timebase>{fps}</timebase><ntsc>FALSE</ntsc></rate>`, the `<timecode>`
  block (`00:00:00:00`, frame 0, source `source`, displayformat `NDF`), and `<media>` with the fixed
  `<video><format><samplecharacteristics>` (width/height, anamorphic FALSE, pixelaspectratio square,
  fielddominance none, rate) and the fixed `<audio>` block (numOutputChannels 2, samplerate 48000, depth
  16, the `<outputs><group>` 2-channel downmix structure).
- **Track ordering:** video tracks emitted **reversed** (FCP7 bottom→top vs model top→bottom); audio tracks
  **natural** order. Each track filtered to clips with a **resolvable URL**, sorted by `startFrame`. Per
  track emits `<track><enabled>{!hidden / !muted}</enabled><locked>FALSE</locked>` then per-clip optional
  left fade, `<clipitem>`, optional right fade.
- **`<clipitem id="clipitem-{clip.id}">`** children in **exact order**: `masterclipid`, `name`
  (resolver.displayName), `enabled TRUE`, `duration` (source dur frames), `rate`, `start` (startFrame),
  `end` (endFrame), `in` (trimStartFrame), `out` (trimStartFrame+sourceFramesConsumed), `<file>`, filters,
  `<link>` nodes. `masterclipid` = `masterclip-{linkGroupId}` if linked else
  `masterclip-{mediaRef}-{video|audio}`. `sourceFramesConsumed = round(durationFrames*speed)` using
  **`f64::round` ties-away-from-zero** (reconciliation carry-forward; never `round_ties_even`).
- **`<file id="file-{mediaRef}-{video|audio}">`** emitted in **full once** per (mediaRef, isAudio);
  repeats collapse to `<file id="..."/>`. Full form: `name` (url.lastPathComponent), `pathurl`, `rate`,
  `duration`, `<timecode>`, `<media>`. **`pathurl`** = `url.absoluteString` with `file://`→`file://localhost//`
  (Premiere needs the extra slash; reproduce byte-for-byte), fallback `media/{mediaRef}`. Image entries:
  file `duration`=1 and `<media><video>` gets `<duration>1</duration>` before samplecharacteristics.
- **`rateTags(forFPS)`:** `timebase=round(fps)`; `ntsc=TRUE` iff
  `|raw - timebase*1000/1001| < |raw - timebase|` (catches 23.976/29.97/59.94). Audio `<media>`:
  samplerate 48000, depth 16, channelcount 2.
- **Unit tests:** emit `golden_project_minimal.palmier` (single video clip, 1 track) and assert against
  `golden_xmeml_minimal.xml` byte-exact; assert file-dedup collapse (two clips of one media → one full
  `<file>` + one `<file id=.../>`); assert reversed-video / natural-audio ordering on a 2-video-1-audio
  fixture.

**Implementation context:** Crate `palmier-export`. Reference `Sources/PalmierPro/Export/XMLExporter.swift`
`Builder.build()`, `docs/reference/export.md` §B "Document shell" / "Track ordering" / "clipitem" / "file".
Uses `palmier-model` Timeline/Clip/Track + a `MediaResolver` (resolver.displayName, entry.duration,
`secondsToFrame(s,fps)=Int(s*fps)` from `Utilities/TimeFormatting.swift`). **Dependencies:** E6-S1 (render
core), Epic 2 (`palmier-model`). **Parallel-safe?** Partially — shares `palmier-export` XMEML module with
E6-S1/S3/S4; sequence after E6-S1, can interleave with E6-S3/S4 if module is split by file (builder vs
filters vs timecode). Treat E6-S2..S4 as **one dev's sequential thread** unless split.

---

### E6-S3 — XMEML filters: motion / crop / opacity / time-remap / audio-levels (+ keyframes)

*As a dev, I want the per-clip XMEML filter blocks emitted with the reference's diff-only and keyframe
sampling rules, so transforms, speed, opacity, crop and audio levels round-trip into Premiere correctly.*

**Acceptance criteria:** each filter wrapped `<filter><effect>...` with effect children
`name, effectid, [effectcategory], effecttype, mediatype, body`, per `docs/reference/export.md` §B "Filters":
- **Time Remap** (only if `speed != 1.0`): id `timeremap`, type `motion`; params variablespeed(value 0),
  speed (`value=%.4f` of speed×100), reverse FALSE, frameblending FALSE.
- **Basic Motion** (video): id `basic`, type `motion`. **Static — emit only params that differ:** scale if
  `|scaledPct-100| > 0.1`, rotation if `|−rot| > 0.05`, center if `|c| > 0.001`. `scalePct = sourceW>0 ?
  (seqW/sourceW)*t.width*100 : t.width*100`. **`rotation = −t.rotation`** (FCP7 CCW-positive; model
  CW-positive). **`center = (centerX−0.5, centerY−0.5)`** normalized → `<value><horiz>%.5f</horiz><vert>%.5f</vert></value>`
  (center-based Transform per ruling #7). Keyframed: sample the **union** of position+scale+rotation kf
  frames; all three params always emitted.
- **Crop** (video; skip if identity & no kf): id `crop`, type `motion`, category `motion`; params
  left/right/top/bottom (min 0 max 100), value = `crop.{edge}*100` (model stores 0–1 fractions).
- **Opacity** (video; skip if no kf and opacity==1.0): id `opacity`, type `motion`; param `opacity`
  (min 0 max 100), value `%.1f` = opacity×100. Keyframed from `rawOpacityAt`.
- **Audio Levels** (audio; skip if no kf and volume==1.0): id `audiolevels`, type `audio`; param `level`
  (name "Level", min 0 max 3.98107), value `%.4f`, level = `clamp(volume, 0, 3.98)`. Keyframed:
  `<keyframe><when>{frame-startFrame}</when><value>..</value></keyframe>` from `rawVolumeAt`.
- Keyframe sampling uses the **same interpolation as the model** (Smooth default, ruling #8) at sampled
  frames; XML carries **no easing** (documented limitation — imports with default easing).
- **Unit tests:** export `golden_project_keyframes.palmier` (keyframed transform + opacity + crop) and
  assert byte-exact against `golden_xmeml_keyframes.xml`; assert the diff-only thresholds (a transform with
  scale=100, rot=0, center=(0.5,0.5) emits **no** basic-motion params); assert `rotation` sign negation and
  `center` offset-from-0.5 on a rotated, off-center clip.

**Implementation context:** Crate `palmier-export`. Reference `XMLExporter.swift` filter emitters,
`docs/reference/export.md` §B "Filters"; keyframe sampling shares `palmier-model` keyframe API (ruling #8
Smooth default; carry-forward `f64::round` for any frame math). **Dependencies:** E6-S2, Epic 2 (keyframe
sampling). **Parallel-safe?** Shares the XMEML module — sequence after E6-S2 (same dev thread).

---

### E6-S4 — XMEML timecode formatting, fades (transitionitem), linked A/V (`<link>`), tmcd read

*As a dev, I want drop-frame timecode, fade transitions, A/V link nodes and source-timecode reading ported
exactly, so the emitter is complete and the full golden set diffs byte-exact.*

**Acceptance criteria:**
- **`formatTimecode` (golden-critical):** NDF separator `:`, DF separator `;`. DF correction copied
  **exactly** (a hand-rolled approximation — do NOT substitute a textbook SMPTE formula):
  `drop = round(fps*0.066666)` (2@30, 4@60); `d=f/(fps*600)`, `m=f%(fps*600)`;
  `f += drop*9*d + (m>drop ? drop*((m-drop)/(fps*60)) : 0)`; then `ff=f%fps, ss=(f/fps)%60,
  mm=(f/(fps*60))%60, hh=f/(fps*3600)`, `%02d{sep}` ×4. `dropFrame = ntsc && timebase%30==0`.
- file `<timecode>`: `rate`, `string` (formatTimecode), `frame` (startFrame from source tmcd or 0),
  `displayformat` DF/NDF. **tmcd start frame** read from QuickTime `tmcd` track: first sample's 4 bytes as
  **big-endian UInt32** via a Rust demuxer (FFmpeg or `mp4`/`symphonia`); **default 0 if unavailable**
  (reference tolerates nil → 0 — acceptable for v1 goldens per `docs/reference/export.md` Open Questions).
- **Fades → `<transitionitem>`** (single-sided dissolve to black/silence) emitted as sibling before (left)
  / after (right) the clipitem; `frames = clip.fadeFrames(edge)`, skip if 0. **Left:** start=startFrame,
  end=+frames, alignment `start-black`, cutFrames=0. **Right:** start=endFrame−frames, end=endFrame,
  alignment `end-black`, cutFrames=frames. Children `start`, `end`, `alignment`, then audio
  (`rate` + effect `Cross Fade ( 0dB)` / effectid `KGAudioTransCrossFade0dB` type transition mediatype
  audio) / video (`cutPointTicks = Int64(cutFrames)*(254_016_000_000/fps)`, `rate`, effect `Cross Dissolve`
  / effectid `Cross Dissolve` category `Dissolve` type transition mediatype video, body `wipecode 0,
  wipeaccuracy 100, startratio 0, endratio 1, reverse FALSE`).
- **Linked A/V (`<link>`):** for clips sharing `linkGroupId` (group >1), one `<link>` per partner
  (including self): `linkclipref clipitem-{partner.id}`, `mediatype audio|video`, `trackindex`, `clipindex`
  (both 1-based, from `indexAddresses` over the **same** reversed-video/natural-audio, URL-filtered,
  startFrame-sorted lists used for emission — **identical filtering or links point at wrong clips**).
- **Does NOT transport:** text overlays, flips (h/v), keyframe interpolation curves — documented as
  XML-unsupported (FR-22 consequence).
- **Unit tests:** DF timecode unit test at 29.97 and 59.94 over a frame sweep (assert against reference
  values, incl. the `m>drop` branch); export `golden_project_text.palmier` (has fades/links where present)
  byte-exact vs `golden_xmeml_text.xml`; a fade fixture asserting both transitionitems and `cutPointTicks`;
  a linked-A/V fixture asserting trackindex/clipindex come from the same filtered lists. **CI gate
  (FOUNDATION §11.3/§905):** all `golden_xmeml_*.xml` diffs are exact; goldens regenerate **only** via
  `--update-golden` (review-gated) and **any diff blocks merge** (R-5, SM-7).

**Implementation context:** Crate `palmier-export`; tmcd read in `palmier-media` (Rust demuxer — FFmpeg or
`mp4`). Reference `XMLExporter.swift` timecode/transition/link emitters + `AVAssetReader` tmcd read;
`docs/reference/export.md` §B "Timecode formatting" / "Fades" / "Linked A/V" + §"macOS/Apple APIs to replace"
(tmcd). **Dependencies:** E6-S3 (completes the filter/clipitem); Epic 2; tmcd read may need a minimal
`palmier-media` demux helper (Epic 4/5 boundary) — if not ready, default-0 path lands first, tmcd read
follows. **Parallel-safe?** Shares XMEML module — same dev thread as E6-S2/S3. tmcd-read sub-task is
parallel-safe (separate `palmier-media` file).

---

### E6-S5 — Video export pipeline: composition → wgpu texture → readback → FFmpeg encode → mux

*As a dev, I want the video render pipeline that reuses the preview composition and feeds FFmpeg, so a
project exports to H.264 / H.265 / ProRes 422 with correct compositing semantics.*

**Acceptance criteria:**
- **renderSize:** `scale = shortSidePx / min(w,h)`; each dim `= (round(dim*scale)/2)*2` (snap to **even**,
  min 2) — encoders reject odd dims; keep the `(round/2)*2` even-snap (`docs/reference/export.md` §A.1 /
  Port risks). Resolutions 720p=720, 1080p=1080, 4k=2160 (short-side px).
- **Per-output-frame loop** `0..total_frames * (output_fps / project_fps)` (FOUNDATION §6.12; for v1
  output_fps = project_fps unless re-scoped — see Open Questions): build the composition via the **shared
  `palmier-engine` frame builder** (same path as preview), **fetching decoded frames from `palmier-media`'s
  LRU `FrameCache` via a handle** — `palmier-export`/`palmier-engine` **never open an `AVFormatContext`**
  (Glossary one-decode-owner contract). Render `LayerRender`s bottom→top to an **offscreen** wgpu texture
  (no WebView presentation — independent of Spike S-1). Read back (or NVENC zero-copy per S-4 outcome) and
  push to the FFmpeg encoder/mux per the **S-4 config** (libx264 / libx265 or `h264_nvenc`/`hevc_nvenc` /
  `prores_ks`).
- **Codecs:** H.264 (`.mp4`), H.265 (`.mp4`), **ProRes 422 LPCM** (`.mov`) — ruling #17, **no 4444/alpha**
  (OQ-1 deferred). Color tags **BT.709** primaries/transfer/YCbCr matrix on the video stream.
  (`docs/reference/export.md` §A.6 documents the reference's h265 720p→1080p-preset quirk and ProRes-422
  discrepancy — encode **true 720p** and **ProRes 422 LPCM** here per the rulings; note the divergence.)
- **Output-file precondition:** delete existing output before encoding (reference behavior; FFmpeg-side mux
  must not fail on existing file). **Cancellation:** a flag checked **at each frame boundary** (FOUNDATION
  §6.12), surfaced as a clean cancel (not an error) — mirrors the reference's
  `NSUserCancelledError`-as-cancel. **Progress** 0.0–1.0 polled/emitted (reference polls `session.progress`
  every 200ms → emit a Tauri progress event, FR-21).
- **SM-5 acceptance:** export 1 min 1080p H.264 (NVENC, balanced) **< real time on RTX 4060**, measured
  end-to-end. **CPU fallback** below the GPU floor: composite via FFmpeg libavfilter, degraded/frame-stepped
  (FOUNDATION §3, R-8) — still produces a correct file.
- **Test:** an integration test exports `golden_project_minimal` to H.264 and asserts a decodable
  MP4 of the expected dimensions/duration/frame-count and BT.709 tags; a timed SM-5 bench on the GPU lane.

**Implementation context:** Crates `palmier-export` (orchestrator — port `ExportService.makeExportSession`
semantics), `palmier-engine` (shared composition graph — **reuse Epic 5's frame builder, do not duplicate**),
`palmier-media` (FrameCache handle + FFmpeg encode/mux). Reference
`Sources/PalmierPro/Export/ExportService.swift` (renderSize, per-format dispatch, progress poll, cancel),
`Preview/CompositionBuilder.swift` (shared composition), `docs/reference/export.md` §A + §"macOS/Apple APIs
to replace" + §"Mapping to FOUNDATION crates" (frame count `0..total_frames*(output_fps/project_fps)`).
**Dependencies:** **E6-S0 (Spike S-4)** — encoder boundary; **Epic 5 composition-graph API** (`palmier-engine`
frame builder + `palmier-media` FrameCache handle). **Parallel-safe?** No — depends on Epic 5; touches the
shared `palmier-engine`/`palmier-media` composition boundary. Run in its **own** dev thread after Epic 5's
composition API stabilizes; isolate the export-sink code in `palmier-export`.

---

### E6-S6 — Export text baking: rasterize text track → per-frame RGBA overlay textures

*As a dev, I want the text track rasterized to per-frame overlay textures composited into the export
pipeline, so exported video shows captions/titles (replacing the reference's CALayer/CoreAnimation bake).*

**Acceptance criteria:**
- Replaces `AVVideoCompositionCoreAnimationTool` + `CALayer` text baking (`docs/reference/export.md` §A.4 /
  §"macOS/Apple APIs to replace"): **rasterize the entire text track to RGBA overlay textures per output
  frame** (FOUNDATION §6.x) and composite into the export pipeline (E6-S5) — text is **not** a composition
  track, it is an overlay rasterized per frame.
- Geometry origin matches the **wgpu NDC convention** (reference parent layer is `isGeometryFlipped=true`,
  `beginTime=AVCoreAnimationBeginTimeAtZero`) — reproduce the flip so text lands at the same position as
  preview. Reuses `palmier-text` (cosmic-text) — the **same** text rasterization as preview (no separate
  export path), so export and preview text are pixel-consistent.
- Text preroll 30 frames (carry-forward note) honored.
- **Test:** export `golden_project_text.palmier` to H.264 and assert text overlay is present at a known
  frame (position/alpha within tolerance vs the preview render of the same frame — reuses the SM-C1
  rendered-frame comparison harness from Epic 5; **video is fidelity-checked via SSIM/tolerance, only XMEML
  is byte-golden**).

**Implementation context:** Crates `palmier-text` (rasterization, shared with preview), `palmier-engine`
(composite into the frame), `palmier-export` (wire into the per-frame loop). Reference
`Preview/TextLayerController.swift` `buildForExport(timeline,fps,renderSize)` (returns parentLayer +
videoLayer), `docs/reference/export.md` §A.4. **Dependencies:** E6-S5 (per-frame loop), Epic 5
(`palmier-text` rasterization shared with preview). **Parallel-safe?** No — extends E6-S5's loop and shares
`palmier-text`/`palmier-engine` with Epic 5. Sequence after E6-S5.

---

### E6-S7 — `.palmier` self-contained bundle export

*As a dev, I want the bundle exporter that collects all media into a portable `.palmier` directory, so a
project can be moved/shared with no dangling external references.*

**Acceptance criteria:** port `PalmierProjectExporter.export` exactly per `docs/reference/export.md` §C:
1. Stage to a temp dir `palmier-export-{uuid}/`; create `media/` (`Project.mediaDirectoryName`).
2. For each manifest entry resolve source (`External` → absPath; `Project` → `projectURL/rel`). If
   **missing** → `report.missing += {id, name}`, keep entry dangling. **Dedup** by
   `srcURL.standardizedFileURL.path`.
3. Copy to `media/{name}`: `Project` entries keep `lastPathComponent`; `External` →
   `import-{id.prefix(8)}.{ext}`. **Collisions** get `-1, -2, …` suffix (`uniqueURL`). Rewrite entry
   `source = Project { relative_path: "media/{file}" }`. External → `collected += id`; project-copied →
   `copiedInternal += 1`. Sum `totalBytes`.
4. Encode (serde_json) **`project.json`** (timeline), **`media.json`** (rewritten manifest),
   **`generation-log.json`**; copy `thumbnail.jpg` and the **`chat/`** dir if present (reference filenames
   per **ruling #3** — `timeline.json`/`manifest.json`/etc. would break sample import).
5. Remove existing dest, mkdir parent, **move** staging → destURL. Bundle ext `.palmier`, UTI
   `io.palmier.project`.
- **Report** struct maps 1:1: `{ collected: Vec<String>, copied_internal: i64, missing: Vec<Missing>,
  total_bytes: i64 }` (FR-23). Reuses `MediaManifest` / `MediaSource { External | Project }` serde types
  from `palmier-project`/`palmier-model` (must match the committed `.palmier` JSON schema for golden
  fixtures — `docs/reference/export.md` §"Mapping to FOUNDATION crates").
- **Unit/integration test:** export a fixture project with one external + one project media + one missing
  ref to a temp dir; assert: `media/` contains both copied files (external renamed `import-{id8}.{ext}`,
  project keeping its name), a name collision gets `-1`, `media.json` rewrites both to
  `Project { relative_path }`, `report` = `{collected:[external-id], copied_internal:1, missing:[the one],
  total_bytes:>0}`, and the result round-trips back through Epic 2's bundle reader (SM-7).

**Implementation context:** Crate `palmier-export` (+ `palmier-project`/`palmier-model` serde types).
Reference `Sources/PalmierPro/Export/PalmierProjectExporter.swift`, `Models/MediaManifest.swift`
(`MediaSource { external(absolutePath), project(relativePath) }`), `Utilities/Constants.swift` (`Project.*`
filenames), `docs/reference/export.md` §C + §"macOS/Apple APIs to replace" (FileManager → `std::fs`,
JSONEncoder → serde_json). **Dependencies:** Epic 2 (`palmier-project` bundle read/write + serde model +
`MediaManifest`/`MediaSource`). **Parallel-safe?** Yes — pure filesystem + serde, isolated file in
`palmier-export`, no GPU/composition. Can run fully in parallel with the XMEML thread (E6-S1..S4) and the
video thread (E6-S5/S6).

---

### E6-S8 — Social-platform sidecar (`<export>.palmier-meta.json`) — best-effort (M1) / frozen (M5)

*As a dev, I want "Export to Social" to emit an MP4 plus a metadata sidecar, so the external TypeScript
social platform can consume the export (UJ-5).*

**Acceptance criteria:**
- "Export to Social" runs the video export (E6-S5/S6) → MP4, then writes a sidecar
  **`<export>.palmier-meta.json`** alongside it containing the FOUNDATION §6.12 fields: **transcript,
  chapter markers, AI-suggested captions, source project hash** (FR-24).
- **M1 scope (this story):** emit the MP4 + a **best-effort sidecar under a provisional schema** — the
  export epic is **NOT** held to a frozen schema at M1 (PRD §4.6 / §12 M1/M5 boundary; **OQ-7**). Fields
  populate from available data: transcript from Epic 10 if transcribed (else empty/omitted, not an error),
  source project hash computed over the bundle. The provisional schema is documented in a `docs/` artifact.
- **M5 (out of this story's M1 bar, noted for traceability):** the **frozen schema** + full UJ-5 land at
  **M5** once **OQ-7** resolves jointly with the social-platform team; this story's provisional emitter is
  the M1 deliverable only.
- **Test:** "Export to Social" on a transcribed fixture writes both the MP4 and a sidecar parseable as JSON
  containing all four fields; on a non-transcribed project the sidecar still writes with transcript
  empty/omitted and the export succeeds (graceful, per UJ-1/UJ-5 edge handling).

**Implementation context:** Crate `palmier-export` (+ reads transcript from `palmier-transcribe` when
present, Epic 10). Reference: FOUNDATION §6.12 (sidecar fields), PRD §4.6 FR-24 / §8 §13.7 (OQ-7),
`docs/reference/export.md` (export orchestration). **Dependencies:** E6-S5 (MP4 path); **soft** dependency
on Epic 10 (`palmier-transcribe`) for transcript/captions — degrade gracefully if absent (transcript lands
M3, so at M1 the field is best-effort/empty). **Parallel-safe?** Partially — depends on E6-S5; the sidecar
serializer itself is an isolated `palmier-export` file. Sequence after E6-S5; the schema/serializer can be
drafted in parallel.

---

## Story dependency & parallelism summary

- **Start immediately, fully parallel, no spike gate:** **E6-S1** (XMEML render core), **E6-S7** (bundle),
  **E6-S0** (Spike S-4). These need only Epic 2's model and have **no GPU / S-1 dependency**.
- **XMEML thread (one dev, sequential):** E6-S1 → E6-S2 → E6-S3 → E6-S4. Shares the `palmier-export` XMEML
  module. Independent of Epic 5 and Spike S-1. This is the **golden-critical SM-7 path** — prioritize.
- **Video thread (one dev, gated):** E6-S0 (S-4) → E6-S5 → E6-S6, **then** E6-S8. Depends on **Epic 5's
  composition API** and **Spike S-4**; renders **offscreen** so it is **NOT** gated by Spike S-1's WebView
  presentation outcome.
- **E6-S7 (bundle)** and **E6-S8 (sidecar)** complete the FR-23/FR-24 surface; S8 soft-depends on Epic 10.
- **M1 exit:** XMEML byte-exact vs goldens (SM-7), bundle report correct + round-trips (SM-7), video export
  < real time (SM-5), best-effort sidecar emitted. Frozen sidecar schema is explicitly **M5** (OQ-7).
