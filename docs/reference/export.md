---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# export — reference port notes

## Purpose
Port the three export modes of `Sources/PalmierPro/Export/`: (1) rendered **video**
(H.264/H.265/ProRes) via the AVFoundation composition pipeline, (2) the **FCP7 XMEML 4** XML
emitter (Premiere/DaVinci/FCP interchange — golden-test-critical), and (3) the self-contained
**`.palmier` bundle** export. Behavior-parity port: the XMEML byte structure must match golden
fixtures; the video path can swap engines (AVFoundation → wgpu+FFmpeg per FOUNDATION) but must
preserve the same compositing semantics.

## Key types & files (cite paths under Sources/PalmierPro/Export/...)
- `ExportService.swift` — orchestrator. `ExportFormat {h264,h265,prores,xml}`,
  `ExportResolution {r720p=720,r1080p=1080,r4k=2160}` (short-side px), `ExportError`. `export(...)`
  dispatches xml→`XMLExporter` (sync, returns early), else builds an `AVAssetExportSession`.
  `exportPalmierProject(...)`→`PalmierProjectExporter`. Progress polled every 200ms from
  `session.progress`.
- `XMLExporter.swift` — `enum XMLExporter.export(timeline:resolver:outputURL:)` →
  `Builder.build()→String`. Internal `XMLNode` tree + `render(node,indent:)` (owns ALL whitespace
  + escaping). ~520 lines, the parity core.
- `PalmierProjectExporter.swift` — `enum PalmierProjectExporter.export(...) throws -> Report`.
  `Report {collected:[String], copiedInternal:Int, missing:[Missing], totalBytes:Int64}`.
- `ExportView.swift` — SwiftUI panel: `ExportMode {video,xml,palmierProject}`,
  `VideoCodec {h264,h265,prores}`. Holds file-size estimates (bitrate table below), preview
  thumbnail, save-panel wiring. UI only — port to Tauri front-end separately.
- Cross-refs: `Preview/CompositionBuilder.swift` (composition graph, shared with preview),
  `Preview/TextLayerController.swift` (text baking), `Utilities/TimeFormatting.swift`
  (`secondsToFrame(s,fps)=Int(s*fps)`), `Models/Timeline.swift` (Clip/Track model),
  `Models/MediaResolver.swift`, `Models/MediaManifest.swift` (`MediaSource {external(absolutePath),
  project(relativePath)}`), `Utilities/Constants.swift` (`Project.*` filenames).

## Core behaviors & algorithms (concrete — downstream story/dev agents implement from this)

### A. Video export (ExportService.makeExportSession)
1. `renderSize = resolution.renderSize(canvas)`: `scale = shortSidePx / min(w,h)`; each dim
   `= (round(dim*scale)/2)*2` (snap to even), min 2.
2. `CompositionBuilder.build(timeline, resolveURL, renderSize)` builds the same AV composition as
   preview (see preview reference). Returns composition + audioMix + videoComposition.
3. `AVAssetExportSession(asset, presetName)` per format×resolution (preset table below).
   `session.audioMix = result.audioMix`.
4. **Text baking:** `TextLayerController.buildForExport(timeline,fps,renderSize)` returns
   `(parentLayer, videoLayer)`; wired via `AVVideoCompositionCoreAnimationTool(postProcessing
   AsVideoLayer:videoLayer, in:parent)` set on a mutable copy of the videoComposition. Text is
   NOT a composition track — it is a CALayer overlay rasterized per frame. Parent layer is
   `isGeometryFlipped=true`, `beginTime=AVCoreAnimationBeginTimeAtZero`.
5. Delete existing output (AVAssetExportSession fails if file exists), then `session.export(to:as:)`.
   Cancellation surfaces as `NSCocoaErrorDomain/NSUserCancelledError`.

**Preset map (ExportService.exportPresetName):**
- h264: 720p→`AVAssetExportPreset1280x720`, 1080p→`...1920x1080`, 4k→`...3840x2160`.
- h265: 720p AND 1080p→`AVAssetExportPresetHEVC1920x1080` (NOTE: 720p maps to the 1080p HEVC
  preset — preserve or document; FFmpeg port should encode true 720p), 4k→`...HEVC3840x2160`.
- prores: always `AVAssetExportPresetAppleProRes422LPCM` (NOTE: this is ProRes **422 LPCM**, not
  ProRes 4444 — FOUNDATION §6.12 says "ProRes 4444 (alpha)". DISCREPANCY: reference ships
  422-no-alpha; FOUNDATION wants 4444+alpha. Flag for product decision.)
- Color tags on videoComposition: BT.709 primaries/transfer/YCbCr matrix.
- File-size estimate bitrates (bytes/sec, ExportView.estimatedFileSize): h264 .85/1.3/2.8e6;
  h265 .45/.65/2.2e6; prores 8/18.5/65e6 for 720/1080/4k. UI-only, not load-bearing.

