---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
title: "Epic 4 — Media Import & Panel (story decomposition)"
created: 2026-06-20
governing_reference: docs/reference/media-panel.md
foundation_section: "§6.2"
prd_features: [FR-14, FR-15, FR-16]
milestone: M1
---

# Epic 4 — Media Import & Panel

## Epic goal

Build the left-dock **Media Panel**: the project's asset browser plus the three secondary surfaces
(Media / Captions / Music tabs). This epic owns asset **import** (native drop + file picker + paste,
recursive folder mirroring), **browse** (4 sort modes, filter chips, folder/flat/grouped views,
thumbnail-size slider, inline folder create/rename, marquee select, name search), the **folder
hierarchy** model + cycle-guarded moves with snapshot undo, and the **thumbnail / waveform** decode
pipeline in `palmier-media`. The Captions and Music tabs are **forms only** in this epic — their
generation backends land in Epic 10 (transcription) and Epic 9 (generation); this epic ships the
form UI as stubs wired to placeholder commands. The visual/spoken **search-results panel** and the
CLIP/index-status pill are likewise UI stubs here; their search backends land in Epic 11.

Realizes **UJ-2** (folder of clips imported recursively, mirroring the directory tree) and **UJ-1**
(import the `.mp4` into the Media panel). Milestone **M1 — Hand-Edit MVP** (PRD §12).

### PRD §4.4 / §10 acceptance this epic must satisfy

From the §10 Epic 4 row and §4.4 (FR-14..FR-16):

- **FR-14 Import.** Native Tauri drop + file picker, multi-file, recursive multi-folder import that
  mirrors the directory tree as a folder hierarchy; supported extensions per FOUNDATION §6.2.
