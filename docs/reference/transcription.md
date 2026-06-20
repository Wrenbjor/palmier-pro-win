---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# transcription — reference port notes

## Purpose
Document the macOS reference's speech-to-caption pipeline so it can be re-implemented clean-room
on Windows+Linux with **behavior parity**. Two layers: (1) the transcription engine producing a
`TranscriptionResult` (words/segments/timestamps in *source seconds*), and (2) the `CaptionBuilder`
that splits each segment into screen-ready, timed phrases and emits `TextClipSpec` records placed
through a clip's trim/speed. The reference uses Apple Speech (`SpeechAnalyzer`/`SpeechTranscriber`);
the port uses whisper.cpp via `whisper-rs`. The result model and the entire CaptionBuilder algorithm
are *engine-independent* and must port byte-for-byte — CaptionBuilder is unit-tested (parity oracle).

## Key types & files (cite paths under Sources/PalmierPro/Transcription/...)
- `Transcription/Transcription.swift` — result model + Apple-Speech engine.
  - `TranscriptionWord { text: String, start: Double?, end: Double? }` (timestamps optional/nullable).
  - `TranscriptionSegment { text: String, start: Double, end: Double }` (one endpointed utterance).
  - `TranscriptionResult { text, language: String?, words: [Word], segments: [Segment] }`; method
    `offsetting(by: Double)` adds an offset to every word/segment timestamp (no-op when offset==0).
  - `TranscriptionError` enum: `unsupportedLocale, modelInstallFailed, decodeFailed,
    audioExtractionFailed, analysisFailed` (each with a user-facing string).
  - `enum Transcription` static API: `transcribeVideoAudio`, `transcribe(fileURL:)`,
    `supportedLocales`, `bestSupportedLocale`, `matchLocale`, private `extractAudioTrack`,
    private `decodeResults`.
- `Transcription/TranscriptCache.swift` — `actor TranscriptCache.shared`, disk+memory cache.
- `Transcription/TranscriptSearch.swift` — exact keyword search over cached segments (see search doc).
- `MediaPanel/CaptionsTab/CaptionBuilder.swift` — **the phrase-split/timing algorithm** + `specs(...)`.
- `Editor/ViewModel/EditorViewModel+Captions.swift` — orchestration: target selection, transcribe,
  phrase→clip assignment, casing, track placement.
- `MediaPanel/CaptionsTab/CaptionTab.swift` — UI controls + style/placement state.
- `Agent/Tools/ToolExecutor+Captions.swift` — `add_captions` MCP tool wrapper.
- `Models/Timeline.swift` — `Clip.timelineFrame(sourceSeconds:fps:)` (the source-sec→frame mapping).
- `UI/AppTheme.swift` `enum Caption` — constants below.
- Tests: `Tests/PalmierProTests/Captions/CaptionBuilderTests.swift` (port these verbatim).

## Core behaviors & algorithms (concrete — downstream story/dev agents implement from this)

### A. Audio extraction (`extractAudioTrack`)
Decode the asset's audio track to a temp PCM file. Reference uses AVAssetReader → linear PCM,
**16 000 Hz, 1 channel (mono), 16-bit, little-endian, non-float, interleaved**, written to
`tempdir/palmier-stt-<UUID>.caf`. Optional `range: ClosedRange<Double>` (source seconds) limits the
reader to `[lower, upper]`. Errors → `audioExtractionFailed`. Port: FFmpeg decode to the *same*
16 kHz/mono/s16le format Whisper expects (per FOUNDATION §6.9). Caller deletes the temp file (`defer`).

### B. Engine run (`transcribe`)
1. If `sourceRange` given: extract that range to temp file, transcribe it, then
   `result.offsetting(by: range.lowerBound)` to shift timestamps back into source time. (Same in
   `transcribeVideoAudio`: extract then offset by `range?.lowerBound ?? 0`.)
2. Locale: prefer `preferredLocale` if `matchLocale` finds it in supported; else `bestSupportedLocale`
   (from `Locale.preferredLanguages + Locale.current`); else throw `unsupportedLocale`.
   `matchLocale` = first candidate whose **language code** has any supported match, preferring exact
   region, else first same-language. Port: Whisper detects language from audio (or user override);
   replicate "prefer user locale, else auto-detect, else error".
