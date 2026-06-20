---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# search — reference port notes

## Purpose
Two parallel local search subsystems over a project's media assets, surfaced in the Media panel
as "Moments" (visual) + "Spoken" (transcript) sections, and exposed to the agent via the
`search_media` MCP tool (scope = visual | spoken | both):

1. **Visual / CLIP search** — sample distinct frames from videos (and stills from images), embed
   each with a SigLIP2 image encoder, store per-asset embedding files; at query time embed the
   text query with the SigLIP2 text encoder and rank frames by cosine (dot of L2-normalized vecs).
2. **Spoken / transcript search** — transcribe audio/video on-device, cache full transcripts as
   JSON, and run exact case/diacritic-insensitive keyword match over transcript segments.

Both are gated by a single user toggle (`searchIndexEnabled`, default ON). Indexing runs as a
per-project background queue that pauses while any export is in flight.

## Key types & files (cite paths under Sources/PalmierPro/Search/...)
- `SearchIndexConfig.swift` — constants: `visualMatchCosineFloor = 0.05`; model manifest
  (model `siglip2-base-patch16-256`, version 1, embeddingDim 768, imageSize 256, contextLength 64);
  hosted model base URL `https://huggingface.co/palmier-io/siglip2-base-coreml/resolve/main`.
- `SearchIndexCoordinator.swift` — `@MainActor @Observable` per-project queue + search entrypoint.
  Worker is a single `Task(priority: .utility)`; tracks `batchTotal/batchCompleted/currentAssetFraction`.
- `Indexing/FrameSampler.swift` — `samplerVersion = 1`; emits `Frame{time, image, isNewShot}` stream.
- `Indexing/VisualIndexer.swift` — drives sampler → embedder → `EmbeddingStore`. Idempotent per
  `(file identity, model, modelVersion, samplerVersion)`.
- `Indexing/EmbeddingStore.swift` — binary `.embed` disk format (`PALMEMB1` magic). `AssetIndex{header, rows, vectors}`.
- `Models/VisualEmbedder.swift` — wraps 2 CoreML models (image + text) + tokenizer; `encode(image:)`, `encode(text:)`.
- `Models/VisualModelLoader.swift` — `@MainActor` singleton; state machine; loads model on launch (never auto-downloads).
- `Models/ModelDownloader.swift` — download/verify(SHA256)/unzip(ditto)/compile(mlpackage→mlmodelc)/install.
- `Models/TextTokenizer.swift` — HuggingFace `Tokenizers` `AutoTokenizer`; pads to contextLength with token 0.
- `Query/VisualSearch.swift` — `Hit{assetID, time, shotStart, shotEnd, score}`; BLAS sgemv ranking + best-per-shot dedupe.
- Transcript side (sibling `../Transcription/`): `TranscriptSearch.swift` (keyword match),
  `TranscriptCache.swift` (disk+memory JSON cache), `Transcription.swift` (Apple Speech engine).
- UI: `../MediaPanel/MediaTab/MediaTab+Search.swift`. Agent tool: `../Agent/Tools/ToolExecutor+Search.swift`.

## Core behaviors & algorithms (concrete — downstream story/dev agents implement from this)

### Frame sampling cadence (FrameSampler.sample)
- Default `Options`: `candidateInterval = 2.0s`, `coverageFloor = 8.0s`, `promoteDiff = 12`,
  `maxSize = 512×512`, `highResEdge = 3000`.
- If video natural size's larger edge ≥ 3000 px, **double** the interval (→ 4s).
- Candidate times: `stride(from: interval/2, to: duration, by: interval)`; if empty use `[duration/2]`.
- Uses AVAssetImageGenerator batch `images(for:)`; tolerance = `max(interval/2, 1.0)s` before/after
  (so the decoder snaps to nearest sync frame — exact frame times not required). `maximumSize = 512×512`,
  `appliesPreferredTrackTransform = true`. Skip frames whose actualTime ≤ previous actualTime.
- **Shot detection:** downsample each frame to an 8×8 luma grid (`LumaGrid`, BT.601 weights
  0.299/0.587/0.114 on premultiplied-RGBA pixels). `meanDiff` = mean abs per-cell delta vs previous
  kept grid. `isNewShot = meanDiff > promoteDiff (12)`; first frame is always a new shot.
- **Keep rule:** emit a frame iff `isNewShot || (t - lastKeptTime) >= coverageFloor (8s)`. This keeps
  long static shots represented while collapsing near-duplicate frames.

