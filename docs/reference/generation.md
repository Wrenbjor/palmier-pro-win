---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# generation — reference port notes

## Purpose
The AI generation lifecycle: fetch the live model catalog from Convex, validate a generation
request per-model, create placeholder `MediaAsset`s, upload reference files, submit a Convex job,
subscribe to that job reactively until it settles, download results, and gate the UI on credit
budget (`can_generate`). Maps to the `palmier-gen` crate ("Convex client, generation lifecycle, queue").

## Key types & files (cite paths under Sources/PalmierPro/Generation/...)
- `GenerationService.swift` — orchestrator. `generate(...)` (the one big entry point, ~120 lines) +
  `runJob`, `downloadAndFinalize`, `uploadReferences`, `createPlaceholder`, `retryDownload`.
- `GenerationBackend.swift` — RPC layer (Convex). `subscribe`, `uploadReference`, `submit`. Defines
  `BackendGenerationParams` (enum: video/image/audio/upscale), `BackendGenerationStatus`
  (`queued|running|succeeded|failed`), `BackendGenerationJob`, `GenerationBackendError`.
- `Catalog/ModelCatalog.swift` — `ModelCatalog` (`@Observable` singleton) live-subscribes to model
  list; `CatalogEntry` decoder + `VideoCaps/ImageCaps/AudioCaps/UpscaleCaps`; `ModelRegistry.byId`.
- `Catalog/CostEstimator.swift` — pure credit math (video/image/audio/upscale + rerun `cost(for:)`).
- `Catalog/{Video,Image,Audio,Upscale}ModelConfig.swift` — per-kind config wrappers, `validate(...)`,
  and the `*GenerationParams` Encodable structs (the JSON sent to Convex).
- `Catalog/ModelPreferences.swift` — user-disabled model ids, persisted in UserDefaults key
  `disabledModelIds`.
- `Submission/{Video,Image,Audio,Music}GenerationSubmission.swift` — assemble a `generate(...)` call
  shared by UI and agent; `VideoGenerationSubmission.InputAssets.validate(for:)` does reference-count
  validation.
- `UI/GenerationView.swift` — form, live `estimatedCost`, `canAffordGeneration`, `canSubmit`, submit button gating.
- Cross-refs: `Account/AccountService.swift` (credits/budget, Convex client owner),
  `Editor/ViewModel/EditorViewModel+Cost.swift` (`generation-log.json` append-only ledger),
  `Models/MediaAsset.swift` (`GenerationStatus` enum, `freshRemoteURL` upload cache).

## Core behaviors & algorithms (concrete — downstream story/dev agents implement from this)
**Catalog fetch.** `ModelCatalog.configure()` (idempotent via `didConfigure`) subscribes to Convex
query `models:list` yielding `[CatalogEntry]`. On each push `apply()` rebuilds `video/image/audio/upscale`
arrays + `byId: [String: ModelKind]`, sets `isLoaded=true`. `CatalogEntry` has a custom decoder:
`kind` (`video|image|audio|upscale`) drives which `*Caps` struct decodes from `uiCapabilities`.
Pricing fields: `creditsPerSecond`, `audioDiscountRate`, `creditsPerImage` (all `[String:Double]`,
keyed by resolution or `"res|quality"` or quality, with `""` as default key), `audioPricing`
(tagged enum `perThousandChars|perSecond|flat`), `creditsPerSecondUpscale`.

**Per-call validation (pre-submit).** `*ModelConfig.validate(...)` checks duration/aspectRatio/resolution
against caps, returning a human-readable error string or nil (e.g. `unsupportedValue(...)`). Reference
validation lives in `VideoGenerationSubmission.InputAssets`: type checks, per-type max counts
(`maxReferenceImages/Videos/Audios`, `maxTotalReferences`), combined duration caps
(`maxCombinedVideoRefSeconds/AudioRefSeconds`), frames-vs-references exclusivity, first/last-frame
support. `GenerationView.modelValidationError` runs these and blocks submit.

**Cost gating (`can_generate`).** No single function; computed in `GenerationView`:
- `remainingCredits = max(0, AccountService.budgetCredits - spentCredits)` where
  `budgetCredits = plan.monthlyBudgetCredits + user.purchasedCredits`, `spentCredits = user.spentCreditsThisPeriod`.
- `estimatedCost` = `CostEstimator.<kind>Cost(...)` for current form.
- `canAffordGeneration`: if budget unknown → true; else `estimatedCost <= remaining` (or `remaining > 0` if cost unknown).
- `canSubmit` = `canAffordGeneration && model-specific required-inputs present`. Submit button is
  `.disabled(!canSubmit)` (when AI allowed). `hasInsufficientCredits` colors the cost label red.
