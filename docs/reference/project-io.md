---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# project-io — reference port notes

## Purpose
How a Palmier project is persisted, loaded, autosaved, registered, and materialized. The reference
models a project as a macOS **NSDocument package** (a directory the Finder presents as one file).
Target crates: `palmier-project` (bundle I/O, registry, autosave, samples) and `palmier-model`
(the Codable shapes round-tripped: `Timeline`, `MediaManifest`, `GenerationLog`, `ChatSession`).
DISCREPANCY (flag, do not silently follow FOUNDATION): FOUNDATION §5.7 names the bundle files
`timeline.json`, `manifest.json`, `generation_log.json`, `chatsessions/`. The reference uses
`project.json`, `media.json`, `generation-log.json`, `chat/`. Pick one set and stay consistent;
if interop with real reference `.palmier` bundles or sample-server payloads matters, the reference
names are authoritative (the sample resolve endpoint writes `project.json`/`media.json` and a
`chat/<name>` relative path — see SampleProjectService below). Documented both; engineer must choose.

## Key types & files (cite paths under Sources/PalmierPro/...)
- `Utilities/Constants.swift:104` `enum Project` — the canonical constants:
  `fileExtension="palmier"`, `registryFilename="project-registry.json"`,
  `typeIdentifier="io.palmier.project"`, `defaultProjectName="Untitled Project"`,
  `timelineFilename="project.json"`, `manifestFilename="media.json"`,
  `generationLogFilename="generation-log.json"`, `thumbnailFilename="thumbnail.jpg"`,
  `mediaDirectoryName="media"`. `storageDirectory` = `~/Documents/Palmier Pro` (created on first read).
- `Project/VideoProject.swift` — `final class VideoProject: NSDocument`. The whole package read/write
  state machine. `autosavesInPlace = true`.
- `Project/ProjectRegistry.swift` — `@MainActor @Observable ProjectRegistry.shared`; `ProjectEntry`
  `{ id:UUID, url:URL, createdDate:Date, lastOpenedDate:Date }`. Disk via private `actor ProjectRegistryDisk`.
- `Project/SampleProjectService.swift` — fetch/list/materialize sample `.palmier` bundles from Convex.
- `App/AppState.swift` — project lifecycle orchestration (create/open/openSample/openFromPanel).
- `Agent/ChatSessionStore.swift` — `enum ChatSessionStore` (dirName=`chat`), per-session JSON I/O.
- `Models/MediaManifest.swift`, `Editor/ViewModel/EditorViewModel+Cost.swift` (GenerationLog),
  `Models/Timeline.swift`, `Models/MediaResolver.swift`, `Models/MediaAsset.swift` (manifest entry <-> asset).

## Core behaviors & algorithms (concrete — downstream implements from this)
### Bundle layout (on disk, directory-as-document)
```
<Name>.palmier/
  project.json          # required — Timeline (JSONEncoder default, NO pretty/sortedKeys/iso8601)
  media.json            # optional — MediaManifest
  generation-log.json   # optional — GenerationLog (append-only)
  thumbnail.jpg         # optional — JPEG, video<=320x180 / image<=640px, quality 0.7
  media/                # internalized media; manifest entries store path "media/<file>"
    <file>.<ext>
  chat/                 # one JSON per non-empty session, "<session-uuid>.json"
    <uuid>.json
```
### Read (`VideoProject.read(from:ofType:)` :31)
1. `fileWrapper.fileWrappers?["project.json"]?.regularFileContents` MUST exist → else
   `CocoaError(.fileReadCorruptFile)`. Decode `Timeline` (JSONDecoder default). Retain the whole
   `FileWrapper` as `packageWrapper` (so children not touched on save survive).
2. If `media.json` present → decode `MediaManifest`; decode FAILURE here also throws `.fileReadCorruptFile`.
3. If `generation-log.json` present → `try?` decode (failure tolerated, log only).
4. Decoding runs off-main; values applied on main in `makeWindowControllers` (:177). Chat sessions are
   loaded SEPARATELY from the live `fileURL` directory, not from the wrapper (see ChatSessionStore).