- **FR-15 Browse/sort/filter/view.** **4 sort modes** (dateAdded = insertion order, name, duration,
  type — **ruling #15**); filter chips; folder/flat/grouped views; thumbnail-size slider; inline
  folder create/rename; marquee select; name search.
- **FR-16 Thumbnails & waveforms.** Video thumb = single JPEG **sprite-sheet + JSON sidecar**;
  waveform = **150 samples/s capped 20000**; cache key `sha256(path|size|mtime).prefix16`; gates
  **waveform = 2, image-thumb = 4, video-thumb ungated** (**ruling #16**). *(Watch R-7: Windows
  coarse-FS mtime may false-hit — carry-forward note.)*
- **Music tab is a generation form** (**ruling #14**) — wired in Epic 9; this epic ships the form
  shell only. NOT a `/v1/music` sample library (that spec line is void).

### Spike / risk note

**Epic 4 is NOT spike-gated.** It has no dependency on Spike S-1 (wgpu→WebView) — the panel uses
decoded RGBA textures / data-URLs handed to the webview for thumbnails, not the live wgpu preview
surface. It can be built fully in M1 in parallel with the preview spike. Watch items carried into QA:
- **R-7** — `sha256(path|size|mtime)` cache key may false-hit on coarse Windows FS mtime (2 s
  resolution on FAT/exFAT). Watch; fall back to content-prefix hashing if false-hits surface.
- **Music tab spec conflict** (reference = generation form; FOUNDATION §6.2 = sample library) is
  already resolved by **ruling #14** in favor of the reference. Do not build a `/v1/music` browser.

---

## Story map (dependency order)

| id | title | crate(s) | depends on | parallel-safe |
|---|---|---|---|---|
| E4-S1 | ClipType extension table + asset metadata loader | palmier-media, palmier-model | Epic 2 (model) | yes |
| E4-S2 | Disk cache + SHA256 key + concurrency gates | palmier-media | — | yes |
| E4-S3 | Video sprite-sheet thumbnail pipeline | palmier-media | E4-S1, E4-S2 | yes |
| E4-S4 | Waveform pipeline (150/s, cap 20000) | palmier-media | E4-S1, E4-S2 | yes |
| E4-S5 | Image thumbnail pipeline (EXIF-aware) | palmier-media | E4-S1, E4-S2 | yes |
| E4-S6 | Folder model + cycle-guarded moves + snapshot undo | palmier-model, palmier-project, palmier-history | Epic 2, Epic 3 (history) | no (model) |
| E4-S7 | Import orchestration (drop + picker + paste, one undo step) | palmier-media, palmier-model, src-ui/media-panel | E4-S1, E4-S6 | no |
| E4-S8 | Panel shell: rail + 3 tabs + Zustand store + reveal events | src-ui/media-panel | — | yes |
| E4-S9 | Media tab: toolbar, sort/filter, 3 view modes + grid math | src-ui/media-panel | E4-S8 | no |
| E4-S10 | Asset/folder tiles: thumbnails, badges, rename, context menu | src-ui/media-panel | E4-S3, E4-S5, E4-S9 | no |
| E4-S11 | Selection, marquee, keyboard nav, breadcrumb | src-ui/media-panel | E4-S9 | no |
| E4-S12 | In-panel + drag-out drag-drop (URI schemes, moves) | src-ui/media-panel | E4-S6, E4-S9 | no |
| E4-S13 | Name search + search-results panel scaffold + index pill stub | src-ui/media-panel | E4-S9 | no |
| E4-S14 | Captions + Music tab form shells (stubs) | src-ui/media-panel | E4-S8 | yes |

**Parallel-safe** = touches no files another in-flight sibling touches; can run in its own worktree.
The `palmier-media` backend stories (E4-S1..S5) are mutually parallel after their stated deps. The
`src-ui/media-panel` stories (E4-S9..S13) serialize on shared panel files; E4-S8 and E4-S14 are the
two UI stories that touch disjoint files and can run alongside.

---

## Story E4-S1 — ClipType extension table + asset metadata loader

**Intent:** As the import pipeline, I need the single extension→type gate and per-asset metadata so
every imported file is classified and described exactly as the reference does.

**Crate(s):** `palmier-media` (decode/metadata), `palmier-model` (`ClipType`, `MediaAsset`).

### Acceptance criteria

- **`ClipType::from_extension(ext)`** is the single gate, lowercasing the ext, mapping exactly
  (`docs/reference/media-panel.md` "Import & supported extensions"):
  - video: `mov, mp4, m4v` · audio: `mp3, wav, aac, m4a`
  - image: `png, jpg, jpeg, tiff, heic, webp` · lottie: `json, lottie`
  - any other ext → `None` (caller emits a `mediaPanelToast` "unsupported file type" and drops it).
- **Lottie second gate:** a `.json` is accepted as `lottie` **only if** a Lottie-sniff
  (`is_lottie(path)`, port of `LottieVideoGenerator.isLottie(at:)`) passes; a non-Lottie `.json` is
  **rejected**, not imported. Unit test: a plain `{}` JSON file is refused; a real Lottie JSON passes.
- **`load_metadata(asset)`** populates duration, natural width/height (with rotation/display-matrix
  correction so vertical video reports correct W/H), `nominalFrameRate`, and audio-track presence:
  - video/audio via **FFprobe / `ffmpeg-next`** stream metadata (replaces `AVURLAsset.load`); apply
    the display-matrix rotation exactly like the Swift `preferredTransform` correction.
  - image via the `image` crate (decode dims), respecting EXIF orientation.
  - lottie via the Lottie parser (first-frame dims, duration from the animation).
- **Asset-tile base thumbnail** produced by `load_metadata` (distinct from the visual-cache strips):
  video frame at `t=0` maxSize **320**; image ImageIO-equivalent thumb at maxPixelSize **1568**;
  lottie first frame. (`docs/reference/media-panel.md` "Image thumbnail" parenthetical.)
- Unit tests cover every ext in the table (positive + negative), the lottie sniff, and a
  rotation-corrected vertical-video metadata case.

### Implementation context

- Reference: `Sources/PalmierPro/Models/ClipType.swift`, `Models/MediaAsset.swift` (`loadMetadata`),
  `MediaPanel/MediaTab/MediaTab.swift` import gating. Doc: `media-panel.md` §"Import & supported
  extensions" + §"macOS/Apple APIs to replace" (AVURLAsset→ffprobe, ImageIO→`image` crate).
- macOS API replacements: `AVURLAsset.load` → ffprobe/`ffmpeg-next`; `ImageIO CGImageSource*` →
  `image` crate (respect EXIF); `NSImage` → decoded RGBA.

**Depends on:** Epic 2 (`palmier-model` `MediaAsset`/`ClipType` shapes exist). **Parallel-safe:** yes.

---

## Story E4-S2 — Disk cache, SHA256 key, concurrency gates

**Intent:** As the visual-cache subsystem, I need the shared disk cache, cache-key derivation, and
the concurrency semaphores so thumbnail/waveform generation is deduped, gated, and invalidated on
source edits exactly per the reference.

**Crate(s):** `palmier-media` (`MediaVisualCache` core).

### Acceptance criteria

- **Disk cache key:** `sha256("<path>|<size>|<mtime_epoch>")` hex, **first 16 bytes (`.prefix(16)`)**,
  using the `sha2` crate (replaces CryptoKit). Same `path|size|mtime` seed. A source edit (changed
  size or mtime) yields a new key and misses the stale entry. (`media-panel.md` §"Disk cache key";
  **ruling #16**.)
- **Cache dir** `MediaVisualCache` lands under `%APPDATA%\PalmierProWin\Cache\` (Win) /
  XDG cache dir (Linux), per FOUNDATION mapping in `media-panel.md` §"Mapping to FOUNDATION crates".
- **Concurrency gates** via `tokio::sync::Semaphore`: **waveform = 2**, **image-thumbnail = 4**,
  **video-thumbnail = ungated** (priority `userInitiated`; others `utility`). An in-flight
  `HashSet<String>` per kind **dedupes** duplicate requests for the same key. (**ruling #16**.)
- Synchronous lookup reads (`samples` / `thumbnails` / `imageThumbnail`) are `Mutex<HashMap>`
  snapshot reads safe to call from the draw/serialization path (replaces
  `MainActor.assumeIsolated`).
- **R-7 watch:** record a TODO/QA note that coarse Windows-FS mtime granularity may false-hit; do not
  change the key (parity), but leave a feature-flagged content-prefix-hash fallback hook.
- Unit tests: key stability for identical inputs, key change on size/mtime change, semaphore caps (2
  concurrent waveform / 4 image), in-flight dedupe collapses two identical requests to one job.

### Implementation context

- Reference: `Sources/PalmierPro/Timeline/MediaVisualCache.swift` (cache key, semaphores, in-flight
  sets, `DiskCache(named:"MediaVisualCache")`). Doc: `media-panel.md` §"Thumbnails / waveforms".
- macOS replacements: `CryptoKit SHA256` → `sha2`; `AsyncSemaphore` → `tokio::sync::Semaphore`.

**Depends on:** none (pure infra). **Parallel-safe:** yes.

---

## Story E4-S3 — Video sprite-sheet thumbnail pipeline

**Intent:** As the media tab, I need video thumbnail strips generated and cached as a single JPEG
sprite-sheet plus a JSON sidecar so cells render the reference strip with progressive publishing.

**Crate(s):** `palmier-media`.

### Acceptance criteria

- **Times:** `video_thumbnail_times(duration)` — interval = **1.0 s if duration < 10**, else
  **2.0 s**, from `0` to `duration`. (`media-panel.md` §"Video thumbnail strip".)
- **Extraction:** FFmpeg seek+scale (`thumbnail(media_ref, source_seconds, max_size)` style), max
  size **120×68**, tolerance **±1.0 s** (replaces `AVAssetImageGenerator`). **Ungated** (no
  semaphore), priority `userInitiated`.
- **Progressive publish:** frames published every **50** so the UI can render a partial strip; import
  must not block on full extraction (carry-forward gotcha).
- **Persisted format:** ONE JPEG **sprite-sheet** grid (**≤ 50 columns**, quality **0.75**) plus a
  `.thumbs.json` sidecar (`tileWidth, tileHeight, columns, times[]`). **Sidecar written LAST** = the
  completion marker. (FOUNDATION's "frame sequence" is **superseded** by the reference sprite-sheet —
  **ruling #16**; flag honored.)
- A `thumbnail(media_ref, source_seconds, max_size)` Tauri command is exposed for the async
  search "moment" thumbnails (`MomentThumbnail`, keyed `path@time`) — used by Epic 11's search panel.
- Unit/integration: times formula at duration 5/10/30; sprite grid column count ≤ 50; sidecar present
  ⇒ complete, absent ⇒ incomplete; progressive callback fires at 50-frame boundaries.

### Implementation context

- Reference: `MediaVisualCache.swift` (`videoThumbnailTimes`, sprite write, `.thumbs.json`),
  `MediaTab+Search.swift` (`MomentThumbnail`). Doc: `media-panel.md` §"Video thumbnail strip" +
  §"macOS/Apple APIs to replace" (AVAssetImageGenerator → FFmpeg).

**Depends on:** E4-S1 (metadata/duration), E4-S2 (cache+gates). **Parallel-safe:** yes.

---

## Story E4-S4 — Waveform pipeline (150 samples/s, cap 20000)

**Intent:** As the media tab and timeline, I need waveforms computed and cached so audio/video assets
draw the reference waveform with the exact sample density.

**Crate(s):** `palmier-media`.

### Acceptance criteria

- **Sample count:** `waveform_sample_count(duration)`:
  `duration >= 133.3 s → 20000`; else `max(4000, round(duration * 150))`; `duration <= 0 → 4000`.
  (= **150 samples/s capped 20000** — **ruling #16**; FOUNDATION's "~2000/min" is superseded.)
- **Generation:** `symphonia` decode → RMS/peak downsample to the sample count (replaces
  DSWaveformImage `WaveformAnalyzer`). Output normalized **0 = loud … 1 = silence** to match the
  reference draw axis. *(Open question carried: confirm DSWaveformImage's curve is linear amplitude
  vs perceptual before locking; default to linear amplitude RMS, note the assumption.)*
- **Persisted format:** raw `Vec<f32>` little-endian `.waveform` blob (FOUNDATION §6.2 "Vec<f32>").
- **Gated at 2** concurrent (from E4-S2). Image/text/lottie produce no waveform; audio → waveform;
  video → waveform + (E4-S3) strips.
- Unit tests: sample-count formula at duration 0 / 5 / 130 / 200; output length matches; normalization
  direction (a silent buffer → ~1.0, full-scale → ~0.0).

### Implementation context

- Reference: `MediaVisualCache.swift` (`waveformSampleCount`, `WaveformAnalyzer().samples(count:)`,
  `.waveform` blob). Doc: `media-panel.md` §"Waveform" + §"macOS/Apple APIs to replace"
  (DSWaveformImage → symphonia).

**Depends on:** E4-S1, E4-S2. **Parallel-safe:** yes.

---

## Story E4-S5 — Image thumbnail pipeline (EXIF-aware)

**Intent:** As the media tab, I need image thumbnails generated and cached so image assets render
small tiles with correct orientation.

**Crate(s):** `palmier-media`.

### Acceptance criteria

- **Generation:** `image` crate decode + resize to maxPixelSize **120**, **respecting EXIF
  orientation** (the reference uses `kCGImageSourceCreateThumbnailWithTransform`). (`media-panel.md`
  §"Image thumbnail".)
- **Gated at 4** concurrent (from E4-S2); cached under the E4-S2 key.
- Image/text/lottie: image → image-thumbnail; text/lottie → no visual-cache thumbnail (tile uses the
  E4-S1 base thumbnail / lottie first frame).
- Unit tests: a landscape and a portrait-with-EXIF-rotation image both produce a correctly-oriented
  ≤120px thumbnail.

### Implementation context

- Reference: `MediaVisualCache.swift` (`imageThumbnail`, ImageIO transform flag). Doc: `media-panel.md`
  §"Image thumbnail" + §"macOS/Apple APIs to replace" (ImageIO → `image` crate).

**Depends on:** E4-S1, E4-S2. **Parallel-safe:** yes.

---

## Story E4-S6 — Folder model + cycle-guarded moves + snapshot undo

**Intent:** As the model layer, I need the `MediaFolder` tree, folder CRUD, and asset/folder moves
with cycle guards and snapshot undo so the panel's hierarchy operations are correct and reversible.

**Crate(s):** `palmier-model` (`MediaFolder`), `palmier-project` (manifest persistence),
`palmier-history` (snapshot undo).

### Acceptance criteria

- **`MediaFolder { id, name, parent_folder_id }`** persisted in `media.json` (the `MediaManifest.folders`
  collection; reference filename per **ruling #3**). (`media-panel.md` §"Folder model & moves".)
- **`move_folders_to_folder`** guards reject: move into **self**, move into a **descendant**
  (`is_descendant`), and **no-op** (already-parent). Reproduce these three guards exactly.
- **`move_assets_to_folder`** reparents asset(s) to a target folder.
- **`delete_folders`** deletes the folder + **all descendants** + their assets + **any timeline clips
  referencing those assets**, then prunes empty tracks — as **one snapshot undo** entry.
- **`apply_parent_changes`** swap-undo: snapshot prior state → write → register inverse undo. All
  folder/asset moves go through it. Moves register on the **user** undo stack (Epic 3 history).
- **`import_folder`** recursion (used by E4-S7): creates a `MediaFolder` named after the dir, lists
  contents with hidden-files skipped, sorts by **localized-standard compare**, recurses subdirs,
  imports files whose ext maps to a `ClipType`. Directory tree → folder tree, **1:1**.
- Unit tests: reject move-into-self, reject move-into-descendant, reject no-op; delete cascades to
  descendant folders + assets + referencing clips + empty-track prune; move undo restores prior parents.

### Implementation context

- Reference: `Editor/ViewModel/EditorViewModel+Folders.swift` (folder CRUD/move, `moveFoldersToFolder`,
  `isDescendant`, `deleteFolders`, `applyParentChanges`), `Models/MediaFolder` in the manifest. Doc:
  `media-panel.md` §"Folder model & moves" + §"Mapping to FOUNDATION crates" (model ops →
  palmier-model/palmier-project, snapshot undo → palmier-history).

**Depends on:** Epic 2 (manifest/model), Epic 3 (`palmier-history` snapshot undo stack).
**Parallel-safe:** no (touches shared model/manifest types).

---

## Story E4-S7 — Import orchestration (drop + picker + paste, one undo step)

**Intent:** As a user, I want to import media by drag-drop, the file picker, or paste, with each whole
import (including recursive folders) batched into a single undo step, so I can add a shoot's worth of
clips in one reversible action.

**Crate(s):** `palmier-media`, `palmier-model`, `src-ui/media-panel` (import wiring + Tauri commands).

### Acceptance criteria

- **`import_finder_items(urls, into)`** is **one undo step** named **"Import Media"**: disable undo
  registration during the loop, then register **one** snapshot-restore undo iff anything changed
  (matches `disableUndoRegistration` window — carry-forward gotcha). (`media-panel.md` §"Import &
  supported extensions".)
- **Native drop:** Tauri `tauri://drag-drop` (`onDragDropEvent`) feeds the React drop zone → calls
  `import_finder_items`. The AppKit `MediaPanelDropArea` / `DropHostingView` is **not ported** (dead).
- **File picker:** Tauri `dialog` plugin `open` (multiple + directory) replaces `NSOpenPanel`;
  allowed types = movie/image/audio/json(+lottie); imports into `current_folder_id`.
- **Paste** (`handle_clipboard_paste`): if clipboard has file URLs → import them; else `.png`/`.tiff`
  image data → write `pasted-<8hex>.<ext>` into `<project>/media/` (or temp) then move into the
  current folder. A `clipboard_has_importable_media` check gates the paste menu item. Uses Tauri
  clipboard + `arboard` (replaces `NSPasteboard`).
- **Recursion:** folder URLs go through E4-S6 `import_folder` (directory tree → folder tree 1:1).
- **Unsupported ext** → `mediaPanelToast` "unsupported file type", file dropped (no asset).
- **Post-import** `finalize_imported_asset(asset)` async: `load_metadata` (E4-S1) →
  `update_manifest_metadata` → `search_index.schedule(asset)` (Epic 11 hook; no-op stub here) →
  kick visual cache: video → waveform (E4-S4) + video strips (E4-S3); audio → waveform; image →
  image-thumb (E4-S5); text/lottie → none.
- Integration test: dropping a folder containing N media + a subfolder produces the mirrored hierarchy
  and exactly **one** undo entry that fully reverses the import.

### Implementation context

- Reference: `EditorViewModel+MediaLibrary.swift` (`importFinderItems`, `finalizeImportedAsset`,
  `importMedia`, `handleClipboardPaste`, `importPastedImageData`), `MediaTab+Drag.swift`
  (`handleProviderDrop` file-URL branch). Doc: `media-panel.md` §"Import & supported extensions" +
  §"macOS/Apple APIs to replace" (NSOpenPanel→dialog, NSPasteboard→clipboard+arboard, drop host→Tauri).

**Depends on:** E4-S1 (metadata + ClipType), E4-S6 (folder import/recursion). **Parallel-safe:** no.

---

## Story E4-S8 — Panel shell: rail + 3 tabs + store + reveal events

**Intent:** As a user, I want the left-dock panel with a 3-icon rail switching Media / Captions /
Music, backed by a reactive store, so the panel mirrors editor state and responds to backend reveal
signals.

**Crate(s):** `src-ui/media-panel`.

### Acceptance criteria

- **`MediaPanelView`** hosts a 3-icon vertical rail switching `PanelTab { media, captions, music }`;
  reacts to a `mediaPanelShowMediaTabTick` event to force the Media tab.
- **Zustand store** mirrors the `editor.*` signals the panel consumes; all side effects go through
  Tauri commands, reactive state via Tauri events (FOUNDATION §4 strict layering).
- **External reveal entry points** wired as Tauri events (backend → UI): `mediaPanelRevealAssetId`,
  `mediaPanelOpenFolderId`, `mediaPanelScrollTarget`, `mediaPanelPasteRequestTick`,
  `mediaPanelShowMediaTabTick`. (`media-panel.md` §"Selection / navigation".)
- Tab switch renders the correct tab body (Media = E4-S9, Captions/Music = E4-S14 shells).

### Implementation context

- Reference: `MediaPanelView.swift` (rail + tab switch + tick reaction). Doc: `media-panel.md`
  §"Key types & files" + §"Mapping to FOUNDATION crates" (src-ui/media-panel scope).

**Depends on:** none (UI scaffold). **Parallel-safe:** yes.

---

## Story E4-S9 — Media tab: toolbar, sort/filter, 3 view modes + grid math

**Intent:** As a user, I want the Media tab toolbar with sort/filter/view controls and the
folder/flat/grouped grids laid out exactly per the reference math, so browsing matches the Mac app.

**Crate(s):** `src-ui/media-panel`.

### Acceptance criteria

- **4 sort modes** (`SortMode { name, dateAdded, duration, type }` — **ruling #15**):
  `dateAdded` = **insertion order, no sort** (preserve array order — do **not** sort by a timestamp);
  `name` = case-insensitive asc; `duration` = **desc**; `type` = rawValue asc. (`media-panel.md`
  §"Sorting / filtering / view modes".)
- **`passes_filters`:** type ∈ `filterTypes` (or empty) **AND** (`!filterAI || asset.isGenerated`)
  **AND** name case-insensitively contains the trimmed query. Filterable chips =
  `[video, audio, image]` only (text/lottie excluded).
- **Thumbnail-size slider 80–200**, presets small 80 / medium 110 / large 150 / xlarge 200.
- **View modes:**
  - `folder` — drill-in with breadcrumb; cells = subfolders of `currentFolderId` then its assets.
  - `flat` — every asset, no folders, sorted/filtered.
  - `grouped` — bucket assets by `folderId` once; root section "Library" + one section per folder,
    sections sorted by full folder path (`folderPath` joined " / "), collapsible per `folderId`.
- **`grid_dimensions(width)`** exact math: `spacing = Spacing.xl`, `outerPadding = Spacing.md*2`,
  `cols = max(1, floor((usable + spacing) / (thumbnailSize + spacing)))`,
  `tileWidth = max(thumbnailSize, (usable - (cols-1)*spacing) / cols)`. Tiles render **16:9**.
- **Published for nav:** `mediaPanelColumnCount` and `mediaPanelOrderedItemIds` per mode; folder cell
  keys are `"folder-<id>"` (`MediaPanelItemKey`), asset keys are raw ids.
- Unit tests: each sort order on a fixed asset set; `passes_filters` truth table; `grid_dimensions`
  column/tileWidth at representative widths and thumbnail sizes.

### Implementation context

- Reference: `MediaTab/MediaTab.swift` (toolbar, `sortAndFilter`, `passesFilters`, `ViewMode`,
  `SortMode`, slider presets), `MediaTab+Grids.swift` (`gridDimensions`/`computeLayout`, three grid
  bodies). Doc: `media-panel.md` §"Sorting / filtering / view modes".

**Depends on:** E4-S8 (shell + store). **Parallel-safe:** no (shared media-tab files).

---

## Story E4-S10 — Asset/folder tiles: thumbnails, badges, rename, context menu

**Intent:** As a user, I want each asset and folder tile to render its thumbnail, duration, badges,
inline rename, and a context menu, so I can identify and act on items in the grid.

**Crate(s):** `src-ui/media-panel`.

### Acceptance criteria

- **Asset tile** (`AssetThumbnailView`): renders the E4-S3/E4-S5 thumbnail (or E4-S1 base thumbnail),
  type/AI badges, and duration; inline rename; tap = single-select + open preview; shift-tap toggles
  set + opens preview tab. Context menu items: **Reveal in Finder, Copy Path, Relink, Delete, AI Edit,
  Move-to-Folder**. (`media-panel.md` §"Key types & files" + §"Selection / navigation".)
- **Folder tile** (`FolderTileView`): inline rename; single-click selects (shift adds); double-click
  (within `doubleClickInterval`) opens.
- **Reveal in Finder** → Windows `explorer /select,<path>`; Linux `xdg-open` parent (or DBus
  `FileManager1.ShowItems`) via Tauri `opener` plugin. **Copy Path** writes newline-joined paths to
  the clipboard (Tauri clipboard).
- **Relink** opens the Tauri dialog picker (replaces `NSOpenPanel`) to repoint a missing asset.
- Thumbnails arrive as decoded RGBA / data-URLs to the webview (no `NSImage`).

### Implementation context

- Reference: `MediaTab/AssetThumbnailView.swift`, `MediaTab/FolderTileView.swift`. Doc:
  `media-panel.md` §"Key types & files" + §"macOS/Apple APIs to replace" (Reveal/Copy-Path,
  NSImage→RGBA).

**Depends on:** E4-S3, E4-S5 (thumbnails), E4-S9 (grid bodies host the tiles). **Parallel-safe:** no.

---

## Story E4-S11 — Selection, marquee, keyboard nav, breadcrumb

**Intent:** As a user, I want rubber-band selection, keyboard arrow navigation, and a breadcrumb so I
can select sets of items and move through folders like the Mac app.

**Crate(s):** `src-ui/media-panel`.

### Acceptance criteria

- **Marquee:** drag (minDistance 3) in the `"mediaGrid"` coordinate space; **ignores drags that start
  on a cell**; shift extends current selection; intersect `assetFrames` rects to build the selection.
  Selection persists across re-renders by id. (`media-panel.md` §"Selection / navigation".)
- **Keyboard arrow nav** via `mediaPanelColumnCount` + `mediaPanelOrderedItemIds` (E4-S9):
  `moveMediaSelection` arrow logic; scroll-to-reveal the focused item.
- **Breadcrumb** `[Library, ...folderPath]`; non-leaf chips navigate; each chip is a **drop target**
  (drop wiring in E4-S12).
- **Shortcuts** via React key handlers / Tauri menu accelerators (replacing `KeyCommandSink`):
  **Ctrl+Shift+N** = new folder, **Ctrl+Up** (was keyCode 126) = navigate up (FOUNDATION §6.1).
- Frame tracking uses a frame-reporting mechanism equivalent to the preference key (`assetFrames`).

### Implementation context

- Reference: `MediaTab.swift` (marquee `DragGesture`, `moveMediaSelection`), `MediaTab+Grids.swift`
  (frame-tracking preference key, `assetFrames`), `KeyCommandSink` (→ React/Tauri accelerators). Doc:
  `media-panel.md` §"Selection / navigation" + §"macOS/Apple APIs to replace".

**Depends on:** E4-S9. **Parallel-safe:** no.

---

## Story E4-S12 — In-panel + drag-out drag-drop (URI schemes, moves)

**Intent:** As a user, I want to drag assets/folders within the panel to reorganize them and drag
assets out to the timeline, using the exact URI payload contract, so moves and timeline drops work
and stay compatible with other surfaces.

**Crate(s):** `src-ui/media-panel`.

### Acceptance criteria

- **URI schemes** (byte-for-byte contract — keep exact): `palmier-folder://<id>`,
  `palmier-asset://<id>`, and a search-moment `palmier-asset://<id>#<start>-<end>` where start/end are
  **source seconds formatted `%.3f`**. Drag payload for a selected asset emits **all selected ids
  newline-joined**; else just that id. (`media-panel.md` §"Drag-drop" + §"Port risks": drag payload
  format is the contract between panel, timeline drop, and agent moment drags.)
- **`handle_provider_drop`:** file-URL provider → `import_finder_items` (E4-S7); NSString/text provider
  → `resolve_text_drop` which splits lines and routes folder ids → `move_folders_to_folder`, asset ids
  → `move_assets_to_folder` (E4-S6).
- **Drop targets:** panel root, breadcrumb chip (E4-S11), folder tile (E4-S10), grouped-section header
  (E4-S9).
- **Drag-out to timeline** uses the same `palmier-asset://` payload + a thumbnail/badge drag preview
  (count badge when multi-select).
- Integration test: dragging two selected assets onto a folder tile reparents both in one undo step;
  the emitted payload is the two ids newline-joined; a moment drag emits `...#<start>-<end>` with `%.3f`.

### Implementation context

- Reference: `MediaTab/MediaTab+Drag.swift` (URI schemes, payload/preview, `handleProviderDrop`,
  `resolveTextDrop`). Doc: `media-panel.md` §"Drag-drop (in-panel + out)" + §"Port risks" (payload
  format) + §"macOS/Apple APIs to replace" (drop host → Tauri file-drop event).

**Depends on:** E4-S6 (moves), E4-S9 (drop-target hosts). **Parallel-safe:** no.

---

## Story E4-S13 — Name search + search-results panel scaffold + index-pill stub

**Intent:** As a user, I want live name search plus the Moments/Spoken/Files results layout and the
index-status pill, so the panel's search surface is in place ahead of the Epic 11 search backend.

**Crate(s):** `src-ui/media-panel`.

### Acceptance criteria

- **Name search:** live filter via `passes_filters` (E4-S9) on every asset (substring,
  case-insensitive). Always works with no backend. (`media-panel.md` §"Search".)
- **`searchResults` panel** (shown only when query non-empty): three sections — **Moments** (visual
  `VisualSearch.Hit {assetID,time,shotStart,shotEnd,score}`), **Spoken** (`TranscriptSearch.Hit
  {assetID,start,end,text}`), **Files** (name matches). Moments/Spoken collapsible; empty → "No
  matches". Build the **layout + section components + collapsible state** now; the Moments/Spoken data
  is fed by **Epic 11** (`search_media`) — wire the call sites behind a stub returning empty.
- **`scheduleMomentSearch`** scaffold: cancel prior task, **250 ms debounce**, then call the (stubbed)
  search; only video/audio assets feed it. Keep the debounce + cancel logic; the search call is a stub.
- **Moment thumbnails:** `MomentThumbnail` async tile keyed `path@time` calls the E4-S3
  `thumbnail(media_ref, source_seconds, max_size)` command. Hit interaction: tap a moment →
  `selectMediaAsset(atSourceFrame:)`; moments/spoken drag with the `#start-end` payload (E4-S12);
  images drag plain.
- **Index-status pill** (`MediaTab+IndexStatus`) stub: render the state machine
  notInstalled→download / downloading% / preparing / indexing N/M / failed, driven by a stubbed
  status event (real progress from Epic 11's `SearchIndexCoordinator`).

### Implementation context

- Reference: `MediaTab/MediaTab+Search.swift` (`searchResults`, `scheduleMomentSearch`,
  `MomentThumbnail`), `MediaTab/MediaTab+IndexStatus.swift`. Doc: `media-panel.md` §"Search" +
  §"Mapping to FOUNDATION crates" (search → palmier-search/palmier-transcribe, Epic 11).

**Depends on:** E4-S9 (filter + tab body). **Parallel-safe:** no.

---

## Story E4-S14 — Captions + Music tab form shells (stubs)

**Intent:** As a user, I want the Captions and Music tab forms present in the panel, so the surfaces
exist in M1 ahead of their generation backends (Epic 10 / Epic 9).

**Crate(s):** `src-ui/media-panel`.

### Acceptance criteria

- **Captions tab form** (`CaptionTab`): Source (auto = selected clips else all captionable audio, or
  pick a track), Language (Auto + a supplied locale list — Whisper-equivalent of
  `Transcription.supportedLocales()`, sourced in Epic 10), Style (font/size/color/bg/case/profanity-
  censor; case = **auto/upper/lower only**, **ruling #18** — no title-case), Placement (centerX/Y with
  center-snap guides + threshold). Generate button calls a stubbed `generate_captions(CaptionRequest)`
  command (real impl + `CaptionBuilder` land in **Epic 10**). Agent-mode menu hands a prompt draft to
  the agent panel (no compute). (`media-panel.md` §"Captions tab".)
- **Music tab form** (`MusicTab`): the **generation form** (**ruling #14**) — video-to-music (scores
  selected timeline span / whole timeline) **or** text-to-music (duration 1–600 s placed at marked-
  range start or playhead). Models filtered to `AudioModelConfig` where category == music & inputs
  contain video. Cost via a stubbed `CostEstimator`; gated on credits + sign-in. The real submit +
  credit gate land in **Epic 9**. **Do NOT build a `/v1/music` library** (FOUNDATION §6.2 line is void).
- Both forms render and validate inputs; Generate is disabled / shows "backend not available" until
  its owning epic wires the command. No `/v1/music` browse UI exists anywhere.

### Implementation context

- Reference: `CaptionsTab/CaptionTab.swift` (+ `CaptionBuilder.swift`, read for parity only),
  `MusicTab.swift`. Doc: `media-panel.md` §"Captions tab" + §"Music tab" + §"Port risks" (Music tab
  spec conflict resolved to reference).

**Depends on:** E4-S8 (shell). **Parallel-safe:** yes (disjoint Captions/Music files).

---

## Cross-epic dependencies (summary)

- **Epic 2** (palmier-model/palmier-project): `MediaAsset`, `ClipType`, `MediaFolder`, `media.json`
  manifest shapes — needed by E4-S1, E4-S6.
- **Epic 3** (palmier-history): user snapshot undo stack — needed by E4-S6 (moves), E4-S7 (import).
- **Epic 9** (palmier-gen): Music tab generation backend + `CostEstimator` + credit gate — fills the
  E4-S14 Music stub.
- **Epic 10** (palmier-transcribe/palmier-text): Captions generation + `CaptionBuilder` + locale list —
  fills the E4-S14 Captions stub.
- **Epic 11** (palmier-search): visual/spoken search + `SearchIndexCoordinator` + index progress —
  fills the E4-S13 Moments/Spoken/index-pill stubs; consumes E4-S3's `thumbnail` command.

## Test/golden coverage owned by this epic

- Unit (FOUNDATION §11.1): ClipType table + lottie sniff (E4-S1); cache-key stability/invalidation +
  semaphore caps + in-flight dedupe (E4-S2); video-times + sprite/sidecar (E4-S3); waveform sample-
  count formula + normalization (E4-S4); EXIF-aware image thumb (E4-S5); folder cycle guards + delete
  cascade + move undo (E4-S6); single-undo recursive import (E4-S7); sort/filter/grid-math (E4-S9);
  drag-payload format + reparent-in-one-undo (E4-S12).
- This epic owns **no golden-fixture suite** (XMEML/CaptionBuilder/rendered-frame goldens belong to
  Epics 6/10/5). The **14 CaptionBuilder tests** are Epic 10's; E4-S14 only renders the form shell.
