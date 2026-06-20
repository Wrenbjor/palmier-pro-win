---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# timeline-model — reference port notes

## Purpose
Implementation-level reference for the macOS Palmier Pro timeline/track/clip data model and the
timeline-canvas rendering (geometry, ruler, playhead, per-type clip visuals, volume/opacity rubber
bands, fades, range selection, snapping, drag/trim). Source of truth for `palmier-model` (data
structures + pure sampling/geometry math). Confirms and in several places **contradicts** FOUNDATION
§5 — discrepancies are flagged inline and gathered in Port risks. All paths below are under
`../palmier-pro/Sources/PalmierPro/`.

## Key types & files
- `Models/Timeline.swift` — `Timeline`, `Track`, `Clip`, `Transform`, `Crop`, `CropAspectLock`,
  `ClipLocation`, `FadeEdge`. Holds every sampling method on `Clip` (opacity/volume/transform/fade).
- `Models/Keyframe.swift` — `Interpolation`, `Keyframe<V>`, `KeyframeTrack<V>`, `AnimPair`,
  `AnimatableProperty`, `smoothstep`, `KeyframeInterpolatable`, plus all `Clip` keyframe helpers
  (`upsertKeyframe`, `removeKeyframe`, `setInterpolation`, `moveKeyframe`, `sample`).
- `Models/ClipType.swift` — `ClipType{video,audio,image,text,lottie}`, `isVisual`, `isCompatible`,
  `init?(fileExtension:)`.
- `Inspector/InspectorView.swift:1072` — `VolumeScale` (linear↔dB).
- `Utilities/Constants.swift` — `Layout`, `Snap`, `Zoom`, `Trim`, `Defaults`, `TrackSize`,
  `TimelineAutoScroll`, `Project`.
- `Timeline/TimelineGeometry.swift` — pure layout math (frame↔x, track↔y, clip rects, drop targets,
  keyframe/fade hit rects). Shared by drawing + hit testing.
- `Timeline/ClipRenderer.swift` — all per-clip canvas drawing (fill, strip, thumbnails, waveform,
  rubber bands, fades, labels, keyframe diamonds, trim handles).
- `Timeline/TimelineRuler.swift` — ruler ticks + timecode labels.
- `Timeline/PlayheadOverlay.swift` — `CAShapeLayer` playhead (line + triangle).
- `Timeline/SnapEngine.swift` — snap target collection + sticky snap finder.
- `Timeline/DragState.swift` — drag state enum (move/trim/volumeKf/fadeKnee/marquee/range/scrub).
- `Timeline/TimelineInputController.swift` — mouse handling, hit testing, zoom/pan, drag commits.
- `Timeline/TimelineRangeSelection.swift` — shift-drag time range value type.
- `Timeline/TimelineView.swift:201` — `drawContent`: draw order (backgrounds → range fill → clips →
  gaps → generating overlays → drag ghosts → marquee → insertion line → razor preview → ruler).

## Core behaviors & algorithms

### Data model (storage, not FOUNDATION's restatement)
- `Timeline`: `fps:Int=30, width:Int=1920, height:Int=1080, settingsConfigured:Bool=false,
  tracks:[Track]`. `totalFrames = max(track.endFrame)`.
- `Track`: `id:String(UUID), type:ClipType, muted=false, hidden=false, syncLocked=true, clips:[Clip]`.
  `displayHeight:CGFloat=50` is NOT serialized (CodingKeys omit it; reset to 50 on open).
  `endFrame = max(clip.endFrame)`. Lenient decode (every field defaulted).
- `Clip` (`Models/Timeline.swift:75`): `id, mediaRef:String, mediaType:ClipType=.video,
  sourceClipType=.video, startFrame, durationFrames, trimStartFrame=0, trimEndFrame=0, speed=1.0,
  volume=1.0, fadeInFrames=0, fadeOutFrames=0, fadeInInterpolation=.linear, fadeOutInterpolation=
  .linear, opacity=1.0, transform, crop, linkGroupId?, captionGroupId?, textContent?, textStyle?,
  6 optional KeyframeTracks`. Derived: `endFrame = startFrame + durationFrames`;
  `sourceFramesConsumed = round(durationFrames * speed)`;
  `sourceDurationFrames = sourceFramesConsumed + trimStartFrame + trimEndFrame`.
- **IDs are `String` (UUID strings), not `Uuid`** — FOUNDATION §5 says `Uuid`. Lenient JSON decode
  regenerates a UUID string if `id` missing. mediaRef is a plain String, not a typed Uuid.
