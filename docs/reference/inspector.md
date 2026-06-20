---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# inspector — reference port notes

## Purpose
The Inspector is the right-rail context panel of the editor. Its header + body switch on the current
selection: **no selection** -> project/format metadata ("Timeline"); **clip(s) selected** -> a
tabbed clip inspector ("Inspector"); **media asset selected** -> read-only source metadata + AI Edit
("Source"). It owns the per-property editing widgets (scrubbable numbers, color, font, position) and
the keyframes side panel + per-property lanes. This doc captures the exact tab-visibility rules,
field ranges, and interaction algorithms so the port reproduces behavior 1:1.

## Key types & files (paths under Sources/PalmierPro/Inspector/...)
- `InspectorView.swift` (1112 lines) — root view, header/title resolution, tab gating, project +
  asset metadata, Transform/Playback/Levels sections, crop/flip, `VolumeScale`, `sharedClipValue`.
- `TextTab.swift` — text-clip inspector (Typography/Appearance/Layout/Content).
- `AIEditTab.swift` — AI Enhance + AI Audio actions for a clip or asset.
- `Keyframes/KeyframesLane.swift` — `KeyframesMetrics`, `ClipRulerBlock`, `KeyframesLaneRow`,
  `KeyframesPanel`, ruler view.
- `Components/ScrubbableNumberField.swift` — drag-to-scrub numeric field + AppKit mouse area.
- `Components/InspectorPositionFields.swift` — X/Y pair.
- `Components/ColorField.swift` — swatch driving the shared `NSColorPanel`.
- `Components/FontPickerField.swift` — bundled + system font menu with live hover preview.
- `Components/TextContentField.swift` — NSTextView-backed multi-line editor.
- `Components/InspectorRow.swift`, `InspectorSection.swift`, `GenerationReferencesStrip.swift`.
- Cross-refs (read-only): `EditorViewModel` (apply*/commit* methods, keyframe queries),
  `Utilities/BundledFonts.swift` (`families`, `systemFamiliesForPicker`), `SnapEngine`,
  model: `Clip`, `Transform`, `TextStyle`, `AnimatableProperty`.

## Core behaviors & algorithms (concrete — downstream implements from this)

### Header / title (InspectorView.headerTitle/headerIcon)
- visual or audio clip selected -> title "Inspector", icon `slider.horizontal.3`.
- else media asset selected -> "Source", icon `info.circle`.
- else -> "Timeline", icon `info.circle`.
- While marquee-selecting (`editor.isMarqueeSelecting`): show "Inspector" + body
  "`N` selected" centered.

### Clip tab set (availableTabs) — order matters
Compute `visuals = selected visual clips`, `audios = selected audio clips`,
`nonText = visuals where mediaType != text`, `isSingle = visuals.count + audios.count == 1`,
`isSingleText = isSingle && first visual is text`. Build tabs in this exact order:
1. `Text` — iff `isSingleText`.
2. `Video` — iff `nonText` non-empty.
3. `Audio` — iff `audios` non-empty.
4. `AI Edit` — iff `aiEditEligible && !AccountService.isMisconfigured`.

`aiEditEligible`: exactly one visual clip, it resolves to a visual `MediaAsset`, and any selected
audio clips are all link-partners of that visual (a linked A/V pair counts as one). Tab bar is hidden
when only one tab exists. Active tab = `preferredTab` if still available, else first available.
`resolvePreferredTab` (fires on selection change): single text -> force `.text`; if leaving text,
drop back to `.video`; always clears `cropEditingActive`. Switching away from `.video` also clears
`cropEditingActive`.

### Project metadata (no selection)
Section "Project" (only if a project URL exists): Name (file stem), Path (middle-truncated).
Section "Format": Resolution `W × H`, Frame Rate `fps fps`, Aspect Ratio (reduce `W:H` by gcd),
Duration `formatDuration(totalFrames/fps)` (`H:MM:SS` or `M:SS`).

### Video tab
- "Transform" section is collapsible (`transformExpanded`, default true) with a reset button:
  resets `transform = Transform()`, `opacity = 1`, nulls opacity/position/scale/rotation tracks,
  zeroes fade in/out frames, resets fade interpolations to linear. Action name "Reset Transform".
