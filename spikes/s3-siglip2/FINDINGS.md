# Spike S-3 — SigLIP2 visual-search encoder in Rust: FINDINGS & DECISION RECORD

**Spike:** S-3 (PRD section 11). De-risks **M4 / Epic 11** (palmier-search visual search).
**Question:** can the reference's **SigLIP2** model (base patch16-256, 768-dim — NOT
OpenAI CLIP, per ruling #13) run in Rust (candle or ort) and produce embeddings
that reproduce the reference's .embed index within cosine tolerance?

**Status:** **RESOLVED for the build/format/parity surface; live encode pending a model
download.** The parity-critical Rust code (preprocessing, tokenizer, L2-normalize,
raw-dot ranking with the 0.05/0.85 cutoffs, and the .embed/PALMEMB1 byte format)
is implemented, **compiles, and passes 19 tests** with NO model and NO heavy runtime.
The ort (ONNX Runtime) encode path **type-checks against ort 2.0.0-rc.10** behind
--features ort. A real image+text encode (the siglip_encode bin) is wired and
ready but not yet run here — it needs the ~hundreds-of-MB ONNX weights downloaded.

---

## 1. Verdict

> **Runtime: ort (ONNX Runtime 2.0).** **Weights:
> onnx-community/siglip2-base-patch16-256-ONNX** (vision_model.onnx +
> text_model.onnx + the Gemma tokenizer.json). **candle is the documented
> fallback, not the pick** — candle ships SigLIP1; SigLIP2 is an OPEN, unmerged PR
> (huggingface/candle #3510), so candle would mean hand-porting/maintaining the model
> graph. ort consumes a ready, community-maintained export with a one-call DirectML
> (Windows GPU) + automatic CPU fallback.
>
> **Embeddings should reproduce the reference cutoffs (0.05 / 0.85) — with one
> mandatory fix:** the ONNX pooler_output is **NOT L2-normalized** by the graph (the
> reference CoreML embedding output IS). The port **must** L2-normalize after the
> encode (done in embed.rs). Given that, dot==cosine and the reference floors hold,
> modulo f16 (~1e-3) and a preprocessing-resampler nuance (section 6) — **expect parity,
> plan a one-off cutoff sanity check, not a re-tune.**
>
> **.embed format: reproduce PALMEMB1 byte-exactly** (done) so a macOS-built cache is
> reusable, BUT the **embeddings themselves will differ** (CoreML 8-bit palettized vs
> ONNX fp16/fp32 weights), so cross-OS *index* reuse is NOT semantically safe -> **bump
> modelVersion to force a clean re-index on the port** while keeping the format.

---

## 2. The weight source

**Reference (macOS):** palmier-io/siglip2-base-coreml — CoreML .mlpackage zips,
**8-bit palettized**, Apache-2.0. Derived from google/siglip2-base-patch16-256.
CoreML-only; **cannot** be reused by the port (ruling #13).

**Port (recommended): onnx-community/siglip2-base-patch16-256-ONNX**
(https://huggingface.co/onnx-community/siglip2-base-patch16-256-ONNX)
- License: **Apache-2.0** (from the google/siglip2-base-patch16-256 base —
  GPLv3-compatible, fine to bundle/redistribute). 16.7K downloads, by onnx-community.
- **Split encoders** (matches the reference two-encoder design exactly):
  - onnx/vision_model.onnx — in pixel_values [1,3,256,256] f32 -> out pooler_output [1,768] f32
  - onnx/text_model.onnx   — in input_ids [1,64] i64           -> out pooler_output [1,768] f32
- **Variants and sizes** (pick per perf/parity — see section 3):

  | file | size | use |
  |---|---|---|
  | vision_model.onnx (fp32) | 372 MB | max parity, CPU/GPU |
  | vision_model_fp16.onnx   | 186 MB | GPU (DirectML) — recommended |
  | vision_model_q4f16.onnx  | 54.7 MB | smallest, accuracy risk |
  | text_model.onnx (fp32)   | 1,130 MB | max parity |
  | text_model_fp16.onnx     | 565 MB | GPU — recommended |
  | text_model_q4f16.onnx    | 443 MB | smaller |

  (There are also fused model*.onnx combined files; we want the **split** vision/text
  pair to mirror the reference two encode() entry points.)
- **Tokenizer** (repo root): tokenizer.json (34.4 MB), tokenizer.model (4.24 MB
  SentencePiece), special_tokens_map.json, tokenizer_config.json.
  **GemmaTokenizer**, pad <pad>=0, eos <eos>=1, add_eos_token=true,
  add_bos_token=false, do_lower_case=true. The tokenizers crate reads the SAME
  tokenizer.json the reference AutoTokenizer reads -> token-id parity by construction.
- **Preprocessing** (preprocessor_config.json): resize **256x256** (resample=2 =
  bilinear), rescale 1/255, normalize mean/std **0.5/0.5** => pixels in **[-1, 1]**,
  no center crop. Matches the reference squash-resize + the CoreML "[-1,1]" note.

**SHA256 / exact bytes:** computed at download time (the OnnxManifest in
src/manifest.rs carries <fill-from-download> placeholders; fill them when the
fp16-vs-fp32 choice is locked, exactly as the reference manifest does for its zips).
**Action for E11:** either (a) re-host the chosen *.onnx + tokenizer.json under a
palmier-io/...-onnx repo (mirrors the reference self-hosting), or (b) pull from
onnx-community at a **pinned revision** + verify SHA256 (reference ModelDownloader.verify).

**Cross-check:** khasinski/siglip2-rb (https://github.com/khasinski/siglip2-rb) is a
working library that runs **this exact ONNX repo** — confirming our contract:
input pixel_values/input_ids, output pooler_output, **explicit L2-normalize in
code** before similarity, pad-to-64 id 0, lowercase + Gemma tokenizer, no position_ids.

---

## 3. Runtime recommendation — ort over candle (and GPU vs CPU)

| | **ort (ONNX Runtime 2.0)** [PICK] | candle (pure Rust) |
|---|---|---|
| SigLIP2 base patch16-256 today | **ready** (onnx-community export) | **not landed** — SigLIP1 only; SigLIP2 open PR #3510 |
| Effort | load 2 .onnx + tokenizer; done | hand-port the model graph + safetensors map; track upstream |
| Windows GPU | **DirectML EP** (DX12, AMD/NVIDIA/Intel) one call | wgpu/CUDA — no DirectML; AMD GPU on Win weak |
| CPU fallback | **automatic** (EP list falls through) | CPU works but is the only easy Win path |
| Risk | rc API churn (pin =2.0.0-rc.x) + ship onnxruntime.dll | model-correctness + maintenance |

**Pick ort.** Matches the reference two-encoder shape, the export already exists and
is exercised by a third party, and DirectML gives this **AMD/Windows** box GPU accel with
a one-line CPU fallback — the closest analogue to the reference MLComputeUnits.all.

**Pinned set (verified to type-check here):** ort = "=2.0.0-rc.10" with features
["ndarray","directml","load-dynamic"], ndarray = "0.16". load-dynamic loads
onnxruntime.dll at runtime via ORT_DYLIB_PATH (avoids static-link pain on Windows;
the DLL ships next to the .exe like the FFmpeg DLLs — see windows-harness-notes).

**GPU vs CPU embed latency (to MEASURE in E11 on this AMD box):**
- Index time is **per-frame**: a 25-min video at the sampler cadence emits tens to low-
  hundreds of frames/asset (shot-gated + 8s coverage floor), each a single 256x256
  vision forward (~94M-param vision tower). CPU is tolerable but not instant; **DirectML
  fp16 should give a multi-x speedup** -> recommended default, **CPU fp32 fallback**.
- Query time is **one text forward** (375M text tower dominates; text_model.onnx is
  1.1 GB fp32 / 565 MB fp16). At 250ms search debounce a CPU text forward may bottleneck;
  **fp16+GPU or caching the query embedding** matters more here than for the image side.
- **Export contention:** indexing must pause during export (ExportPauseCounter);
  FFmpeg+wgpu export and ort+DirectML encode share the GPU — keep the single-worker,
  one-asset-at-a-time model + the export-pause refcount.

---

## 4. Preprocessing + tokenizer spec (implemented and tested)

**Image (preprocess.rs):**
1. **squash-resize** to 256x256, **no aspect crop** (image::resize_exact, bilinear),
   over an **opaque black** background (alpha blends to black) — reproduces the reference
   CGContext fill-black-then-draw-into-square.
2. normalize to **[-1, 1]**: (x/255 - 0.5)/0.5 == x/127.5 - 1.0.
3. emit **channel-first** RGB [1,3,256,256] f32 (ONNX pixel_values).
   Reference diff: the CoreML buffer is **BGRA**; ONNX wants **RGB CHW**. Same
   geometry/values, different channel order/layout — handled here, not a parity risk
   (model trained on RGB; CoreML ImageType does the BGRA->model mapping).
   Tested: white->+1.0, black->-1.0, mid-grey->~0, exact-square no-crop.

**Text (tokenize.rs):**
- load tokenizer.json with the tokenizers crate (same file as the reference
  AutoTokenizer); encode(text, add_special_tokens=true) so the post-processor appends
  <eos> and the normalizer lowercases — matching add_eos_token/do_lower_case.
- **truncate to 64, right-pad with id 0, NO attention mask** (reference
  TextTokenizer.swift). Emit i64 for ONNX input_ids.
  Tested: pads to 64 with 0, truncates, empty->all-pad.

**Embedding (embed.rs): L2-normalize the pooler_output** — the load-bearing fix
(section 1). Ranking then does a raw dot product == cosine.

---

## 5. Ranking + .embed format (implemented and tested)

**Ranking (rank.rs)** mirrors VisualSearch.search exactly: per-asset raw dot
product, **best-per-shot dedupe** (highest score per shotStart, first-seen on tie),
sort desc, drop < minScore (0.05), require top>0, prefix(limit) then keep
score >= top*0.85. Tested: relative cutoff, best-per-shot, cosine floor.

**.embed / PALMEMB1 (store.rs)** reproduces EmbeddingStore **byte-exactly**:
"PALMEMB1" + u32 LE jsonLen + JSON Header{model,modelVersion,samplerVersion,dim,count}
(camelCase, reference field order) + count rows of f64x3 (time,shotStart,shotEnd) LE
+ dim x f16 LE. Total = 12 + jsonLen + count*(24 + dim*2). Apple is LE and
Float16==half::f16 bitwise, so a file written here is identical to a macOS-written
one **given the same header JSON bytes**. Tested: round-trip, exact byte-size formula,
header key order, prefix-only header read, the cache-key SHA256[:32] identity.

**Cache key (store::cache_key)**: SHA256("<path>|<mtime epoch>|<size>")[:32],
matching EmbeddingStore.key. Windows FS mtime is coarser than macOS -> may false-HIT
(serve a stale index), not false-miss; carry the ruling-#16 watch.

---

## 6. Will embeddings match the reference cutoffs (0.05 / 0.85)?

**Expected: yes, with the L2-normalize fix — but verify once with a real encode.**
Sources of drift between CoreML and ONNX, all bounded:
- **Weights:** reference is **8-bit palettized** CoreML; ONNX fp16/fp32 differs slightly.
  The CoreML card claims **cosine >=0.99 vs the PyTorch original**; the ONNX export is
  also from the same Google checkpoint, so CoreML-vs-ONNX should likewise be >=~0.99 —
  inside the 0.05 floor / 0.85 relative-cutoff margins. Not interchangeable enough to
  reuse a macOS index, but enough that the **cutoffs do not need re-tuning**.
- **Resampler:** reference uses CG "high" interpolation; we use bilinear (the model-card
  resample). Sub-pixel diffs -> negligible drift. (Production: fast_image_resize.)
- **f16 storage:** ~1e-3 round-trip error (reference already accepts this).

**Plan:** run siglip_encode on a few known image/caption pairs; confirm (a) the matching
caption clearly wins and (b) absolute scores land where 0.05/0.85 expect. If scores are
systematically offset, the only knob is COSINE_FLOOR (not 0.85, which is relative) —
record any change. **Do NOT re-tune pre-emptively.**

---

## 7. The .embed index decision

- **Keep the PALMEMB1 byte format** (reproduced + tested) — cheap, keeps the door open
  to reading a macOS-authored cache header.
- **But force a re-index on the port:** ONNX embeddings are not bit-equivalent to the
  CoreML ones, so a macOS .embed vectors are invalid for ONNX-side queries. Set the
  port **modelVersion = 2** (or a distinct model id) so isCurrent() treats any
  macOS index as stale and rebuilds. Format-compat (header readable) without
  semantic-mismatch (vectors rebuilt): **same magic, bumped modelVersion, re-index on
  first run** — the lowest-risk choice.

---

## 8. What is proven vs what needs a real encode

**Proven here (compiles + 19 tests pass, no model, no GPU):**
- image preprocessing (squash 256^2, no crop, black fill, [-1,1], CHW)
- tokenizer pad-to-64/id-0/no-mask via the real tokenizers crate
- L2-normalize + raw-dot ranking with best-per-shot + 0.05/0.85 cutoffs
- PALMEMB1 .embed reader/writer byte-exact + cache-key identity
- the ort encode path **type-checks** against ort 2.0.0-rc.10 (--features ort)

**NOT yet proven (the live-confirmation step):**
- a **real SigLIP2 ONNX encode** (image+text->768 vec) and the resulting **cosine
  numbers** vs the reference. Blocked only on downloading vision_model_fp16.onnx
  (186 MB) + text_model_fp16.onnx (565 MB) + tokenizer.json + an onnxruntime.dll.
- **DirectML actually engaging** on this AMD box (vs CPU fallback) + measured latency.

**To run the live confirmation** (from spikes/s3-siglip2/):

    # download vision_model.onnx, text_model.onnx, tokenizer.json into a dir, set:
    $env:SIGLIP_MODEL_DIR = "C:\models\siglip2-onnx"
    $env:ORT_DYLIB_PATH   = "C:\onnxruntime\onnxruntime.dll"
    pwsh -File ..\..\scripts\with-msvc.ps1 cargo run --features ort --bin siglip_encode -- cat.jpg "a cat" "a car"
    # expect cosine(image,"a cat") much greater than cosine(image,"a car")

---

## 9. The Epic 11 implementation plan

1. **Model acquisition + manifest** — choose fp16 (GPU) / fp32 (parity), compute
   SHA256/bytes, fill OnnxManifest (re-host under palmier-io OR pin onnx-community
   revision). Port ModelDownloader to Rust: reqwest streaming + sha2 verify +
   install under %APPDATA%\PalmierProWin\Models\<model>-v<version>\ (no .mlpackage
   compile step — ONNX needs none; just place the 2 .onnx + tokenizer.json).
2. **palmier-search crate (FOUNDATION L148):** lift preprocess/tokenize/embed/
   rank/store (PALMEMB1) modules from this spike. Add the ort VisualEmbedder
   (onnx.rs) behind a small trait so a candle backend could swap in if #3510 lands.
3. **DirectML + CPU fallback** wired (done in onnx.rs); ship onnxruntime.dll as a
   Tauri resource next to the .exe (set ORT_DYLIB_PATH); Apache-2.0 + ONNX Runtime
   MIT in third-party notices.
4. **FrameSampler** (FFmpeg seek+decode+scale, per search.md "macOS APIs to replace")
   feeding the embedder; reproduce shot detection (8x8 luma grid, promoteDiff 12,
   coverageFloor 8s) + the keep rule. (Separate; FFmpeg toolchain already resolved.)
5. **Coordinator queue** (single utility worker, one asset at a time, transcript+visual
   concurrent per asset, export-pause refcount) — search.md "Coordinator queue".
6. **Set modelVersion = 2** (section 7) so first run re-indexes; verify 0.05/0.85 against
   a real encode (section 6) before locking.
7. **(perf) Ranking** can stay a Rust dot product (matrixmultiply/ndarray) or move to
   wgpu (FOUNDATION L74) — defer to a measurement; the encoder, not the dot, is the cost.

---

## 10. What the orchestrator must decide before Epic 11

1. **fp16 vs fp32 ONNX variant** — fp16 (GPU, smaller, recommended) vs fp32 (max parity).
   Drives the manifest + the section-6 cutoff check. (Recommend: **fp16 default, fp32 available**.)
2. **Host strategy** — re-host the chosen ONNX files under palmier-io (control, matches
   reference) vs pin onnx-community + verify SHA. (Recommend: **re-host for v1**.)
3. **onnxruntime.dll packaging** — confirm shipping the ORT runtime DLL (+ DirectML
   helper dll) as a Tauri resource (the ort load-dynamic model; parallels the FFmpeg
   DLLs). MIT, easy.
4. **.embed modelVersion bump** — confirm section 7 (keep PALMEMB1 magic, set
   modelVersion=2, re-index; do NOT reuse macOS index vectors).
5. **Run the live encode** (section 8) once weights are downloadable on the build box, to
   turn "expected parity" into "measured parity" and lock COSINE_FLOOR.

---

## 11. Files

- Cargo.toml — standalone (own [workspace]); default = parity core, --features ort = real encode.
- src/lib.rs — module map + the spec constants (the parity contract).
- src/preprocess.rs — squash-256^2 + black fill + [-1,1] CHW. (tested)
- src/tokenize.rs — Gemma tokenizer, pad-64/id-0/no-mask. (tested)
- src/embed.rs — **L2-normalize** (the load-bearing fix) + cosine. (tested)
- src/rank.rs — VisualSearch.search parity (best-per-shot, 0.05/0.85). (tested)
- src/store.rs — PALMEMB1 .embed reader/writer + cache key. (tested)
- src/manifest.rs — the new ONNX download manifest shape. (tested)
- src/onnx.rs — the ort SigLIP2 encoder (DirectML+CPU). (type-checks under --features ort)
- src/bin/siglip_encode.rs — live encode + cosine sanity (needs weights).
- tests/pipeline.rs — full preprocess->encode(synthetic)->store->rank round-trip. (passes)

**Build/test (Windows, from this dir):**

    pwsh -File ..\..\scripts\with-msvc.ps1 cargo test                  # 19 tests, no model
    pwsh -File ..\..\scripts\with-msvc.ps1 cargo check --features ort  # ort path type-checks