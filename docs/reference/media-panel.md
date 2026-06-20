---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# media-panel — reference port notes

## Purpose
The left-dock Media Panel is the project's asset browser and three secondary generation surfaces.
`MediaPanelView` hosts a 3-icon vertical rail switching between **Media**, **Captions**, **Music**
tabs (`MediaPanelView.PanelTab`). The Media tab owns import (drag-drop + file picker + paste),
folder hierarchy, thumbnail/duration rendering, sort/filter/view-modes, name + visual + spoken
search, and drag-out to the timeline. This doc covers behavior parity for `palmier-media`
(decode/thumbnail/waveform) and `src-ui/media-panel` (the React panel). FOUNDATION §6.2 is the
locked spec; discrepancies are flagged below.

## Key types & files (under Sources/PalmierPro/MediaPanel/...)
- `MediaPanelView.swift` — tab rail + tab switch; reacts to `editor.mediaPanelShowMediaTabTick`.
- `MediaTab/MediaTab.swift` — toolbar, state, sort/filter, marquee, import (`NSOpenPanel`),
  `ViewMode {folder,flat,grouped}`, `SortMode {name,dateAdded,duration,type}`, thumbnail presets.
- `MediaTab/MediaTab+Grids.swift` — `gridDimensions`/`computeLayout` math, three grid bodies
  (folder/flat/grouped), `MediaCell`, frame-tracking preference key, cell + folder-tile renderers.
- `MediaTab/MediaTab+Drag.swift` — drag URI schemes, payload/preview, provider/Finder/clipboard
  drop resolution, `resolveTextDrop` (in-panel asset/folder moves).
- `MediaTab/MediaTab+Search.swift` — `searchResults` (Moments/Spoken/Files sections),
  `scheduleMomentSearch` (250ms debounce), `MomentThumbnail` (async AVAssetImageGenerator).
- `MediaTab/MediaTab+IndexStatus.swift` — CLIP model download / indexing status pill.
- `MediaTab/MediaPanelDropArea.swift` — **AppKit-only** `NSHostingView` drop host (see replace).
- `MediaTab/AssetThumbnailView.swift` — asset tile: thumbnail/badges/duration, rename, context menu
  (Reveal in Finder, Copy Path, Relink, Delete, AI Edit, Move-to-Folder), tap/shift-tap selection.
- `MediaTab/FolderTileView.swift` — folder tile, inline rename, single/double-click open.
- `MusicTab.swift` — video-to-music / text-to-music generation form (NOT a music library — flag).
- `CaptionsTab/CaptionTab.swift` + `CaptionBuilder.swift` — transcription/caption generation form.
- Cross-refs (read-only): `Editor/ViewModel/EditorViewModel+MediaLibrary.swift` (import),
  `+Folders.swift` (folder CRUD/move), `Models/MediaAsset.swift`, `Models/ClipType.swift`,
  `Timeline/MediaVisualCache.swift` (thumbnails/waveforms), `Search/SearchIndexCoordinator.swift`.

## Core behaviors & algorithms

### Import & supported extensions
- `ClipType(fileExtension:)` is the single gate (lowercased ext):
  - video: `mov, mp4, m4v` · audio: `mp3, wav, aac, m4a` · image: `png, jpg, jpeg, tiff, heic, webp`
    · lottie: `json, lottie`. Unknown ext → `mediaPanelToast` "unsupported file type", asset dropped.
- `lottie` extra check: `LottieVideoGenerator.isLottie(at:)` must pass or it is rejected (a `.json`
  that is not a Lottie animation is refused — a port-relevant validation, not just extension match).
- `importFinderItems(urls, into:)` is **one undo step** ("Import Media"). It disables undo
  registration during the loop, then registers a single snapshot-restore undo if anything changed.
- Folder recursion: `importFolder(at:into:)` creates a `MediaFolder` named after the dir, lists
  contents with `.skipsHiddenFiles`, sorts by `localizedStandardCompare`, recurses subdirs, imports
  files whose ext maps to a `ClipType`. Directory tree → folder tree, 1:1.