- Rows: Position (`InspectorPositionFields`), Scale, Rotation, Opacity, Crop, Flip.
- Playback section: Speed.
- Multi-clip: a field shows a shared value via `sharedClipValue` (returns nil if values differ ->
  field renders "—" mixed state, scrubbing disabled). apply* fans out to every selected clip;
  commit wraps all in one undo group with a named action.
- "Keyframes" toggle bar at bottom (`editor.keyframesPanelVisible`), enabled only when exactly one
  clip selected. When on + single clip, the tab splits into a two-column HStack: controls on the
  left (with right padding `controlsColumnWidth + sm` to align with lanes), a Divider, and
  `KeyframesPanel` on the right.

### Scrub field ranges (exact)
| Field | range (raw) | displayMultiplier | format + suffix | dragSensitivity |
|---|---|---|---|---|
| Position X | -10...10 | `canvasW` | `%.0f` | (default 1) |
| Position Y | -10...10 | `canvasH` | `%.0f` | (default 1) |
| Scale (W) | 0.01...∞ | 100 | `%.0f %` | (default 1) |
| Rotation | -3600...3600 | 1 | `%.0f °` | (default 1) |
| Opacity (video) | 0...1 | 100 | `%.0f %` | (default 1) |
| Speed | 0.25...4.0 | 1 | `%.2f x` | 0.01 |
| Volume | -60...15 dB (`VolumeScale.floorDb...ceilingDb`) | 1 | `%.1f dB`, override "-∞ dB" at floor | 0.3 |
| Fade In/Out | 0...maxSeconds | 1 | `%.2f s` | 0.02 |
| Font Size (text) | 12...300 | 1 | `%.0f pt` | (default 1) |
| Opacity (text) | 0...1 | 100 | `%.0f %` | (default 1) |

- Position X/Y bind to `clip.topLeftAt(frame).x/.y` (normalized top-left, 0..1 in canvas space; range
  allows -10..10 so off-canvas placement is permitted; displayed in pixels via canvas dims).
- Scale binds to `sizeAt(frame).width`; rotation to `rotationAt(frame)` (degrees); opacity to
  `rawOpacityAt(frame)`. All sample at `editor.activeFrame` so values reflect keyframe state.
- Fade `maxSeconds` = single clip's `durationFrames/fps`, else 60.0. Seconds<->frames:
  `frames = round(seconds * fps)`.
- Volume binds to `liveVolumeKfDb(at: activeFrame) ?? VolumeScale.dbFromLinear(clip.volume)`.

### VolumeScale (InspectorView.swift bottom)
`floorDb = -60`, `ceilingDb = 15`. `dbFromLinear(l) = l<=0 ? floor : clamp(20*log10(l), floor, ceil)`.
`linearFromDb(db) = db<=floor ? 0 : 10^(min(db,ceil)/20)`. At/below floor the field renders "-∞ dB"
and the model stores a hard 0 (true mute).

### Audio tab
Section "Levels": Volume row, "Fade In" (edge left), "Fade Out" (edge right). Speed section shown
only when no visual clip is selected (`nonTextVisualClips.isEmpty`). Same single-clip keyframe split
(volume aligns to its lane via a header-height spacer = `KeyframesMetrics.headerHeight`).

### Keyframe controls per animatable row (keyframeControls)
For a single selected clip, each animatable row appends: [prev-kf chevron] [diamond stamp] [next-kf
chevron]. Stamp button: filled diamond if a keyframe exists at `activeFrame` (toggles remove), hollow
otherwise (adds via `stampKeyframe`). Disabled (40% opacity) when playhead is outside the clip
(`clip.contains(timelineFrame:)`). Chevrons navigate to `previousKeyframeFrame`/`nextKeyframeFrame`,
disabled when none. `AnimatableProperty` cases: position, scale, rotation, opacity, crop, volume.

### Crop (cropRow / cropMenu / applyCropPreset)
Single clip only (disabled + 40% when multi/none). Toggle button drives `editor.cropEditingActive`
(canvas overlay). A `CropAspectLock` menu (cases incl. `.free`, `.original`, presets with
`pixelAspect`): `.free` leaves crop untouched; `.original` commits `Crop()`; preset commits
`cropFittingAspect(for:targetPixelAspect:)`. Crop is keyframeable (kf controls with `.crop`).