### B. XMEML 4 emitter (XMLExporter.Builder.build) — GOLDEN-CRITICAL
Output prolog (exact): `<?xml version="1.0" encoding="UTF-8"?>\n<!DOCTYPE xmeml>\n` then the tree.
`render`: leaf→`<name attrs>escaped-text</name>`; element with children→open tag, `\n`, children
joined by `\n` at indent+2, `\n`, close; empty+no-text→`<name/>` self-closing. Indent = 2 spaces
per level, starts at 0. Escape order: `& < > " '` → `&amp; &lt; &gt; &quot; &apos;`. Bools render
literal `TRUE`/`FALSE`.

**Document shell:**
```
<xmeml version="4">
  <sequence id="sequence-1">
    <name>Timeline Export</name>
    <duration>{timeline.totalFrames}</duration>
    <rate><timebase>{fps}</timebase><ntsc>FALSE</ntsc></rate>   (rate = el, children on own lines)
    <timecode><rate.../><string>00:00:00:00</string><frame>0</frame>
             <source>source</source><displayformat>NDF</displayformat></timecode>
    <media>
      <video><format><samplecharacteristics>{width,height,anamorphic FALSE,
             pixelaspectratio square, fielddominance none, rate}</samplecharacteristics></format>
             {videoTrackNodes...}</video>
      <audio><numOutputChannels>2</numOutputChannels><format><samplecharacteristics>
             <samplerate>48000</samplerate><depth>16</depth></samplecharacteristics></format>
             <outputs><group><index>1</index><numchannels>2</numchannels><downmix>0</downmix>
             <channel><index>1</index></channel><channel><index>2</index></channel></group></outputs>
             {audioTrackNodes...}</audio>
```
**Track ordering:** video tracks emitted **reversed** (FCP7 is bottom→top; model is top→bottom);
audio tracks in natural order. Each track filtered to clips with a resolvable URL, sorted by
`startFrame`. Per track: `<track><enabled>{!hidden / !muted}</enabled><locked>FALSE</locked>`,
then per clip optional left fade transition, `<clipitem>`, optional right fade transition.

**`<clipitem id="clipitem-{clip.id}">` children (order matters):**
`masterclipid`, `name`(resolver.displayName), `enabled TRUE`, `duration`(source dur frames),
`rate`, `start`(clip.startFrame), `end`(clip.endFrame), `in`(trimStartFrame),
`out`(trimStartFrame+sourceFramesConsumed), `<file>`, then filters (time remap, then
video[motion,crop,opacity] OR audio[levels]), then `<link>` nodes.
- `masterclipid`: `masterclip-{linkGroupId}` if linked, else `masterclip-{mediaRef}-{video|audio}`.
- `sourceFramesConsumed = round(durationFrames*speed)`; `sourceDurationFrames` from resolver
  (`secondsToFrame(entry.duration,fps)`) else clip's stored value.

**`<file id="file-{mediaRef}-{video|audio}">`:** emitted in full once per (mediaRef,isAudio);
repeats collapse to `<file id="..."/>`. Full form children: `name`(url.lastPathComponent),
`pathurl`, `rate`, `duration`, `<timecode>`, `<media>`.
- `pathurl`: `url.absoluteString` with `file://`→`file://localhost//` (Premiere needs the extra
  slash; canonical single-slash fails). Fallback `media/{mediaRef}`.
- Image entries: file `duration`=1, and `<media><video>` gets a `<duration>1</duration>` before
  samplecharacteristics. Video duration = `secondsToFrame(entry.duration,fps)`.
- `rateTags(forFPS)`: `timebase=round(fps)`; `ntsc=TRUE` iff `|raw - timebase*1000/1001| <
  |raw - timebase|` (catches 23.976/29.97/59.94). audio `<media>`: samplerate 48000, depth 16,
  channelcount 2.
- file `<timecode>`: `rate`, `string`(formatTimecode), `frame`(startFrame from source tmcd or 0),
  `displayformat` DF/NDF. `dropFrame = ntsc && timebase%30==0`. startFrame read from QuickTime
  `tmcd` track: first sample's 4 bytes as big-endian UInt32 (`AVAssetReader`).

**Timecode formatting (formatTimecode, golden-critical):** NDF sep `:`, DF sep `;`. DF correction:
`drop = round(fps*0.066666)` (2@30, 4@60); `d=f/(fps*600)`, `m=f%(fps*600)`;
`f += drop*9*d + (m>drop ? drop*((m-drop)/(fps*60)) : 0)`. Then
`ff=f%fps, ss=(f/fps)%60, mm=(f/(fps*60))%60, hh=f/(fps*3600)`, `%02d{sep}` ×4.