- File picker (`importMedia`): `NSOpenPanel`, multi-select, dirs allowed, allowedContentTypes =
  `[.movie, .image, .audio, .json, lottie?]`. Imports into `currentFolderId`.
- Paste (`handleClipboardPaste`): if pasteboard has file URLs → import them; else `.png`/`.tiff`
  data → `importPastedImageData` (writes `pasted-<8hex>.<ext>` into `<project>/media/` or temp) then
  moves into current folder. `clipboardHasImportableMedia` gates the paste menu item.
- After import, `finalizeImportedAsset(asset)` runs async: `loadMetadata()` (AVURLAsset/ImageIO/
  Lottie) → `updateManifestMetadata` → `searchIndex.schedule(asset)` → kick visual cache:
  video → waveform + video-thumbnails; audio → waveform; image → image-thumbnail; text/lottie → none.

### Sorting / filtering / view modes
- `sortAndFilter(assets)`: filter by `passesFilters` then sort. `passesFilters`: type ∈ filterTypes
  (or filter empty) AND (`!filterAI || asset.isGenerated`) AND name case-insensitive contains the
  trimmed query. `dateAdded` = stable insertion order (no sort), `name` = caseInsensitive asc,
  `duration` = desc, `type` = rawValue asc.
- Filterable types: `[.video, .audio, .image]` only (`.text`/`.lottie` excluded from chips).
- `thumbnailSize` slider 80–200 (presets small 80 / medium 110 / large 150 / xlarge 200).
- **View modes:**
  - `folder` — drill-in with breadcrumb. Cells = subfolders of `currentFolderId` then assets in it.
  - `flat` — every asset, no folders, sorted/filtered.
  - `grouped` — bucket assets by `folderId` once; root section "Library" + one section per folder,
    sections sorted by full folder path (`folderPath`-joined " / "), collapsible per `folderId`.
- `gridDimensions(width)`: `spacing = Spacing.xl`, `outerPadding = Spacing.md*2`,
  `cols = max(1, floor((usable+spacing)/(thumbnailSize+spacing)))`,
  `tileWidth = max(thumbnailSize, (usable - (cols-1)*spacing)/cols)`. Tiles render 16:9.
- `mediaPanelColumnCount` is published for keyboard arrow navigation (`moveMediaSelection`).
- `mediaPanelOrderedItemIds` published per mode for scroll-to-reveal + arrow nav; folder cell keys
  are `"folder-<id>"` (`MediaPanelItemKey`), asset keys are raw ids.

### Selection / navigation
- Asset tap = single-select + open preview (`selectMediaAsset`); shift-tap toggles set + opens
  preview tab. Folder single-click selects (shift adds), double-click (`< doubleClickInterval`)
  opens. Marquee: `DragGesture(minDistance 3)` in `"mediaGrid"` space; ignores drags that start on a
  cell; shift extends current selection; intersect `assetFrames` rects to build selection.
- Breadcrumb: `[Library, ...folderPath]`; non-leaf chips navigate; each chip is a drop target.
- `Cmd+Shift+N` = new folder, `Cmd+Up` (keyCode 126) = navigate up — via `KeyCommandSink`
  `NSViewRepresentable` (replace with React key handlers / Tauri menu accelerators).
- External reveal entry points: `editor.mediaPanelRevealAssetId`, `mediaPanelOpenFolderId`,
  `mediaPanelScrollTarget`, `mediaPanelPasteRequestTick`, `mediaPanelShowMediaTabTick` — backend→UI
  signals; port as Tauri events.

### Drag-drop (in-panel + out)
- URI schemes: `palmier-folder://<id>`, `palmier-asset://<id>`, and a search "moment"
  `palmier-asset://<id>#<start>-<end>` (source seconds, `%.3f`). Drag payload for a selected asset
  emits all selected ids newline-joined; else just that id.
- `handleProviderDrop`: file URL provider → `importFinderItems`; NSString provider → `resolveTextDrop`
  which splits lines, routes folder ids → `moveFoldersToFolder`, asset ids → `moveAssetsToFolder`.
  Drop targets: panel root, breadcrumb chip, folder tile, grouped-section header.