### Visual indexing (VisualIndexer)
- `needsIndex` = `!EmbeddingStore.isCurrent(key, model, modelVersion, samplerVersion)`.
- Video: for each emitted frame, on `isNewShot` push a shot-start (first shot starts at 0, else frame.time);
  `vectors += encode(image)`; record `time` + `shotIndex`. Row's `shotStart` = its shot's start,
  `shotEnd` = next shot's start (or `duration` for last shot).
- Image (still): skip sampler — decode a ≤512px thumbnail (ImageIO), one embedding, row `(time:0, shotStart:0, shotEnd:0)`.
- Saves header `{model, modelVersion, samplerVersion, dim, count}` + rows + vectors.
- Indexing yields to export: calls `waitWhileExportActive()` before each frame embed and per asset.

### Embedding model + index format/states
- **Model:** SigLIP2 base patch16-256, CoreML, 768-dim embeddings, image 256×256, text context 64.
  Image preproc (VisualEmbedder.pixelBuffer): create 256×256 BGRA `CVPixelBuffer`, fill black first
  (recycled buffer memory), then **squash-resize** the CGImage to the square (no aspect crop), high
  interpolation. Output feature key `"embedding"`; reads float32 fast path else element-by-element.
- **Text:** tokenize → clip to 64 → right-pad with token id 0 (no attention mask; must match Python
  SigLIP reference exactly). Feed as `MLMultiArray [1,64] int32` under input key `"tokens"`.
- **`.embed` disk format** (EmbeddingStore): `magic "PALMEMB1"` (8B) + `UInt32 LE json length` +
  JSON header + `count` rows. Each row = 3× Float64 (`time, shotStart, shotEnd`) + `dim`× **Float16**
  embedding values. Total file = `magic+4 + jsonLen + count*(24 + dim*2)`. Written atomically.
  Loaded vectors are widened to Float32 (for BLAS). Header is read cheaply via FileHandle prefix.
- **Cache key (file identity):** `SHA256("<path>|<mtime epoch>|<size>")` hex, first 32 chars. Any
  file edit (mtime/size change) ⇒ new key ⇒ re-index naturally. Stored under
  `Caches/<subsystem>/Embeddings/<key>.embed`.
- **Model loader states:** `unknown → notInstalled | preparing → ready | downloading(frac) | failed`.
  `prepare()` loads an installed model but never downloads; `download()` fetches + compiles +
  installs then loads. On load it runs `encode(text:"warm up")` to warm the model, then sweeps all coordinators.

### Query path (visual)
- `SearchIndexCoordinator.search(query, limit=20, within ids?)`: trim query (empty ⇒ []). Snapshot
  candidate (video|image) assets on main, then off-actor (`Task.detached(.userInitiated)`): for each
  candidate load its `AssetIndex` (from in-memory `loadedIndexes` if key matches, else disk), encode
  query text → 768 vec, rank.
- `VisualSearch.search(query, indexes, limit=20, relativeCutoff=0.85, minScore=0.05)`:
  - Per asset: `scores = vectors(count×dim) · query` via `cblas_sgemv(RowMajor, NoTrans)`. (Vectors are
    pre-normalized by the model, so dot ≈ cosine.)
  - **Best-per-shot dedupe:** keep only the highest-scoring frame per `shotStart` so one scene can't
    flood results. Emit a Hit per surviving shot.
  - Sort desc by score; drop `< minScore (0.05)`; require top > 0; keep `prefix(limit)` then filter
    to `score >= top * 0.85` (relative cutoff). Returns `[Hit]`.

### Transcript path (spoken)
- Transcription engine (`Transcription.transcribe`): **Apple `Speech` `SpeechTranscriber` +
  `SpeechAnalyzer`** (on-device). Picks best supported locale from preferred languages; auto-installs
  the locale model via `AssetInventory.assetInstallationRequest`. For video, extracts audio first via
  `AVAssetReader` → 16 kHz mono PCM16 `.caf`. Produces `TranscriptionResult{text, language, words[], segments[]}`;
  each `segment` is one endpointed utterance (`text,start,end`); words carry per-token audio time ranges.
- `TranscriptCache` (actor): disk JSON at `Caches/<subsystem>/Transcripts/<key>.json` (same identity
  key scheme as embeddings) + in-memory LRU (max 4, cleared wholesale when full). Only **full**
  transcripts cached; windowed requests filter a cached full transcript by range.
