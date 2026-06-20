---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
title: "Epic 2 — Project I/O & Data Model (implementation stories)"
governing_reference: [docs/reference/project-io.md, docs/reference/timeline-model.md]
milestone: M1
crates: [palmier-model, palmier-project]
---

# Epic 2 — Project I/O & Data Model

## Epic goal

Build the `palmier-model` (serde data shapes + pure sampling/geometry/computed-property math) and
`palmier-project` (`.palmier` directory-bundle read/write, project registry, autosave/dirty-tracking,
sample materialization, media-path resolution, directory-as-document UX) crates so that a Palmier
project authored by the macOS reference — or by the Convex sample server — opens, edits, saves, and
reopens with **byte-identical model state** and **identical computed/sampled values**, on Windows and
Linux, with no line of Swift reused. This is the data foundation every later epic (3, 5, 6, 7, 8, 9, 10,
11) consumes.

## PRD acceptance this epic must satisfy (PRD §4.2 / §10 Epic 2)

- **FR-5 Bundle round-trip** — read/write `project.json`, `media.json`, `generation-log.json`,
  `thumbnail.jpg`, `media/`, `chat/` (ruling #3). Import → edit → save → reopen yields byte-identical
  model state (SM-7). Per-field Date encoding: chat = iso8601+pretty+sortedKeys; project/media/log =
  Apple reference-epoch (seconds since 2001-01-01) doubles — **provisional until Spike S-1b confirms the
  Convex sample payload (R-6)**, with a per-field-codec fallback and a round-trip regression gate.
- **FR-6 Center-based Transform with legacy migration** — Transform stored center-based; legacy top-left
  projects migrate on load (`centerX = oldX + w − 0.5`) (ruling #7); a reference-authored project opens
  with clips positioned identically.
- **FR-7 Deterministic model semantics** — keyframe sampling default **Smooth** (ruling #8); all
  source↔timeline conversions use **`f64::round` ties-away-from-zero**, never `round_ties_even`
  (carry-forward note); serde round-trip unit tests pass for every shape.
- **FR-8 Directory-as-document UX** — on Windows the file picker presents `.palmier` directories as one
  document via a Tauri custom dialog; on Linux it behaves as a directory naturally.

PRD §10 acceptance, additionally: **SM-1b — open existing 30-clip 1080p project < 1 s** on §10 HW;
reference filenames `project.json`/`media.json`/`generation-log.json`/`chat/`/`project-registry.json`
(ruling #3); **golden-fidelity model gates (FOUNDATION §11.1):** (a) keyframe sampling unit-tested at
segment boundaries (`t=0`, `t=end`, exact-on-key, between-keys) against reference values for
**Smooth / Linear / Hold**; (b) frame-rounding parity test asserting `f64::round` ties-away on the x.5
divergence cases for both source↔timeline directions.

**Milestone (PRD §12): M1 — Hand-Edit MVP** (Epics 1–6). Spike **S-1b lands before the Epic 2 serde
lock**. This epic's bundle-round-trip integration test is the **§11.2 M1 round-trip gate** and a
prerequisite for the §11.3 hand-editing e2e exit gate.

---

## Spike / risk gate

This epic is **not** spike-blocked the way Epic 5 is (S-1), but its **serde Date encoding is provisional
until Spike S-1b lands** (PRD §11, R-6). The ruling:

- **S-1b [M1-CRITICAL, before the Epic 2 serde lock]** — fetch one real `/v1/samples` + `/resolve`
  payload (or a captured fixture if Convex access is blocked at M1) and document the exact Date encoding
  per field. **Pass bar:** a recorded sample payload + a round-trip unit test proving the chosen
  per-field serde re-encodes it identically.
- **Sequencing ruling for this epic:** do **not** block stories E2-S1..E2-S5 (model shapes, computed
  math, keyframes) on S-1b — they do not touch `Date` wire format. **Gate only the two Date-bearing
  serde decisions** (`MediaManifest.created_at` / `cached_remote_url_expires_at`, `GenerationLogEntry`
  timestamps, `ChatSession.updated_at`) on S-1b, behind a **per-field `Date` codec abstraction** so the
  field's encoder can be switched without touching the rest of the model (E2-S8 owns this seam). Until
  S-1b confirms, implement the documented strategy (Apple reference-epoch doubles for project/media/log;
  iso8601 for chat) with a round-trip regression gate that fails loudly if a captured payload no longer
  round-trips.

The other binding rulings consumed here: **#3** (reference bundle filenames), **#7** (center Transform +
legacy migration), **#8** (Smooth keyframe default), **#9** (volume dB −60…+15, three distinct constants),
**#12** (all visual types interchangeable, via `ClipType.isCompatible`), and the carry-forward
**`f64::round` ties-away** rule.

---

## Story decomposition

> Crate convention: `palmier-model` = pure serde shapes + pure functions (no fs, no async). `palmier-project`
> = bundle I/O, registry, autosave, samples, path resolution (owns fs + reqwest + tokio). Stories E2-S1..E2-S6
> are `palmier-model`; E2-S7..E2-S12 are `palmier-project`. Most `palmier-model` stories are parallel-safe
> against each other because they land in **separate source files** under `palmier-model/src/`.

### E2-S1 — `palmier-model` crate scaffold + core enums (`ClipType`, `Interpolation`, `AnimatableProperty`)

**Intent:** As a dev building the data foundation, I want the `palmier-model` crate scaffolded with the
small leaf enums every other shape depends on, so that sibling stories can build `Clip`/`Track`/`Timeline`
on a stable base.

**Acceptance criteria:**
- `palmier-model` crate exists in the workspace (`crates/palmier-model`, lib), depends on
  `serde`/`serde_json`; `cargo test -p palmier-model` runs.
- `ClipType { video, audio, image, text, lottie }` ported from `Models/ClipType.swift`. Implement
  `is_visual()` = `video|image|text|lottie`; **`is_compatible(other)` = `self==other || (self.is_visual()
  && other.is_visual())`** (ruling #12 — ALL visual types interchangeable; do NOT restrict text/lottie to
  own-type as FOUNDATION §6.3 wrongly states). `from_file_extension(&str) -> Option<ClipType>` mapping
  reference extensions, with `.json`/`.lottie` → `lottie`.
- `Interpolation { Linear, Hold, Smooth }` with **serde default = `Smooth`** (ruling #8). `smoothstep(t)
  = t*t*(3.0 - 2.0*t)`. Provide `lerp` and a `KeyframeInterpolatable` trait stub for `f64`.
- `AnimatableProperty { opacity, position, scale, rotation, crop, volume }`.
- Unit tests: `is_compatible` truth table (video↔image↔text↔lottie all true; audio only with audio);
  `from_file_extension` for representative extensions; `smoothstep(0)=0`, `smoothstep(1)=1`,
  `smoothstep(0.5)=0.5`; `Interpolation` serde default round-trips to `Smooth` when the field is absent.

**Implementation context:** target crate `palmier-model`. Reference: `Sources/PalmierPro/Models/ClipType.swift`,
`Models/Keyframe.swift` (`Interpolation`, `smoothstep`, `AnimatableProperty`). docs/reference/timeline-model.md
"Keyframes & sampling", "ClipType compatibility".

**Dependencies:** none (first story). **Parallel-safe?** No — it creates the crate; everything else waits
on it. (After it lands, S2–S6 are mutually parallel.)

---

### E2-S2 — `Transform` (center-based) + `Crop` with legacy migration

**Intent:** As a dev, I want `Transform` and `Crop` stored exactly as the reference persists them, so that
reference-authored projects open with clips positioned pixel-identically.

**Acceptance criteria:**
- `Transform` stored **center-based**: serde fields `centerX=0.5, centerY=0.5, width=1.0, height=1.0,
  rotation=0.0, flipHorizontal=false, flipVertical=false` (match the reference JSON key spelling exactly —
  these are the persisted keys). Expose computed `top_left()` = `(centerX - width/2, centerY - height/2)`
  (ruling #7). Normalized 0..1 canvas space.
- **Legacy migration on decode:** old projects carrying top-left-ish `x`/`y` keys migrate via
  **`centerX = oldX + width − 0.5`** (and the y analogue) (ruling #7, docs/reference/timeline-model.md
  "Transform … legacy decode migrates old `x/y` keys"). A naive top-left field is FORBIDDEN — it breaks
  round-trip.
- `Crop` = edge insets `left/top/right/bottom` in 0..1 source space; `is_identity()`;
  `visible_width_fraction() = max(0, 1 - left - right)`. `Crop` implements `KeyframeInterpolatable`
  (4-componentwise lerp).
- Unit tests: a JSON object with `centerX/centerY/width/height` round-trips byte-stable; a legacy
  `{x,y,width,height}` object decodes to the migrated center (`centerX = x + w − 0.5`); `top_left()`
  inverse of center for a non-trivial size; `Crop.is_identity` and `visible_width_fraction`.

**Implementation context:** `palmier-model` (`src/transform.rs`). Reference: `Models/Timeline.swift`
(`Transform`, `Crop`, `CropAspectLock`). docs/reference/timeline-model.md "Data model", Port risks
"Transform storage is center-based".

**Dependencies:** E2-S1. **Parallel-safe?** Yes (own file; only depends on S1's enums).

---

### E2-S3 — Keyframe types + sampling (`Keyframe<V>`, `KeyframeTrack<V>`, `AnimPair`, `sample`)

**Intent:** As a dev, I want the keyframe storage and the `sample(at:)` algorithm ported exactly, so that
every authored animation and fade curve reproduces the reference's value at every frame.

**Acceptance criteria:**
- `Keyframe<V> { frame: i32, value: V, interpolation_out: Interpolation }` with `interpolation_out`
  defaulting to **`Smooth`** (ruling #8). **Frames stored CLIP-RELATIVE**; the public API converts to
  absolute via `to_abs/to_offset = frame ± start_frame` (document this seam to avoid double-offset).
- `KeyframeTrack<V> { keyframes: Vec<Keyframe<V>> }`, `is_active() = !keyframes.is_empty()`. `upsert`
  keeps the vec sorted, unique frames. `move(from,to)` is a **no-op if the target frame is occupied**.
- `sample(at, fallback)` algorithm verbatim (docs/reference/timeline-model.md "Keyframes & sampling"):
  empty→fallback; 1 kf→that value; `frame ≤ first.frame`→first; `frame ≥ last.frame`→last; else first kf
  with `frame > target`, segment `[a,b]`, `raw = (frame − a.frame)/(b.frame − a.frame)`, switch on
  **`a.interpolation_out`**: `Hold`→`a`, `Linear`→`lerp(a,b,raw)`, `Smooth`→`lerp(a,b,smoothstep(raw))`.
- `AnimPair { a, b }` (used as position `(x,y)` AND scale `(w,h)`), componentwise `KeyframeInterpolatable`.
  `Double` and `Crop` (from S2) also interpolatable.
- Clamp helpers: `clamp_keyframes_to_duration()` drops kfs with `frame < 0 || frame > duration`;
  `rescale_keyframes(by)` multiplies frames (used on speed change).
- **Golden boundary tests (FOUNDATION §11.1, PRD §10 gate (a)):** for each interp **Smooth / Linear /
  Hold**, assert `sample` at `t=0`, `t=end`, exact-on-key, and a between-keys frame against hand-computed
  reference values. Include a 2-keyframe and 3-keyframe track. Name the test module
  `keyframe_boundary_sampling`.

**Implementation context:** `palmier-model` (`src/keyframe.rs`). Reference: `Models/Keyframe.swift`
(`Keyframe`, `KeyframeTrack`, `AnimPair`, `sample`, `upsert`, `move`, clamp/rescale helpers).

**Dependencies:** E2-S1, E2-S2 (needs `Crop` interpolatable, `smoothstep`). **Parallel-safe?** Partially —
own file; can run concurrently with S4 once S2 has landed.

---

### E2-S4 — `VolumeScale` (linear↔dB), the three dB constant pairs

**Intent:** As a dev, I want the linear↔dB conversions and the distinct dB ranges ported exactly, so that
volume keyframes (stored in dB) and the linear static volume agree with the reference and the inspector
field range is correct.

**Acceptance criteria:**
- `VolumeScale`: `FLOOR_DB = -60.0`, `CEILING_DB = 15.0` (ruling #9 — NOT FOUNDATION's −120).
  `db_from_linear(l) = if l > 0 { (20*log10(l)).clamp(FLOOR_DB, CEILING_DB) } else { -60.0 }`;
  `linear_from_db(db) = if db > -60 { 10f64.powf(db/20.0) } else { 0.0 }` (hard mute below floor).
- Keep the **three distinct dB constant pairs separate** (do not collapse): (1) `VolumeScale` editing
  range `+15 / −60`; (2) rubber-band DRAW axis `+6 / −60` (consumed by Epic 3 rendering — define the
  constants here as `RUBBER_BAND_TOP_DB = 6.0`, `RUBBER_BAND_BOTTOM_DB = -60.0`); (3) the
  keyframe-storage floor — **flagged unverified** (ruling #9 / PRD OQ): leave a `// VERIFY: keyframe
  storage dB floor against reference before locking` marker; do NOT silently assume −60 for storage.
- Unit tests: `db_from_linear(1.0)=0`, `linear_from_db(0)=1`, hard mute (`linear_from_db(-60)=0`,
  `db_from_linear(0)=-60`), ceiling clamp (`db_from_linear(10.0)` clamps to `+15`), round-trip
  `linear_from_db(db_from_linear(x))≈x` for x in (0,1].

**Implementation context:** `palmier-model` (`src/volume.rs`). Reference: `Inspector/InspectorView.swift:1072`
(`VolumeScale`); docs/reference/timeline-model.md "VolumeScale", Port risks "Volume dB floor is -60".

**Dependencies:** E2-S1. **Parallel-safe?** Yes (own file).

---

### E2-S5 — `Clip` shape + computed/derived properties + value sampling

**Intent:** As a dev, I want `Clip` with all stored fields and the render-critical computed/sampled methods
(`endFrame`, `sourceFramesConsumed`, `opacityAt`, `volumeAt`, `fadeMultiplier`, `transformAt`,
`timelineFrame`), so that the timeline, preview, export, and MCP tools all read identical values.

**Acceptance criteria:**
- `Clip` (`Models/Timeline.swift:75`): `id: String` (UUID **string**, NOT typed `Uuid` — ruling-aligned
  carry-forward; lenient decode regenerates a UUID string if missing), `media_ref: String`,
  `media_type: ClipType = video`, `source_clip_type = video`, `start_frame`, `duration_frames`,
  `trim_start_frame = 0`, `trim_end_frame = 0`, `speed = 1.0`, `volume = 1.0` (**linear**),
  `fade_in_frames = 0`, `fade_out_frames = 0`, `fade_in_interpolation = Linear`,
  `fade_out_interpolation = Linear`, `opacity = 1.0`, `transform`, `crop`, `link_group_id: Option<String>`,
  `caption_group_id: Option<String>`, `text_content: Option<String>`, `text_style: Option<TextStyle>`, and
  **6 optional `KeyframeTrack`s** (opacity, position, scale, rotation, crop, volume). `link_group_id`/
  `caption_group_id`/`media_ref` are plain strings.
- Derived: `end_frame = start_frame + duration_frames`;
  **`source_frames_consumed = f64::round(duration_frames as f64 * speed) as i32`** (ties-away — carry-forward);
  `source_duration_frames = source_frames_consumed + trim_start_frame + trim_end_frame`.
- Value sampling ported verbatim (docs/reference/timeline-model.md "Clip value sampling"):
  - `opacity_at(frame) = raw_opacity_at(frame) * fade_multiplier(frame)` but fade applied **only when
    `media_type != audio` and a fade exists**. `raw_opacity_at` = opacity track sample (fallback static
    `opacity`).
  - `volume_at(frame) = volume * kf_gain * fade_multiplier`; `kf_gain = linear_from_db(volume_track.sample(
    .., fallback = 0 dB))` when the volume track is active, else `1.0`. **Volume kf values are dB; static
    `volume` is linear.** `raw_volume_at` omits fade.
  - `fade_multiplier(frame)`: `rel = frame − start_frame`, returns 0 outside `[0, duration]`;
    `in_mul = if fade_in>0 { t=min(1, rel/fade_in); smooth→smoothstep(t) else t } else 1`; `out_mul`
    symmetric on `duration_frames − rel`; return `min(in_mul, out_mul)`. **Linear and Hold both behave as a
    linear ramp for fades; only `Smooth` bends.**
  - `transform_at(frame)`: top_left from position track (`AnimPair` a,b) if active else `transform`
    center − half size; size from scale track (`AnimPair` a=w,b=h) else `transform.width/height`; rotation
    from rotation track else `transform.rotation`; crop from crop track else static `crop`.
  - `timeline_frame(source_seconds, fps)`: `source_frame = t*fps`; `offset = source_frame − trim_start`
    (None if < 0); **`frame = f64::round(start_frame + offset/max(speed, 1e-4))`** (ties-away); None unless
    in `[start_frame, end_frame)`. This is the transcript-seconds→timeline-frame map consumed by Epic 7/10.
- `set_duration(d)` runs clamp + fade-clamp (delegates to S3 helpers).
- Unit tests: derived properties (incl. `source_frames_consumed` rounding on a non-integer
  `duration*speed`); `fade_multiplier` at edges and mid-fade for Linear vs Smooth; `volume_at` with a dB
  keyframe track vs static linear volume; `opacity_at` audio-vs-visual fade gating; `timeline_frame`
  in-range and out-of-range; a missing-`id` JSON decodes with a regenerated UUID string.

**Implementation context:** `palmier-model` (`src/clip.rs`). Reference: `Models/Timeline.swift` (`Clip`,
`FadeEdge`, all sampling methods). docs/reference/timeline-model.md "Data model", "Clip value sampling".
Depends on `TextStyle` — define a minimal `TextStyle` serde shape here or stub it (full text styling is
Epic 5/10; this story only needs the serde shape to round-trip, fields per `Models/Timeline.swift`).

**Dependencies:** E2-S1, E2-S2, E2-S3, E2-S4. **Parallel-safe?** No against S2/S3/S4 (consumes all);
parallel against the `palmier-project` stories.

---

### E2-S6 — `Track`, `Timeline`, lenient/defaulted decode, non-serialized `displayHeight`

**Intent:** As a dev, I want `Track` and `Timeline` with the reference's lenient-decode and
non-serialized-field semantics, so that old/partial projects load instead of erroring and look correct.

**Acceptance criteria:**
- `Timeline`: `fps: i32 = 30, width: i32 = 1920, height: i32 = 1080, settings_configured: bool = false,
  tracks: Vec<Track>`. `total_frames() = tracks.iter().map(end_frame).max().unwrap_or(0)`. fps/resolution
  are frozen-after-first-clip semantics (enforcement lives in Epic 3; the field is just stored here).
- `Track`: `id: String (UUID string), type: ClipType, muted = false, hidden = false, sync_locked = true,
  clips: Vec<Clip>`. **`display_height` is NOT serialized** (omit from `CodingKeys`/serde; reset to `50.0`
  on open). `end_frame() = clips.iter().map(end_frame).max().unwrap_or(0)`.
- **Lenient decode is load-bearing** (every field defaulted via `#[serde(default)]`): a `Track`/`Timeline`
  JSON object missing any field decodes with the documented default; an unknown extra field is ignored.
- Unit tests: a minimal `{}`-ish timeline JSON decodes to all defaults (fps 30 / 1920×1080 / no tracks);
  a track JSON omitting `muted/hidden/sync_locked` decodes to `false/false/true`; after decode
  `display_height == 50.0` even if the input JSON contained a different value; `total_frames`/`end_frame`
  over a multi-clip fixture.

**Implementation context:** `palmier-model` (`src/timeline.rs`). Reference: `Models/Timeline.swift`
(`Timeline`, `Track`). docs/reference/timeline-model.md "Data model", Port risks "non-serialized
`displayHeight`".

**Dependencies:** E2-S1, E2-S5 (Track holds `Clip`s). **Parallel-safe?** No against S5; this is the
`palmier-model` capstone for the timeline shapes.

---

### E2-S7 — `MediaManifest` / `MediaSource` / `GenerationLog` shapes + lenient + version fallbacks

**Intent:** As a dev, I want the media-manifest and generation-log serde shapes with the reference's
externally-tagged `MediaSource`, version defaults, and legacy `cost` migration, so that `media.json` and
`generation-log.json` round-trip and old bundles still open.

**Acceptance criteria:**
- `MediaManifest { version: u32, entries: Vec<MediaManifestEntry>, folders: Vec<MediaFolder> }`. **Version
  default = 1 on decode** when absent (docs/reference/project-io.md "manifest version default 1"; note
  FOUNDATION §5.6 says current 2 — honor the reference's *decode default of 1* and write the current
  version on encode). `MediaFolder { id, name, parent_id: Option }`.
- **`MediaSource` externally-tagged** to match Swift's derived Codable: JSON shape
  `{"external":{"absolutePath":"…"}}` / `{"project":{"relativePath":"…"}}` (ruling-aligned carry-forward,
  docs/reference/project-io.md "Media path resolution"). A flat or internally-tagged enum is FORBIDDEN —
  it breaks round-trip and sample import.
- `GenerationLog { version, entries: Vec<GenerationLogEntry> }`, append-only semantics. `GenerationLogEntry`
  custom decode with **legacy `cost` dollars → `cost_credits = ceil(dollars * 100.0)`** fallback
  (docs/reference/project-io.md Port risks "lenient decode is load-bearing").
- `MediaAsset` serde shape per FOUNDATION §5.6 (id, name, asset_type, folder_id, duration_seconds,
  source_width/height, source_fps, has_audio, created_at, generation_input?, generation_status,
  cached_remote_url?, cached_remote_url_expires_at?). `generation_status: None | Generating | Downloading |
  Rendering | Failed(String)`; `is_open`-style boolean defaults default true where the reference does
  (e.g. `ChatSession.is_open` default true — that shape lands in E2-S8).
- **Date fields are routed through the per-field `Date` codec seam (E2-S8)** — do not hard-code a single
  serde Date format here; `created_at`/`cached_remote_url_expires_at` use the project/media/log codec.
- Unit tests: `MediaSource` external/project JSON round-trips in the externally-tagged shape; a manifest
  JSON omitting `version` decodes to version 1; a generation-log entry with legacy `cost` dollars decodes
  to `cost_credits = ceil(dollars*100)`; a manifest fixture round-trips byte-stable (excluding Date, which
  S8 covers).

**Implementation context:** `palmier-model` (`src/manifest.rs`, `src/genlog.rs`, `src/media_asset.rs`).
Reference: `Models/MediaManifest.swift`, `Editor/ViewModel/EditorViewModel+Cost.swift` (GenerationLog),
`Models/MediaAsset.swift`. FOUNDATION §5.6. docs/reference/project-io.md "Mapping to FOUNDATION crates",
Port risks "lenient decode".

**Dependencies:** E2-S1 (ClipType for asset_type). **Parallel-safe?** Yes against S2–S6 (own files).
Note: shares the `Date` codec seam owned by E2-S8 — coordinate so S8's `mod date` exists; if S8 hasn't
landed, gate the two Date fields behind a temporary local alias and let S8 unify (no merge conflict if S8
owns `src/date.rs` exclusively).

---

### E2-S8 — Per-field `Date` codec seam (Apple reference-epoch doubles + iso8601) + S-1b round-trip gate

**Intent:** As a dev, I want a single per-field `Date` codec abstraction so that project/media/log Dates
serialize as Apple reference-epoch doubles and chat Dates as iso8601, switchable per field, so that S-1b's
confirmation can flip an encoder without touching every shape — and a round-trip regression gate proves it.

**Acceptance criteria:**
- A `date` module in `palmier-model` exposing two serde codecs usable via `#[serde(with = …)]`:
  (1) **`apple_epoch`** — `DateTime<Utc>` ↔ **f64 seconds since 2001-01-01T00:00:00Z** (the Swift default
  `JSONEncoder` numeric reference-date), used by project/media/log; (2) **`iso8601`** — RFC3339 string,
  used by chat. (docs/reference/project-io.md Port risks "Encoder config is type-specific",
  phase0-reconciliation carry-forward "Project I/O Date encoding".)
- Both codecs are **per-field** (applied at the field via `serde(with)`), so a single field's wire format
  can change in isolation (this is the R-6 fallback seam). `MediaManifest`/`GenerationLogEntry`/`MediaAsset`
  Date fields use `apple_epoch`; `ChatSession.updated_at` uses `iso8601`.
- **Provisional until Spike S-1b** (PRD §11, R-6): include a `// PROVISIONAL until S-1b confirms /v1/samples
  payload` marker and a **round-trip regression test** that decodes a captured/fixture sample payload and
  re-encodes it **identically** (the S-1b pass-bar test). If S-1b is not yet available, use a committed
  fixture payload and mark the test `#[ignore = "unblocked by S-1b"]` with a TODO to un-ignore.
- Unit tests: `apple_epoch` round-trips a known instant to the exact f64 (e.g. 2001-01-01 → 0.0; a known
  later date → its exact seconds-since-2001 double); `iso8601` round-trips RFC3339; a struct using both
  codecs on two fields encodes each field in the correct format.

**Implementation context:** `palmier-model` (`src/date.rs`). Reference: docs/reference/project-io.md
"Encoder config is type-specific" (NUMERIC reference-date vs iso8601), Port risks; PRD §11 S-1b, R-6.

**Dependencies:** E2-S1. **Should land before / alongside E2-S7 and the chat shape**, since both consume
the codec. **Parallel-safe?** Yes if it owns `src/date.rs` exclusively; S7 only references the module path.

---

### E2-S9 — `palmier-project` crate scaffold + bundle reader/writer (`VideoProject` read/fileWrapper logic) with atomic whole-directory save

**Intent:** As a dev, I want the `.palmier` directory-bundle read and write ported with the reference's
exact required/optional/soft-error severities and atomic whole-directory save, so that a bundle round-trips
byte-identically and a half-written save can never corrupt a project.

**Acceptance criteria:**
- `palmier-project` crate exists (`crates/palmier-project`, lib; depends on `palmier-model`, `std::fs`,
  `serde_json`). Reference bundle filenames **per ruling #3**: `project.json`, `media.json`,
  `generation-log.json`, `thumbnail.jpg`, `media/`, `chat/`. (Do NOT use FOUNDATION §5.7's
  `timeline.json`/`manifest.json`/`generation_log.json`/`chatsessions/`.)
- **Read** (port `VideoProject.read(from:ofType:)`):
  1. `project.json` MUST exist → else a **hard corrupt error** (map `CocoaError(.fileReadCorruptFile)` to a
     crate error variant `BundleError::Corrupt`). Decode `Timeline` (default decoder).
  2. `media.json` present → decode `MediaManifest`; **decode failure here is ALSO a hard corrupt error.**
  3. `generation-log.json` present → decode tolerantly; **failure is SOFT (logged, ignored)** — absence
     triggers `seed_generation_log_from_assets()` later (Epic 9 detail; here just don't error).
  Preserve these three severities exactly (docs/reference/project-io.md "Read", Port risks "project.json
  missing = corrupt").
- **Save** (port `fileWrapper`/`captureSaveSnapshot`): encode `project.json` (required — missing →
  `BundleError::WriteUnknown`), then `media.json`/`generation-log.json`/`thumbnail.jpg` if present, then a
  freshly built `chat/` dir, then — **only if a live `media/` dir already exists on disk** — snapshot
  `media/` into the package (so newly imported media is captured; importing media must create `media/`
  under the live bundle before save). Encoders are **type-specific** (S8 Date codecs; chat uses
  pretty+sortedKeys, project/media/log use compact default — match per file).
- **Whole-directory atomic save:** write the package to a sibling temp dir, fsync, then **atomic-rename the
  directory** into place (replicating NSDocument safe-save). A partial write must never leave a half-saved
  bundle (docs/reference/project-io.md Port risks "Whole-directory atomic save").
- Unit/integration tests: a bundle missing `project.json` → `Corrupt`; a bundle with corrupt `media.json`
  → `Corrupt`; a bundle with corrupt `generation-log.json` → opens successfully (soft); save→reopen of a
  fixture bundle yields an equal `Timeline`+`MediaManifest` (the **byte-identical round-trip**, SM-7);
  an injected mid-save failure leaves the original bundle intact (atomicity).

**Implementation context:** `palmier-project` (`src/bundle.rs`). Reference:
`Project/VideoProject.swift` (read :31, save :57, fileWrapper :66, captureSaveSnapshot :90),
`Utilities/Constants.swift:104 enum Project` (filename constants). docs/reference/project-io.md "Bundle
layout", "Read", "Save / fileWrapper".

**Dependencies:** E2-S6, E2-S7, E2-S8 (needs Timeline/Manifest/GenLog shapes + Date codecs).
**Parallel-safe?** No — it scaffolds `palmier-project`; S10/S11/S12 build on it.

---

### E2-S10 — Bundle round-trip integration test (SM-7 + SM-1b) + golden fixtures (project / keyframes / text)

**Intent:** As a dev, I want the §11.2 import→edit→save→reopen integration test and committed golden
fixtures, so that round-trip fidelity (SM-7) and open-speed (SM-1b) are enforced in CI and the M1 gate is
verifiable.

**Acceptance criteria:**
- **§11.2 round-trip integration test** (`palmier-project/tests/round_trip.rs`): load a fixture `.palmier`
  → mutate the in-memory model (e.g. move a clip, add a keyframe) → save → reopen → assert the reopened
  model equals the saved model **byte-for-byte on the serialized JSON** (SM-7). Covers every shape:
  Timeline/Track/Clip, MediaManifest/MediaSource, GenerationLog, ChatSession.
- **Golden fixtures committed** (FOUNDATION §11.5, PRD §11.5 → Epic 2 owner): `golden_project` (a
  representative bundle), `golden_project_keyframes` (clips with Smooth/Linear/Hold keyframe tracks on
  multiple properties), `golden_project_text` (text clips with `text_content`/`text_style`). These are the
  same fixtures Epic 5 (SM-C1 rendered-frame) and Epic 6 (XMEML) consume — define them here.
- **SM-1b open-speed assertion:** opening a **30-clip 1080p** fixture project completes **< 1 s** on the §10
  reference HW; encode as a `criterion`-style or timed test with the budget asserted (mark the HW
  assumption; the gate is the timing test existing + passing in the CI lane).
- Golden-update discipline: regenerating fixtures is gated behind a `--update-golden` flag/env; **any
  golden diff in CI blocks merge** (mirrors SM-13/XMEML treatment, R-5).

**Implementation context:** `palmier-project/tests/`, `palmier-project/tests/fixtures/`. Reference:
docs/reference/project-io.md (bundle layout), FOUNDATION §11.2/§11.5. Consumed downstream by Epics 5/6.

**Dependencies:** E2-S9 (and the full model S1–S8). **Parallel-safe?** No — depends on S9; it is the M1
gate capstone.

---

### E2-S11 — `ProjectRegistry` + `ProjectEntry` (atomic full-array writes, standardized-URL dedup, register/remove/delete/update_url/sorted)

**Intent:** As a dev, I want the project registry with the reference's atomic writes and path-normalized
dedup, so that the Home window lists projects newest-first, rename updates the entry instead of orphaning
it, and delete moves the bundle to the Recycle Bin/Trash.

**Acceptance criteria:**
- File: **`project-registry.json`** (ruling #3) at the platform registry dir
  (`%APPDATA%\PalmierProWin\` on Windows, `~/.config/palmier-pro/` on Linux per FOUNDATION/`dirs` crate) =
  a JSON array of `ProjectEntry { id: Uuid, url: PathBuf, created_date, last_opened_date }`.
- Methods 1:1 with the reference (docs/reference/project-io.md "Registry", FOUNDATION §6.1):
  - `register(url)`: **standardize/normalize the URL** (lexical normalizer — `dunce`/normalize, NOT
    `canonicalize` which fails on non-existent paths) as the dedup key; if present, bump
    `last_opened_date = now`; else append a new entry (new UUID, `created_date = now`,
    `last_opened_date = now`).
  - `remove(url)` deletes the entry only. `delete(url)` **trashes the bundle on disk** (Recycle Bin via
    `trash` crate / `SHFileOperation` on Windows; XDG trash on Linux) then removes the entry.
  - `update_url(old, new)` rewrites url + bumps `last_opened_date` (driven by Save-As/rename — see S12).
  - `sorted_entries()` = by `last_opened_date` **descending**. `ProjectEntry::name()` = url last path
    component minus extension; `is_accessible()` = file exists.
- **Every mutation writes the whole array atomically** (`Data.write(.atomic)` → write-temp + atomic
  rename). A synchronous Rust registry is acceptable (the reference's async-load + `pendingMutations`
  replay complexity can be dropped) **as long as atomic full-array writes + standardized-URL dedup are
  preserved** (docs/reference/project-io.md Port risks "Registry race").
- Unit tests: `register` twice with the same path (differing only by separators/`.`/`..`) dedups to one
  entry and bumps `last_opened_date`; `sorted_entries` orders newest-first; `update_url` moves the entry
  (not orphan); `delete` removes the entry (trash call mocked/feature-gated in test); the on-disk JSON is a
  well-formed array after each mutation.

**Implementation context:** `palmier-project` (`src/registry.rs`). Reference:
`Project/ProjectRegistry.swift` (`ProjectRegistry`, `ProjectEntry`, `ProjectRegistryDisk`).
docs/reference/project-io.md "Registry", FOUNDATION §6.1.

**Dependencies:** E2-S9 (crate). **Parallel-safe?** Yes (own file; independent of S10/S12 except crate).

---

### E2-S12 — Media-path resolution (`MediaResolver` / internalize-on-save) + autosave/dirty-tracking + Save-As→registry update + directory-as-document UX (FR-8)

**Intent:** As a dev, I want media-path resolution, autosave/dirty-tracking, the Save-As→registry update
linkage, and the Windows directory-as-document file dialog, so that media internalizes correctly, switching
projects force-flushes, rename never orphans the registry entry, and a `.palmier` directory presents as one
document.

**Acceptance criteria:**
- **MediaResolver / `MediaAsset.to_manifest_entry`** (docs/reference/project-io.md "Media path resolution"):
  `expected_url`: external → absolute path; project → `project_url + relative_path` (relative_path already
  contains the `media/` segment). `to_manifest_entry`: if the asset url is under `project_url` →
  `Project { relative_path = path after project_url/ }`; else `External { absolute_path }` (the
  internalize-on-save heuristic). On open, `restore_assets_from_manifest` rebuilds assets, **logs + skips
  missing files** (does not error), and triggers downstream regeneration hooks (waveform/thumb/metadata —
  Epic 4/5 own the actual regen; here, fire the hook/event).
- **Autosave / dirty-tracking** (docs/reference/project-io.md "Autosave"): a `dirty` flag + a document type
  owning the bundle path; switching away from a project (Home) **force-flushes if dirty** (port
  `AppState.showHome` autosave-before-hide). A chat-session change marks the document dirty (so it is
  autosaved on save — chat persists **on save**, ruling #4). `autosaves_in_place` semantics replicated as a
  debounced save.
- **Save-As / rename → registry:** any bundle path change calls `ProjectRegistry::update_url(old, new)`
  (port `fileURL.didSet`), so a rename updates the entry instead of orphaning it (docs/reference/project-io.md
  Port risks "Save-As rename").
- **FR-8 directory-as-document UX:** on Windows, the open/save file picker presents `.palmier`
  **directories as a single document** via a Tauri custom dialog (open panel: `canChooseDirectories=false`,
  `treatsFilePackagesAsDirectories=false` equivalent — the user picks the `.palmier` dir as one item); on
  Linux it behaves as a directory naturally. (Optional Explorer shell-extension is OUT of scope for this
  story — Tauri custom dialog only; note the §10/FR-8 "both" but scope to the dialog.)
- Unit/integration tests: `to_manifest_entry` returns `Project` for a path under the bundle and `External`
  otherwise; `expected_url` for both variants; opening a manifest with a missing media file logs+skips (no
  error); a dirty document saved on project-switch persists; `update_url` is invoked on a simulated rename;
  the Windows dialog config asserts directory-as-single-document selection (mock the dialog).

**Implementation context:** `palmier-project` (`src/resolver.rs`, `src/document.rs`, `src/dialog.rs`).
Reference: `Models/MediaResolver.swift`, `Models/MediaAsset.swift` (`toManifestEntry`),
`Project/VideoProject.swift` (autosave, `fileURL.didSet`), `App/AppState.swift` (`showHome`, create/open).
docs/reference/project-io.md "Media path resolution", "Autosave", "Create/Open", "macOS APIs to replace".

**Dependencies:** E2-S9 (bundle), E2-S11 (registry — for `update_url`), E2-S7 (manifest/MediaSource).
**Parallel-safe?** Partially — `resolver.rs`/`document.rs`/`dialog.rs` are own files, but it consumes S9 +
S11; safe to run concurrently with S10 once S9/S11 have landed.

---

## Dependency summary

- **Foundation:** E2-S1 (model scaffold) → unblocks all `palmier-model` work.
- **`palmier-model` fan-out (parallel after S1):** S2, S4, S8 are mutually parallel; S3 needs S2; S5 needs
  S2+S3+S4; S6 needs S5; S7 needs S1 (+ Date seam from S8).
- **`palmier-project`:** S9 needs S6+S7+S8 → S10 (round-trip gate) needs S9; S11 needs S9 (parallel to
  S10/S12); S12 needs S9+S11+S7.
- **Cross-epic:** Spike **S-1b** gates the *Date-format confirmation* in S8 only (provisional-until). The
  golden fixtures defined in **S10** are consumed by **Epic 5** (SM-C1 rendered-frame) and **Epic 6**
  (XMEML golden). Epic 3 consumes the `palmier-model` shapes + sampling (S1–S6) and `VolumeScale` rubber-band
  draw constants (S4). Epics 7/9/10 consume `MediaManifest`/`GenerationLog` (S7) and `timeline_frame` (S5).

## Parallelization note for the orchestrator

Each story lands in distinct source files (named in each story's Implementation context), so an agent per
story in its own worktree avoids file collisions. The two shared seams to coordinate: (1) `palmier-model`
crate-root `lib.rs` `mod` declarations — have each `palmier-model` story add only its own `mod` line
(append-only, low conflict); (2) the `Date` codec module (S8) is referenced by S7 — land S8 first or let S7
use a temporary local alias until S8 unifies. S10 is the integration capstone and must land after the full
model + S9.
