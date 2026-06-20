---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# edit-engines — reference port notes

## Purpose
Implementation-precise spec for the timeline editing engines (ripple, overwrite, snap, trim, split)
and all drag / selection / tool-mode behavior in the macOS reference, so they can be ported as
pure-function Rust crates with unit tests. **Reality check vs FOUNDATION §6.4:** the reference ships
**RippleEngine, OverwriteEngine, and SnapEngine only**. There is **no Slip engine and no Slide
engine** — `grep slip|slide` returns nothing. FOUNDATION §6.3 already flags slide as a stretch goal;
slip is likewise unimplemented in the reference. Trim and Split are *not* separate engines — they live
in the EditorViewModel (orchestration), built on OverwriteEngine + per-clip frame math documented below.

All frame math is integer. Source↔timeline conversion uses `speed` and rounds with banker's-free
`(x).rounded()` (Swift `.toNearestOrAwayFromZero`); Rust must use `f64::round()` (ties away from zero),
NOT `round_ties_even`.

## Key types & files (cite paths under Sources/PalmierPro/Editor/...)
- `Editor/RippleEngine.swift` — pure fns: `computeRippleShifts`, `computeRippleShiftsForRanges`,
  `computeRipplePush`, `mergeRanges`. Types `ClipShift{clipId,newStartFrame}`, `FrameRange{start,end}`
  (half-open `[start,end)`, `length=end-start`), `GapSelection{trackIndex,range}`.
- `Editor/OverwriteEngine.swift` — pure fn `computeOverwrite(clips,regionStart,regionEnd) -> [Action]`;
  `Action = remove | trimEnd | trimStart | split`.
- `Editor/ToolMode.swift` — `enum ToolMode { pointer, razor }` (V / C keys).
- `Timeline/SnapEngine.swift` — pure-ish fns `collectTargets`, `findSnap`; types `SnapTarget{frame,kind}`
  (`kind = playhead | clipEdge`), `SnapResult{frame,probeOffset,x}`, `SnapState{currentlySnappedTo,
  currentProbeOffset}`. NOTE: `findSnap` has ONE side effect (`NSHapticFeedbackManager`) — strip it.
- `Timeline/DragState.swift` — `enum DragState` = `idle | scrubPlayhead | moveClip | trimLeft | trimRight
  | audioVolumeKf | fadeKnee | marquee | timelineRange`, with per-case payload structs.
- `Timeline/TimelineGeometry.swift` — pure layout math: `frameAt(x)`, `xForFrame`, `trackAt(y)`,
  `dropTargetAt(y) -> TrackDropTarget{existingTrack(i)|newTrackAt(i)}`, `clipRect`.
- `Timeline/TimelineInputController.swift` — the orchestrator: `mouseDown/Dragged/Up/Moved`, hit-testing,
  snap wiring, zoom/pan. This is the behavior spec for drag/selection/tool modes.
- `Timeline/TimelineRangeSelection.swift` — `{startFrame,endFrame}` + `normalized`, `isValid`, `contains`.
- `Editor/ViewModel/EditorViewModel+Ripple.swift` — ripple delete/insert/gap + `validateShifts`.
- `Editor/ViewModel/EditorViewModel+ClipMutations.swift` — `splitClip`, `clearRegion`, `moveClips`,
  `applyClipSpeed`, playhead-relative trim/split, `withTimelineSwap` undo wrapper.
- `Editor/ViewModel/EditorViewModel+Linking.swift` — `commitTrim`, `trimValues`, `expandToLinkGroup`,
  `linkedPartnerIds`, link-group offsets.
- `Models/ClipType.swift` — `isCompatible(with) = self==other || (self.isVisual && other.isVisual)`;
  `isVisual = video|image|text|lottie`. So video↔image↔text↔lottie all interchange; audio is its own zone.
- Constants (`Utilities/Constants.swift`): `Snap.thresholdPixels=8.0`, `Snap.stickyMultiplier=1.5`,
  `Snap.playheadMultiplier=1.5`; `Trim.handleWidth=4.0`; `Layout.dragThreshold=3`, `insertThreshold=10`,
  `dropZoneHeight=60`, `rulerHeight=24`, `trackHeight=50`; `Zoom.min=0.05 max=40 scrollSensitivity=0.04
  magnifySensitivity=1.5 panSpeed=5.0`; `Defaults.pixelsPerFrame=4.0`.

## Core behaviors & algorithms (concrete)

### RippleEngine (pure)
- `mergeRanges(ranges)`: sort by `start`; fold — if `range.start <= last.end` extend `last.end =
  max(last.end, range.end)` else push. (Touching ranges merge because `<=`.)