- Gating is **advisory/client-side**; the Convex `generations:submit` mutation is the real enforcer
  (returns `jobId`, presumably rejects on insufficient credit — backend not in this repo).

**CostEstimator math.** `ceilCredits(x) = x<=0 ? 0 : Int(x.rounded(.up))`. video:
`ceil(rate * duration)`, rate from `creditsPerSecond[resolution] ?? [""]`, `* audioDiscount` when
`!generateAudio`. image: 2D `["res|quality"]` lookup → quality-only → resolution lookup, `* numImages`.
audio: perThousandChars `rate*chars/1000` / perSecond `rate*secs` / flat. upscale: `rate*max(1,secs)`.
`cost(for: GenerationInput)` re-derives via `ModelRegistry.byId` for reruns + the persisted log.

**The `generate(...)` pipeline** (`GenerationService`, `@MainActor`):
1. `count = max(1, min(4, numImages))`; create `count` placeholders via `createPlaceholder`:
   new UUID, dest `…/gen-<id8>.<ext>` under `Project.mediaDirectoryName` (or temp dir if no project),
   `generationStatus = .generating`, appended to `editor.mediaAssets`. Returns `placeholders[0].id` synchronously.
2. In a detached `Task @MainActor`: resolve reference URLs — apply `trimmedSourceOverride`
   (`VideoTrimExtractor.extract`), `preprocessRef` (e.g. `VideoCompressor.compressIfNeeded` for video refs,
   run concurrently in a throwing task group), then `uploadReferences`. Temp files tracked + cleaned in `defer`.
3. `uploadReferences`: concurrent task group; per ref, reuse `asset.freshRemoteURL` cache if present,
   else `GenerationBackend.uploadReference`; results sorted back into input order.
   Cache recorded only for pristine (non-trimmed, non-preprocessed) assets, TTL `6*24*60*60` (6 days).
4. Build `finalGenInput` (`snapshotRefs` or set `imageURLs`), stamp `createdAt = Date()`, attach to placeholders.
5. `buildParams(uploaded)` → `BackendGenerationParams`; call `runJob`.

**`uploadReference` (3-step Convex upload).** (a) `convex.mutation("uploads:generateUploadTicket")` →
`{uploadUrl}`. (b) HTTP `POST` bytes to that URL with `Content-Type` (mapped from extension/ClipType)
via `URLSession.upload(for:fromFile:)`; assert 2xx, decode `{storageId}`. (c)
`convex.action("uploads:commitUpload", {storageId})` → `{url}` (the hosted URL).

**`runJob`.** `GenerationBackend.submit(model, params, projectId)` → `convex.mutation("generations:submit")`
→ `{jobId}`. Then `subscribe(jobId)` → `convex.subscribe("generations:byId", {id: jobId})` yielding
`BackendGenerationJob?`. The Combine publisher is bridged to an `AsyncStream` (`.receive(on: main)`,
`sink`; `continuation.onTermination` cancels the subscription). Loop over the stream:
`succeeded` → `finalizeSuccess`; `failed` → set all placeholders `.failed(errorMessage ?? "Generation failed")`,
call `onFailure`; `queued|running` → continue. Submit failure / nil publisher ("Backend not configured")
also fails all placeholders.

**`finalizeSuccess` + `downloadAndFinalize`.** `job.resultUrls` (array) mapped 1:1 to placeholders by
index; missing/extra → that placeholder `.failed`. Per asset: `generationStatus = .downloading`,
`URLSession.download(from:)`, fix extension from the remote path (if a known `ClipType`),
remove old file, `moveItem` into place, `generationStatus = .none`, `importMediaAsset(skipAppend:true)`,
`appendGenerationLog`, `finalizeImportedAsset`. Download failure stores `pendingDownloadURL` +
`.failed(msg)` (retryable via `retryDownload`). First success fires `AppNotifications.generationComplete`.

**MediaAsset status machine** (`Models/MediaAsset.swift`): `none | generating | downloading | rendering | failed(String)`.
`generationInput != nil` ⇒ `isGenerated`. Upload cache fields `cachedRemoteURL` + `cachedRemoteURLExpiresAt`;
`freshRemoteURL` returns the URL only if unexpired.

**Generation log.** `EditorViewModel+Cost.swift`: append-only `generation-log.json` (`GenerationLog`,
version 1). Each `GenerationLogEntry{model, costCredits, createdAt}`; `costCredits` via `CostEstimator.cost`.
Legacy migration: old `cost` (dollars) → `Int(dollars*100, ceil)`. `seedGenerationLogFromAssets` backfills.

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `ConvexMobile` (`convex.mutation/action/subscribe`, `ConvexEncodable`, `ClientError`) → a Rust Convex
  client. No official Rust SDK; implement WebSocket sync protocol or use HTTP (`/api/mutation`,
  `/api/action`, `/api/query_ts` + subscription via WS). Reactive subscribe → `tokio` stream / channel.