- Drag-out to timeline uses the same `palmier-asset://` payload + a thumbnail/badge drag preview
  (count badge when multi-select).

### Folder model & moves
- `MediaFolder { id, name, parentFolderId }` persisted in `MediaManifest.folders`.
- `moveFoldersToFolder` guards against cycles: rejects move into self, into a descendant
  (`isDescendant`), or no-op. `deleteFolders` deletes folder + all descendants + their assets + any
  timeline clips referencing those assets (then prunes empty tracks), one snapshot undo.
- All folder/asset moves use `applyParentChanges` swap-undo (snapshot prior → write → inverse undo).

### Thumbnails / waveforms (palmier-media core — see MediaVisualCache.swift)
- **Concurrency gates:** waveform `AsyncSemaphore(value: 2)`, image thumbnail
  `AsyncSemaphore(value: 4)`. Video-thumbnail generation has NO semaphore (priority `.userInitiated`,
  others `.utility`). In-flight `Set<String>` per kind dedupes duplicate requests. FOUNDATION §6.2
  states "2 waveform, 4 image thumbnails" — matches; note video thumbs are ungated in the reference.
- **Disk cache key:** `SHA256("<path>|<size>|<mtime_epoch>").prefix(16)` hex — source edits
  invalidate. Cache dir `DiskCache(named:"MediaVisualCache")`.
- **Video thumbnail strip:** `videoThumbnailTimes`: interval = 1.0s if duration<10 else 2.0s, from 0
  to duration. `AVAssetImageGenerator` maxSize 120×68, tolerance ±1.0s. Frames published
  progressively every 50. Persisted as ONE JPEG sprite grid (≤50 columns, q=0.75) + `.thumbs.json`
  sidecar (`tileWidth,tileHeight,columns,times[]`); sidecar written last = completion marker.
  FOUNDATION says cache as "sequence of JPEG frames" — reference uses a single sprite-sheet; flag.
- **Waveform:** `waveformSampleCount`: `>=133.3s → 20000` samples, else `max(4000, duration*150)`,
  duration<=0 → 4000. Generated via `WaveformAnalyzer().samples(count:)` (DSWaveformImage,
  normalized 0=loud..1=silence). Persisted as raw `[Float]` little-endian `.waveform` blob.
  FOUNDATION's "~2000 samples/min" differs from reference's 150/sec (=9000/min); flag — match
  reference's 150 samples/sec, 20000 cap.
- **Image thumbnail:** ImageIO `CGImageSourceCreateThumbnailAtIndex` maxPixelSize 120, transform
  applied. (Asset-tile thumbnail itself comes from `MediaAsset.loadMetadata`: video frame at t=0
  maxSize 320, image ImageIO thumb 1568, lottie first frame.)
- Lookups (`samples`/`thumbnails`/`imageThumbnail`) are sync `MainActor.assumeIsolated` reads for
  draw calls — in Rust this is a `Mutex<HashMap>` or per-frame snapshot.

### Search (entry points + algorithms)
- Name search: live filter via `passesFilters` on every asset (substring, case-insensitive).
- `searchResults` panel sections (only when query non-empty): **Moments** (visual `VisualSearch.Hit
  {assetID,time,shotStart,shotEnd,score}`), **Spoken** (`TranscriptSearch.Hit {assetID,start,end,
  text}`), **Files** (name matches). Moments/Spoken collapsible; empty → "No matches".
- `scheduleMomentSearch`: cancels prior task, 250ms debounce, then runs `TranscriptSearch.search`
  (keyword, always available) and `await searchIndex.search(query:)` (CLIP cosine, top-20,
  `visualMatchCosineFloor`, relativeCutoff 0.85). Only video/audio assets feed it.
- `SearchIndexCoordinator` (per project): queue + single `.utility` worker, indexes image (CLIP
  still) / video (CLIP sampled frames + optional transcript split 0.5/0.5), pauses while any export
  active (refcounted across windows), progress as `batchCompleted/batchTotal + currentAssetFraction`.
  Index status pill states: notInstalled→download, downloading%, preparing, indexing N/M, failed.