3. `censorProfanity` → Apple `.etiquetteReplacements`. Port: replace matched words with bracketed
   equivalents or Whisper token suppression (FOUNDATION §6.9).
4. Decode (`decodeResults`): for each engine Result (= one endpointed segment):
   - append its raw text to `fullText`; trimmed non-empty text → a `TranscriptionSegment` with
     `start = range.start.seconds`, `end = range.end.seconds`.
   - walk per-token runs: trim each run; skip empty; `start = run.start`, `end = run.start+duration`;
     append `TranscriptionWord`. (Whisper gives word-level timestamps directly — map 1:1.)
   - `result.text = fullText.trimmed`; `language = locale bcp47`.

### C. CaptionBuilder.phrases(for segment, fits, minDuration) — UNIT-TESTED, PORT EXACTLY
`phrases = enforceMinDuration(distribute(split(text), start, end), minDuration)`.

`split(text, fits) -> [String]` (recursive):
- `t = text.trimmed(whitespaces)`; empty → `[]`.
- if `fits(t)` → `[t]` (done).
- `parts = breakOnce(t)`; if `parts.count <= 1` → `[t]` (an unbreakable over-long single word is kept).
- else `parts.flatMap { split($0, fits) }`.

`breakOnce(text)` = first non-nil of: `breakOn(".!?")`, then `breakOn(",;:")`, then `breakAtMidWord`.

`breakOn(text, delimiters) -> [String]?` — split *after* a delimiter **only when the next char is a
space or end-of-string** (so "U.S." and "3.14" stay intact). Walk chars accumulating `current`; when
`current`'s last char ∈ delimiters AND next is break → push trimmed `current`, reset. Push trailing
`tail`. Return pieces **only if count > 1**, else nil. (Test `keepsPunctuatedTokensIntact`:
"U.S. army here" with fits≤6 → `["U.S.","army","here"]`.)

`breakAtMidWord(text)` — split on spaces into words; if ≤1 word → `[text]`; else
`mid = words.count/2` (integer div), return `[words[0..<mid] joined, words[mid...] joined]`.
(Test: "a b c d" fits≤3 → `["a b","c d"]`.)

`distribute(texts, start, end) -> [Phrase]` — proportional by **character count**:
- `total = Σ max(text.count,1)`; `span = max(end-start, 0)`; `t = start`.
- each: `dur = span * max(text.count,1)/total`; phrase `(text, t, t+dur)`; `t += dur` (back-to-back).
- (Test `distributesTimeByCharacterCount`: "aaaa bb" 0..6 → starts [0,4], ends [4,6].)

`enforceMinDuration(phrases, minDuration) -> [Phrase]` — forward pass over indices:
- if `phrase[i].end - phrase[i].start < minDuration` → `phrase[i].end = phrase[i].start + minDuration`.
- if next exists and `phrase[i+1].start < phrase[i].end`: `shift = phrase[i].end - phrase[i+1].start`;
  add `shift` to **both** `phrase[i+1].start` and `.end` (cascades; can push final end past segment end).
- (Test `enforcesMinimumDurationAndShifts`: "aa bbbb" 0..6 minDur3 → starts [0,3], ends [3,7].)

### D. CaptionBuilder.specs(...) — phrase → TextClipSpec, mapped through clip placement
For each phrase `p`, given `sourceClip: Clip`, `fps`, `trackIndex`, `style`, `captionGroupId`,
`transformFor: (String)->Transform?`, `minDurationFrames = 1`:
1. Visible source window (in source frames): `visStart = trimStartFrame`,
   `visEnd = visStart + durationFrames * max(speed, 0.0001)`.
2. `pStart = p.start*fps`, `pEnd = p.end*fps`. **Drop the phrase** unless `pEnd > visStart && pStart < visEnd`.
3. Map: `mappedStart = clip.timelineFrame(sourceSeconds: p.start, fps)`,
   `mappedEnd = timelineFrame(p.end)`; fall back to `clip.startFrame`/`endFrame` when nil.
4. `s = mappedStart ?? startFrame`, `e = mappedEnd ?? endFrame`;
   `durationFrames = max(minDurationFrames, min(clip.endFrame, e) - max(clip.startFrame, s))`.