### Save / fileWrapper (`save` :57, `fileWrapper(ofType:)` :66, `captureSaveSnapshot` :90)
- `save(to:…)` records `fileModificationDate` from the URL, then `captureSaveSnapshot()` on main,
  then `super.save`. Snapshot encodes (default JSONEncoder): `timeline`→snapshotTimeline,
  `mediaManifest`→snapshotManifest, `generationLog`→snapshotGenerationLog, thumbnail bytes, and chat
  session files = `agentService.sessions.filter{!messages.isEmpty}` each via
  `ChatSessionStore.encodeSession` named `"<uuid>.json"`.
- `fileWrapper(ofType:)` rebuilds children on `packageWrapper`: replaceChild for `project.json`
  (required; missing snapshot → `.fileWriteUnknown`), then manifest/log/thumbnail if present, then a
  freshly built `chat/` directory wrapper, then — only if a live `media/` dir already exists on disk —
  an `.immediate` FileWrapper snapshot of `media/` (so newly imported media gets captured into the
  package). `replaceChild(name,data)` removes the old child of that name and adds the new one.
- Off-main guard: if `fileWrapper()` runs off-main and snapshot wasn't prepared → `.fileWriteUnknown`.
- Atomicity: NSDocument writes the package to a temp dir and swaps it in (safe-save). The port must
  replicate write-to-temp-then-atomic-rename for the whole directory.
### Create (`AppState.createNewProject` :123)
NSSavePanel (allowedContentTypes = the `.palmier` UTType package) defaulting into `storageDirectory` →
make `VideoProject()`, set `fileURL` + `fileType`, `makeWindowControllers`, `showWindows`, add to
NSDocumentController, then `save(... .saveOperation ...)`; on completion `ProjectRegistry.register(url)`.
### Open (`AppState.openProject` :143 / `openProjectFromPanel` :172)
`VideoProject(contentsOf:ofType:)` → makeWindowControllers → showWindows → addDocument → register.
Open panel: `canChooseDirectories=false`, `treatsFilePackagesAsDirectories=false` (package = one file).
### Autosave
`autosavesInPlace=true`. `AppState.showHome()` (:44): if `isDocumentEdited`, calls
`project.autosave(withImplicitCancellability:false)` BEFORE hiding the window and registering. So
switching away from a project force-flushes it. `updateChangeCount` mirrors `isDocumentEdited` onto
the view model; `agentService.onSessionsChanged` calls `updateChangeCount(.changeDone)` so a chat
edit dirties the document and will be autosaved.
### Registry (`ProjectRegistry`)
- File: `storageDirectory/project-registry.json` = JSON array of `ProjectEntry`.
- Async load on init via `ProjectRegistryDisk` actor; mutations arriving mid-load are queued in
  `pendingMutations` and replayed after `finishLoading`. Every mutation writes the whole array
  atomically (`Data.write(options:.atomic)`).
- `register(url)`: standardize URL; if present, bump `lastOpenedDate=now`; else append new entry
  (new UUID, createdDate=now, lastOpenedDate=now). `remove` deletes the entry only. `delete` trashes
  the bundle on disk (`FileManager.trashItem`) then removes the entry. `updateURL(old,new)` (called
  from `VideoProject.fileURL.didSet` on rename/Save-As) rewrites url + bumps lastOpened.
- `sortedEntries` = by `lastOpenedDate` desc. `ProjectEntry.name` = url last path component minus ext;
  `isAccessible` = file exists.
### Sample materialization (`SampleProjectService`)
- List: `GET {convexHttpURL}/v1/samples` → `[Summary{slug,title,posterUrl?}]`.
- Resolve: `GET /v1/samples/resolve?slug=<slug>` → JSON `{title, project, manifest, generationLog?,
  posterUrl?, downloads:[{id,relativePath,url}], chat:[{name,url}]}`.