- `TranscriptSearch.search(query, assets, limit=20)`: split query into terms (strip edge punctuation,
  drop empties); a segment matches iff it contains **all** terms (`.caseInsensitive, .diacriticInsensitive`).
  Returns `Hit{assetID, start, end, text}` in asset/segment order, capped at limit. Reads disk-only
  (`cachedOnDisk`) — no transcription triggered at query time.

### Coordinator queue + scheduling
- `schedule(asset)`: requires model enabled+embedder ready, asset not generating, not already queued/failed.
  Enqueues if `needsVisual` (video/image needing index) **or** `needsTranscript`. `wantsTranscript` =
  audio, or video with audio. `needsTranscript` = wants it AND no cached transcript on disk.
- Worker dequeues sequentially; `indexOne` runs transcript (async let) + visual concurrently. Progress
  split: if transcribing, visual counts for 0.5 of the asset's fraction else 1.0.
- Export pause is a process-global refcounted counter (`ExportPauseCounter`); indexing sleeps in 2s
  loops while `exportActive`. `workerGeneration` guards against a stale worker clobbering a newer one.
- App-level fan-out via a weak registry: `sweepAll`, `cancelAll`, `resetAll`, `clearIndexGlobally`
  (also `EmbeddingStore.clearAll()`). Coordinator `loadedIndexes[id]` is cleared when an asset re-indexes.

### UI result navigation (MediaTab+Search)
- Search debounced 250ms (`scheduleMomentSearch`); runs spoken (sync) + visual (async) into
  `visualHits` / `spokenHits`. Three collapsible sections: Moments (frame grid), Spoken (rows), Files
  (name match — separate plain-string filter, not in this subsystem).
- Moment card: thumbnail at `hit.time` (AVAssetImageGenerator, 240px, 1s tolerance), label, timecode
  `shotStart–shotEnd`. Tap → `selectMediaAsset(asset, atSourceFrame: secondsToFrame(shotStart, fps))`.
  Draggable payload = asset id + source segment `[shotStart, max(shotEnd, shotStart+0.1)]` (stills drag
  as plain asset — no segment).
- Spoken row: thumbnail at `hit.start`, transcript text (3 lines), `name · timecode`. Tap → seek to
  `start`. Draggable with segment `[start, max(end, start+0.1)]`.

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- **CoreML (`MLModel`, `MLModelConfiguration.computeUnits=.all`, `MLMultiArray`, mlpackage→mlmodelc
  compile)** for SigLIP image+text encoders → **`candle` or `ort` (ONNX Runtime)** per FOUNDATION L92.
  Need ONNX (or candle safetensors) SigLIP2 weights instead of the CoreML `.mlpackage`; the hosted
  `palmier-io/siglip2-base-coreml` repo is CoreML-specific — port must source/convert ONNX weights and
  publish a new manifest (different SHA256/sizes). DirectML/CUDA/CPU execution providers via `ort`.
- **Apple `Speech` (`SpeechTranscriber`, `SpeechAnalyzer`, `AssetInventory`)** → **whisper.cpp via
  `whisper-rs`** (CPU or CUDA/Vulkan/DirectML) per FOUNDATION L91. Must reproduce segment endpointing +
  per-word time ranges (whisper gives segments + word timestamps with `--max-len`/token timestamps).
- **`AVAssetImageGenerator` (frame sampling + thumbnails)** → **FFmpeg seek+decode+scale** (project
  already standardizes on FFmpeg for decode, FOUNDATION §6.5/§6.2). Replicate "nearest sync frame within
  tolerance" by seeking to candidate time and taking the decoded keyframe/nearest frame.
- **`AVURLAsset` / `AVAssetReader` (audio extraction to 16kHz mono PCM)** → **FFmpeg decode + `symphonia`
  / resample (`rubato`)** to 16 kHz mono f32/i16 for whisper.
- **`CGImage` / `CVPixelBuffer` / `CGContext` (squash-resize to BGRA square, black fill)** → CPU image
  resize (e.g. `image`/`fast_image_resize` crate) or wgpu compute; reproduce squash-to-square (no crop)
  + black background + sRGB.
- **`Accelerate` `cblas_sgemv`** → **`ndarray`+BLAS / matrixmultiply / `candle` matmul** for the
  count×768 · 768 dot product. Or wgpu compute (FOUNDATION L74 mentions cross-platform compute for visual search).
- **CryptoKit `SHA256`** → **`sha2` crate**. **`Float16`** row storage → `half::f16` crate.
- **HuggingFace `Tokenizers` (`AutoTokenizer`)** → **`tokenizers` crate** (FOUNDATION L93). Must keep
  pad-to-64 with id 0, no attention mask.
