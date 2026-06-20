---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
---

# Epic 10 — Transcription & Captions

## Epic goal

Deliver the speech-to-caption pipeline as a clean-room Windows/Linux port with **behavior parity**
to the macOS reference. Two engine-independent layers ship here:
1. **Transcription** (`palmier-transcribe`): FFmpeg audio extraction → whisper.cpp (via `whisper-rs`)
   → a `TranscriptionResult` of words + segments timestamped in *source seconds*, with locale
   resolution, profanity censoring, range-extract+offset, and a disk+memory transcript cache.
2. **CaptionBuilder** (`palmier-text`): the phrase-split/timing algorithm that turns a segment into
   screen-ready, timed phrases, plus `specs(...)` mapping each phrase through a clip's trim/speed into
   `TextClipSpec` records — and the editor-side orchestration (`generateCaptions`) that selects
   targets, assigns phrases to clips, applies casing, and places a caption track.

It also wires the agent-facing surface: the `add_captions` and `get_transcript` tools in
`palmier-tools`, and the **transcript-driven cut** (UJ-1 climax: `get_transcript` → identify
dead-air/filler in source seconds → convert to project frames → `ripple_delete_ranges`).

## PRD acceptance this epic must satisfy (§4.10 / §10 Epic 10)

- **FR-36 Transcription:** extract audio (FFmpeg → 16 kHz mono 16-bit PCM) → Whisper → word + segment
  timestamps; bundle `small.en`, offer `medium.en`/`large-v3` as downloads (OQ-4); cache key
  `sha256(content)+model+language` (**ruling #19**, hash first N MB if 25-min hashing is slow).
  **SM-9:** a 25-min recording transcribes **< 2 min on RTX 4060 CUDA with `small.en`**.
- **FR-37 CaptionBuilder:** split → distribute time by character count → enforce min duration **0.7 s**
  (cascade) → map to timeline frames through trim/speed → emit `TextClipSpec`. **Port the 14 reference
  unit tests verbatim** (grapheme-aware counts; `enforceMinDuration` may push the final phrase end past
  the segment end; `breakOn` delimiter+space so "U.S."/"3.14" don't split). Caption case
  **auto/upper/lower** only (**ruling #18** — no title-case). **SM-13:** all **14** CaptionBuilder tests
  pass byte/timing-exact against committed expected `TextClipSpec` fixtures; regeneration is gated behind
  `--update-golden` review and **any diff in CI blocks merge** (R-5, mirrors SM-7's XMEML treatment).
- **FR-38 Transcript-driven cut:** agent `get_transcript` → identify dead-air/filler in source seconds →
  convert to project frames via placement/trim/speed → `ripple_delete_ranges` (realizes UJ-1's climax).
- **Frame-rounding parity (carry-forward):** every source↔timeline conversion uses `f64::round`
  ties-**away**-from-zero (never `round_ties_even`); speed floored at `0.0001` everywhere.

**Governed by:** `docs/reference/transcription.md`; FOUNDATION §6.9. **Milestone: M3** (PRD §12 —
"Generation + Transcription"). Realizes **UJ-1**.

## Spike / gating note

Epic 10 itself is **not** spike-gated on a presentation mechanism (that is Epic 5 / Spike S-1). Its M3
sibling Epic 9 (generation) is gated by **Spike S-2** (Convex HTTP+WS), but transcription and captions
need **no Convex backend** — they run fully local (whisper.cpp + FFmpeg). Therefore Epic 10 can land its
core (E10-S1..S6) as soon as its dependency epics are done, independent of S-2. The only cross-epic
prerequisites are real-code dependencies, not spikes:
- **Epic 2** (`palmier-model`): `TextClipSpec`, `Transform`, `Clip`, `Clip.timelineFrame`, keyframe model.
- **Epic 4** (`palmier-media`): FFmpeg decode plumbing for audio extraction; caches/dirs.
- **Epic 3** (`palmier-edit`): `ripple_delete_ranges` engine for the transcript-driven cut (FR-38).
- **Epic 7** (`palmier-tools` + MCP dispatch): the tool-registration surface for `add_captions` /
  `get_transcript` (the tool *shells* are surfaced in M2/Epic 7; this epic supplies their real bodies).

> **Model artifact prerequisite:** `small.en` must be bundled/installed before E10-S2 can hit SM-9.
> `medium.en`/`large-v3` are optional downloads (OQ-4) and out of scope as bundled defaults.

---

## Stories

### E10-S1 — `palmier-transcribe` crate scaffold: result model + `offsetting` + error enum

**Intent:** As a dev agent, I want the engine-independent `TranscriptionResult` data model, its
`offsetting(by:)` shift, and the error taxonomy in place, so the Whisper engine and cache can build on a
parity-exact foundation.

**Acceptance criteria:**
- Define, in `palmier-transcribe`, exactly the FOUNDATION §6.9 / reference shapes:
  - `TranscriptionWord { text: String, start: Option<f64>, end: Option<f64> }` — **timestamps are
    `Option<f64>`** (nullable per reference `Transcription.swift`; many paths skip `None` words — keep
    optional to match `filter`/`spokenWordCount` semantics). FOUNDATION's non-optional `f64` is the
    common case; the **reference optionality wins** (parity authority) — document the deviation inline.
  - `TranscriptionSegment { text: String, start: f64, end: f64 }` (one endpointed utterance).
  - `TranscriptionResult { text: String, language: Option<String>, words: Vec<TranscriptionWord>,
    segments: Vec<TranscriptionSegment> }`.
- `TranscriptionResult::offsetting(&self, by: f64) -> TranscriptionResult` adds `by` to **every** word
  and segment `start`/`end` (skipping `None` word timestamps); **no-op when `by == 0.0`** (assert
  identity round-trip in a unit test). Times are plain `f64` seconds (reference `CMTime` @ ts 600 → `f64`).
- `TranscriptionError` enum mirrors reference variants with user-facing strings:
  `UnsupportedLocale, ModelInstallFailed, DecodeFailed, AudioExtractionFailed, AnalysisFailed`.
- Serde `Serialize`/`Deserialize` derive on `TranscriptionResult` (used by the JSON disk cache in
  E10-S4); a unit test round-trips a populated result (incl. a `None`-timestamped word) byte-identically.

**Implementation context:**
- Crate: `palmier-transcribe` (FOUNDATION §4: "whisper.cpp wrapper, word/segment alignment").
- Reference: `Sources/PalmierPro/Transcription/Transcription.swift` (result model + `offsetting` +
  `TranscriptionError`); docs/reference/transcription.md §"Key types & files" and §B.4.
- FOUNDATION §6.9 struct block (lines 582-586).

**Dependencies:** none (pure data model; can start immediately).
**Parallel-safe?** Yes — owns only `palmier-transcribe` model files; no shared files.

---

### E10-S2 — FFmpeg audio extraction + whisper.cpp engine run (`transcribe`)

**Intent:** As a dev agent, I want `transcribe(...)` to decode a clip's audio to the exact PCM format
Whisper expects and run whisper.cpp, producing word + segment timestamps in source seconds, so the rest
of the pipeline has real transcripts and SM-9 is met.

**Acceptance criteria:**
- **Audio extraction (`extract_audio_track`):** FFmpeg-decode the asset's audio track to a temp PCM file
  in **16 000 Hz, 1 channel (mono), 16-bit, little-endian, non-float, interleaved** (FOUNDATION §6.9;
  matches what Whisper expects). Optional `range: RangeInclusive<f64>` (source seconds) limits the reader
  to `[lower, upper]`. Temp file at `<tempdir>/palmier-stt-<UUID>.{wav}`; **caller deletes it** (RAII /
  `Drop` guard, mirroring the reference `defer`). Extraction failure → `AudioExtractionFailed`.
  **Force** mono/16 kHz on the *output* (reference gotcha: AVAssetReader yields the source channel layout
  before output settings apply — FFmpeg must force, not merely request, the downmix/resample).
- **Engine run (`transcribe(file_url, censor_profanity, locale, range)`):**
  - If `range` given: extract that range, transcribe it, then `result.offsetting(by: range.lower)` to
    shift timestamps back into source time. `transcribe_video_audio` extracts then offsets by
    `range.map(lower).unwrap_or(0.0)`.
  - **Decode results:** for each Whisper segment, append raw text to `full_text`; trimmed non-empty text →
    a `TranscriptionSegment { start, end }`. Walk per-word runs: trim each; skip empty; `start = run.start`,
    `end = run.start + duration`; push `TranscriptionWord`. Set `result.text = full_text.trim()`,
    `result.language = <bcp47 of resolved locale>`. (Whisper provides word-level timestamps directly —
    map 1:1; requires token-timestamps / `--max-len` precision — confirm `whisper-rs` exposes per-word
    `start`/`end`.)
  - Speed floored at `0.0001` is not relevant here (no clip math), but **do not invent** segment merging:
    Whisper segmentation differs from Apple endpointing; segment count/`segments[].text` will *not* match
    the reference and that is expected (only CaptionBuilder's transform of a given segment is parity-tested).
- **Backends:** compile `whisper-rs` with CUDA + Vulkan on Windows, Vulkan on Linux, **CPU fallback**
  (FOUNDATION §6.9). Engine must run on CPU when no GPU backend is available.
- **SM-9 perf gate:** a 25-min recording transcribes **< 2 min on RTX 4060 CUDA with `small.en`** —
  asserted by a benchmark/integration test on the §10 reference HW (CPU-lane variant records its own time,
  not held to the 2-min bar).
- Analysis failure (Whisper init/run error) → `AnalysisFailed`; missing/uninstallable model →
  `ModelInstallFailed`.

**Implementation context:**
- Crate: `palmier-transcribe`. Audio decode reuses FFmpeg plumbing established in `palmier-media`
  (Epic 4) — depend on it, do **not** open a second decode path.
- Reference: `Transcription.swift` — `extractAudioTrack`, `transcribe`, `transcribeVideoAudio`,
  `decodeResults`; docs/reference/transcription.md §A (extraction), §B (engine run), §"macOS APIs to
  replace" (AVAssetReader→FFmpeg, Speech→whisper-rs, CMTime→f64).
- FOUNDATION §6.9; SM-9 perf row (FOUNDATION §10, line 900).

**Dependencies:** E10-S1 (result model); **Epic 4 `palmier-media`** (FFmpeg decode); model artifact
`small.en` bundled (prerequisite above).
**Parallel-safe?** Mostly — owns `palmier-transcribe` engine files; shares a **read-only** dependency on
`palmier-media`'s decode API. Safe to run alongside E10-S5/S6 (different crates) once E10-S1 lands.

---

### E10-S3 — Locale resolution + profanity censoring

**Intent:** As a dev agent, I want "prefer user locale, else auto-detect, else error" resolution and
profanity censoring, so transcription matches the reference's language/etiquette behavior.

**Acceptance criteria:**
- **Locale resolution:** prefer `preferred_locale` if a `match_locale` finds it among supported; else
  fall back to a best-supported locale derived from OS locale (`sys-locale` crate) / config; else
  `UnsupportedLocale`. `match_locale` = first candidate whose **BCP-47 language code** has any supported
  match, **preferring exact region**, else first same-language. For `.en` Whisper models, English only.
  Whisper detects language from audio when no override is given; the override path uses this resolver.
- **Profanity censoring (`censor_profanity == true`):** replace matched words with **bracketed
  equivalents** (reference `.etiquetteReplacements`), or use Whisper token suppression (FOUNDATION §6.9).
  Unit-test that a known profane token is bracketed/suppressed and a clean transcript is unchanged.
- `result.language` is set to the resolved locale's BCP-47 tag.

**Implementation context:**
- Crate: `palmier-transcribe`.
- Reference: `Transcription.swift` — `supportedLocales`, `bestSupportedLocale`, `matchLocale`,
  `.etiquetteReplacements`; docs/reference/transcription.md §B.2/§B.3 + §"macOS APIs to replace".
- `sys-locale` crate for OS locale (replaces `Locale.preferredLanguages`/`Locale.current`).

**Dependencies:** E10-S1.
**Parallel-safe?** Yes within `palmier-transcribe` if S2 and S3 partition files (S3 owns a
`locale.rs`/`profanity.rs` module; S2 owns `engine.rs`). If sharing one engine file, sequence after S2.

---

### E10-S4 — TranscriptCache (disk + memory) with FOUNDATION cache key + windowed filter

**Intent:** As a dev agent, I want a transcript cache keyed by content+model+language that stores only
full transcripts and filters windowed requests, so repeated transcribe calls are fast and never diverge.

**Acceptance criteria:**
- **Cache key = `sha256(file_content) + model_id + language`** (FOUNDATION §6.9, **ruling #19**) — adopt
  FOUNDATION's key, **NOT** the reference `sha256(path|mtime|size)` (mtime false-hits; model/lang must
  invalidate). **Hash first N MB if hashing a 25-min file is slow** (record the chosen N; full-content vs
  first-N is a documented decision). Flag the deviation from the reference key inline.
- **Only full-file transcripts are cached.** A windowed request (`range` given) must **filter a cached
  full transcript** via `filter(result, to: range)` — segments where `end > lo && start < hi`; words with
  **both** timestamps present and the same overlap; `text = segments.joined(" ")` — and **must NOT
  re-transcribe** when a full transcript exists (re-transcribing diverges timestamps and dominant-track
  logic). Unit-test: full transcript cached → windowed call returns the filtered subset, no engine run.
- **Memory cache:** cap **4 entries**; on overflow **clear all** (reference behavior — not LRU eviction).
- **Disk cache:** `<caches>/<subsystem>/Transcripts/<key>.json`, JSON-encoded `TranscriptionResult`
  (uses E10-S1 serde). Caches dir resolved per FOUNDATION logging paths (`%LOCALAPPDATA%` on Windows,
  `~/.cache` on Linux; `dirs` crate).
- **Cache bypass:** if `censor_profanity || locale.is_some()` the orchestration **bypasses the cache**
  (option variants differ); otherwise it goes through `TranscriptCache`. (Bypass decision is enforced at
  the caller in E10-S6; this story exposes a cache that supports being skipped.)

**Implementation context:**
- Crate: `palmier-transcribe`. `actor TranscriptCache.shared` → a `tokio`-friendly cache type
  (e.g. `OnceCell`/`Mutex`-guarded singleton) exposing `transcript(...)` and `filter(result, to:)`.
- Reference: `Transcription/TranscriptCache.swift`; docs/reference/transcription.md §"Cache" + the
  cache-key DISCREPANCY note (adopt FOUNDATION key).
- `sha2`/`ring` for SHA-256; `dirs` for cache dir.

**Dependencies:** E10-S1 (serde model). Independent of S2/S3 (consumes the model, not the engine).
**Parallel-safe?** Yes — owns `cache.rs` in `palmier-transcribe`.

---

### E10-S5 — CaptionBuilder phrase algorithm in `palmier-text` — port 14 tests verbatim (SM-13)

**Intent:** As a dev agent, I want the entire CaptionBuilder phrase split/distribute/min-duration
algorithm ported byte-exact with the 14 reference unit tests, because CaptionBuilder is the parity oracle
and any off-by-one breaks visible caption timing.

**Acceptance criteria:**
- Implement in `palmier-text`, exactly per docs/reference/transcription.md §C:
  - `phrases(for segment, fits, min_duration) = enforce_min_duration(distribute(split(text, fits), start,
    end), min_duration)`.
  - `split(text, fits) -> Vec<String>` (recursive): trim whitespace; empty → `[]`; if `fits(t)` → `[t]`;
    else `parts = break_once(t)`; if `parts.len() <= 1` → `[t]` (an unbreakable over-long single word is
    kept); else `parts.flat_map(|p| split(p, fits))`.
  - `break_once(text)` = first non-`None` of `break_on(".!?")`, then `break_on(",;:")`, then
    `break_at_mid_word`.
  - `break_on(text, delimiters) -> Option<Vec<String>>`: split **after** a delimiter **only when the next
    char is a space or end-of-string** (so "U.S." and "3.14" stay intact); return pieces **only if count > 1**,
    else `None`.
  - `break_at_mid_word(text)`: split on spaces; ≤1 word → `[text]`; else `mid = words.len() / 2` (integer
    div), return `[words[0..mid].join(" "), words[mid..].join(" ")]`.
  - `distribute(texts, start, end) -> Vec<Phrase>`: `total = Σ max(text.count, 1)`; `span = max(end-start,
    0)`; running `t = start`; each `dur = span * max(text.count,1)/total`; phrase `(text, t, t+dur)`;
    `t += dur` (back-to-back).
  - `enforce_min_duration(phrases, min_duration)`: forward pass — if `phrase[i].end - phrase[i].start <
    min_duration` set `phrase[i].end = phrase[i].start + min_duration`; if next exists and
    `phrase[i+1].start < phrase[i].end` then `shift = phrase[i].end - phrase[i+1].start` is added to
    **both** `phrase[i+1].start` and `.end` (cascades; **can push the final end past the segment end —
    do NOT clamp it back**).
- **Grapheme-aware counts:** `text.count` uses Swift `Character` (grapheme clusters). Use
  `unicode-segmentation` so `.count` matches the reference — **not** `char`/scalar or byte counts.
- **Port all 14 `CaptionBuilderTests` verbatim** from
  `Tests/PalmierProTests/Captions/CaptionBuilderTests.swift`, including at minimum the named oracles:
  - `keepsPunctuatedTokensIntact`: "U.S. army here", fits≤6 → `["U.S.","army","here"]`.
  - mid-word split: "a b c d", fits≤3 → `["a b","c d"]`.
  - `distributesTimeByCharacterCount`: "aaaa bb", 0..6 → starts `[0,4]`, ends `[4,6]`.
  - `enforcesMinimumDurationAndShifts`: "aa bbbb", 0..6, minDur 3 → starts `[0,3]`, ends `[3,7]`.
- **SM-13 gate:** the 14 tests pass timing/byte-exact; fixtures regenerated only behind `--update-golden`
  review; **any diff in CI blocks merge** (mirrors SM-7).
- Caption text **case = auto/upper/lower only** (**ruling #18**; reject "title").

**Implementation context:**
- Crate: `palmier-text` (FOUNDATION §4: "Text layout, font registry, caption styling"; FOUNDATION §11.1
  explicitly assigns "caption phrase splitting … duration distribution … minimum-duration cascade" to
  `palmier-text`).
- Reference: `Sources/PalmierPro/MediaPanel/CaptionsTab/CaptionBuilder.swift` (`phrases`, `split`,
  `breakOnce`, `breakOn`, `breakAtMidWord`, `distribute`, `enforceMinDuration`);
  `Tests/PalmierProTests/Captions/CaptionBuilderTests.swift` (port verbatim).
- docs/reference/transcription.md §C; constants `minDisplayDuration = 0.7` (AppTheme.Caption).
- `unicode-segmentation` crate for grapheme counts.

**Dependencies:** none on transcription (algorithm is engine-independent). Lands in `palmier-text`.
**Parallel-safe?** Yes — owns CaptionBuilder files in `palmier-text`; no overlap with `palmier-transcribe`.

---

### E10-S6 — `CaptionBuilder.specs(...)` mapping + `generateCaptions` orchestration + caption track placement

**Intent:** As a dev agent, I want phrases mapped through clip trim/speed into `TextClipSpec` records and
the full caption-generation orchestration (target selection, transcribe, phrase→clip assignment, casing,
track placement) wired into the editor, so a user/agent can generate captions onto the timeline.

**Acceptance criteria:**
- **`specs(...)` mapping** (docs/reference/transcription.md §D), per phrase `p` given `source_clip: Clip`,
  `fps`, `track_index`, `style`, `caption_group_id`, `transform_for: Fn(&str)->Option<Transform>`,
  `min_duration_frames = 1`:
  1. Visible source window (source frames): `vis_start = trim_start_frame`,
     `vis_end = vis_start + duration_frames * max(speed, 0.0001)`.
  2. `p_start = p.start*fps`, `p_end = p.end*fps`; **drop the phrase** unless
     `p_end > vis_start && p_start < vis_end`.
  3. `mapped_start = clip.timeline_frame(p.start, fps)`, `mapped_end = clip.timeline_frame(p.end, fps)`;
     fall back to `clip.start_frame`/`end_frame` when `None`.
  4. `s = mapped_start.unwrap_or(start_frame)`, `e = mapped_end.unwrap_or(end_frame)`;
     `duration_frames = max(min_duration_frames, min(clip.end_frame, e) - max(clip.start_frame, s))`.
  5. Emit `TextClipSpec { track_index, start_frame: s, duration_frames, content: p.text, style,
     transform: transform_for(p.text), caption_group_id }`.
- **`Clip.timeline_frame(source_seconds t, fps)`** (parity, in `palmier-model`/Epic 2 — verify/consume):
  `src_frame = t*fps`; `off = src_frame - trim_start_frame`; `None` if `off < 0`;
  `frame = round(start_frame + off / max(speed, 0.0001))`; `None` unless `start_frame <= frame < end_frame`
  (**half-open** `[start_frame, end_frame)`). **`round` = `f64::round` ties-away** (carry-forward — never
  `round_ties_even`). A unit test covers clamp-past-clip-end, trimmed-in clip, and drop-before-trim.
- **`generateCaptions` orchestration** (docs/reference/transcription.md §E):
  - **Targets** (`captionTargets`): video/audio clips with audio; a video clip linked (`link_group_id`) to
    an audio clip yields to the audio clip (skip the video). Sorted by `start_frame`.
  - **Transcribe** one result per distinct `media_ref`. Range = `visibleSourceUnion` = union of every
    target clip's visible source window for that ref, **padded ±1.0 s, clamped at 0**, in seconds. If
    `censor_profanity || locale.is_some()` → **bypass cache**; else `TranscriptCache.transcript(...)`
    (E10-S4).
  - **autoDetect:** keep only the **dominant speech track** (max spoken word count; a word counts if its
    midpoint falls in the clip's visible window — uses `Option` word timestamps, skip `None`).
  - **Phrase→clip:** `result.segments.flat_map(|s| CaptionBuilder::phrases(s, caption_line_fits, 0.7))`;
    each phrase assigned to the clip with **most overlap**, **only if** overlap > 0 **and**
    `overlap >= phrase_len / 2`.
  - **Casing** applied per phrase (auto/upper/lower — ruling #18). Then `specs(...)` with a shared
    `group_id = UUID()` and a `transform_for` computing each box's `Transform` from natural text size.
  - **`caption_line_fits`:** text natural width ≤ `timeline.width * 0.9`
    (`captionPreviewMaxTextWidthRatio = 0.9`).
  - **Placement:** insert a **new video track at index 0**, place the text clips, register **one undo
    group "Generate Captions"** (the undo-group name must match the reference exactly — agent-undo parity).
- **Constants (AppTheme.Caption):** `minDisplayDuration = 0.7`, `defaultFontSize = 48`, `minFontSize = 12`,
  `maxFontSize = 300`, `minPosition = 0`, `maxPosition = 1`, `centerSnapValue = 0.5`,
  `centerSnapThreshold = 0.02`, `defaultCenter = (0.5, 0.9)`, `captionPreviewMaxTextWidthRatio = 0.9` —
  carry these into `palmier-text`/the caption style config.
- **Transform parity:** emitted `Transform` is **center-based** (`centerX, centerY, width, height,
  rotation`, flips), normalized 0..1 (**ruling #7**); never top-left.

**Implementation context:**
- Crates: `palmier-edit` (owns `specs` + orchestration since they depend on `Clip`/`TextClipSpec`/
  `timeline_frame`), consuming `palmier-text` (CaptionBuilder phrases + `caption_line_fits`/natural size)
  and `palmier-transcribe` (transcribe + cache). `src-ui/media-panel` (Captions tab controls/state).
- Reference: `MediaPanel/CaptionsTab/CaptionBuilder.swift` (`specs`),
  `Editor/ViewModel/EditorViewModel+Captions.swift` (`generateCaptions`, targets, union, dominant track,
  assignment, casing, placement), `MediaPanel/CaptionsTab/CaptionTab.swift` (UI),
  `Models/Timeline.swift` (`Clip.timelineFrame`).
- docs/reference/transcription.md §D, §E, §Constants.

**Dependencies:** E10-S5 (phrases), E10-S2 (transcribe) + E10-S4 (cache); **Epic 2** (`Clip`,
`TextClipSpec`, `Transform`, `Clip.timeline_frame`); **Epic 3** (`palmier-edit` track-insert/undo
machinery); **Epic 4** (media refs/natural-size assets) — and **Epic 5**'s text natural-size layout if
`caption_line_fits` depends on the same `palmier-text` measurement (consume, do not duplicate).
**Parallel-safe?** No — integration story spanning `palmier-edit` + `src-ui/media-panel`; depends on most
siblings. Run after S2/S4/S5 land.

---

### E10-S7 — `add_captions` + `get_transcript` tool bodies in `palmier-tools`

**Intent:** As a dev agent, I want the `add_captions` and `get_transcript` tool bodies implemented in the
single shared `palmier-tools` dispatcher, so both the MCP server and the in-app agent can transcribe and
caption via the same code path.

**Acceptance criteria:**
- **`add_captions`** wraps `generateCaptions` (E10-S6): given a target selection + style/case/locale/
  censor params, transcribes, builds phrases, and places the caption track as one undoable agent action.
  Ported from `Sources/PalmierPro/Agent/Tools/ToolExecutor+Captions.swift`. Case param accepts only
  **auto/upper/lower** and **rejects "title"** (ruling #18 — match the reference rejection).
- **`get_transcript`** returns the cached/fresh `TranscriptionResult` for a media ref, honoring the
  **pagination caps: 400 segments / 10000 words** (carry-forward; do not conflate with the
  `maxFrames ≤ 12` image-frame ceiling — different cap classes). When no transcript exists, it returns
  **empty** and the contract text instructs the agent to transcribe first (UJ-1 edge case — agent must
  not guess cut points).
- Both tools are registered in `palmier-tools` with **verbatim contract-text descriptions** (tool
  descriptions are contract text — port exactly). Exactly **one** implementation per tool name, invoked by
  both the MCP server (Epic 7) and the in-app agent (Epic 8) — no duplication (FOUNDATION §4).
- Each tool has the FOUNDATION §11.1 unit coverage: **happy path + 2 error cases**.

**Implementation context:**
- Crate: `palmier-tools` (consumes `palmier-transcribe` + `palmier-edit` caption orchestration).
- Reference: `Agent/Tools/ToolExecutor+Captions.swift`; docs/reference/transcription.md §"Key types"
  (`add_captions` wrapper); docs/reference/mcp-tools.md for the `get_transcript` pagination caps
  (400 segments / 10000 words) and contract-text rules.
- These two tools are part of the **30-tool surface** (ruling #1); their *shells* are registered in
  Epic 7 (M2) and return "not implemented" until this story supplies the bodies in M3.

**Dependencies:** E10-S6 (`add_captions` body), E10-S2 + E10-S4 (`get_transcript` source); **Epic 7**
(`palmier-tools` dispatcher + tool registration surface).
**Parallel-safe?** Partly — owns the two caption-tool files in `palmier-tools`. Safe alongside other
tool stories if it touches only its own tool modules; depends on S6, so sequence after it.

---

### E10-S8 — Transcript-driven cut path (FR-38) — `get_transcript` → ranges → `ripple_delete_ranges`

**Intent:** As a dev agent, I want the source-seconds→project-frames conversion that lets the agent turn
dead-air/filler ranges from a transcript into a `ripple_delete_ranges` call, so UJ-1's climax (collapse
dead air in one atomic, undoable op) works end to end.

**Acceptance criteria:**
- Provide the conversion that maps **source-seconds ranges** (identified by the agent from
  `get_transcript`) to **project frames** via each affected clip's placement + trim + speed, then calls
  the existing **`ripple_delete_ranges`** engine (Epic 3) to cut and close gaps in one atomic operation.
- Conversion uses the same `Clip.timeline_frame` math (`f64::round` ties-away; speed floor `0.0001`;
  half-open `[start_frame, end_frame)`) as E10-S6 — **reuse it, do not reimplement**.
- The deletion is registered on the **agent undo stack** as a single reversible action with the
  reference's exact undo-group name (agent-undo refuses after an interleaved user edit — carry-forward).
- e2e gate: the **§11.3 agent-cut workflow** (transcription-gated, deferred to M3 per §12) runs —
  transcribe a clip → agent identifies filler/silence → `ripple_delete_ranges` → timeline gaps close;
  if transcription hasn't run, `get_transcript` returns empty and the agent is told to transcribe first
  (UJ-1 edge case), not guess.
- No new editing engine is introduced — this is glue over `get_transcript` (E10-S7) + `ripple_delete_ranges`
  (Epic 3).

**Implementation context:**
- Crates: `palmier-tools` (the agent-facing path; reuses `ripple_delete_ranges` from `palmier-edit`/Epic 3).
- Reference: transcript-driven cut (reference commit `561f04d`, FOUNDATION §6.9 lines 597); FR-38;
  docs/reference/transcription.md §D `Clip.timelineFrame`.
- Realizes **UJ-1** climax.

**Dependencies:** E10-S7 (`get_transcript`), E10-S6 (`timeline_frame` reuse); **Epic 3**
(`ripple_delete_ranges`), **Epic 7** (tool dispatch). Effectively the last story in the epic.
**Parallel-safe?** No — sits on top of S6/S7 and the Epic 3 ripple engine; run last.

---

## Suggested execution order

1. **E10-S1** (model) and **E10-S5** (CaptionBuilder, fully independent) — start in parallel.
2. **E10-S2** (engine), **E10-S3** (locale/profanity), **E10-S4** (cache) — parallel after S1.
3. **E10-S6** (specs + orchestration) — after S2/S4/S5.
4. **E10-S7** (tools) — after S6.
5. **E10-S8** (transcript-driven cut) — last, after S7 + Epic 3 ripple.

## Notes / risks carried into this epic

- **R-5 (CaptionBuilder golden):** the 14 tests are unforgiving; a Rust grapheme-count or float-format
  drift breaks parity silently. Gate golden updates behind `--update-golden`; any CI diff blocks merge.
- **Cache-key deviation (ruling #19):** adopt `sha256(content)+model+language`, NOT the reference
  `path|mtime|size`. Decide and record the first-N-MB hashing cutoff if full hashing is too slow on
  25-min files (Open Question in transcription.md).
- **Whisper vs Apple segmentation differs:** segment boundaries/`segments[].text` will not match the
  reference; only the CaptionBuilder transform of a given segment is parity-testable. Confirm downstream
  (search, transcript-cut) tolerates different segmentation.
- **Word-timestamp precision:** `spokenWordCount` dominant-track logic needs per-word `start`/`end` —
  confirm `whisper-rs` exposes them with enough precision (token-timestamps / `--max-len`).
- **Title-case DISCREPANCY:** FOUNDATION §6.7/§6.9 list `title` case; reference and ruling #18 are
  **auto/upper/lower only** — drop title-case in the port.
- **FOUNDATION word-timestamp type:** FOUNDATION §6.9 uses non-optional `f64`; the reference uses
  `Option<f64>` and the `filter`/`spokenWordCount` paths depend on it — **reference wins** (E10-S1).