- `computeRippleShiftsForRanges(clips, removedRanges)`: `merged = mergeRanges(removedRanges)`. For each
  clip (iterate sorted by startFrame): `shift = sum(r.length for r in merged if r.end <= clip.startFrame)`.
  Emit `ClipShift(clip.id, clip.startFrame - shift)` only when `shift > 0`. (A clip only shifts left by
  the gaps that lie entirely before it; a clip overlapping a gap is assumed already removed/cleared.)
- `computeRippleShifts(clips, removedIds)`: derive removedRanges from the removed clips' `[startFrame,
  endFrame)`, then call `…ForRanges` on the *remaining* clips.
- `computeRipplePush(clips, insertFrame, pushAmount, excludeIds)`: every clip with `startFrame >=
  insertFrame` and id∉excludeIds → `ClipShift(id, startFrame + pushAmount)`.

### Ripple orchestration (EditorViewModel+Ripple)
- **Ripple delete selected** (`rippleDeleteSelectedClips`): build `globalRemovedRanges` from selected
  clips' spans. Per track: if track has its own removed clips → `computeRippleShifts`. Else if
  `track.syncLocked` → `computeRippleShiftsForRanges(globalRemovedRanges)`, then `validateShifts`; on
  failure **refuse the WHOLE edit** (beep, no mutation). Apply: remove clips, then apply all shifts.
- **Ripple delete ranges on a track** (`rippleDeleteRangesOnTrack`, drives MCP `ripple_delete_ranges`):
  `merged = mergeRanges(ranges.filter length>0)`; total removed = Σ lengths. `clearTrackIds` = anchor
  track + tracks holding linked partners of any clip the ranges overlap (`r.start < c.endFrame &&
  r.end > c.startFrame`). Pre-validate every *non-cleared* sync-locked track; refuse on collision.
  Then inside one undo swap: for each cleared track call `clearRegion` per merged range (prune=false);
  for cleared OR sync-locked tracks apply `computeRippleShiftsForRanges(merged)`; re-sort. Returns a
  report (removedFrames, clearedTracks, shiftedClips, resulting fragments, removedClipIds).
- **Ripple insert** (`rippleInsertClips`): `totalPush = Σ clipDurationFrames`. For target track + every
  sync-locked track, `computeRipplePush(insertFrame=atFrame, totalPush)`; then create clips at atFrame.
- **`validateShifts(trackIndex, shifts)`**: apply the shift map to that track's clips; refuse if any
  `start < 0` ("would move past timeline start") or any two intervals overlap after sort ("no room").
- **Gap ripple** (`rippleDeleteSelectedGap`): gap = `[prevClipEnd, nextClipStart)`; shift gap track +
  sync-locked followers left by the gap length; followers validated, gap track not.

### OverwriteEngine (pure) — `computeOverwrite(clips, regionStart, regionEnd)`
Guard `regionEnd > regionStart`. For each clip with `cs=startFrame, ce=endFrame`:
1. `ce <= regionStart || cs >= regionEnd` → no overlap, skip.
2. `cs >= regionStart && ce <= regionEnd` → `remove(clipId)`.
3. `cs < regionStart && ce > regionEnd` (region strictly inside clip) → `split`: leftDuration =
   `regionStart-cs`; rightStartFrame = `regionEnd`; rightTrimStart = `clip.trimStartFrame +
   round((regionEnd-cs)*speed)`; rightDuration = `ce-regionEnd`; new UUID for right.
4. `cs < regionStart` (overlaps left, ce inside) → `trimEnd(newDuration = regionStart-cs)`.
5. else (overlaps right, cs inside) → `trimStart`: newStartFrame=`regionEnd`; newTrimStart =
   `clip.trimStartFrame + round((regionEnd-cs)*speed)`; newDuration=`ce-regionEnd`.
Applied by `clearRegion`: trimEnd recomputes `trimEndFrame += round((oldDur-newDur)*speed)`; the
`split` action re-splits via `splitClip(atFrame=start)` then removes/re-splits the right fragment
(the engine's returned right-fragment fields are advisory — the VM re-derives them).

### Split — `splitClip` / `splitSingleClip` (clip-relative math)
Guard `startFrame < atFrame < endFrame`. `splitOffset = atFrame - startFrame`.
`leftSource = round(splitOffset*speed)`, `rightSource = round((durationFrames-splitOffset)*speed)`.
- **left** = copy: `durationFrames = splitOffset`, `trimEndFrame += rightSource`, `fadeOutFrames=0`, clamp fades.
- **right** = copy + new UUID: `startFrame = atFrame`, `durationFrames = orig.durationFrames -
  splitOffset`, `trimStartFrame += leftSource`, `fadeInFrames=0`, clamp fades.
- **Volume keyframes** migrate: boundary value = `track.sample(at: splitOffset)`. Left keeps kfs with
  `frame <= splitOffset`, appends a boundary kf at `splitOffset` if absent. Right keeps kfs with
  `frame >= splitOffset`, **re-bases each by `frame -= splitOffset`**, inserts a boundary kf at frame 0
  if absent. (Keyframe frames are clip-relative offsets — matches FOUNDATION §5.5.)
- **Linked clips**: split every member of the link group at the same `atFrame`; regroup the right halves
  under a fresh `link_group_id`. Razor-tool split and `Ctrl+K`/playhead split both route here.

### Trim (drag + commit; not a separate engine)
- Drag-left (`trimLeft`): `candidateStart = frameAt(x)`, snapped to targets. `delta = snappedStart -
  originalStartFrame`. Clamp: `maxDelta = originalDuration-1`; `minDelta = hasNoSourceMedia ?
  -originalStartFrame : -originalTrimStart`. (`hasNoSourceMedia` = image/text → can extend freely.)
- Drag-right (`trimRight`): `candidateEnd = max(originalStart+1, frameAt(x))`, snapped. `delta =
  snappedEnd - originalEndFrame`; `minDelta = -(originalDuration-1)`; if hasNoSourceMedia only clamp
  min, else `maxDelta = originalTrimEnd` (can't expand past available tail source).
- **Commit** (`commitTrim`→`trimValues`→`trimClips`→`trimClipInternal`): converts the timeline-frame
  `deltaFrames` to a SOURCE-frame trim. `sourceDelta = round(deltaFrames*speed)`. Left edge: new
  `trimStartFrame = (unbounded? : max(0,)) old + sourceDelta`. Right edge: new `trimEndFrame =
  (unbounded? : max(0,)) old - sourceDelta`. `trimClipInternal` then back-converts: `deltaStartTimeline
  = round((newTrimStart-oldTrimStart)/speed)`, `deltaEndTimeline = round((newTrimEnd-oldTrimEnd)/speed)`,
  `newDuration = oldDuration - deltaStartTimeline - deltaEndTimeline`, `newStart = oldStart +
  deltaStartTimeline`. Trim does NOT ripple neighbors (overwrite-style, in place).
- `propagateToLinked` (on unless Option held): same source-delta applied to each linked partner.
- **Playhead trims** (`Q`/`W`): require `start < playhead < end`; `delta = playhead-start` (Q) or
  `end-playhead` (W); `sourceDelta = round(delta*speed)`; add to trimStart (Q) / trimEnd (W).

### SnapEngine
- `collectTargets(tracks, playheadFrame, excludeClipIds, includePlayhead)`: every clip's start AND end
  frame (skip excluded clips) as `clipEdge`; plus playhead as `playhead` kind when `includePlayhead`.
- `findSnap(position, probeOffsets=[0], targets, &state, baseThreshold=8px, pixelsPerFrame)`:
  `baseFrameThreshold = baseThreshold / pixelsPerFrame`.
  - **Sticky**: if `state.currentlySnappedTo` set and the sticky probe is within `baseFrameThreshold *
    1.5` of it AND target still exists → return the held snap. Else clear state.
  - **Find best**: for each `probeOffset`, `probePos = position + probeOffset`; for each target,
    threshold = `baseFrameThreshold * 1.5` for playhead else `baseFrameThreshold`; if `|probePos -
    target.frame| <= threshold` and it's the smallest distance so far, record it. Return the closest
    `(probeOffset, target)` as `SnapResult{frame=target.frame, probeOffset, x=target.frame*pxPerFrame}`;
    set sticky state. (Playhead's 1.5× threshold gives it priority via the wider catch radius.)
  - **DISCREPANCY w/ FOUNDATION §6.3**: FOUNDATION says sticky = "2.5× the threshold" and snap-target
    "every clip edge + playhead, base 8px, playhead ×1.5". Reference sticky multiplier is **1.5**, not
    2.5. Port the reference value (1.5) and flag the spec.
- **Move-drag probes**: for every dragged clip the controller pushes TWO probe offsets — the clip's
  start offset and `start + durationFrames` — relative to the lead clip's original frame, so any edge of
  any selected clip can snap. `deltaFrames = (snap.frame - snap.probeOffset) - lead.originalFrame`.

### Selection & tool modes (TimelineInputController)
- **Tool mode**: `pointer` (V) = select/move/trim; `razor` (C) = on mouseDown over a clip, split at
  `razorPreviewFrame ?? frameAt(x)` (razor uses its own snap state in `mouseMoved` to preview the cut).
- **mouseDown over a clip (pointer)**: linkedOn = `!Option`.
  - Shift+click toggles membership (expanding to link group when linkedOn).
  - Plain click on an unselected clip → selection = `linkedOn ? expandToLinkGroup([id]) : [id]`.
  - Then decide drag sub-mode by `localX = point.x - clipRect.minX`: fade-knee hit → `fadeKnee`;
    audio volume-kf hit → `audioVolumeKf`; `Cmd` on audio body → add volume keyframe (no drag);
    `localX <= 4` → `trimLeft`; `localX >= width-4` → `trimRight`; else → `moveClip`.
  - Option during a body grab → `isDuplicate=true` (drop duplicates instead of moving).
- **mouseDown on empty space**: clears selection unless Shift; sets `selectedGap` via `hitTestGap`
  (`[prevClipEnd, nextClipStart)`); begins `marquee` with `baseSelection = current selection`.
- **Marquee**: rect = min/max of origin↔point. Select = baseSelection ∪ {clips whose `clipRect`
  intersects rect}; if not Option, `expandToLinkGroup`. Cancels gap selection once rect exceeds
  `dragThreshold=3`.
- **moveClip drop** (mouseUp): no-op if same track and `deltaFrames==0`. `frameDelta = max(-minOrigFrame,
  deltaFrames)` (can't push any clip before frame 0). `dropTargetAt(y)` → existing track (clamped to a
  type-compatible track via `clampedTrackDelta`, stepping toward 0 until all movers fit) or
  `newTrackAt(i)` (inserts a track). **Pinned companions** (linked partners of lead, OR clips whose type
  is incompatible with lead's destination type) keep their own row; others shift by the track delta.
  Commit via `moveClips` (or `duplicateClipsToPositions` when Option). `moveClips` pulls movers off
  source tracks, `clearRegion`s each destination (overwrite), drops each at its exact `toFrame`.
- **Cross-track rule**: enforced by `ClipType.isCompatible`. Visual↔visual freely; audio↔audio only.
- **Playhead / ruler**: click ruler = scrub seek (`interactiveScrub` during drag, `exact` on release).
  Shift-drag on ruler = `timelineRange` selection (snaps to edges+playhead). Range edges re-draggable
  within 8px slop.

### Speed change (`applyClipSpeed`/`setClipSpeed`)
`sourceFrames = basis.durationFrames * basis.speed`; `newDuration = max(1, round(sourceFrames/newSpeed))`.
Re-clamp keyframes+fades to new duration. If end moved, ripple the **contiguous chain** of clips
abutting the old end on the same track by `rippleDelta = (start+newDuration) - oldEnd` (only clips that
touch end-to-start; a gap breaks the chain — see `contiguousClipIds`).

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `NSHapticFeedbackManager.defaultPerformer.perform(.alignment)` in `SnapEngine.findSnap` →
  **drop entirely** (FOUNDATION §6.3: silent on Win/Linux). Return snap result as a value; let the UI
  layer optionally fire feedback. Keep `findSnap` side-effect-free in Rust.
- `NSSound.beep()` on ripple refusal → frontend toast / `tauri-plugin-notification` (or no-op).
- `NSCursor.*` (`resizeLeftRight`, `crosshair`, `pointingHand`, `openHand`, `arrow`) → CSS cursors in
  the React timeline canvas; Rust engine returns a hit-zone enum, TS picks the cursor.
- `NSEvent.modifierFlags` (`.shift/.option/.command`) → JS pointer-event modifiers (`shiftKey`,
  `altKey`, `ctrlKey/metaKey`). Per FOUNDATION the macOS Cmd maps to **Ctrl** on Win/Linux; Option→Alt.
- `NSRect`/`NSPoint`/`CGFloat` geometry, `NSEvent.scrollingDeltaY`, `magnification` → plain `f64`
  structs + wheel/pinch deltas from the webview. `intersects`/`contains` → port to a tiny rect type.
- `UndoManager` (grouping, `registerUndo`, `setActionName`) → **palmier-history** crate (FOUNDATION §4),
  with the reference's two patterns: `withTimelineSwap` (whole-Timeline before/after snapshot, atomic)
  and `registerClipPropertySwap`/`registerClipStateSwap` (bidirectional per-clip swaps). User and agent
  undo stacks separated (FOUNDATION §6.14 agent undo).
- `Timer`/`RunLoop` auto-scroll during playhead drag → frontend rAF or a Rust transport tick.

## Mapping to FOUNDATION crates (palmier-edit, palmier-model)
- **palmier-edit** (pure, unit-tested, no UI/IO):
  - `ripple` mod: `compute_ripple_shifts`, `compute_ripple_shifts_for_ranges`, `compute_ripple_push`,
    `merge_ranges`, plus `validate_shifts(track, shifts) -> Result<(), RefuseReason>`. Matches
    FOUNDATION §6.4 signatures exactly.
  - `overwrite` mod: `compute_overwrite(clips, region_start, region_end) -> Vec<OverwriteAction>`.
  - `snap` mod: `collect_targets`, `find_snap` (takes `&mut SnapState`, returns `Option<SnapResult>`,
    no side effects).
  - `split` mod: `split_clip(clip, at_frame) -> Option<(Clip /*left*/, Clip /*right*/)>` incl. volume-kf
    migration; `trim_values(clip, edge, delta) -> (trim_start, trim_end)` and the source↔timeline
    round-trip from `trimClipInternal`.
  - `geometry` mod: `frame_at`, `x_for_frame`, `track_at`, `drop_target_at`, `clip_rect`.
  - `drag` mod: the `DragState` machine + clamping (move/trim min/max, `clamped_track_delta`,
    `pinned_companions`) as pure transitions driven by pointer events.
- **palmier-model**: `Clip`, `Track`, `Timeline`, `ClipType::is_compatible`, `FrameRange`, `ClipShift`,
  `KeyframeTrack`, `setDuration`/`clampFadesToDuration` equivalents (per FOUNDATION §5). The edit crate
  imports model types; model has no dependency on edit.
- Orchestration (link-group expansion, sync-lock fan-out, undo grouping, `clearRegion` apply loop) lives
  above the pure crates — likely a `palmier-engine`/command layer wrapping `palmier-edit` + `palmier-history`.

## Port risks & gotchas
- **Rounding parity**: every source↔timeline conversion is `round(x*speed)` / `round(x/speed)` ties
  away from zero. Use `f64::round`. Mismatched rounding drifts trims by ±1 frame and breaks split/trim
  round-trips. Add golden tests over speed ∈ {0.25, 0.5, 1.0, 1.7, 4.0}.
- **Sticky multiplier 1.5 vs FOUNDATION 2.5** — port the reference (1.5) and correct FOUNDATION.
- **Half-open ranges**: `FrameRange` is `[start, end)`; `mergeRanges` merges touching ranges (`<=`).
  `TimelineRangeSelection.contains` is also half-open (`>= start && < end`). Keep this exact.
- **Refuse-the-whole-edit semantics**: sync-locked ripple validation must run BEFORE any mutation and
  abort atomically. Don't partially apply.
- **Overwrite `split` action is advisory**: the VM ignores the engine's right-fragment fields and
  re-derives them via `splitClip`. Replicate the VM behavior, not just the engine return, or test both.
- **Pinned companions**: linked partners AND type-incompatible co-selected clips stay on their row
  during a cross-track move. Easy to miss; drives correct A/V-pair behavior.
- **Move snap uses both edges of every selected clip** as probes (not just the lead start). A naive
  single-probe port loses end-snapping and multi-clip snapping.
- **Speed ripple only affects the contiguous abutting chain**, not all later clips, and stops at the
  first gap.
- **Linked split regroups right halves under a new link id** — forgetting this leaves the two halves
  cross-linked and they move together wrongly.
- **`hasNoSourceMedia` (image/text)** removes the source-material trim cap and allows negative trim
  fields / free extension. Distinct clamp path from video/audio.

## Open questions
- Slip and Slide (FOUNDATION §6.3/§6.4): **no reference implementation exists.** Decide whether to ship
  them at all; if yes they are net-new design, not a port. Recommend deferring to match parity scope.
- Audio volume-keyframe drag and fade-knee drag are timeline-canvas interactions, not "edit engines" per
  the task — included here because they share the DragState machine. Confirm whether they belong in
  palmier-edit or in the inspector/keyframe crate.
- `clearRegion` recursion via `splitClip`→`removeClips` happens under a single undo swap in the
  reference; confirm the Rust history model coalesces nested mutations into one user-visible undo step
  (the reference's `withTimelineSwap` skips registration when nested).
- Drop-zone / new-track insertion geometry (`dropTargetAt`, `insertThreshold=10`, `dropZoneHeight=60`)
  assumes vertical stacked tracks with a video/audio divider; verify the React canvas reproduces the
  same hit regions pixel-for-pixel for snap/drop parity.