5. Emit `TextClipSpec(trackIndex, startFrame: s, durationFrames, content: p.text, style,
   transform: transformFor(p.text), captionGroupId)`.
`Clip.timelineFrame(sourceSeconds t, fps)`: `srcFrame = t*fps`; `off = srcFrame - trimStartFrame`;
nil if `off<0`; `frame = round(startFrame + off/max(speed,0.0001))`; nil unless
`startFrame <= frame < endFrame`. (Tests cover clamping past clip end, trimmed-in clips, drop-before-trim.)

### E. Orchestration (`EditorViewModel.generateCaptions`)
- Targets: `captionTargets` filters clips that are video/audio with audio; for video clips linked
  (`linkGroupId`) to an audio clip, the audio wins (skip the video). Sorted by `startFrame`.
- `transcribe`: one result per distinct `mediaRef`. Range = `visibleSourceUnion` = union of every
  target clip's visible source window for that ref, **padded ±1.0 s**, clamped at 0, in seconds.
  If `censorProfanity || locale != nil` → bypass cache (option variants differ); else
  `TranscriptCache.shared.transcript(...)`.
- `autoDetect`: keep only the **dominant speech track** (max spoken word count; a word counts if its
  midpoint falls in the clip's visible window).
- Phrase→clip: `result.segments.flatMap { CaptionBuilder.phrases(for:$0, fits: captionLineFits,
  minDuration: 0.7) }`; each phrase assigned to the clip with most overlap, **only if** overlap > 0
  AND `overlap >= (phraseLen)/2`.
- Casing applied per phrase (`auto`/`upper`/`lower`). Then `CaptionBuilder.specs(...)` with a shared
  `groupId = UUID()` and a `transformFor` computing each box's `Transform` from natural text size.
- Placement: insert a NEW video track at index 0, place text clips, register one undo group
  "Generate Captions". `captionLineFits`: text natural width ≤ `timeline.width * 0.9`.

### Constants (AppTheme.Caption)
`minDisplayDuration = 0.7`, `defaultFontSize = 48`, `minFontSize = 12`, `maxFontSize = 300`,
`minPosition = 0`, `maxPosition = 1`, `centerSnapValue = 0.5`, `centerSnapThreshold = 0.02`,
`defaultCenter = (0.5, 0.9)`. `captionPreviewMaxTextWidthRatio = 0.9`.

### Cache (`TranscriptCache`)
- Key = `prefix32(sha256("<path>|<mtime>|<size>"))` — file-identity, so edits invalidate naturally.
  **FOUNDATION §6.9 says key = `sha256(file_content)+model_id+language`** — DISCREPANCY: reference
  keys on path+mtime+size (not content hash) and does NOT include model/language. Port should adopt
  FOUNDATION's content+model+language key (more correct across model swaps); flag the change.
- Only **full-file** transcripts are cached; a windowed request filters a cached full transcript via
  `filter(result, to: range)` (segments where `end>lo && start<hi`; words with both ts and same
  overlap; `text = segments.joined(" ")`). Memory cap = 4 entries (clears all on overflow). Disk:
  `<caches>/<subsystem>/Transcripts/<key>.json` (JSON-encoded `TranscriptionResult`).

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `Speech.SpeechTranscriber` / `SpeechAnalyzer` / `AssetInventory` model install →
  **whisper.cpp via `whisper-rs`**; model download/manage per FOUNDATION (bundle `small.en`, optional
  `medium.en`/`large-v3`). `transcriber.results` async stream → Whisper segment/word callbacks.
- `SpeechTranscriber.supportedLocales` / `Locale` matching → Whisper supported-language list + a
  language-tag matcher (BCP-47 lang code first, region preference). For `.en` models, English only.
- `.etiquetteReplacements` (profanity) → word-list replacement with bracketed token or Whisper
  token suppression.
- `AVAssetReader` + `AVAssetReaderTrackOutput` + `AVAudioFile` PCM extraction → **FFmpeg** decode to
  16 kHz mono s16le. CAF temp container → WAV/raw PCM temp file.
- `CMTime`/`CMTimeRange` (timescale 600) → plain `f64` seconds.
- `Locale.preferredLanguages` / `Locale.current` → OS locale APIs (`sys-locale` crate) or config.
- `CryptoKit.SHA256` → `sha2`/`ring`. `FileManager` caches dir → `dirs`/`%LOCALAPPDATA%` +
  `~/.cache` per FOUNDATION logging paths.

## Mapping to FOUNDATION crates (palmier-transcribe, palmier-text)
- **palmier-transcribe** ← `Transcription.swift` engine + `TranscriptCache` + audio extraction.
  Owns: whisper-rs wrapper, FFmpeg PCM extraction, `TranscriptionResult` model, `offsetting`,
  range-extract+offset, locale resolution, profanity, cache (use FOUNDATION cache key). Exposes
  `transcribe(file/video, censor, locale, range)` mirroring the reference signatures.
- **palmier-text** ← `CaptionBuilder` (`phrases`, `split`/`breakOnce`/`breakOn`/`breakAtMidWord`,
  `distribute`, `enforceMinDuration`) + caption styling/`TextStyle` + text natural-size layout
  (`captionLineFits`/`naturalSize`). FOUNDATION §built-list explicitly assigns "caption phrase
  splitting … duration distribution … minimum-duration cascade" to `palmier-text`.
- `specs(...)` + orchestration live with the editor/timeline crate (palmier-edit) since they depend on
  `Clip`/`TextClipSpec`/`timelineFrame`; but the *algorithm* (D's mapping rules) is documented here.
- `TranscriptSearch` → palmier-search.

## Port risks & gotchas
- **CaptionBuilder is the parity oracle.** Port all 14 `CaptionBuilderTests` verbatim; any off-by-one
  in char-count distribution, integer mid-split, or the min-duration cascade breaks visible timing.
- `break*` operate on Swift `Character`s (grapheme clusters). Whisper/Rust default to `char`
  (scalars) or bytes — use grapheme-aware counting (`unicode-segmentation`) so `.count` matches.
- `breakOn` boundary rule (delimiter followed by space/EOS) is the subtle bit: "U.S.", "3.14" must
  NOT split. Test it explicitly.
- `enforceMinDuration` can push the final phrase's end **past the segment end** (test expects end=7
  for a 0..6 segment) — do not clamp it back.
- Timestamp `start/end` are `Option<f64>` on words; many code paths skip words with `None`. Whisper
  generally provides them, but keep them optional to match `filter`/`spokenWordCount` semantics.
- Time math is in **source seconds**, then ×fps; `timelineFrame` rounds and bounds with
  `[startFrame, endFrame)` (half-open). speed floored at `0.0001` everywhere to avoid div-by-zero.
- Cache stores only full transcripts; windowed calls MUST filter, not re-transcribe, when a full
  transcript exists — otherwise timestamps and dominant-track logic diverge.
- `visibleSourceUnion` pad ±1.0 s and clamp-at-0 affect which words exist near clip edges; replicate.
- AVAssetReader yields whatever channel layout the source has *before* the 16 kHz/mono output settings
  apply — ensure FFmpeg forces mono/16 kHz, not just requests it.

## Open questions
- FOUNDATION cache key (`sha256(content)+model+language`) vs reference (`path+mtime+size`): adopt
  FOUNDATION — confirm content-hash cost is acceptable for 25-min files, or hash first N MB.
- FOUNDATION §6.9 / §6.7 list text case `upper/lower/**title**`; reference `CaptionCase` is only
  `auto/upper/lower` (no title; `ToolExecutor+Captions` rejects anything else). Decide whether to add
  `title` case in the port or drop it from FOUNDATION. DISCREPANCY flagged.
- Whisper segment boundaries differ from Apple's endpointing → segment count/`segments[].text` will
  not match the reference; only the *CaptionBuilder transform of a given segment* is parity-testable.
  Confirm downstream (search, transcript-cut) tolerates different segmentation.
- Whisper word timestamps require `--max-len`/token-timestamps; confirm `whisper-rs` exposes per-word
  `start/end` with enough precision for `spokenWordCount` midpoint logic.
- Does the port keep the "insert new video track at index 0 + single undo group" placement, or use a
  dedicated caption track type? (UI/grouping decision — `caption_group_id` is already in the model.)
