---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
---

# Epic 11 — Visual & Transcript Search

## Epic goal

Stand up the two local search subsystems that complete the B-roll-directed workflow (UJ-2): a
**visual** index over sampled video frames + still images using **SigLIP2 base patch16-256, 768-dim**
embeddings (ruling #13), and a **transcript** search over already-cached `TranscriptionResult` segments
(exact keyword — always available, no model — plus optional semantic). Both surface in the Media panel as
"Moments" (frame grid) + "Spoken" (transcript rows) sections, and through the agent via the single
`search_media` tool (`scope = visual | spoken | both`). The whole subsystem is gated by one user toggle
(`searchIndexEnabled`, default ON), runs as a per-project background queue that **pauses while any export
is in flight**, and re-indexes naturally on file edits via the `sha256(path|mtime|size)[:32]` identity key.

Governing reference: `docs/reference/search.md` (constants, algorithms, file map, port-notes). Crate home
per FOUNDATION §4/§14: **`palmier-search`** ("CLIP frame index + transcript full-text", L148), consuming
`TranscriptionResult` from `palmier-transcribe` (Epic 10) and `MediaAsset` from `palmier-model`; visual
encode/dot may run on the wgpu compositor stack (FOUNDATION §6.10 / L82) or via candle/ort + BLAS.

## PRD acceptance this epic must satisfy (§4.11 / §10 Epic 11)

- **FR-39 (Visual search):** Embed sampled frames via SigLIP2 base patch16-256, 768-dim; index to
  `<project>/.search/visual_index.bin`; query via the text encoder, cosine similarity, top-K. Exact
  preprocessing (carry-forward / reconciliation L68-69): **256×256 squash (no crop, black fill, sRGB
  BGRA), tokenizer pad-to-64 with id 0 no attention mask, raw dot-product on L2-normalized output, cosine
  floor 0.05, relative cutoff 0.85**. Reproduce the `.embed` magic (`PALMEMB1`) format OR adopt a new
  magic and re-index (not interchangeable with OpenAI CLIP).
- **FR-40 (Transcript search):** Index transcribed segments/words; exact keyword (always available, no
  model) + semantic (BGE-small / all-MiniLM via candle). Click hit → jump preview + select asset; "Use as
  B-roll" → drop at playhead.
- **SM-12 (Correctness exit):** `search_media` returns the planted B-roll frame in top-K on the
  `golden_search` fixture (visual scope); transcript exact-keyword recall = **100%** on the
  `golden_project_text` / transcript fixture (spoken scope). **§11.4 search-index-query benchmark runs at
  1k / 10k / 100k frames** as an explicit acceptance item (Criterion).
- **Milestone: M4 — Visual Search + Captions polish** (PRD §12). Spike **S-3 (SigLIP2 weight conversion)**
  must be resolved by M4 (runnable in parallel from M1). Realizes UJ-2 fully; validates the **§11.3
  B-roll-directed e2e** as a milestone exit gate.

---

## SPIKE / RISK GATE (read before sequencing)

**Epic 11 is gated by Spike S-3 (PRD §11, R-3).** SigLIP2 base patch16-256 768-dim weights ship as CoreML
in the reference, which the port **cannot** use. We must source/convert ONNX (`ort`) or candle weights and
prove L2-normalized embeddings match the reference preprocessing within tolerance — otherwise we re-index
with a **new** `.embed` magic. **S-3 is story E11-S1 and every visual-indexing/query story is gated on it.**
Do NOT write embedding-format or ranking-calibration stories that assume CoreML-compatible embeddings: the
`0.05` cosine floor and `0.85` relative cutoff are only valid if the converted model outputs equivalent
normalized vectors (R-3); S-1 of this epic decides whether they hold or must be recalibrated. The
**transcript-search stories (E11-S8, E11-S9) have NO model dependency** (keyword mode) and can proceed in
parallel from the start.

S-3 pass bar (PRD §11): converted weights produce embeddings whose cosine similarity to reference
embeddings on a fixture frame set is within tolerance (or a documented re-index decision), and the
converted artifact size is recorded (confirms the ≈0.8–1 GB §13.3 estimate; downloadable, not bundled).

---

## Stories

### E11-S1 — [SPIKE] SigLIP2 weight conversion + embedding-parity harness
> **Status:** DONE-production-port (story/E11-S1-embedder-port) — Spike S-3 ported into
> production `palmier-search`: `preprocess` (256×256 squash, black-fill, sRGB, [-1,1]
> CHW), `tokenize` (Gemma pad-to-64 id-0 no-mask), `embedder` (explicit **L2-normalize**
> → raw dot == cosine; `VisualEmbedder::{encode_image, encode_text}`), `model_loader`
> (state machine `unknown → notInstalled | preparing → ready | downloading | failed`,
> `prepare()` never downloads, `download_stub()` left for E11-S6), `manifest`
> (`siglip2-base-patch16-256`, modelVersion **2**, SHA/size placeholders + base-URL const).
> The **`ort` ONNX encode path is FEATURE-GATED** (`--features ort`, OFF by default) so
> default builds need no `onnxruntime.dll`; under the feature it uses ort 2.0.0-rc.10 with
> DirectML + CPU fallback (type-checks). 52 unit/integration tests pass with no weights;
> the **live ort encode + real cosine is gated on downloading ~750 MB weights +
> onnxruntime.dll** — left as an `#[ignore]`d `live_encode_cosine_sanity` test with the
> exact run command. Magic decision = keep `PALMEMB1`, modelVersion = 2 (consumes E11-S2).
>
> **Original note:** Satisfied by Spike S-3 (`spikes/s3-siglip2/`) — runtime = `ort` + ONNX (`onnx-community/siglip2-base-patch16-256-ONNX`); parity surface (preprocess/tokenize/L2-normalize/rank/.embed) proven, magic decision = keep `PALMEMB1`, bump `modelVersion` to 2.

**Intent:** As the build, I want SigLIP2 base patch16-256 768-dim weights as ONNX/candle plus a parity
harness, so visual indexing produces L2-normalized embeddings that keep the `0.05` floor / `0.85` cutoff
valid (or so we decide to re-index with a new magic).

**Acceptance criteria:**
- Source/convert SigLIP2 base patch16-256 (768-dim, image 256×256, text context 64) to ONNX (`ort`) or
  candle safetensors. Record the converted-artifact **size** (confirm/refute the ≈0.8–1 GB §13.3 estimate)
  and publish a **new download manifest** (model id `siglip2-base-patch16-256`, version, embeddingDim 768,
  imageSize 256, contextLength 64, new SHA256/sizes, new hosted base URL — the reference
  `palmier-io/siglip2-base-coreml` repo is CoreML-only and cannot be used as-is).
- Reproduce the **exact** preprocessing (reconciliation L68-69): image → 256×256 BGRA `CVPixelBuffer`
  analogue, **black fill first**, then **squash-resize (no aspect crop)**, sRGB, high interpolation;
  text → tokenize, clip to 64, **right-pad with token id 0, no attention mask** (must match Python SigLIP
  reference exactly). Output feature key `"embedding"`; input key `"tokens"` as int32 `[1,64]`.
- **Parity gate (S-3 pass bar):** on a committed fixture frame set, cosine similarity between converted-
  model embeddings and reference embeddings is **within tolerance**; document the tolerance. If the model
  does NOT emit L2-normalized vectors, add an explicit normalize step at index time (the ranking path uses
  a raw dot product with no normalization — R-3). If parity cannot be met, record a **documented re-index
  decision** (new `.embed` magic) and note that `0.05`/`0.85` may need recalibration in E11-S5.
- Decide ort execution providers (DirectML/CUDA/CPU) with CPU fallback; record per-frame embed latency on
  CPU-only Windows (open question in `search.md`).

**Implementation context:** crate `palmier-search` (model layer). Reference files:
`Sources/PalmierPro/Search/Models/VisualEmbedder.swift` (`encode(image:)`, `encode(text:)`, `pixelBuffer`
squash-resize), `Models/VisualModelLoader.swift` (state machine), `Models/TextTokenizer.swift`
(`AutoTokenizer`, pad-to-64 id 0), `Models/ModelDownloader.swift`,
`SearchIndexConfig.swift` (model manifest constants). docs/reference/search.md §"Embedding model + index
format/states", §"macOS/Apple APIs to replace" (CoreML→ort/candle; HF `Tokenizers`→`tokenizers` crate).
PRD §11 S-3, R-3, §8 §13.3. Map ports: CoreML→`ort`/`candle`; `Tokenizers`→`tokenizers` crate;
`CVPixelBuffer`/`CGContext` squash→`fast_image_resize`/`image` crate or wgpu compute.

**Dependencies:** none (parallelizable from M1). Soft consumer of Epic 4 `MediaAsset` types for the
fixture set but the spike can use raw frames.

**Parallel-safe?** Yes — owns only the model loader/embedder/tokenizer/downloader + manifest; no overlap
with transcript stories.

---

### E11-S2 — `.embed` binary store + cache-key identity (EmbeddingStore)
> **Status:** DONE (story/E11-S2-embed-store) — PALMEMB1 byte-exact, modelVersion=2

**Intent:** As `palmier-search`, I want a byte-faithful `.embed` reader/writer keyed by file identity, so
indexes persist, reload cheaply, and re-index naturally on any file edit.

**Acceptance criteria:**
- Implement the `.embed` format **byte-exactly** (search.md §"Embedding model + index format/states"):
  `magic "PALMEMB1"` (8B) + `UInt32 LE` json length + JSON header `{model, modelVersion, samplerVersion,
  dim, count}` + `count` rows. **Each row = 3× Float64 LE (`time, shotStart, shotEnd`) + `dim`× Float16
  (`half::f16`) embedding values.** Total file = `magic+4 + jsonLen + count*(24 + dim*2)`. Written
  **atomically**. Loaded Float16 vectors **widened to Float32** for BLAS. Header read cheaply via a
  FileHandle/seek prefix (no full-file load). *If E11-S1 chose a new magic, use it consistently here.*
- **Cache key (file identity):** `SHA256("<path>|<mtime epoch>|<size>")` hex, **first 32 chars** (`sha2`
  crate). Stored under the port cache dir, e.g. `%APPDATA%\PalmierProWin\Cache\embeddings\<key>.embed`
  (`directories` crate). Any mtime/size change ⇒ new key ⇒ natural re-index. *Note the Windows coarse-mtime
  false-hit risk (R-7-adjacent) — same scheme as Epic 4 thumbnails.*
- `isCurrent(key, model, modelVersion, samplerVersion)` returns true iff a matching-header file exists.
  `clearAll()` wipes the embeddings dir.
- **Unit test:** Float16 round-trip introduces ~1e-3 error — assert round-trip within the reference's
  tolerance; assert exact byte layout for a known `(count, dim)` against a golden byte buffer.

**Implementation context:** crate `palmier-search`. Reference:
`Sources/PalmierPro/Search/Indexing/EmbeddingStore.swift` (`AssetIndex{header, rows, vectors}`,
`PALMEMB1` magic, atomic write, `isCurrent`, `clearAll`). docs/reference/search.md §"Embedding model +
index format/states", §"Port risks & gotchas" (`.embed` byte-exactness, Float16 ~1e-3, cache key).
Note: PRD §3 Glossary names the on-disk index `<project>/.search/visual_index.bin`; reconcile path/naming
with E11-S6 coordinator (project-scoped `.search/` dir vs global cache) — store format/magic is the
load-bearing contract, the directory is a port choice.

**Dependencies:** E11-S1 (magic decision: reuse `PALMEMB1` vs new).

**Parallel-safe?** Yes — owns `EmbeddingStore.rs` only.

---

### E11-S3 — FrameSampler (shot detection + keep cadence)
> **Status:** DONE (story/E11-S3-framesampler) — 8×8 BT.601 LumaGrid, promoteDiff=12,
> coverage-floor 8s, interval-doubling ≥3000px, samplerVersion=1. Reuses Epic 4
> `palmier_media::extract_frame_timed` (one FFmpeg seek+decode+scale path, returns frame + actual PTS).

**Intent:** As the visual indexer, I want a frame-sampling stream over a video that emits distinct,
shot-aware frames, so embeddings cover scenes without flooding near-duplicates.

**Acceptance criteria (search.md §"Frame sampling cadence", `samplerVersion = 1`):**
- Default `Options`: `candidateInterval = 2.0s`, `coverageFloor = 8.0s`, `promoteDiff = 12`,
  `maxSize = 512×512`, `highResEdge = 3000`. **If the video's larger edge ≥ 3000 px, double the interval
  (→ 4s).**
- Candidate times: `stride(from: interval/2, to: duration, by: interval)`; if empty use `[duration/2]`.
- Decode via **FFmpeg seek+decode+scale** (replacing `AVAssetImageGenerator.images(for:)`): seek to each
  candidate and take the decoded keyframe/nearest frame; tolerance = `max(interval/2, 1.0)s` before/after
  ("nearest sync frame within tolerance"). `maxSize 512×512`, preferred-track-transform applied.
  **Skip frames whose actualTime ≤ previous actualTime.**
- **Shot detection:** downsample each frame to an **8×8 luma grid** (`LumaGrid`, BT.601 weights
  0.299/0.587/0.114 on premultiplied-RGBA). `meanDiff` = mean abs per-cell delta vs previous **kept** grid.
  `isNewShot = meanDiff > promoteDiff (12)`; **first frame is always a new shot.**
- **Keep rule:** emit a frame iff `isNewShot || (t - lastKeptTime) >= coverageFloor (8s)`.
- Emits `Frame{time, image, isNewShot}` stream.
- **Unit tests:** synthetic clip with a known scene cut asserts `isNewShot` at the cut; a long static clip
  asserts coverage-floor keeps; high-res-edge clip asserts interval doubling.

**Implementation context:** crate `palmier-search` (indexing) + `palmier-media` for FFmpeg decode.
Reference: `Sources/PalmierPro/Search/Indexing/FrameSampler.swift`. docs/reference/search.md §"Frame
sampling cadence", §"macOS/Apple APIs to replace" (`AVAssetImageGenerator`→FFmpeg seek+decode+scale,
`CGImage`→`image`/`fast_image_resize`). FOUNDATION §6.5/§6.2 (FFmpeg decode ownership in `palmier-media`).