### Flip (flipRow)
Two icon toggles reading `transform.flipHorizontal/.flipVertical` (from first clip). Each commits
`flipH/V = !current` across all selected clips under one undo group ("Flip Horizontal"/"Vertical").

### Text tab (TextTab.swift)
`style = clip.textStyle ?? TextStyle()`. Sections:
- Content: `TextContentField` (NSTextView, min height 80). On every keystroke ->
  `applyClipProperty(rebuild:true)` set textContent + `fitTextClipToContent`; on end-editing -> commit.
  Plain text, rich text/quote/dash/spell substitutions all disabled, `allowsUndo=false`.
- Typography: Font (`FontPickerField`), Size (scrub 12...300 pt). Size + font changes call
  `fitTextClipToContent`.
- Appearance: Color (`ColorField`), Opacity (0..1 ->%), Background/Border/Shadow each a
  toggle+ColorField pair. Color edits route through `debouncedCommitTextStyle(key:)` with keys
  `textColor`/`backgroundColor`/`borderColor`/`shadowColor`; the enable toggle commits immediately.
- Layout: Alignment segmented picker (left/center/right) committing immediately; Position
  (`InspectorPositionFields(clips:[clip])`).

### ScrubbableNumberField interaction (Components/ScrubbableNumberField.swift)
- Drag horizontally to scrub; click to type. Backed by an AppKit `NSView` mouse area with
  `resizeLeftRight` cursor. Drag detect threshold: `abs(dx) > 3` px (window-space).
- Per-pixel delta: `next = clamp(dragStartValue + dx * sens / mult)` where `mult = displayMultiplier`
  (treated as 1 if 0). Modifiers: **Shift => sens × 10 (coarse)**, **Command => sens × 0.1 (fine)**.
  `onChanged` fires live during drag; `onCommit` fires on mouse-up. Click (no drag) enters edit mode.
- Edit parse: strip suffix, trim, replace "," with ".", parse Double, divide by displayMultiplier,
  clamp to range, commit. Mixed value (`value == nil`) shows "—" and blocks scrub.

### InspectorPositionFields
Two scrub fields, X then Y, fieldWidth 36, trailingLabel "X"/"Y", each driven by
`topLeftAt(frame).x/.y` shared across clips. `apply` calls `editor.applyPosition(setX:setY:)` per clip
(one axis at a time, other = nil); `commit` wraps all clips in one undo group "Change Position".

### ColorField
A swatch button. On click activates a singleton `ColorPanelBridge` wrapping the shared system color
panel; `colorDidChangeNotification` fires during drag (live), unlike SwiftUI ColorPicker which only
fires on mouse-up. Sets `showsAlpha = supportsOpacity`. Suppresses the first notification triggered by
seeding the panel's initial color. Emits sRGB RGBA.

### FontPickerField
Custom popup menu. Two groups: "Featured" = `BundledFonts.families`, then "All fonts" =
`BundledFonts.systemFamiliesForPicker` (`[(name, previewable)]`). Each item's title is rendered in its
own font when previewable (`NSFont(name:size:13)`). Hover (`willHighlight`) calls `onPreview` (live
non-committing); selecting calls `onChange`; closing without a pick calls `onCancel` (reverts the
preview). Current font shows a checkmark; button label uses `NSFont.familyName`.

### KeyframesPanel + lanes (Keyframes/KeyframesLane.swift)
- `KeyframesMetrics`: rulerHeight 18, stripHeight 14, headerHeight 32, rowHeight 22,
  stampButtonWidth 22, navButtonWidth 6, controlsColumnWidth 34 (=6*2+22), diamondSize 8.
- Frame<->x mapping: `t = clamp((f - clipStart)/span, 0, 1)`, `x = t*width`; inverse rounds.
  `span = max(1, endFrame - startFrame)`.
- Panel rows: video clip -> Position, Scale, Rotation, Opacity, Crop; audio clip -> Volume only.
  Top `ClipRulerBlock` (ruler + tinted clip strip with label) where drag seeks the playhead.
- `KeyframesLaneRow`: diamonds drawn via Canvas (filled tint + 0.4 black hairline stroke). Each kf has
  an invisible hit area (`hitTolerance*2` wide = 14 px) bearing a right-click context menu.