- Hit interaction: tap a moment → `selectMediaAsset(atSourceFrame:)`; moments/spoken drag with the
  `#start-end` segment payload (images drag plain — segment meaningless for stills).

### Captions tab (CaptionTab + CaptionBuilder)
- Form: Source (auto = selected clips else all captionable audio, or pick a track), Language (Auto +
  `Transcription.supportedLocales()`), Style (font/size/color/bg/case/profanity-censor), Placement
  (centerX/Y with center-snap guides + threshold). Generate → `editor.generateCaptions(CaptionRequest)`.
- `CaptionBuilder` (read for parity, FOUNDATION §6.9): per segment recursively split to fit screen
  (sentence `.!?` → clause `,;:` → midpoint), distribute time by char count, min display 0.7s,
  cascade to prevent overlap, map source secs → timeline frames via trim+speed → `TextClipSpec`.
- Agent-mode menu hands a prompt draft to `agentService` (remove fillers, fix names, add emoji,
  translate to 10 langs). No direct compute — just opens agent panel with a draft.

### Music tab
- NOT a browseable music library. It is a generation form: video-to-music (scores selected timeline
  span / whole timeline) or text-to-music (duration 1–600s placed at marked-range start or playhead).
  Models = `AudioModelConfig` where category==music & inputs contains video. Cost via `CostEstimator`,
  gated on credits + sign-in. **FOUNDATION §6.2 says Music tab = "built-in music library from Convex
  /v1/music, browse + audition + drag" — this directly contradicts the reference. Flag: the reference
  Music tab is a generation panel, not a sample library.** Port the reference behavior; treat the
  FOUNDATION line as a spec error to reconcile.

## macOS/Apple APIs to replace (→ Windows/Linux/Rust equivalent)
- `MediaPanelDropArea` / `NSHostingView.registerForDraggedTypes` / `performDragOperation` (AppKit
  drag host) → Tauri native file-drop event (`onDragDropEvent` / `tauri://drag-drop`) feeding a React
  drop zone. The whole `DropHostingView` class is dead on our platforms.
- `NSOpenPanel` (import + relink pickers) → Tauri `dialog` plugin (`open` with multiple + directory).
- `NSPasteboard` (paste, copy-path) → Tauri clipboard plugin + `arboard` for image data
  (`.png`/`.tiff` → import). Copy-Path writes newline-joined paths to clipboard.
- `NSWorkspace.activateFileViewerSelecting` (Reveal in Finder) → Windows `explorer /select,<path>`;
  Linux `xdg-open` on parent dir (or `dbus FileManager1.ShowItems`). Tauri `opener` plugin.
- `NSEvent.modifierFlags` / `doubleClickInterval` / `keyCode 126` (`KeyCommandSink`) → React
  pointer/keyboard events + Tauri menu accelerators (Ctrl+Shift+N, Ctrl+Up per FOUNDATION §6.1).
- `AVAssetImageGenerator` (asset thumbnail, video strip, moment thumbs) → FFmpeg seek+scale in
  `palmier-media`; expose a `thumbnail(media_ref, source_seconds, max_size)` Tauri command for the
  async moment thumbnails (`MomentThumbnail` keyed by `path@time`).
- `AVURLAsset.load` metadata (duration, naturalSize×preferredTransform, nominalFrameRate, audio
  tracks) → FFprobe/`ffmpeg-next` stream metadata; apply display-matrix/rotation like the Swift
  `preferredTransform` correction so vertical video reports correct W/H.
- `ImageIO CGImageSource*Thumbnail*` → `image` crate decode + resize (respect EXIF orientation,
  which the `kCGImageSourceCreateThumbnailWithTransform` flag does here).
- `DSWaveformImage WaveformAnalyzer` → `symphonia` decode + RMS/peak downsample to the sample count
  formula above; store `Vec<f32>` per FOUNDATION §6.2.