- `Transform` is stored as **`centerX=0.5, centerY=0.5, width=1, height=1, rotation=0,
  flipHorizontal, flipVertical`** with `topLeft` computed as `(centerX - w/2, centerY - h/2)`.
  This DIRECTLY CONTRADICTS FOUNDATION §5.4, which says `top_left` is the stored field. Legacy decode
  migrates old `x/y` (top-left-ish) keys: `centerX = oldX + w - 0.5`. Port must store center, expose
  topLeft, and keep the legacy migration.
- `Crop` = edge insets `left/top/right/bottom` (0..1 source space), `isIdentity`,
  `visibleWidthFraction = max(0, 1-left-right)`.

### Keyframes & sampling (`Models/Keyframe.swift`)
- `Keyframe<V>{ frame:Int, value:V, interpolationOut:Interpolation = .smooth }`. **Default interp is
  `.smooth`**, contradicting FOUNDATION §5.5/§5.2 ("default linear"). Frames stored CLIP-RELATIVE;
  public API converts to absolute via `toAbs/toOffset = frame ± startFrame`.
- `KeyframeTrack<V>{ keyframes:[Keyframe<V>] }`, `isActive = !keyframes.isEmpty`. `upsert` keeps
  array sorted, unique frames. `move(from:to:)` is a no-op if target frame already occupied.
- `sample(at:fallback:)`: empty→fallback; 1 kf→that value; `frame<=first.frame`→first;
  `frame>=last.frame`→last; else find first kf with `frame>kf`, segment `[a,b]`,
  `raw=(frame-a.frame)/(b.frame-a.frame)`, switch on **`a.interpolationOut`**: hold→`a`, linear→
  `lerp(a,b,raw)`, smooth→`lerp(a,b,smoothstep(raw))`. `smoothstep(t)=t*t*(3-2*t)`.
- Interpolatable: `Double`, `AnimPair`(componentwise), `Crop`(4-componentwise). `AnimPair{a,b}` is
  position `(x,y)` AND scale `(w,h)`.
- `AnimatableProperty{opacity,position,scale,rotation,crop,volume}`.
- On duration shrink call `clampKeyframesToDuration()` (drops kfs with `frame<0 || frame>duration`),
  `clampVolumeKfsToDuration()`, `clampFadesToDuration()`. `rescaleKeyframes(by:)` multiplies frames
  (used on speed change). `setDuration` runs clamp+fade-clamp.

### Clip value sampling (the render-critical math)
- `opacityAt(frame)` = `rawOpacityAt(frame) * fadeMultiplier(at:)`, but fade only applied when
  `mediaType != .audio` and a fade exists. `rawOpacityAt` = opacityTrack sample (fallback static
  `opacity`).
- `volumeAt(frame)` = `volume(static linear) * kfGain * fadeMultiplier`. `kfGain` = `linearFromDb(
  volumeTrack.sample(...,fallback:0 dB))` when track active else 1.0. `rawVolumeAt` omits fade.
  **Volume keyframe values are stored in dB**; static `volume` is linear.
- `fadeMultiplier(at:)`: rel = frame-startFrame, 0 outside `[0,duration]`. inMul = (fadeIn>0)?
  `t=min(1,rel/fadeIn)` (smooth→smoothstep) : 1. outMul symmetric on `durationFrames-rel`.
  Returns `min(inMul,outMul)`. NOTE: linear and hold both treated as linear ramp for fades
  (only `.smooth` bends).
- `transformAt(frame)`: topLeft from positionTrack (AnimPair a,b) if active else from
  `transform.center` minus half size; size from scaleTrack (AnimPair a=w,b=h) else
  `transform.width/height`; rotation from rotationTrack else `transform.rotation`. Crop from cropTrack
  else static `crop`.
- `timelineFrame(sourceSeconds:fps:)`: `sourceFrame = t*fps`; `offset = sourceFrame - trimStart`
  (nil if <0); `frame = round(startFrame + offset/max(speed,1e-4))`; nil unless in `[startFrame,
  endFrame)`. This is the transcript-seconds→timeline-frame map used by caption/cut tools.
- `VolumeScale`: floorDb=-60, ceilingDb=15. `dbFromLinear(l)=l>0? clamp(20*log10 l) : -60`;
  `linearFromDb(db)= db>-60 ? 10^(db/20) : 0` (hard mute below floor).
  **CONTRADICTS FOUNDATION §5.3 "dB floor -120"** — reference floor is -60, ceiling +15.