- Drag gesture (minimumDistance 0): on first change, if pointer is within `hitTolerance = 7` px of a
  kf -> begin a kf drag; else treat as empty-area scrub (seek playhead). During kf drag, raw frame
  from x, then `applySnap`, then `applyMoveKeyframe(from:to:)`; on end, commit if moved else revert.
- Snap (`SnapEngine.findSnap`, `snapThresholdPixels = 4`): targets = in-range playhead, both clip
  edges, and keyframe frames of all *other* properties on the same clip. On snap, draw a dashed
  yellow vertical guide (`snapX`); candidate clamped to `[startFrame, endFrame]`.
- Context menu per kf: Linear / Smooth / Hold (checkmark on current; default shown as `.smooth`) +
  "Delete Keyframe". Interpolation read via `editor.interpolation(...)`.
- Single red playhead overlay across the whole panel via `Playhead.appendPath` (only when playhead is
  inside the clip).

### AI Edit tab (AIEditTab.swift)
Available for a single AI-eligible visual clip or a selected visual media asset (when account not
misconfigured). Scope toggles (when clip context): "Replace clip source" (preserves speed/volume/trim/
transform), "Use trimmed portion only" (only if `trimStart>0 || trimEnd>0`). "AI Enhance" section:
Upscale (menu of `UpscaleModelConfig.models(for:type)` with cost), Edit, Rerun (cost in description),
and Create Video (images only: "Set as first frame"/"Set as reference"). For video assets, "AI Audio"
section: Music + SFX rows ("Generate"), plus a "Place on timeline" toggle when a clip is in context.
Each action computes `availability` (enabled + disabled reason). Submissions route through
`EditSubmitter` + `editor.seedGenerationPanel`. Replacement uses a `FirstOnlyFlag` so batch-image gens
only swap with the first result.

### Media asset (Source) inspector
Tab bar [Details, AI Edit] only when asset is visual and account ok; else Details only. Details:
identity header (name + "AI" badge if generated), File section (Type, Dimensions if non-audio,
Duration if >0 and not image, Size via byte formatter, Path middle-truncated), then for generated
assets: References strip (`GenerationReferencesStrip`), Generated (Model display name, aspect ratio,
resolution, duration), Prompt (copyable). `GenerationReferencesStrip` labels reference slots by model
capability (Source/Reference/First Frame/Last Frame/Image Ref/Video Ref/Audio Ref/Source Video).

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `NSColorPanel` + `colorDidChangeNotification` (live color, ColorField) -> React color picker
  component with live `onChange` during drag (e.g. react-colorful) feeding a Tauri command. The
  drag-vs-mouseup distinction must be preserved (live preview, commit on release).
- `NSColorPanel.showsAlpha` -> picker alpha toggle bound to `supportsOpacity`.
- `NSView` mouse-tracking (`ScrubMouseArea`: mouseDown/Dragged/Up, `resizeLeftRight` cursor) -> JS
  pointer events on a div: `pointerdown` + `pointermove` with `cursor: ew-resize`, 3 px drag
  threshold, `e.shiftKey`/`e.ctrlKey` for coarse/fine. (Note: Command -> Ctrl on Win/Linux.)
- `NSMenu` font popup with per-item `attributedTitle` font preview + `willHighlight` hover ->
  custom dropdown listing bundled + system families (from a Rust font enumeration command using
  `fontdb`); render each row in its own font; hover fires non-committing preview.
- `NSFont(name:)`, `familyName` -> `fontdb`/`cosmic-text` family resolution exposed via Tauri.
- `NSTextView` multi-line editor (TextContentField) -> `<textarea>`; replicate: plain text only, no
  smart quotes/dashes/spell, no internal undo (app owns undo), commit on blur, live apply on input,
  don't stomp caret on external re-render.
- `NSPasteboard` (PromptCopyButton) -> `navigator.clipboard.writeText` / Tauri clipboard plugin.
- `ByteCountFormatter` (file size) -> Rust byte formatter or `Intl.NumberFormat`-based util.
- `NSHapticFeedbackManager` (snap feedback, referenced in timeline) -> no-op on Win/Linux.
- SwiftUI `@Environment(EditorViewModel)` reactive model -> Zustand store + TanStack Query over Tauri
  commands; `activeFrame` becomes a reactive store value driven by a Tauri event stream.