- `Combine` `AnyPublisher`/`AnyCancellable` + `AsyncStream` bridge → `tokio::sync::watch`/`broadcast` or
  `futures::Stream`; the publisher→AsyncStream bridge becomes a single async stream consumed with `while let`.
- `URLSession.shared.upload(for:fromFile:)` / `.download(from:)` → `reqwest` (multipart/streamed body for
  POST upload; streaming download to temp file). Manual 2xx assertion + JSON error envelope decode.
- `FileManager` (createDirectory, moveItem, removeItem, temporaryDirectory) → `std::fs` / `tempfile`.
- `UUID().uuidString` → `uuid` crate. `Date()`/`addingTimeInterval` → `chrono`/`std::time`. `os.Logger`
  (`Log.generation`) → `tracing`. `@Observable`/`@MainActor` → Tauri state (Mutex/RwLock) + event emit.
- `UserDefaults` (`disabledModelIds`) → persisted settings store (e.g. JSON in app config dir).
- `AppNotifications.generationComplete` → native notification (`notify-rust` / Tauri notification plugin).
- Clerk auth (`ClerkConvexAuthProvider`) → see `palmier-auth`; Convex client must inject the auth token.

## Mapping to FOUNDATION crates (palmier-gen)
FOUNDATION assigns `palmier-gen` = "Convex client, generation lifecycle, queue". Everything here lands
there: catalog model, cost estimator, validation, submission assembly, the upload/submit/subscribe/download
lifecycle, status enum. Boundaries: credit/account state (`AccountService`) → likely `palmier-auth` +
account state (gate logic re-expressed in the gen layer). MediaAsset model lives in `palmier-media`/`palmier-model`.
UI gating (`GenerationView`) → `palmier-tauri` frontend; backend exposes `estimate_cost`/`can_generate` commands.

## Port risks & gotchas
- **No true job cancellation.** Nothing cancels an in-flight Convex `generations:submit` job — only the
  client-side subscription is cancelled on stream termination. Closing/leaving leaves the job running on the
  backend (and likely still bills). If the port wants real cancel, add a Convex mutation — not present here.
- **Convex reactive subscriptions are the hard part.** No Rust SDK; the WS sync protocol (query
  subscriptions that re-push on data change) must be reimplemented faithfully or the live catalog/job/account
  updates break.
- **3-step upload contract is exact.** ticket mutation → raw POST with correct `Content-Type` → commit
  action. Content-Type map (jpg/png/webp/heic/gif, mp4/m4v/mov, mp3/wav/m4a) must match; backend may key on it.
- **Params JSON shape is a wire contract.** Each `*GenerationParams.encode` emits a `kind` discriminator
  ("video"/"image"/"audio"/"upscale") and omits empty reference arrays / nil fields. Field names
  (`startFrameURL`, `referenceImageURLs`, `generateAudio`, `numImages`, `durationSeconds`, etc.) must be byte-identical.
- **Index-based result mapping.** `resultUrls[i]` → `placeholders[i]`; fewer URLs than placeholders marks
  extras failed; preserve ordering through the upload task group (results are re-sorted by index — keep that).
- **Cost gating is advisory only.** Don't treat client `canAffordGeneration` as authoritative; the backend
  mutation is the gate. Float→credit rounding is `ceil`; replicate exactly to match displayed estimates.
- **Upload cache TTL = 6 days**, keyed on the MediaAsset, skipped for trimmed/preprocessed bytes. Persisted
  with the asset manifest (`cachedRemoteURL`/`...ExpiresAt`, dropped when stale on serialize).
- **numImages clamp** `[1,4]`; image cost/params clamp to `maxImages`.

## Open questions
- Exact Convex deployment URL / auth header format and the WS subscription framing (not in this repo;
  backend functions `models:list`, `generations:submit`, `generations:byId`, `uploads:*`, `account:get`,
  `billing:*` are referenced but their server impl/credit-enforcement is external).
- Does `generations:submit` reject on insufficient credits, and with what error code/shape
  (`BackendErrorEnvelope{error:{code,message}}`)? Determines server-side `can_generate` behavior.
- Is there a backend endpoint to cancel/abort a queued/running job? None used here.
- `BackendGenerationJob` exposes `costCredits`/`completedAt` but the client recomputes cost locally via
  `CostEstimator` for the log — should the port trust the server `costCredits` instead for accuracy?
- `rendering` status exists in the enum but is not set anywhere in Generation/ — confirm its source (export?).