- Build bundle at `cacheRoot/<safeSlug>/<safeTitle>.palmier` where
  `cacheRoot = ApplicationSupport/PalmierPro/Samples` (fallback temp dir). `safeName` strips `/ : \`.
  Clear stale slug dir first (`removeItem`). Create `media/`. Write `project`→`project.json`,
  `manifest`→`media.json`, optional `generationLog`→`generation-log.json`, optional poster→`thumbnail.jpg`.
- Downloads: media entries use server `relativePath` AS-IS (already `media/<file>`); chat entries get
  `relativePath = "chat/<name>"`. All downloaded CONCURRENTLY via task group; progress = completed/total
  reported on main (`onProgress(0..1)`). Any failure → remove whole slug dir, rethrow. `downloadFile`
  = URLSession download to temp, mkdir parent, remove existing, move into place.
- `cachedURL(slug)` returns first `*.palmier` in the slug dir (skip re-download). Samples are NOT
  registered in the project registry (`openSample` calls `openProject(register:false)`).
### Media path resolution (`MediaResolver` / `MediaAsset.toManifestEntry`)
- `MediaSource` enum: `.external(absolutePath)` | `.project(relativePath)`. Swift's derived Codable
  emits `{"external":{"absolutePath":"…"}}` / `{"project":{"relativePath":"…"}}` — the port MUST
  match this externally-tagged shape for round-trip.
- `expectedURL`: external → `URL(fileURLWithPath:absolutePath)`; project → `projectURL +
  relativePath` (relativePath already contains the `media/` segment).
- `toManifestEntry`: if asset url is under `projectURL.path` → `.project(relativePath = path after
  "projectURL/")`; else `.external(absolutePath)`. This is the internalize-on-save heuristic.
- On open, `restoreAssetsFromManifest` (:289) rebuilds `MediaAsset`s, logs+skips missing files, and
  triggers waveform/thumbnail/metadata regeneration. `generation-log.json` absence →
  `seedGenerationLogFromAssets()` derives the log from each asset's `generationInput`.

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `NSDocument` (package document, autosave, change tracking, safe-save) -> hand-rolled
  `palmier-project` document type: a struct owning the bundle path + dirty flag + an autosave debounce.
- `FileWrapper(directoryWithFileWrappers:)` / `.immediate` snapshot / `addFileWrapper` / `removeFileWrapper`
  -> direct `std::fs` directory writes; write children individually, no wrapper tree.
- macOS package bit (UTType conforming to `.package`, Finder shows dir as one file; NSOpen/SavePanel
  `treatsFilePackagesAsDirectories=false`) -> Windows has NO native package concept: implement a
  Tauri custom file dialog + (optional) Explorer shell extension so a `.palmier` dir presents as one
  document; Linux behaves naturally as a directory. (FOUNDATION §5.7 "Critical".)
- `FileManager.trashItem` -> Recycle Bin (`trash` crate / SHFileOperation) on Windows; XDG trash on Linux.
- `URLSession.download/data` -> `reqwest`. `withThrowingTaskGroup` -> `tokio` `JoinSet` / `futures`.
- `JSONEncoder`/`JSONDecoder` -> `serde_json`. NOTE encoder configs differ per type (below).
- `AVAssetImageGenerator` / `NSBitmapImageRep` JPEG (thumbnail) -> FFmpeg seek+scale + JPEG encode
  (covered by `palmier-media`); `palmier-project` only orchestrates and stores `thumbnail.jpg`.
- `FileManager.homeDirectoryForCurrentUser/Documents` & `applicationSupportDirectory` -> `dirs`/`directories`
  crate: registry+default storage and samples cache (FOUNDATION puts registry at
  `%APPDATA%\PalmierProWin\registry.json`, samples at `%APPDATA%\PalmierProWin\Samples\<slug>\`).
- `URL.standardizedFileURL` (registry dedup key) -> canonicalize/normalize path (`dunce`/`std::fs::canonicalize`
  with care: canonicalize fails for non-existent paths; use a lexical normalizer for dedup).

## Mapping to FOUNDATION crates (palmier-project, palmier-model)
- `palmier-model`: `Timeline`, `Track`, `MediaManifest{version,entries,folders}`, `MediaManifestEntry`,
  `MediaSource`, `GenerationLog{version,entries}`, `GenerationLogEntry`, `ChatSession`. These are the
  serde shapes; round-trip + default-fallback decode tests live here (FOUNDATION §testing).
- `palmier-project`: bundle reader/writer (the `VideoProject` read/fileWrapper logic), `ProjectRegistry`
  + `ProjectEntry`, autosave/dirty-tracking, `SampleProjectService`, `MediaResolver` path logic.
- FOUNDATION §6.1 registry methods (register/remove/delete/update_url/sorted_entries) map 1:1 to
  `ProjectRegistry`. §6.1 sample flow maps to `SampleProjectService`. §5.7 = bundle layout.

## Port risks & gotchas
- **Filename divergence** (top of doc): reference `project.json`/`media.json`/`generation-log.json`/`chat/`
  vs FOUNDATION `timeline.json`/`manifest.json`/`generation_log.json`/`chatsessions/`. Decide once;
  the sample server emits the REFERENCE names, so deviating breaks sample import unless you remap.
- **Encoder config is type-specific.** `ChatSessionStore` uses pretty + sortedKeys + iso8601 dates;
  Timeline/Manifest/GenerationLog use the DEFAULT JSONEncoder (compact, NUMERIC reference-date for
  `Date` = seconds since 2001-01-01, NOT iso8601). Manifest/log carry `Date` fields
  (`createdAt`, `cachedRemoteURLExpiresAt`) → those serialize as Apple reference-epoch doubles in
  project.json/media.json but as iso8601 in chat. A naive single serde Date format will corrupt
  round-trips. Match per-file.
- **Lenient decode is load-bearing.** `MediaManifest`/`GenerationLogEntry`/`ChatSession` have custom
  `init(from:)` with `decodeIfPresent` + version/legacy fallbacks (e.g. legacy `cost` dollars →
  `costCredits = ceil(dollars*100)`; manifest version default 1; session `isOpen` default true). Port
  with serde `#[serde(default)]` + a custom legacy path, or old bundles fail to open.