**Dependencies:** Epic 4 (`palmier-media` FFmpeg decode path) for frame extraction; none within Epic 11.

**Parallel-safe?** Yes — owns `FrameSampler.rs`; consumes the `palmier-media` decode handle read-only.

---

### E11-S4 — VisualIndexer (sampler → embedder → store), idempotent
**Intent:** As the coordinator, I want to turn one asset into a persisted `AssetIndex`, idempotently per
file/model/sampler identity, so re-indexing only happens when something actually changed.

**Acceptance criteria (search.md §"Visual indexing"):**
- `needsIndex` = `!EmbeddingStore.isCurrent(key, model, modelVersion, samplerVersion)`.
- **Video:** for each emitted frame, on `isNewShot` push a shot-start (**first shot starts at 0**, else
  `frame.time`); `vectors += encode(image)`; record `time` + `shotIndex`. A row's `shotStart` = its shot's
  start, `shotEnd` = next shot's start (or `duration` for the last shot).
- **Image (still):** skip the sampler — decode a ≤512px thumbnail (ImageIO analogue), one embedding, row
  `(time:0, shotStart:0, shotEnd:0)`. *(`shotStart 0` is load-bearing for best-per-shot dedupe — E11-S5.)*
- Save header `{model, modelVersion, samplerVersion, dim, count}` + rows + vectors via E11-S2.
- **Export yield:** call `waitWhileExportActive()` **before each frame embed and per asset** (export-pause
  refcount from E11-S6).