- `CryptoKit SHA256` (cache key) → `sha2` crate, same `path|size|mtime` seed, 16-byte prefix.
- `CALayer.render` / `compositeCapture` (capture-frame-to-media) → wgpu readback compositing
  (FOUNDATION §6.5); only relevant to the "capture current frame" entry, not core panel.
- `NSImage` thumbnails everywhere → decoded RGBA textures / data URLs to the webview.
- `@MainActor @Observable` view models / `AsyncSemaphore` → Rust `tokio::sync::Semaphore` +
  Tauri-event-driven Zustand store.

## Mapping to FOUNDATION crates
- **palmier-media**: `MediaVisualCache` (video sprite-sheet thumbnails, waveform `Vec<f32>`, image
  thumbnails), the two concurrency semaphores (2 waveform / 4 image), SHA256 disk cache with
  `path|size|mtime` invalidation, `MediaAsset.loadMetadata` via FFmpeg/ffprobe, `ClipType` extension
  table. Thumbnail/waveform cache dirs land under `%APPDATA%\PalmierProWin\Cache\` per FOUNDATION.
- **src-ui/media-panel**: `MediaPanelView` rail + the three tabs; MediaTab toolbar/sort/filter/view
  switch; the three grid layouts + grid-dimension math; marquee selection; breadcrumb; inline
  folder/asset rename; drag-drop wiring to Tauri events; search-results panel (Moments/Spoken/Files);
  index-status pill; Captions + Music forms. State backed by Zustand mirroring `editor.*` signals.
- Cross-crate: folder CRUD + asset moves are model ops → `palmier-model`/`palmier-project` with
  snapshot undo in `palmier-history`. Visual/spoken search → `palmier-search` / `palmier-transcribe`.
  Caption/music generation → `palmier-transcribe` / `palmier-gen` (forms only live in this panel).

## Port risks & gotchas
- **Music tab spec conflict** (above): reference = generation form; FOUNDATION = sample library.
  Must reconcile before building; default to reference behavior for parity.
- **Sort modes:** reference has `name, dateAdded, duration, type` (4); FOUNDATION §6.2 lists only 3
  (drops `type`). Implement all 4.
- **Waveform sample density** & **thumbnail cache format** differ from FOUNDATION numbers (150/sec +
  sprite-sheet vs ~2000/min + frame sequence). Use the reference's actual constants for parity.
- **Single undo step for multi-item import** — Tauri/React import flow must batch the whole drop
  (files + recursive folders) into one history entry, matching `disableUndoRegistration` window.
- **In-flight dedupe + progressive publish:** video thumbnails publish every 50 frames; the UI must
  tolerate partial strips. Don't block import on full extraction.
- **Lottie validation**: `.json` must be sniffed as a real Lottie or rejected — extension alone is
  insufficient.
- **Folder cycle guards** on move (self / descendant / equality) are load-bearing; reproduce exactly.
- **dateAdded sort = insertion order**, not a stored timestamp — preserve array order; don't sort by
  `created_at` unless you also seed it from import order.
- **Drag payload format** (`palmier-asset://id#start-end`, `%.3f`) is the contract between panel and
  timeline drop + agent moment drags; keep byte-for-byte if any other surface parses it.
- **Cache key uses mtime** — on Windows FAT/exFAT mtime resolution (2s) could cause false cache hits
  across rapid edits; consider adding size already present, maybe hash a content prefix.

## Open questions
- Exact `VisualModelLoader` model files/sizes and `SearchIndexConfig.visualMatchCosineFloor` value
  (referenced but defined outside MediaPanel) — needed for the index-status size label + cutoff.
- `WaveformAnalyzer` normalization curve (DSWaveformImage) — confirm it is linear amplitude or
  perceptual before reimplementing in symphonia.
- Whether `EmbeddingStore` index lives per-project (`<project>/.search/...` per FOUNDATION §6.10) —
  coordinator references it but path is defined elsewhere.
- `Transcription.supportedLocales()` source (Apple Speech) — Whisper port must supply an equivalent
  language list for the Captions language picker.