- **`/usr/bin/ditto` unzip** → **`zip` crate**. **`URLSession` download w/ progress** → **`reqwest`**
  streaming with byte-progress. **`UserDefaults`** → app settings store. **Caches/ApplicationSupport
  dirs** → `%APPDATA%\PalmierProWin\Cache\{embeddings,transcripts}` + models dir (`directories` crate).

## Mapping to FOUNDATION crates (palmier-search)
- `palmier-search` (FOUNDATION L140: "CLIP frame index + transcript full-text"):
  FrameSampler, VisualIndexer, EmbeddingStore (`.embed` reader/writer), VisualSearch ranking,
  VisualEmbedder (candle/ort), ModelDownloader, TextTokenizer, the coordinator queue, and
  TranscriptSearch keyword index. Transcript *generation* belongs to **`palmier-transcribe`**
  (FOUNDATION L139, whisper.cpp wrapper); `palmier-search` consumes its cached `TranscriptionResult`.
- Model assets live with `palmier-model` (`MediaAsset` type/url/duration drive scheduling).
- Visual encode/dot may run on the **wgpu** compositor stack (FOUNDATION L74) instead of BLAS.

## Port risks & gotchas
- **Embedding parity:** SigLIP CoreML vs candle/ort ONNX must produce equivalent (ideally identical-
  enough) normalized vectors, or the `0.05` cosine floor and `0.85` relative cutoff need recalibration.
  Preprocessing must match exactly: 256×256 squash (no crop), black fill, sRGB, BGRA byte order, plus
  identical tokenizer + pad-to-64-with-0/no-mask. Any drift changes which moments surface.
- **Vectors assumed pre-normalized:** ranking uses a raw dot product (sgemv) with no normalization in
  the search path. The model must output L2-normalized embeddings; if the ONNX export does not, add an
  explicit normalize step at index time or scores/cutoffs break.
- **`.embed` binary format is load-bearing** (magic `PALMEMB1`, LE u32 json len, Float64 times +
  Float16 vectors). Reproduce byte-exactly only if you want cross-build cache reuse; otherwise pick a
  new magic and re-index. Float16 round-trip introduces ~1e-3 error — match the reference's tolerance.
- **Cache key identity** (`path|mtime|size` → SHA256[:32]) ties indexes to absolute file paths; moving
  a file re-indexes. Keep scheme or accept full re-index on first run.
- **Transcription engine swap is the biggest behavioral divergence:** Apple Speech endpointing,
  punctuation/casing, and locale auto-selection differ from whisper. Spoken-search hits (segment
  boundaries + text) will not match the reference 1:1; FOUNDATION L91 explicitly accepts whisper "for
  parity" but parity is approximate. Whisper needs explicit language selection + word timestamps.
- **Export-pause coupling:** indexing must pause during export (CPU/GPU contention). Reproduce the
  process-global refcount; FFmpeg+wgpu export will contend for the same GPU as candle/ort+wgpu encode.
- **Best-per-shot dedupe** depends on `shotStart` equality as the bucket key (Float64). Image stills use
  shotStart 0; keep that or the dedupe map collides differently.
- **Concurrency:** reference indexes one asset at a time (single utility worker) but runs visual +
  transcript concurrently per asset. Don't over-parallelize on Windows — whisper + ort + FFmpeg already
  saturate cores/GPU.

## Open questions
- Exact ONNX SigLIP2 source: convert `palmier-io/siglip2-base-coreml`, use an upstream HF SigLIP2 ONNX
  export, or candle-native weights? Determines the new download manifest + whether embeddings stay compatible.
- Does the port keep CoreML's `.computeUnits = .all` intent (ANE/GPU) as ort GPU EP (DirectML/CUDA) with
  CPU fallback, and is per-frame embed latency acceptable on CPU-only Windows machines?
- Whisper model size/quantization choice (tiny/base/small) and language strategy (auto-detect vs locale)
  to approximate Apple Speech quality + speed — not specified in FOUNDATION.
- Whether visual encode/rank should run on wgpu (FOUNDATION L74) vs candle/ort CPU, and where the
  256×256 squash-resize happens (CPU image crate vs GPU).
- FOUNDATION leaves CLIP-vs-SigLIP terminology loose (calls it "CLIP" at L92/L424 but reference is
  SigLIP2). Note: it is **SigLIP2 base patch16-256, 768-dim**, not OpenAI CLIP — embeddings are not interchangeable.