- **Unit test:** a 2-shot synthetic video produces rows with correct `shotStart`/`shotEnd` boundaries and
  `count` = number of kept frames; a still produces exactly one row `(0,0,0)`.

**Implementation context:** crate `palmier-search`. Reference:
`Sources/PalmierPro/Search/Indexing/VisualIndexer.swift`. docs/reference/search.md §"Visual indexing".
Consumes E11-S1 `VisualEmbedder.encode(image:)`, E11-S2 `EmbeddingStore`, E11-S3 `FrameSampler`.

**Dependencies:** E11-S1, E11-S2, E11-S3. Export-pause hook from E11-S6 (can stub `waitWhileExportActive`
until S6 lands).

**Parallel-safe?** Partially — owns `VisualIndexer.rs` but depends on S1/S2/S3 landing first; not
concurrent with them.

---

### E11-S5 — VisualSearch ranking (BLAS dot, best-per-shot dedupe, cutoffs)
> **Status:** DONE (story/E11-S5-visualsearch) — raw-dot ranking (plain Rust per-row dot), best-per-shot dedupe on `shot_start` bits, 0.05 floor + 0.85 relative cutoff; parity with `VisualSearch.swift`. `palmier_search::{visual_search, Hit}`.

**Intent:** As `search_media` (visual scope), I want to rank an asset's frame embeddings against a query
vector and return deduped top-K hits, so one scene can't flood results and only confident matches surface.

