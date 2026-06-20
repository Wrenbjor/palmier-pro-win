---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
---

# Epic 9 — AI Generation

## Epic goal

Port the macOS Convex-proxied AI media-generation lifecycle to Windows/Linux in the
**`palmier-gen`** crate: fetch the live model catalog, validate a generation request per-model,
create placeholder `MediaAsset`s, upload reference files via Convex tickets, submit a generation
job, subscribe to that job reactively until it settles, download results into the project bundle,
fire a native completion toast, and gate the UI on credit budget (`can_generate`) — plus the
generation form UI (generation panel + the **Music tab = a music-generation form**, ruling #14).
We never hold provider API keys; everything flows through Convex (the real credit gate is the
server mutation; the client gate is **advisory only**, ruling #24). Cancellation is **client
subscription teardown only**; the Convex job keeps running and billing (ruling #24).

**PRD acceptance this epic must satisfy (PRD §4.9 FR-33..FR-35, §10 Epic 9):**

- **FR-33 Generation lifecycle.** Fetch `/v1/models` catalog (24 h cache), validate per-model,
  create placeholder `MediaAsset`(s) (status Generating), optionally upload references via Convex
  tickets, submit `generations:submit`, subscribe `generations:by_id`, on success download to
  `<project>/media/<id>.{ext}` (auto-correct extension from the remote path), notify. *Consequence:*
  status transitions **Generating → Downloading → None | Failed** are reflected in the UI; a native
  toast fires on completion.
- **FR-34 Credit gating.** `can_generate = signed_in && tier_allows && has_remaining_credits` is
  **advisory**; the real gate is the Convex mutation (ruling #24). When false, the generation UI is
  blocked with "Sign in" / "Out of credits".
- **FR-35 Cancellation (v1).** Cancel tears down the client subscription only; the Convex job keeps
  running/billing (ruling #24). A server cancel mutation is **deferred** (backend out of repo).
- **§10 cross-cutting:** Catalog fetch + per-model validation; placeholders → submit → subscribe →
  download; native toast on completion; Convex **HTTP + WebSocket** path proven by **Spike S-2**
  (R-4). The generation tools (`generate_video`, `can_generate`) are **surfaced in M2 (Epic 7)** but
  return "backend not available" / advisory-false **until this epic wires Convex in M3** (ruling #24;
  S-2 gating) — so UJ-3 is end-to-end testable only at M3.

**Milestone (PRD §12):** **M3 — Generation + Transcription** (Epics 9–10). Spike **S-2 first**
(Convex HTTP + WebSocket; S-1b already landed in M1). Gated by **OQ-9** (does the existing Convex
deployment accept the Windows client with a Clerk JWT?). Realizes **UJ-3** (generation in-flow).
**Demonstrable exit (SM-11):** on a `generate_video` request a placeholder `MediaAsset` (status
Generating) appears **< 2 s**; transitions Generating → Downloading → None|Failed are observed in the
UI; a native completion toast fires; the asset lands in `<project>/media/<id>.{ext}` — verified by
the **§11.3 "generative augment" e2e** over the **§11.2 mock-Convex generation-lifecycle
integration**.

**Governing reference:** `docs/reference/generation.md`; FOUNDATION §6.11 / §8.1. Reference source
under `Sources/PalmierPro/Generation/…` (verified present): `GenerationService.swift`,
`GenerationBackend.swift`, `VideoCompressor.swift`; `Catalog/{ModelCatalog,CostEstimator,
Video/Image/Audio/UpscaleModelConfig,ModelPreferences}.swift`;
`Submission/{Video,Image,Audio,Music}GenerationSubmission.swift`; `UI/GenerationView.swift`.

---

## Spike / gating note (READ FIRST)

Epic 9 is **NOT gated by Spike S-1** (wgpu→WebView — that gates Epic 5 only). Epic 9 is gated by
**Spike S-2 — Convex Rust HTTP + WebSocket client** (PRD §11, R-4), which **must land before any
story that talks to a live Convex deployment**. S-1b (Date encoding) already landed in M1 (Epic 2).

**Consequence for sequencing (binding):**

- **E9-S1 is the S-2 verification slice.** It is the first story and proves the two Convex
  transports (`/v1/*` HTTP query/mutation/action **and** a `generations:by_id` WebSocket live-query
  round-trip) against the target deployment (or a captured/mock fixture if OQ-9 access is blocked).
  **No live-Convex story below commits its transport choice until E9-S1 passes** (PRD §11 S-2 exit:
  a passing integration test that issues `/v1/models` over HTTP **and** completes a
  `generations:by_id` WS round-trip).
- **Pure, Convex-free stories build in parallel immediately** (catalog decode, cost math,
  validation, the status machine, submission assembly, params JSON) — they have **no S-2 dependency**
  and are unit-tested against fixtures. Only the lifecycle-orchestration, upload, and live-subscribe
  stories sit behind E9-S1.
- **OQ-9 fallback:** if the existing deployment rejects the Windows client, E9-S1 records the
  decision and the lifecycle stories run against the **§11.2 mock-Convex integration harness**; the
  SM-11 e2e is explicitly defined over that mock. The epic does **not** stall on OQ-9.

**Hard cross-epic dependencies (do NOT re-implement here):**

- **Epic 2 (`palmier-model`, `palmier-project`):** `MediaAsset`, `Project.media/` layout, the serde
  Date codecs (Apple reference-epoch doubles), and the `GenerationInput` shape. The **MediaAsset
  status machine and upload-cache fields** are added to the Epic 2 model — Epic 9 consumes/extends,
  it does not redefine the asset.
- **Epic 7 (`palmier-tools`):** the `generate_video` / `can_generate` (and sibling generation) tool
  *shells* are part of the 30-tool surface and land in M2 returning "backend not available". Epic 9
  **wires the real backing** into those existing tool implementations — exactly one impl per tool
  name, no duplication.
- **Epic 1 / Epic 12 (`palmier-auth`, `AccountService`):** Clerk sign-in, the Convex client owner,
  the Clerk JWT injection, and credit/budget state (`budgetCredits`, `spentCreditsThisPeriod`). Epic
  9 **reads** account/budget state to compute the advisory gate; it does not own auth.
- **Epic 4 (`src-ui/media-panel`):** the Media panel and the **Music tab placeholder** (ruling #14,
  wired-in-Epic-9). Epic 9 fills the Music tab and the generation panel forms.

---

## Stories

### E9-S1 — Convex client transport (S-2 spike slice): HTTP + WebSocket live-query

As the generation backend, I want a proven Rust Convex client that does HTTP query/mutation/action
and a reactive WebSocket subscription, so every lifecycle story has one transport to build on and the
M3 backend dependency is de-risked first.

**Acceptance criteria:**
- Implement a `ConvexClient` exposing `query(name, args)`, `mutation(name, args)`, `action(name,
  args)` over HTTP (`reqwest`) and `subscribe(name, args) -> impl Stream<Item = Option<T>>` over
  WebSocket (`tokio-tungstenite`), replacing macOS `ConvexMobile` (`generation.md` §"macOS/Apple
  APIs to replace": `convex.mutation/action/subscribe`). Decide `convex-rs` vs raw
  `reqwest`+`tokio-tungstenite` and record the choice (PRD §11 S-2).
- The subscription faithfully reimplements the **reactive query re-push on data change** (the
  Combine `AnyPublisher`→`AsyncStream` bridge becomes one `futures::Stream` consumed with
  `while let`; `onTermination`→`Drop`/cancel cancels the WS subscription). This is the
  "Convex reactive subscriptions are the hard part" risk in `generation.md` §"Port risks".
- The Clerk **JWT auth token is injected** on every request/subscription (auth owner = `palmier-auth`
  / `AccountService`; `generation.md` §"macOS/Apple APIs": "Convex client must inject the auth
  token").
- Error envelope decode: a non-2xx / error response decodes the backend
  `BackendErrorEnvelope{error:{code,message}}` into a typed `GenerationBackendError`
  (`GenerationBackend.swift` `GenerationBackendError`).
- **S-2 exit (PRD §11, binding):** a passing integration test that (a) issues `/v1/models` (the
  `models:list` query) over HTTP **and** (b) completes a `generations:by_id` WebSocket round-trip
  against the target deployment. If **OQ-9** blocks access, the test runs against a **captured
  fixture / mock-Convex harness** and the access decision is recorded.

**Implementation context:** crate **`palmier-gen`** (new `convex` module) + **`palmier-auth`**
(JWT). Replaces `GenerationBackend.swift`'s `ConvexMobile` layer. Reference:
`Sources/PalmierPro/Generation/GenerationBackend.swift`; `generation.md` §"Key types & files"
(GenerationBackend) + §"macOS/Apple APIs to replace" + §"Port risks" (reactive subscriptions; auth
token). Convex endpoints referenced: `models:list`, `generations:submit`, `generations:by_id`,
`uploads:generateUploadTicket`, `uploads:commitUpload`, `account:get`.

**Dependencies:** **Spike S-2** (this story *is* the S-2 slice); `palmier-auth` Clerk JWT (Epic 1).

**Parallel-safe?** No — it is the gating story; every live-Convex story (S5, S6, S7, S8) depends on
it. Owns the `convex` module exclusively, so it does not collide with the pure stories (S2–S4, S9).

---

### E9-S2 — Model catalog: `CatalogEntry` decode + `ModelCatalog` live subscription

As the generation layer, I want the typed model catalog and a live subscription that rebuilds it on
each push, so validation, cost, and the form have an accurate model registry.

**Acceptance criteria:**
- `CatalogEntry` **custom decoder** keyed on `kind` (`video|image|audio|upscale`) selecting which
  `*Caps` struct decodes from `uiCapabilities` (`VideoCaps`/`ImageCaps`/`AudioCaps`/`UpscaleCaps`)
  (`generation.md` §"Catalog fetch"; `Catalog/ModelCatalog.swift`).
- Pricing fields decode exactly: `creditsPerSecond`, `audioDiscountRate`, `creditsPerImage` (all
  `[String:Double]`, keyed by resolution / `"res|quality"` / quality, with **`""` as the default
  key**), `audioPricing` (tagged enum `perThousandChars|perSecond|flat`), `creditsPerSecondUpscale`
  (`generation.md` §"Catalog fetch").
- `ModelCatalog.configure()` is **idempotent** (`didConfigure` guard); subscribes to `models:list`;
  on each push `apply()` rebuilds `video/image/audio/upscale` arrays + `byId: [String: ModelKind]`,
  sets `isLoaded = true` (`generation.md` §"Catalog fetch"). Catalog is **24 h cached** and **must
  not block** (mirrors FR-1's non-blocking catalog rule); offline/slow Convex degrades to the
  cached/empty catalog.
- `ModelRegistry.byId` resolves a model id to its `ModelKind` for reruns and the persisted log.
- **Unit tests:** decode each of the four `kind`s from a fixture catalog payload; assert the `""`
  default-key fallback in each pricing map; assert `apply()` partitions models into the four arrays
  and populates `byId`.

**Implementation context:** crate **`palmier-gen`** (`catalog` module). Reference:
`Sources/PalmierPro/Generation/Catalog/ModelCatalog.swift`; `generation.md` §"Catalog fetch". The
live subscription uses the E9-S1 `ConvexClient.subscribe`; the **decode/partition logic is pure** and
testable against a fixture independent of S-1.

**Dependencies:** E9-S1 (only for the *live* subscription wiring; the decoder + `apply()` are pure
and build in parallel against a fixture).

**Parallel-safe?** Yes for the pure decode/`apply()` portion (own `catalog` module). The live-wire
seam waits on E9-S1.

---

### E9-S3 — CostEstimator: pure credit math (video/image/audio/upscale + rerun `cost(for:)`)

As the generation form and the generation log, I want exact credit-cost math, so displayed estimates
and the persisted ledger match the reference to the credit.

**Acceptance criteria:**
- `ceilCredits(x) = x <= 0 ? 0 : Int(x.rounded(.up))` — credit rounding is **ceil** (carry-forward:
  float→credit rounding is `ceil`, replicate exactly; `generation.md` §"CostEstimator math" /
  §"Port risks").
- **video:** `ceil(rate * duration)`, `rate = creditsPerSecond[resolution] ?? creditsPerSecond[""]`,
  multiplied by `audioDiscountRate` when `!generateAudio`.
- **image:** 2D lookup `creditsPerImage["res|quality"]` → quality-only key → resolution key,
  `* numImages` (`generation.md` §"CostEstimator math").
- **audio:** `perThousandChars` → `rate*chars/1000`; `perSecond` → `rate*secs`; `flat` → flat amount.
- **upscale:** `creditsPerSecondUpscale * max(1, secs)`.
- `cost(for: GenerationInput)` re-derives cost via `ModelRegistry.byId` for **reruns** and the
  persisted log (`generation.md` §"CostEstimator math").
- **Unit tests:** one per kind with a fixture catalog asserting exact credit integers, including the
  default-key fallback path, the `!generateAudio` discount, and the `max(1, secs)` upscale floor.

**Implementation context:** crate **`palmier-gen`** (`cost` module). Reference:
`Sources/PalmierPro/Generation/Catalog/CostEstimator.swift`; `generation.md` §"CostEstimator math".
**Pure function — zero Convex dependency.**

**Dependencies:** E9-S2 (consumes `CatalogEntry`/`ModelRegistry` types). No S-1 dependency.

**Parallel-safe?** Yes (own `cost` module, pure).

---

### E9-S4 — Per-model & reference validation (`*ModelConfig.validate`, `InputAssets.validate`)

As the pre-submit gate, I want per-model parameter and reference-count validation, so an invalid
request is rejected before it ever reaches Convex.

**Acceptance criteria:**
- `*ModelConfig.validate(...)` checks duration / aspectRatio / resolution against the model's caps,
  returning a **human-readable error string or nil** (e.g. `unsupportedValue(...)`) (`generation.md`
  §"Per-call validation"; `Catalog/{Video,Image,Audio,Upscale}ModelConfig.swift`).
- Reference validation in `VideoGenerationSubmission.InputAssets.validate(for:)`: per-type checks,
  per-type max counts (`maxReferenceImages` / `maxReferenceVideos` / `maxReferenceAudios`,
  `maxTotalReferences`), combined duration caps (`maxCombinedVideoRefSeconds` /
  `maxCombinedAudioRefSeconds`), **frames-vs-references exclusivity**, and first/last-frame support
  (`generation.md` §"Per-call validation"; `Submission/VideoGenerationSubmission.swift`).
- **numImages clamp `[1,4]`** and image params clamp to `maxImages` (`generation.md` §"Port risks":
  numImages clamp).
- The validators are the same ones `GenerationView.modelValidationError` runs to block submit
  (consumed by E9-S8 form gating).
- **Unit tests:** valid/invalid duration, resolution, aspect ratio per kind; over-count and
  over-combined-duration reference sets; frames+references mutual-exclusion; numImages clamp at 0 and
  5 → 1 and 4.

**Implementation context:** crate **`palmier-gen`** (`validate` module). Reference:
`Sources/PalmierPro/Generation/Catalog/*ModelConfig.swift` +
`Submission/VideoGenerationSubmission.swift` (`InputAssets`); `generation.md` §"Per-call validation".
**Pure — zero Convex dependency.**

**Dependencies:** E9-S2 (caps types). No S-1 dependency.

**Parallel-safe?** Yes (own `validate` module, pure).

---

### E9-S5 — Params JSON wire contract + submission assembly (`*GenerationParams`, `*GenerationSubmission`)

As the wire boundary, I want byte-faithful generation params and the shared submission assemblers, so
the JSON sent to Convex matches the reference exactly and UI + agent share one assembly path.

**Acceptance criteria:**
- Each `*GenerationParams` encode emits a **`kind` discriminator** (`"video"`/`"image"`/`"audio"`/
  `"upscale"`) and **omits empty reference arrays / nil fields** (`generation.md` §"Port risks":
  "Params JSON shape is a wire contract").
- Field names are **byte-identical** to the reference: `startFrameURL`, `referenceImageURLs`,
  `generateAudio`, `numImages`, `durationSeconds`, etc. (a renamed field is a silent backend break —
  `generation.md` §"Port risks").
- `buildParams(uploaded)` maps validated input + uploaded reference URLs → `BackendGenerationParams`
  (enum: video/image/audio/upscale; `GenerationBackend.swift`).
- `{Video,Image,Audio,Music}GenerationSubmission` assemble a `generate(...)` call **shared by UI and
  agent** (`generation.md` §"Key types & files": Submission). The **Music tab** is a
  **music-generation form** (ruling #14), assembled via `MusicGenerationSubmission` (video/text →
  music).
- **Golden serialization tests:** serialize each kind's params and diff **byte-exact** against a
  committed golden JSON fixture (mirrors the wire-contract treatment of XMEML/CaptionBuilder goldens);
  assert empty-array and nil-field omission; assert the `kind` discriminator string per kind.

**Implementation context:** crate **`palmier-gen`** (`params` + `submission` modules). Reference:
`Sources/PalmierPro/Generation/Catalog/*ModelConfig.swift` (the `*GenerationParams` Encodable
structs) + `Submission/{Video,Image,Audio,Music}GenerationSubmission.swift` +
`GenerationBackend.swift` (`BackendGenerationParams`); `generation.md` §"Port risks" (params wire
contract). Use `.sortedKeys`-style canonical JSON to keep byte determinism. **Pure encode — zero
Convex dependency** (the *transport* of these bytes is E9-S6/S7).

**Dependencies:** E9-S2 (model caps), E9-S4 (validated input). No S-1 dependency for the encode; the
submission's *transport* call is wired in E9-S7.

**Parallel-safe?** Yes for the encode/assembly (own `params`/`submission` modules).

---

### E9-S6 — 3-step Convex reference upload (ticket → POST bytes → commit) + upload cache

As the lifecycle, I want reference files uploaded via the exact 3-step Convex contract with the 6-day
cache, so reference-backed generations work and pristine assets are not re-uploaded.

**Acceptance criteria:**
- **Exact 3-step contract** (`generation.md` §"`uploadReference` (3-step Convex upload)"):
  (a) `mutation("uploads:generateUploadTicket")` → `{uploadUrl}`;
  (b) HTTP **POST** the bytes to that URL with the correct **`Content-Type`** (mapped from
  extension/ClipType), assert **2xx**, decode `{storageId}`;
  (c) `action("uploads:commitUpload", {storageId})` → `{url}` (the hosted URL).
- **Content-Type map must match the reference** (jpg/png/webp/heic/gif, mp4/m4v/mov, mp3/wav/m4a) —
  the backend may key on it (`generation.md` §"Port risks": "3-step upload contract is exact").
- `uploadReferences`: a **concurrent task group**; per ref, reuse `asset.freshRemoteURL` cache if
  present (unexpired), else upload; **results re-sorted back into input order** (index ordering is
  load-bearing — `generation.md` §"Port risks": index-based result mapping).
- **Upload cache TTL = `6*24*60*60` (6 days)**, recorded **only for pristine** (non-trimmed,
  non-preprocessed) assets; persisted on the asset as `cachedRemoteURL` / `cachedRemoteURLExpiresAt`,
  dropped when stale on serialize (`generation.md` §"Port risks": upload cache TTL; MediaAsset
  upload-cache fields).
- Reference preprocessing seam: `trimmedSourceOverride` (`VideoTrimExtractor.extract`) and
  `preprocessRef` (`VideoCompressor.compressIfNeeded` for video refs) run **concurrently in a
  throwing task group**; temp files tracked and cleaned in a `defer`/`Drop` (`generation.md`
  §"The `generate(...)` pipeline" step 2-3). Port `VideoCompressor.swift`'s `compressIfNeeded`.
- **Unit/integration tests:** Content-Type mapping table per extension; cache hit skips upload; cache
  is **not** written for trimmed/preprocessed bytes; non-2xx POST surfaces a typed error; ordering
  preserved through the concurrent group.

**Implementation context:** crate **`palmier-gen`** (`upload` module) — `reqwest`
streamed/multipart POST; `std::fs`/`tempfile` for temp files. Reference:
`Sources/PalmierPro/Generation/GenerationService.swift` (`uploadReferences`),
`GenerationBackend.swift` (`uploadReference`), `VideoCompressor.swift`; `generation.md`
§"`uploadReference`" + §"The `generate(...)` pipeline" steps 2-3. Uses E9-S1 `ConvexClient`
mutation/action + raw HTTP POST.

**Dependencies:** **E9-S1** (Convex client), E9-S5 (asset/input types). Extends the Epic-2
`MediaAsset` cache fields.

**Parallel-safe?** Partially — owns the `upload` module but **must land after E9-S1**; can develop in
parallel with S7/S8 against the S1 client.

---

### E9-S7 — The `generate(...)` pipeline: placeholders, submit, subscribe, finalize/download

As Sam mid-edit (UJ-3), I want a single `generate(...)` entry point that creates placeholders,
submits the job, reactively tracks it, and downloads results, so a generation request becomes a clip
on the timeline with live status.

**Acceptance criteria:**
- **Placeholder creation** (`generation.md` §"`generate(...)` pipeline" step 1): `count = max(1,
  min(4, numImages))`; create `count` placeholders — new UUID, dest `…/gen-<id8>.<ext>` under
  `Project.media/` (or temp dir if no project), `generation_status = Generating`, appended to the
  editor media assets; **return `placeholders[0].id` synchronously** (so the UI gets an id < 2 s —
  SM-11).
- Build `finalGenInput` (`snapshotRefs` or set `imageURLs`), stamp `createdAt = Date()`, attach to
  placeholders (step 4); call `buildParams(uploaded)` (E9-S5) → `runJob` (step 5).
- **`runJob`** (`generation.md` §"`runJob`"): `submit(model, params, projectId)` →
  `mutation("generations:submit")` → `{jobId}`; then `subscribe(jobId)` →
  `subscribe("generations:by_id", {id: jobId})` yielding `BackendGenerationJob?`. Consume the stream:
  `succeeded` → `finalizeSuccess`; `failed` → all placeholders `Failed(errorMessage ?? "Generation
  failed")` + `onFailure`; `queued|running` → continue. Submit-failure / nil publisher ("Backend not
  configured") **also fails all placeholders**.
- **`finalizeSuccess` + `downloadAndFinalize`** (`generation.md` §"`finalizeSuccess`"): `job.resultUrls`
  mapped **1:1 to placeholders by index**; missing/extra → that placeholder `Failed`. Per asset:
  `Downloading` → `URLSession.download` (→ `reqwest` streamed download to temp) → **fix extension from
  the remote path** (if a known `ClipType`) → remove old file → `moveItem` into place → `None` →
  `importMediaAsset(skipAppend: true)` → `appendGenerationLog` → `finalizeImportedAsset`. Download
  failure stores `pendingDownloadURL` + `Failed(msg)` (retryable via **`retryDownload`**). First
  success fires `AppNotifications.generationComplete`.
- The detached orchestration runs on the main state owner (macOS `@MainActor` → Tauri state
  `Mutex/RwLock` + event emit); reactive `generation_status` flows to the frontend via **Tauri
  events** (no direct frontend side effects — cross-cutting NFR).
- **Index-based result mapping is preserved** (`generation.md` §"Port risks"): fewer URLs than
  placeholders marks extras Failed; ordering held through the upload group.
- **Integration test (§11.2 generation-lifecycle, mock Convex):** submit → subscribe → succeed with N
  result URLs → all N placeholders transition Generating → Downloading → None and land in
  `<project>/media/`; a failed job transitions all to Failed; resultUrls.count < placeholders.count
  fails the extras.

**Implementation context:** crate **`palmier-gen`** (`service` module) — the `GenerationService`
orchestrator. Reference: `Sources/PalmierPro/Generation/GenerationService.swift`
(`generate`/`runJob`/`downloadAndFinalize`/`finalizeSuccess`/`retryDownload`/`createPlaceholder`);
`generation.md` §"The `generate(...)` pipeline" + §"`runJob`" + §"`finalizeSuccess`". Uploads via
E9-S6; submit/subscribe via E9-S1; params via E9-S5. Consumes the Epic-2 `MediaAsset` status machine.

**Dependencies:** **E9-S1** (submit/subscribe), **E9-S5** (params), **E9-S6** (reference upload),
**E9-S10** (status machine on the asset). Cross-epic: Epic 2 (`MediaAsset`, `Project.media/`).

**Parallel-safe?** No — it is the orchestration hub that composes S1/S5/S6/S10; build after those.
Owns the `service` module exclusively.

---

### E9-S8 — Credit gating (`can_generate`) + generation panel & Music-tab form UI

As a signed-in user with credits (and as the `can_generate` tool, UJ-3 edge), I want the advisory
budget gate and the generation form, so I can configure a generation and the UI blocks me when I am
out of credits or signed out.

**Acceptance criteria:**
- **Advisory gate math** (`generation.md` §"Cost gating (`can_generate`)"):
  `remainingCredits = max(0, budgetCredits - spentCredits)` where
  `budgetCredits = plan.monthlyBudgetCredits + user.purchasedCredits`,
  `spentCredits = user.spentCreditsThisPeriod`; `estimatedCost = CostEstimator.<kind>Cost(...)`
  (E9-S3); `canAffordGeneration`: budget unknown → **true**, else `estimatedCost <= remaining` (or
  `remaining > 0` if cost unknown); `canSubmit = canAffordGeneration && required-inputs present`.
- `can_generate = signed_in && tier_allows && has_remaining_credits` is **advisory only** — the
  Convex `generations:submit` mutation is the **real gate** (ruling #24, FR-34). Do not treat the
  client gate as authoritative.
- **UI blocking (FR-34):** when `can_generate` is false, the generation UI is blocked with **"Sign
  in"** (signed out) / **"Out of credits"** (no budget); the submit button is `.disabled(!canSubmit)`;
  `hasInsufficientCredits` colors the cost label red. `estimatedCost` updates **live** as the form
  changes (`generation.md` §"Key types & files": `GenerationView`).
- The **Music tab** renders the `MusicGenerationSubmission` form (ruling #14) inside the Media panel
  (the Epic-4 Music-tab placeholder seam). Generation panel + Music form both submit through the
  shared E9-S5 submission assemblers → E9-S7 `generate(...)`.
- `ModelPreferences`: user-disabled model ids persisted under settings key **`disabledModelIds`**
  (macOS `UserDefaults` → settings store JSON in app config dir); disabled models hidden from the
  form (`generation.md` §"Key types & files": `ModelPreferences`).
- **Tests:** gate truth table (signed-out, no-credits, unknown-budget→true, affordable, unaffordable);
  submit disabled when required inputs absent; disabled-model filtering.

**Implementation context:** crates **`palmier-gen`** (gate logic re-expressed in the gen layer; reads
`AccountService` budget state from **`palmier-auth`**) + **`src-ui/media-panel`** (generation panel,
Music tab) + **`palmier-tauri`** (`estimate_cost` / `can_generate` commands — the backend exposes
these to the frontend, cross-cutting NFR strict-layering). Reference:
`Sources/PalmierPro/Generation/UI/GenerationView.swift`, `Catalog/ModelPreferences.swift`,
`Account/AccountService.swift` (budget); `generation.md` §"Cost gating". The `can_generate` value also
backs the Epic-7 `can_generate` tool.

**Dependencies:** E9-S3 (cost), E9-S2 (catalog/models for the form), E9-S4 (validation for
`canSubmit`). Cross-epic: `palmier-auth` budget state (Epic 1/12), Epic 4 Music-tab seam, Epic 7
`can_generate` tool shell.

**Parallel-safe?** Partially — the **gate math is pure** and builds in parallel; the form UI touches
`src-ui/media-panel` (coordinate with Epic 4's Music-tab seam) and the `palmier-tauri` command
surface. No S-1 dependency.

---

### E9-S9 — Generation log ledger (`generation-log.json`) + legacy migration + backfill

As the cost ledger, I want the append-only generation log with legacy migration, so every generation
is recorded with its credit cost and old projects migrate cleanly.

**Acceptance criteria:**
- Append-only **`generation-log.json`** (`GenerationLog`, **version 1**); each `GenerationLogEntry{
  model, costCredits, createdAt}` with `costCredits` via `CostEstimator.cost(for:)` (E9-S3)
  (`generation.md` §"Generation log"; `EditorViewModel+Cost.swift`).
- **Legacy migration:** old `cost` (dollars) → `Int(dollars * 100, ceil)`
  (`generation.md` §"Generation log").
- `seedGenerationLogFromAssets` **backfills** the log from existing generated assets
  (`generation.md` §"Generation log").
- **Filename is exactly `generation-log.json`** (ruling #3 — reference filename; FOUNDATION's
  `generation_log.json` is wrong and breaks sample import). Date encoding = **Apple reference-epoch
  doubles** (project/media/log family; reconciliation carry-forward "Project I/O Date encoding") —
  consumes the Epic-2 serde Date codec, does not redefine it.
- **Tests:** append → read round-trips; dollars→credits legacy migration on a fixture old log;
  backfill produces one entry per generated asset; filename/serde Date round-trip matches a golden.

**Implementation context:** crate **`palmier-gen`** (`log` module), persisting into the Epic-2
`Project` bundle. Reference: `Sources/PalmierPro/Editor/ViewModel/EditorViewModel+Cost.swift`;
`generation.md` §"Generation log". **Pure serde — zero Convex dependency.**

**Dependencies:** E9-S3 (`CostEstimator.cost`). Cross-epic: Epic 2 (`Project` bundle + Date codec).

**Parallel-safe?** Yes (own `log` module, pure serde). No S-1 dependency.

---

### E9-S10 — `MediaAsset` generation status machine + completion notification

As every status-aware surface, I want the generation status enum, the `isGenerated` rule, the
upload-cache fields, and the native completion toast, so the UI reflects lifecycle state and a toast
fires on success.

**Acceptance criteria:**
- **Status machine** (`generation.md` §"MediaAsset status machine"): `none | generating | downloading
  | rendering | failed(String)`; `generation_input != nil ⇒ is_generated`. **Note (ruling #24):**
  `rendering` exists in the enum but is **never set** in Generation/ — port the variant, do not add a
  transition into it from this epic (open question in `generation.md` — confirm source elsewhere /
  export).
- **Upload-cache fields** on `MediaAsset`: `cachedRemoteURL` + `cachedRemoteURLExpiresAt`;
  `freshRemoteURL` returns the URL **only if unexpired** (`generation.md` §"MediaAsset status
  machine"). (Consumed by E9-S6.)
- **Native completion toast:** `AppNotifications.generationComplete` → native notification
  (`notify-rust` / Tauri notification plugin) — fires on **first** download success (FR-33 toast;
  `generation.md` §"macOS/Apple APIs"). Reactive `current` status flows to the frontend via Tauri
  events.
- This story **extends the Epic-2 `MediaAsset`** (it does not own the base asset) — coordinate the
  field additions with Epic 2; the status enum + cache fields are the generation-owned additions.
- **Tests:** `is_generated` true iff `generation_input` present; `freshRemoteURL` nil after TTL;
  status serde round-trips including `failed(msg)` and `rendering`.

**Implementation context:** crates **`palmier-gen`** (status/notification glue) + the Epic-2
**`palmier-model`** `MediaAsset` (field additions). Reference:
`Sources/PalmierPro/Models/MediaAsset.swift` (`GenerationStatus`, `freshRemoteURL`); `generation.md`
§"MediaAsset status machine". Notification via Tauri plugin.

**Dependencies:** Cross-epic: **Epic 2** (`MediaAsset` base). No S-1 dependency for the enum/fields;
the notification is exercised live by E9-S7.

**Parallel-safe?** Partially — the enum + cache fields touch the shared Epic-2 `MediaAsset` (must
coordinate with Epic 2 ownership); the notification glue is self-contained in `palmier-gen`.

---

### E9-S11 — Generation tool backing + cancellation (v1) wiring into `palmier-tools`

As an agent calling `generate_video` / `can_generate` (UJ-3), I want the M2 tool shells backed by the
real generation backend and v1 cancellation, so the 30-tool surface actually generates media at M3.

**Acceptance criteria:**
- Wire the **real backend** into the existing Epic-7 `generate_video` / `can_generate` (and sibling
  generation) tool *shells* — **exactly one implementation per tool name**, shared by MCP server and
  in-app agent (cross-cutting NFR: single tool implementation). Before M3 these returned "backend not
  available"; this story replaces that with the live `generate(...)` (E9-S7) / advisory gate (E9-S8).
- `can_generate` tool returns the **advisory** gate (E9-S8), never authoritative (ruling #24).
- **Cancellation (v1, FR-35 / ruling #24):** cancel **tears down the client subscription only** (drop
  the WS stream); the Convex job **keeps running and billing**. **No server cancel mutation** is
  added (backend out of repo) — document this in the tool/cancel behavior. (`generation.md` §"Port
  risks": "No true job cancellation".)
- **UJ-3 edge (PRD §2.3):** when `can_generate` is false (signed out / out of credits) the agent
  refuses and the generation UI is blocked — matches FR-34.
- **SM-11 e2e gate (§11.3 generative-augment over §11.2 mock Convex):** a `generate_video` call
  produces a placeholder (Generating) **< 2 s**, transitions Generating → Downloading → None|Failed in
  the UI, fires the completion toast, and lands the asset in `<project>/media/<id>.{ext}`.
- **Tests:** tool dispatch routes to the single shared impl; cancel drops the subscription without a
  server call; advisory `can_generate` false blocks the path.

**Implementation context:** crates **`palmier-tools`** (tool backing — wire into existing shells, do
not add tools; SM-C2 forbids exceeding 30) + **`palmier-gen`** (`generate`/cancel). Reference:
`Sources/PalmierPro/Generation/GenerationService.swift` (entry) + `generation.md` §"Port risks"
(cancellation). The §11.3 e2e is the demonstrable epic exit.

**Dependencies:** **E9-S7** (the `generate(...)` pipeline), **E9-S8** (`can_generate`), **E9-S1**
(subscription teardown). Cross-epic: **Epic 7** (tool shells + dispatcher; hard dependency).

**Parallel-safe?** No — it composes the whole epic and depends on Epic 7's dispatcher; it is the final
integration/exit story.

---

## Dependency summary

```
S-2 spike ── E9-S1 (Convex client) ─┬─ E9-S6 (upload) ──┐
                                     ├─ E9-S7 (pipeline) ─┴─ E9-S11 (tool backing + cancel)  ← epic exit (SM-11)
E9-S2 (catalog) ─┬─ E9-S3 (cost) ────┼─ E9-S8 (gate + UI) ┘
                 ├─ E9-S4 (validate) ─┘
                 └─ E9-S5 (params/submission) ─ E9-S6/E9-S7
E9-S3 ─ E9-S9 (log)
Epic 2 MediaAsset ─ E9-S10 (status machine) ─ E9-S7
Epic 7 dispatcher ─ E9-S11
```

**Parallel-safe lanes:** S2/S3/S4/S5/S9/S10(enum) are pure and build immediately in parallel against
fixtures (no S-1). S1 gates the live-Convex lane (S6, S7, S11). S8's gate-math is pure; its UI seam
coordinates with Epic 4 (Music tab) and `palmier-tauri` commands.

## Notes / open risks carried into stories

- **R-4 (Convex client maturity)** is retired by E9-S1 (the S-2 slice). **OQ-9** (deployment access)
  is the residual external dependency — the lifecycle stories are defined over the §11.2 mock-Convex
  harness so the epic does not stall if OQ-9 resolves negative.
- **Advisory gate is not authoritative** (ruling #24) — never block solely on the client gate;
  the Convex mutation is the real enforcer.
- **Wire-contract byte fidelity** (params JSON field names + Content-Type map) is silent-break risk;
  E9-S5/E9-S6 carry golden/table tests.
- **`rendering` status is dead in this epic** — port the variant, no transition added here.