**Filters** (each wrapped `<filter><effect>...`; `effect` children:
`name, effectid, [effectcategory], effecttype, mediatype, body`):
- **Time Remap** (only if speed≠1.0): id `timeremap`, type `motion`. params variablespeed(value 0),
  speed(`value=%.4f` of speed×100), reverse FALSE, frameblending FALSE.
- **Audio Levels** (audio; skip if no kf and volume==1.0): id `audiolevels` type `audio`. param
  `level` (name "Level", min 0 max 3.98107), value `%.4f`, level=`clamp(volume,0,3.98)`. Keyframed:
  `<keyframe><when>{frame-startFrame}</when><value>..</value></keyframe>` from `rawVolumeAt`.
- **Basic Motion** (video): id `basic` type `motion`. Static: emit only params that differ —
  scale if `|scaledPct-100|>0.1`, rotation if `|−rot|>0.05`, center if `|c|>0.001`.
  `scalePct = sourceW>0 ? (seqW/sourceW)*t.width*100 : t.width*100`. `rotation = −t.rotation`
  (FCP7 CCW-positive; model CW-positive). `center = (centerX−0.5, centerY−0.5)` normalized,
  rendered as `<value><horiz>%.5f</horiz><vert>%.5f</vert></value>`. Keyframed: sample union of
  position+scale+rotation kf frames; all three params always emitted.
- **Crop** (video; skip if identity & no kf): id `crop` type `motion` category `motion`. params
  left/right/top/bottom (min 0 max 100), value = `crop.{edge}*100` (model stores 0–1 fractions).
- **Opacity** (video; skip if no kf and opacity==1.0): id `opacity` type `motion`. param `opacity`
  (min 0 max 100), value `%.1f` = opacity×100. Keyframed from `rawOpacityAt`.

**Fades → `<transitionitem>` (single-sided dissolve to black/silence):** emitted as a sibling
before (left) / after (right) the clipitem. `frames = clip.fadeFrames(edge)`; skip if 0.
- left: start=clip.startFrame, end=+frames, alignment `start-black`, cutFrames=0.
- right: start=clip.endFrame−frames, end=clip.endFrame, alignment `end-black`, cutFrames=frames.
- children: `start`, `end`, `alignment`, then audio: `rate` + effect `Cross Fade ( 0dB)` /
  effectid `KGAudioTransCrossFade0dB` type transition mediatype audio. Video: `cutPointTicks`
  (`Int64(cutFrames)*(254_016_000_000/fps)`), `rate`, effect `Cross Dissolve`/effectid
  `Cross Dissolve` category `Dissolve` type transition mediatype video, body
  `wipecode 0, wipeaccuracy 100, startratio 0, endratio 1, reverse FALSE`.

**Linked A/V (`<link>`):** for clips sharing `linkGroupId` (group of >1). One `<link>` per partner
(including self): `linkclipref clipitem-{partner.id}`, `mediatype audio|video`, `trackindex`,
`clipindex` (both 1-based, assigned by `indexAddresses` over the same sorted/filtered track lists).

**Does NOT transport:** text overlays, flips (h/v), keyframe interpolation curves (import w/ default
easing). Coordinates in timeline frames.

### C. Palmier bundle export (PalmierProjectExporter.export)
1. Stage to temp dir `palmier-export-{uuid}/`; create `media/` (`Project.mediaDirectoryName`).
2. For each manifest entry: resolve source (`external`→absPath; `project`→`projectURL/rel`). If
   missing→`report.missing += {id,name}`, keep entry dangling. Dedup by
   `srcURL.standardizedFileURL.path`.
3. Copy to `media/{name}`: project entries keep `lastPathComponent`; external →
   `import-{id.prefix(8)}.{ext}`. Collisions get `-1,-2,…` suffix (`uniqueURL`). Rewrite entry
   `source = .project(relativePath:"media/{file}")`. external→`collected+=id`; project copied→
   `copiedInternal+=1`. Sum `totalBytes`.
4. Encode (JSONEncoder) `project.json`(timeline), `media.json`(rewritten manifest),
   `generation-log.json`. Copy `thumbnail.jpg` and chat-sessions dir if present.
5. Remove existing dest, mkdir parent, `moveItem(staging→destURL)`. Bundle ext `.palmier`,
   UTI `io.palmier.project`.

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `AVAssetExportSession` + preset names → FFmpeg muxer/encoder (libx264, libx265/NVENC, prores_ks)
  driven from the wgpu render loop (FOUNDATION §6.12 pipeline). No 1:1 preset enum — build
  bitrate/profile config per format×resolution.
- `AVMutableComposition`/`AVVideoComposition`/`AVMutableAudioMix` → custom wgpu compositor (see
  preview/composition reference) feeding raw frames to FFmpeg `AVPacket` queue; audio mixed → AAC.