**Acceptance criteria (search.md §"Query path (visual)"):**
- `VisualSearch.search(query_vec, indexes, limit=20, relativeCutoff=0.85, minScore=0.05)`:
  - Per asset: `scores = vectors(count×dim) · query` via **`cblas_sgemv(RowMajor, NoTrans)`** analogue
    (`ndarray`+BLAS / `matrixmultiply` / candle matmul, or wgpu compute per FOUNDATION §6.10). **Raw dot
    product — vectors are assumed pre-L2-normalized** (E11-S1 guarantees this), so dot ≈ cosine.
  - **Best-per-shot dedupe:** keep only the highest-scoring frame per `shotStart` (Float64 bucket key — do
    NOT change the key type or stills' `shotStart 0` collide differently). Emit one `Hit` per surviving shot.
  - Sort desc by score; **drop `< minScore (0.05)`**; require `top > 0`; keep `prefix(limit)`, then filter
    to `score >= top * 0.85` (relative cutoff). Returns `[Hit{assetID, time, shotStart, shotEnd, score}]`.
- **If E11-S1 forced a re-index (new magic / non-matching embeddings),** recalibrate `minScore`/`relativeCutoff`
  on the `golden_search` fixture and document the new constants in `SearchIndexConfig`.
- **Unit test (feeds SM-12 visual):** on a fixture with a planted B-roll frame, the planted frame's shot is
  in the returned top-K; a synthetic two-frames-same-shot case asserts only the higher-scoring one survives.

**Implementation context:** crate `palmier-search`. Reference:
`Sources/PalmierPro/Search/Query/VisualSearch.swift` (`Hit`, sgemv ranking, best-per-shot dedupe).
docs/reference/search.md §"Query path (visual)", §"Port risks & gotchas" (vectors-assumed-pre-normalized,
best-per-shot dedupe on `shotStart`). Map: `Accelerate cblas_sgemv`→`ndarray`/`matrixmultiply`/candle/wgpu.

**Dependencies:** E11-S2 (load `AssetIndex`), E11-S1 (query-text encoding + normalization guarantee).

**Parallel-safe?** Yes — owns `VisualSearch.rs`.

---

### E11-S6 — SearchIndexCoordinator (per-project queue, scheduling, export-pause, fan-out)
**Intent:** As the app, I want a per-project background queue that indexes assets one at a time, runs
visual + transcript concurrently per asset, and pauses during export, so search builds without contending
with playback/export.

**Acceptance criteria (search.md §"Coordinator queue + scheduling", §"Model loader states"):**
- `@Observable`-analogue per-project coordinator + search entrypoint. Worker is a **single** utility-
  priority task; tracks `batchTotal / batchCompleted / currentAssetFraction`.
- `schedule(asset)` requires model enabled + embedder ready, asset not generating, not already
  queued/failed. Enqueue if `needsVisual` (video/image needing index) **OR** `needsTranscript`.
  `wantsTranscript` = audio, or video with audio; `needsTranscript` = wants it AND no cached transcript on
  disk.
- Worker dequeues sequentially; `indexOne` runs transcript (async) + visual **concurrently**. **Progress
  split: if transcribing, visual counts for 0.5 of the asset's fraction, else 1.0.**
- **Export pause:** a **process-global refcounted counter** (`ExportPauseCounter`); indexing sleeps in 2s
  loops while `exportActive`. `workerGeneration` guards a stale worker from clobbering a newer one. *(Epic 6
  export increments/decrements this counter — coordinate the shared refcount type.)*
- App-level fan-out via a weak registry: `sweepAll`, `cancelAll`, `resetAll`,
  `clearIndexGlobally` (also `EmbeddingStore.clearAll()`). `loadedIndexes[id]` cleared when an asset
  re-indexes.
- `search(query, limit=20, within ids?)`: trim query (empty ⇒ []); snapshot candidate (video|image) assets;
  off-thread, load each `AssetIndex` (from in-memory `loadedIndexes` if key matches else disk), encode
  query text → 768 vec, rank via E11-S5.
- Single global toggle `searchIndexEnabled` (default ON; pref key family `io.palmier.pro.*` ruling #6 —
  confirm exact key in settings, Epic 12).
- **Model loader states:** `unknown → notInstalled | preparing → ready | downloading(frac) | failed`;
  `prepare()` loads installed model but **never downloads**; `download()` fetches + compiles + installs then
  loads, then warms with `encode(text:"warm up")` and sweeps all coordinators.

**Implementation context:** crate `palmier-search`. Reference:
`Sources/PalmierPro/Search/SearchIndexCoordinator.swift` + `Models/VisualModelLoader.swift`. docs/reference/
search.md §"Coordinator queue + scheduling", §"Model loader states", §"Port risks & gotchas" (export-pause
coupling, concurrency: don't over-parallelize on Windows — whisper + ort + FFmpeg saturate cores/GPU).

**Dependencies:** E11-S4 (visual index), E11-S5 (search/rank), E11-S8 (transcript search for `indexOne`
spoken path), E11-S1 (model loader). Export-pause refcount shared with Epic 6.

**Parallel-safe?** No — integration hub; lands after S4/S5/S8.

---

### E11-S7 — `palmier-transcribe` `TranscriptCache` (disk+memory JSON cache)
**Status:** SUBSUMED by the merged **E10-S4** (`palmier_transcribe::TranscriptCache`). E10-S4 already
ships the disk+memory JSON cache with the disk-only, never-transcribe query read
(`has_cached_on_disk` / `transcript(file, model_id, language, range)`); no second cache was built.
E11-S8 consumes E10-S4 directly. (Note: E10-S4's key is the FOUNDATION content key
`sha256(content)+model+language` per ruling #19, not the reference `path|mtime|size` named below.)

**Intent:** As transcript search, I want full transcripts cached on disk + in memory keyed by file
identity, so spoken search reads cached transcripts without re-transcribing at query time.

**Acceptance criteria (search.md §"Transcript path"):**
- Actor-style cache: disk JSON at `%APPDATA%\PalmierProWin\Cache\transcripts\<key>.json` (**same identity
  key scheme as embeddings**: `SHA256(path|mtime|size)[:32]`) + an in-memory **LRU (max 4, cleared
  wholesale when full)**.
- **Only full transcripts cached;** windowed requests filter a cached full transcript by range.
- `cachedOnDisk(key)` returns the cached `TranscriptionResult` if present, **without triggering
  transcription** (query path is disk-only).
- Consumes the `TranscriptionResult{text, language, words[], segments[]}` produced by Epic 10
  (`palmier-transcribe`); this story owns only the **cache**, not generation.

**Implementation context:** crate `palmier-transcribe` (sibling of `palmier-search` per search.md L166 /
FOUNDATION §4 L139). Reference: `Sources/PalmierPro/Transcription/TranscriptCache.swift`. docs/reference/
search.md §"Transcript path (spoken)". The `TranscriptionResult` type + Whisper generation is **Epic 10
(FR-36)**; this story is the read-cache `palmier-search` consumes.

**Dependencies:** Epic 10 `TranscriptionResult` type definition (can stub the type if Epic 10 not landed;
generation not required for keyword search, which reads whatever is cached).

**Parallel-safe?** Yes — owns `TranscriptCache.rs`; no model dependency.

---

### E11-S8 — TranscriptSearch (exact keyword, all-terms, diacritic-insensitive)
**Status:** DONE — branch `story/E11-S8-transcript-search`. `crates/palmier-search/src/transcript_search.rs`
(`TranscriptSearch::search` / `TranscriptHit`); reads disk-only via the merged **E10-S4**
`TranscriptCache` (E11-S7 subsumed — no second cache). Diacritic+case fold via `unicode-normalization`
(NFD + strip combining marks + lowercase). SM-12 spoken exit asserted: 100% keyword recall +
café/cafe + US/us insensitivity. `model_id`/`language` default to `ggml-small.en`/`en`
(`search_with` lets the E11-S6 coordinator pass the real pair).

**Intent:** As `search_media` (spoken scope), I want exact keyword search over cached transcript segments,
so spoken hits work with no model download and 100% keyword recall.

**Acceptance criteria (search.md §"Transcript path", FR-40, SM-12 spoken):**
- `TranscriptSearch.search(query, assets, limit=20)`: split query into terms (**strip edge punctuation,
  drop empties**); a segment matches iff it contains **all** terms, compared
  **`.caseInsensitive, .diacriticInsensitive`**.
- Returns `Hit{assetID, start, end, text}` in **asset/segment order**, capped at `limit`.
- **Reads disk-only** (`cachedOnDisk` from E11-S7) — **no transcription triggered at query time**.
- Always available — **no model download** required (keyword mode, FR-40).
- **Unit test (SM-12 spoken exit):** on the `golden_project_text` / transcript fixture, exact-keyword
  recall = **100%** (every segment containing all query terms is returned); diacritic/case-insensitivity
  asserted ("café" matches "cafe", "US" matches "us").

**Implementation context:** crate `palmier-search`. Reference:
`Sources/PalmierPro/Transcription/TranscriptSearch.swift`. docs/reference/search.md §"Transcript path
(spoken)".

**Dependencies:** E11-S7 (`TranscriptCache.cachedOnDisk`). Epic 10 transcript fixture for the golden test.

**Parallel-safe?** Yes — owns `TranscriptSearch.rs`; reads S7 cache. No model dependency — can start at M1.

---

### E11-S9 — Semantic transcript search (BGE-small / all-MiniLM via candle) [optional/secondary]
**Intent:** As spoken search, I want optional semantic matching over transcript segments, so queries find
meaning beyond exact keywords (FR-40 semantic clause).

**Acceptance criteria:**
- Embed transcript segments + the query with **BGE-small or all-MiniLM-L6 via `candle`** (FOUNDATION §6.10
  L602); rank by cosine; merge/rank alongside keyword hits.
- Semantic mode is **additive** — keyword (E11-S8) remains the always-available default; semantic engages
  only when its model is present (downloadable). Absent model ⇒ degrade silently to keyword-only.
- **Note:** FOUNDATION §6.10 specifies semantic transcript search; the reference `search.md` only details
  the keyword `TranscriptSearch`. This is a FOUNDATION-driven addition with no reference algorithm — keep
  it behind keyword and do not let it regress the SM-12 100%-keyword-recall gate.

**Implementation context:** crate `palmier-search`. FOUNDATION §6.10 (BGE-small/all-MiniLM via candle);
search.md §"Transcript path" (keyword baseline). No direct reference file (port-additive).

**Dependencies:** E11-S8 (keyword baseline + hit type), E11-S7 (cache). Candle model sourcing.

**Parallel-safe?** Yes — additive layer; owns its own module. Lowest priority; can slip past M4 if needed
without breaking SM-12.

---

### E11-S10 — `search_media` tool wiring (palmier-tools)
**Intent:** As an MCP/agent client, I want one `search_media(query, scope?, media_ref?, limit?)` tool, so
the agent can find visual + spoken moments across the library (UJ-2).

**Acceptance criteria (FOUNDATION §6.14 L709, search.md §Purpose):**
- Implement `search_media` in **`palmier-tools`** (single shared dispatcher — invoked by both the MCP
  server and the in-app agent; no duplication, FOUNDATION §4). Signature `query, scope?, media_ref?,
  limit?`; **scope = `visual | spoken | both`** (default `both`).
- Returns `hits[{score, media_ref, range?, image?}], visual_status` (FOUNDATION §6.14 table). Visual hits
  carry `time`/`shotStart`/`shotEnd` → `range`; spoken hits carry `start`/`end` → `range`. `visual_status`
  reflects the model-loader state (E11-S6: `ready | indexing | model_not_installed | downloading_model |
  preparing | disabled | failed`).
- `media_ref` scopes the search to one asset (maps to coordinator `within ids`); ShortId prefix resolution
  follows the Epic 7 `IdUniverse` rules (≥8-char unique prefix).
- Dispatches: visual → E11-S6 `coordinator.search`; spoken → E11-S8 `TranscriptSearch.search` (+ E11-S9 if
  enabled). Visual scope runs async, spoken sync, per the reference UI path.
- **Integration test (SM-12):** `search_media(scope=visual)` returns the planted `golden_search` B-roll
  frame in top-K; `search_media(scope=spoken)` hits the planted transcript keyword.

**Implementation context:** crate `palmier-tools` (`search_media`). Reference:
`Sources/PalmierPro/Agent/Tools/ToolExecutor+Search.swift`. FOUNDATION §6.14 L709 (tool row), §6.10.
docs/reference/search.md §Purpose (scope semantics). **This is 1 of the 30 tools (ruling #1)** — surfaced
in Epic 7's catalogue; this story supplies its implementation.

**Dependencies:** E11-S5, E11-S6, E11-S8; Epic 7 ShortId/`IdUniverse` for prefix resolution. Tool is
registered in the Epic 7 catalogue (M2) but returns visual_status=`disabled`/empty until this lands (M4).

**Parallel-safe?** No — composes S5/S6/S8.

---

### E11-S11 — Media panel "Moments" + "Spoken" sections (UI + navigation)
**Intent:** As a user, I want Moments (frame grid) and Spoken (transcript rows) sections in the Media panel
with click-to-jump and drag-to-timeline, so I can place searched moments by hand (UJ-2).

**Acceptance criteria (search.md §"UI result navigation"):**
- Search **debounced 250ms** (`scheduleMomentSearch`); runs **spoken (sync) + visual (async)** into
  `visualHits` / `spokenHits`. Three collapsible sections: **Moments** (frame grid), **Spoken** (rows),
  **Files** (name match — separate plain-string filter, already in Epic 4, not this subsystem).
- **Moment card:** thumbnail at `hit.time` (FFmpeg seek, 240px, 1s tolerance), label, timecode
  `shotStart–shotEnd`. **Tap → `selectMediaAsset(asset, atSourceFrame: secondsToFrame(shotStart, fps))`.**
  Draggable payload = asset id + source segment `[shotStart, max(shotEnd, shotStart+0.1)]` (stills drag as
  plain asset — no segment).
- **Spoken row:** thumbnail at `hit.start`, transcript text (**3 lines**), `name · timecode`. **Tap → seek
  to `start`.** Draggable with segment `[start, max(end, start+0.1)]`.
- "**Use as B-roll**" → drop at playhead (FOUNDATION §6.10 L604; via the Epic 3 add-clip path).
- Strict layering (FOUNDATION §4): UI calls Tauri commands → `palmier-tools`/`palmier-search`; no direct
  FFmpeg/fs from the frontend; hits flow back via Tauri events.

**Implementation context:** `src-ui/media-panel`. Reference:
`Sources/PalmierPro/MediaPanel/MediaTab/MediaTab+Search.swift`. docs/reference/search.md §"UI result
navigation". Drop-at-playhead uses the Epic 3 OverwriteEngine/add-clip path; thumbnail at time uses the
Epic 4/`palmier-media` thumbnail path.

**Dependencies:** E11-S6 (coordinator search), E11-S8 (spoken), E11-S10 (tool path optional — UI can call
the coordinator directly via a Tauri command). Epic 4 media panel shell; Epic 3 add-clip for "Use as
B-roll".

**Parallel-safe?** No — frontend integration; lands after the search backend (S6/S8).

---

### E11-S12 — Search-index-query Criterion benchmark (1k/10k/100k) [SM-12 / §11.4 exit]
**Intent:** As the build, I want a Criterion benchmark of visual-index query at 1k/10k/100k frames, so the
§11.4 search-index acceptance item is measured and tracked.

**Acceptance criteria (PRD §11.4, §10 Epic 11, SM-12):**
- A **Criterion** benchmark builds synthetic `AssetIndex`es of **1k / 10k / 100k** frame embeddings (768-dim)
  and measures `VisualSearch.search` (sgemv rank + best-per-shot dedupe + cutoff) wall-time at each scale.
- Runs in CI as an **explicit Epic 11 acceptance item** (PRD §10: "search-index-query benchmark at 1k/10k/
  100k frames runs as an explicit acceptance item"); records numbers (no hard FPS floor specified — this is
  a tracked benchmark, not a pass/fail threshold, unlike SM-2).
- The **SM-12 correctness assertions** (planted B-roll top-K on `golden_search`; transcript keyword recall
  100%) are wired as **gating tests** (from E11-S5 / E11-S8), distinct from this perf benchmark.

**Implementation context:** crate `palmier-search` (`benches/`). PRD §4.11/§10 Epic 11, §11.4 table row
"search index→Epic 11", SM-12. Reference ranking under benchmark = E11-S5 `VisualSearch`.

**Dependencies:** E11-S5 (ranking), E11-S2 (`AssetIndex` construction).

**Parallel-safe?** Yes — owns `benches/search_index.rs`.

---

## Sequencing summary

- **Gate:** E11-S1 (Spike S-3) gates all visual stories (S2 magic decision, S4 indexing, S5 ranking, S6
  coordinator, S10 tool visual scope, S12 bench). **Transcript track (S7→S8→[S9]) has no model dependency
  and runs in parallel from M1.**
- **Parallel-safe leaves (own files, separate worktrees):** S1, S2, S3, S5, S7, S8, S9, S12.
- **Integration hubs (sequence last):** S4 (after S1/S2/S3), S6 (after S4/S5/S8), S10 (after S5/S6/S8),
  S11 (after S6/S8/S10).
- **Cross-epic deps:** Epic 4 (`palmier-media` FFmpeg decode/thumbnails) for S3/S11; Epic 10
  (`palmier-transcribe` `TranscriptionResult`) for S7; Epic 6 export-pause refcount for S6; Epic 7
  ShortId/`IdUniverse` + 30-tool catalogue for S10; Epic 3 add-clip for S11 "Use as B-roll".
- **Milestone:** all land in **M4**; S-3 (E11-S1) resolvable in parallel from M1.