- **`project.json` missing = corrupt** (hard error). Manifest decode failure = hard error. Generation
  log failure = soft (ignored). Preserve these severities exactly.
- **Whole-directory atomic save.** NSDocument swaps a temp package dir atomically; a partial-write port
  can leave a half-saved bundle. Write to a sibling temp dir, fsync, atomic-rename the directory.
- **media/ is only re-snapshotted if it already exists on disk** at save time (`mediaDirWrapper`
  returns nil otherwise). Importing media must create the `media/` dir under the live bundle path
  before save, or it won't be persisted.
- **chat/ is rebuilt from in-memory sessions each save** (only non-empty), and on load merges the
  in-wrapper-independent live-dir read with a fresh blank session inserted at index 0 (`loadSessions`).
  Empty sessions are dropped on save → never persisted.
- **Registry race**: mutations during async load are queued; if you make the registry synchronous in
  Rust you can drop this complexity, but preserve atomic full-array writes and standardized-URL dedup.
- **Save-As rename** drives `fileURL.didSet → ProjectRegistry.updateURL`. Replicate: any path change
  must update the registry entry, not orphan it.
- **Samples are not registry-tracked**; don't auto-register materialized samples.

## Open questions
- Exact `Date` wire format the sample-resolve server emits for manifest/log — Apple reference-epoch
  double (matching default JSONEncoder) or iso8601? Must confirm against a live `/v1/samples/resolve`
  payload before locking serde Date handling for `media.json`/`generation-log.json`.
- Does any reference path migrate OLD bundles using `timeline.json`-style names? (None seen — names look
  stable as `project.json` etc. — but FOUNDATION's different names suggest a possible intended rename.)
- Windows "directory-as-single-document" UX: is a shell extension required for v1, or is a Tauri-only
  custom dialog acceptable (FOUNDATION says both; scope decision needed).
- Thumbnail size contract: code caps video thumb at 320x180 and image at 640px; FOUNDATION §5.7 says
  "<=320x180". Reconcile (image path exceeds it).