- `AVVideoCompositionCoreAnimationTool` + `CALayer` text baking → rasterize text track to RGBA
  overlay textures per output frame (FOUNDATION §6.x: "rasterize entire text track to overlay
  textures, composite into export pipeline"). Geometry-flipped origin → match wgpu NDC convention.
- `AVAssetReader`/`CMBlockBuffer` tmcd read → parse QuickTime `tmcd` timecode track via Rust
  demuxer (FFmpeg or `mp4`/`symphonia`); first sample big-endian u32 = start frame. If unavailable,
  default 0 (reference tolerates nil → 0).
- `AVAssetImageGenerator` (preview thumbnail) → FFmpeg seek+decode single frame.
- `FileManager`/`URL`/`NSSavePanel`/`ByteCountFormatter` → `std::fs`, `camino`/`PathBuf`, Tauri
  dialog API, custom byte formatter. `pathurl` `file://localhost//` quirk must be reproduced byte-
  for-byte for Premiere import.
- `JSONEncoder` → `serde_json` (must match committed `.palmier` JSON schema for golden fixtures).
- `os.Logger`/`Log.export` → `tracing` target `export`.

## Mapping to FOUNDATION crates (palmier-export)
FOUNDATION §arch: `palmier-export` owns "H.264/H.265/ProRes export, FCP7 XML emitter". CI gate
(§905): "XMEML emission diff against committed golden XMLs; timecode formatting (NDF, drop-frame)."
- XMEML emitter → pure function `timeline -> String` (no I/O, no AV) so it golden-tests trivially.
  Keep the `XMLNode`/`render` split (structure vs whitespace) — it is why goldens are stable.
- Video pipeline → shared composition graph with `palmier-preview`; export adds the FFmpeg sink +
  text rasterization. Frame count `0..total_frames*(output_fps/project_fps)` per §6.12.
- Bundle export → `palmier-project`/`palmier-export` boundary; reuses `MediaManifest`/`MediaSource`
  serde types. Report struct maps 1:1.
- Golden fixtures (§924): `golden_xmeml_*.xml` from `golden_project_minimal/keyframes/text.palmier`;
  update only via `--update-golden`.

## Port risks & gotchas
- **Golden byte-fidelity:** indent (2sp), `\n` joins, self-closing `<x/>`, escape order, `TRUE`/
  `FALSE`, `%.4f`/`%.5f`/`%.1f`/`%.2f` format specs, attribute quoting — ANY drift breaks the diff.
  Port `render`/`escapeXML` literally; use a fixed-locale float formatter (Rust `format!` is locale-
  independent; Swift `String(format:)` uses C locale — match it).
- **DF timecode arithmetic** is a hand-rolled approximation (`0.066666`, integer divides) — copy
  exactly, do not "fix" with a textbook SMPTE formula or goldens diverge.
- **ProRes 422 vs 4444+alpha** — reference vs FOUNDATION disagree (see §A). Decide before writing
  the encoder; affects alpha handling end-to-end.
- **h265 720p→1080p preset** — reference upscales; faithful port would too, but a real FFmpeg
  encoder should arguably honor 720p. Flag.
- **rotation negation + normalized center** — easy to get sign/origin wrong; FCP7 is CCW-positive,
  center is offset-from-0.5 not pixels.
- **Track index/order coupling:** `<link>` trackindex/clipindex come from the SAME reversed-video /
  natural-audio, URL-filtered, startFrame-sorted lists used for emission. Indexing and emission
  MUST use identical filtering or links point at the wrong clips.
- **`file://localhost//` pathurl** — non-obvious Premiere requirement; keep it.
- **Even-dimension snap** in renderSize (encoders reject odd dims) — keep `(round/2)*2`, min 2.
- **Repeat-file collapse** (`<file id=.../>`) keyed by (mediaRef,isAudio) — separate ids per media
  type because Premiere rejects a clipitem pointing at a wrong-type file.

## Open questions
- ProRes 4444 + alpha (FOUNDATION) vs ProRes 422 LPCM (reference): which ships? Affects whether the
  composition must preserve an alpha channel through export.
- h265 720p: replicate the 1080p-preset upscale, or encode native 720p?
- Output fps vs project fps: reference exports at project fps (composition frameDuration=1/fps);
  FOUNDATION §6.12 mentions `output_fps/project_fps` frame scaling — is variable output fps in scope
  for v1, or always project fps?
- `tmcd` start-timecode read: is non-zero source timecode needed for v1 goldens, or is 0 acceptable
  (simplifies the demux dependency)?
- Text rasterization fidelity: does the wgpu overlay need to match CALayer/Core Text metrics for
  golden video frames, or is video not golden-diffed (only XMEML is)?