### ClipType compatibility (cross-track move rules)
- `isVisual` = video|image|text|lottie. `isCompatible(other) = self==other || (self.isVisual &&
  other.isVisual)`. So ANY visual type can move onto ANY visual track (video↔image↔text↔lottie all
  interchangeable); audio only with audio. This is looser than FOUNDATION §6.3 ("text/lottie own-type
  only"). Follow the reference: visual-to-visual is allowed. `init?(fileExtension:)` maps extensions
  (note `.json`/`.lottie`→lottie).

### Geometry (`TimelineGeometry`)
- `headerWidth` default 0 (tracks rendered from x=0; ruler/clip x = `headerWidth + frame*pxPerFrame`).
- Track stack Y starts at `rulerHeight(24) + dropZoneHeight(60)`; cumulative per-track heights.
- `clipRect = (x=headerWidth+startFrame*ppf, y=trackY+2, w=durationFrames*ppf, h=trackHeight-4)`.
- `frameAt(x) = max(0, floor((x-headerWidth)/ppf))`. `trackAt(y)` linear scan.
- `dropTargetAt(y)`: top drop zone (`y<firstTrackY`)→newTrackAt(0); between-track boundary within
  `insertThreshold(10)`→newTrackAt(i+1); past last→newTrackAt(count); else existingTrack(i).
- Keyframe/fade hit rects computed here using `ClipRenderer.y(forDb:)` and `fadeHandleRenderX`.

### Ruler (`TimelineRuler`)
- Guard `ppf>0 && finite`. Major tick target ~80px: candidates `[1,2,5,10,15,30,60,120,300,600,
  1200,1800,3600] * fps` frames; pick first whose pixel width ≥ rawFrames.
- Minor subdivisions: try `[10,5,4,2]`, pick first where each minor ≥12px. Midpoint minor tick (when
  even count) drawn taller (6px vs 4px); major tick 8px + timecode label (monospaced digits).

### Playhead (`PlayheadOverlay`)
- `CAShapeLayer`, red (`systemRed`), lineWidth 1, zPosition 100. Vertical line from
  `rulerHeight` to viewport bottom + downward triangle (size 8) at top. x =
  `timelineFrame*ppf - viewport.minX`. Driven by `withObservationTracking` on `playheadState.
  timelineFrame` + `zoomScale`.

### Clip canvas visuals (`ClipRenderer.draw`)
- Card: rounded rect (corner radius 3), fill = `sourceClipType.themeColor` at alpha 0.45 (selected)
  / 0.3. 3px color strip on left edge (sourceClipType color). Border: selected = white α0.9 width
  1.5; else `Border.primary` width 0.5. Missing media (not generating): red wash + red border.
- Label bar height 16 at top: `"<name>  <timecode(durationFrames)>"`; underline name if linked.
  Trim handles (`Trim.handleWidth=4`) drawn as muted fills on both edges.
- Content zone (below label bar): video→thumbnail strip (tiled by aspect, mapped through
  trimStart/sourceFramesConsumed in seconds); image→tiled center image; audio→waveform.
- Keyframe diamonds (opacity/position/scale/crop) drawn as yellow diamonds near clip bottom
  (`y=maxY-5`), x = `minX + handleWidth + (absFrame-startFrame)*ppf'` where
  `ppf' = (width-2*handleWidth)/duration`. Volume kfs are NOT drawn here — they live on the rubber band.

### Waveform (`drawWaveform`)
- Maps trim window to sample indices: `startFrac=trimStart/sourceDuration`,
  `endFrac=(trimStart+sourceFramesConsumed)/sourceDuration`. Bars = `Int(drawWidth)` (1px each).
  Peak-detect MIN over each bar's sample range (samples are dB-normalized, 0=loud, 1=silent).
  dbRange=50; static shift = `dbFromLinear(volume)/50`; per-bar volume only when volumeTrack active
  or fades present (samples `volumeAt`). `amplitude = min(1, (1-loudest)+dbShift)`; bar grows up from
  bottom. Only draws bars inside the dirty/clip region.

### Volume rubber band (audio) — `drawVolumeRubberBand`
- Body rect = clipRect inset by labelBarHeight(16) top, 1px bottom. dB→Y: `y(forDb)` maps
  `volumeRubberBandTopDb=6 .. volumeRubberBandBottomDb=-60` across body height (high dB → smaller y).
  NOTE these draw-axis limits (+6..-60) differ from `VolumeScale` editing range (+15..-60).
- Line: if volumeTrack active, polyline through kfs (filtered `0<=frame<=duration`) with per-segment
  interp (linear=straight, hold=step, smooth=12-step sampled). Else flat line at `dbFromLinear(volume)`.
- Fades drawn as wedges: knee in a fixed "fade lane" `body.minY+4`; left knee x clamped to
  `minX+6`, right to `maxX-6`; silent corner at `body.maxY`. Wedge filled (black α) + curve stroked
  (linear/hold→straight, smooth→12-step). When selected: white kf diamonds (size 7) + knee squares.
- Editing handles: `volumeKeyframeHitSize=14`, knees in same lane. `Cmd-click` on audio body adds a
  volume kf at rounded frame + dB from cursor Y (`db(forY:)`). Dragging a kf clamps frame between
  neighbor kfs and dB to `[floorDb,ceilingDb]`. Dragging a knee sets fade length, capped by
  `duration - otherEdgeFade`.

### Opacity envelope / fades (non-audio) — `drawOpacityFades`
- Only drawn when a fade exists or clip is selected. Same wedge/knee machinery as volume, fill from
  `body.minY` down. No opacity rubber-band LINE is drawn for non-audio (FOUNDATION §6.3 implies an
  "opacity envelope line across video clips" — reference only draws fade wedges + knees, not a draggable
  opacity line). Opacity is still sampled per frame for compositing via `opacityAt`.

### Snapping (`SnapEngine`)
- Targets: every clip start+end on every track (excluding dragged ids) + optionally playhead.
- `findSnap`: frame threshold = `baseThreshold(8px)/ppf`. Sticky: stay snapped until probe moves >
  `threshold * stickyMultiplier(1.5)`. Playhead threshold = `base * playheadMultiplier(1.5)`.
  Probe offsets: move drags supply `[startOffset, startOffset+duration]` per participant so either
  edge snaps; trims supply `[0]`. Returns target frame + which probe snapped + indicator x.
  `NSHapticFeedbackManager.perform(.alignment)` on snap — REPLACE/skip on Win/Linux.

### Drag/trim semantics (`TimelineInputController`)
- Hit zones inside clip: localX ≤ handleWidth → trimLeft; ≥ width-handleWidth → trimRight; fade knee
  / volume kf hit takes priority; else move. Double-click clip → select its media asset.
- trimLeft: delta clamped `[minDelta, originalDuration-1]`; minDelta = `-startFrame` for
  no-source media (image/text) else `-trimStart`. trimRight: maxDelta = `trimEnd` for source media
  (can't expand past source) else unbounded shrink to 1 frame.
- Move: vertical only relocates the lead clip; companions keep their own track row unless they `hop`.
  Track delta clamped to type-compatible tracks (`clampedTrackDelta`). Linked partners (`linkGroupId`)
  and incompatible-type companions are "pinned" (hold their row). NewTrackAt drop inserts a track of
  the lead's type. Option+drag = duplicate. Shift-click adds to selection and expands to link group
  unless Option held.
- Selection always expands to link group by default; Option overrides per drag.

### Range selection (`TimelineRangeSelection`)
- `{startFrame,endFrame}` with `normalized` (swap if reversed), `isValid (end>start)`,
  `contains(frame)` half-open `[start,end)`. Created by Shift-drag on the ruler (or drag a range
  edge, hit-slop 8px). Snaps to clip edges + playhead during drag. Drives ripple-delete-range + MCP.

## macOS/Apple APIs to replace
- AppKit `NSView`/`draw(_:)` + `CGContext` immediate-mode drawing → wgpu/Canvas 2D draw loop in the
  React/WebGPU timeline (FOUNDATION §2.1). All `ClipRenderer`/`TimelineRuler` math is pure and ports
  1:1; only the drawing primitives (fill/stroke/path/clip) change backend.
- `CAShapeLayer`/`CATransaction` (playhead) → a dedicated overlay quad/line in the canvas layer.
- `withObservationTracking` (Swift Observation) → Zustand subscription / signal on `current_frame`.
- `NSHapticFeedbackManager.perform(.alignment)` → no-op on Windows/Linux (FOUNDATION §6.3).
- `NSCursor` (resizeLeftRight/crosshair/openHand/pointingHand/arrow) → CSS cursors on the canvas.
- `NSColor`/`CGColor`, `NSFont.monospacedDigitSystemFont`, `NSAttributedString.draw` → CSS canvas
  text + design-token colors (§9). `NSColor.blended(withFraction:)` → manual RGBA lerp for waveform tint.
- `Timer`/`RunLoop` auto-scroll during drag → `requestAnimationFrame` loop.
- `CGImage` thumbnails → GPU textures / ImageBitmap from `palmier-media`.

## Mapping to FOUNDATION crates (palmier-model)
- All of `Models/Timeline.swift`, `Models/Keyframe.swift`, `Models/ClipType.swift`, `VolumeScale`,
  and the pure halves of `TimelineGeometry`/`SnapEngine` belong in **`palmier-model`** as serde
  structs + pure functions. `clipRect/frameAt/trackAt`, `sample`, `volumeAt/opacityAt/fadeMultiplier/
  transformAt/timelineFrame`, snap finding, and ruler tick math are deterministic and must be unit
  tested for behavior parity (golden values).
- Drawing (`ClipRenderer`, `TimelineRuler` stroke calls, `PlayheadOverlay`) is UI — lives in the
  frontend timeline canvas, not `palmier-model`, but consumes `palmier-model` geometry/sampling.
- Input/drag (`TimelineInputController`, `DragState`) maps to frontend interaction + `palmier-edit`
  (trim/move/split commit functions). `TimelineRangeSelection` → `palmier-model` value type used by
  `palmier-edit` ripple + `palmier-tools` MCP.

## Port risks & gotchas
- **Transform storage is center-based**, FOUNDATION §5.4 says top-left. Persisted JSON keys are
  `centerX/centerY/width/height/rotation/flipHorizontal/flipVertical` (+ legacy `x/y` migration
  `centerX=oldX+w-0.5`). Porting to a top-left field would break round-trip of existing project files.
- **Keyframe `interpolationOut` defaults to `.smooth`**, not linear (FOUNDATION §5.2/§5.5). Wrong
  default changes every authored animation/fade curve.
- **Volume dB floor is -60, ceiling +15** (`VolumeScale`), not FOUNDATION's -120. The rubber-band DRAW
  axis is a third pair (+6..-60). Three separate dB constants — keep them distinct.
- **Cross-track move: all visual types are interchangeable** (`isCompatible`), contradicting
  FOUNDATION §6.3's "text/lottie own-type only". Match the reference.
- **IDs are UUID strings, not typed Uuid**; mediaRef/linkGroupId/captionGroupId are plain strings.
  MCP short-id prefixing operates on these strings.
- Keyframe frames are **clip-relative in storage**, absolute in the public API — every read/write must
  add/subtract `startFrame`. Easy to double-offset.
- Lenient/defaulted JSON decode + non-serialized `displayHeight` (reset to 50 on open) must be
  preserved or old projects fail to load / look wrong.
- `sourceFramesConsumed = round(duration*speed)`; rounding must match (banker's vs half-up) to keep
  trim/waveform mapping identical.
- No draggable opacity line is rendered for video clips (only fade wedges); FOUNDATION §6.3 overstates
  this. Volume kfs are drawn on the rubber band, other-property kfs as bottom diamonds.
- Project on disk uses filenames `project.json`/`media.json` (`Project` enum), NOT FOUNDATION's
  `timeline.json`/`manifest.json` (§5.7). Storage dir is `~/Documents/Palmier Pro` on macOS.
- Snap haptic + AppKit gesture (slip/slide) have no reference implementation in this subtree; slide is
  a FOUNDATION stretch goal — neither found here.

## Open questions
- FOUNDATION §5.7 names `timeline.json`/`manifest.json`; reference `Project` enum uses
  `project.json`/`media.json`. Which filenames does the Windows bundle adopt? (affects sample/Convex
  resolve handoff.)
- FOUNDATION §5.5 lists `volume_track` values in dB with floor -120; reference uses dB floor -60.
  Confirm the port's canonical volume dB range (and whether to widen for parity with external XML export
  clamps).
- Reference `Keyframe.interpolationOut` default `.smooth` vs FOUNDATION `linear` — is the FOUNDATION
  default a deliberate change or an error? Resolve before writing serde defaults.
- Slip (Alt-drag) and Slide (Ctrl+Alt) from FOUNDATION §6.3 are not present in this Timeline subtree —
  confirm whether they exist elsewhere in the reference or are net-new for the port.