- Canvas/Path diamond + playhead drawing -> HTML canvas / SVG / WebGPU overlay.
- `undoManager.beginUndoGrouping/setActionName` -> `palmier-history` grouped transactions named per
  action; multi-clip edits = one group.

## Mapping to FOUNDATION crates (src-ui/editor (inspector))
- All inspector UI lives in `src-ui/editor` (React/TS). It is a pure view over the model: it calls
  Tauri commands (apply*/commit* equivalents) and reads reactive state; it never touches FFmpeg/wgpu.
- apply* (live, no undo entry) vs commit* (named undo group) maps to two command flavors:
  a transient "preview" mutation and a committed mutation pushing onto the user undo stack
  (`palmier-history`).
- Keyframe edits (`stampKeyframe`, `applyMoveKeyframe`/`commitMoveKeyframe`, `setInterpolation`,
  `removeKeyframe`) map to `palmier-model` `KeyframeTrack` ops; sampling already specified in
  FOUNDATION §5.5 (Linear/Hold/Smooth via smoothstep). `AnimatableProperty` enum matches §5.5
  (opacity, position, scale, rotation, crop, volume).
- Font enumeration -> `palmier-text` (`fontdb`/`cosmic-text`); bundle the same `Resources/Fonts/`
  families as "Featured" (FOUNDATION §6.6).
- AI Edit actions -> `palmier-gen` (Convex generation lifecycle, §6.11) + `palmier-tools`
  (`upscale_media`, `generate_video/audio`).

## Port risks & gotchas
- **Volume range discrepancy (flag):** FOUNDATION §5.3/§6.7 say volume keyframe floor -120 dB and
  inspector range "−120…0". The reference `VolumeScale` is **floor -60, ceiling +15** (i.e. allows
  >0 dB gain). Reconcile: the inspector field clamps to [-60, +15]; keyframe storage floor may still
  be -120. Recommend matching the reference's [-60, +15] for the field and noting +15 dB amplification
  is intended; do not silently adopt the FOUNDATION's 0 ceiling.
- **Fine/coarse modifier mapping:** ScrubbableNumberField uses **Shift = coarse (×10)** and
  **Command = fine (×0.1)**. FOUNDATION §6.7 only mentions "Ctrl-drag = fine 0.1×". On Win/Linux map
  Command->Ctrl for fine; keep Shift for coarse (FOUNDATION omits coarse — preserve it).
- **Position range is -10..10**, not 0..1 — off-canvas placement is allowed; displayed value is
  pixels (raw × canvas dim). Don't clamp to canvas.
- Scale upper bound is **infinity** (`0.01...∞`); only the lower bound is enforced. Field shows %.
- apply* must NOT create undo entries; only commit* does, and multi-clip commits must be a single
  named group. Getting this wrong breaks atomic-undo success criterion (FOUNDATION §1.5).
- TextContentField must avoid caret-stomping: only overwrite text when the editor is not first
  responder and strings differ. A naive controlled `<textarea>` re-render on every keystroke loses
  keystrokes — this is exactly the bug the reference's NSTextView wrapper avoids.
- Keyframe lane snap targets include other-property keyframes on the same clip — easy to miss.
- Color edits are debounced per-key (`debouncedCommitTextStyle`); the enable toggle is immediate.
  Mixing these wrong produces undo-stack spam during color drags.
- `sharedClipValue` mixed-state ("—", scrub disabled) must be reproduced for all multi-select fields.
- AI Edit tab gating depends on `AccountService` state (`isMisconfigured`, `aiAllowed`) — wire to the
  port's `palmier-auth` account state.

## Open questions
- Exact `CropAspectLock` case list + `pixelAspect` values (defined outside this subtree; resolve in
  the crop/canvas reference).
- `BundledFonts.systemFamiliesForPicker` ordering + `previewable` heuristic (`canPreviewText`) —
  defined in `Utilities/BundledFonts.swift`; confirm whether system fonts are alphabetized.
- Whether keyframe storage floor is truly -120 dB while the field clamps to -60 (vs FOUNDATION).
- `fitTextClipToContent` exact resize math (auto-fit text box to content) lives in EditorViewModel —
  needs its own note for parity of text-box sizing on font/size/content change.
- `UpscaleModelConfig` / `CostEstimator` / `EditSubmitter` semantics (AI Edit costs + submission) are
  in sibling dirs; document under the generation reference.
