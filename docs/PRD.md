---
kind: doc
domain: [build-orchestration]
type: prd
status: validated
links: [[FOUNDATION]] [[phase0-reconciliation]]
title: Palmier Pro (Windows / Linux)
created: 2026-06-20
updated: 2026-06-20
validation_note: >
  Validated after adversarial review (PM/Architect/QA). All blocking and major gaps resolved:
  17-crate count corrected (§1/§10); Convex Date-encoding lock decoupled into Spike S-1b (M1)
  ahead of the Epic 2 serde lock (was sequenced after it via S-2/M3); every FOUNDATION §10 perf row
  now has an SM (added SM-1b open-project); generation/search/CaptionBuilder given measurable
  milestone exits (SM-11/12/13); SM-C1 given a golden rendered-frame verification + sanctioned
  CPU-fallback waiver; FOUNDATION §11 test strategy mapped to owning epics with named e2e exit gates.
  Remaining items are working-decision Open Questions / external dependencies (OQ-7/9/10/11), not
  blocking gaps — tracked in §13 and the Assumptions Index.
---

# PRD: Palmier Pro — Windows / Linux Port
*Working title — see Open Question OQ-10 (branding).*

## 0. Document Purpose

This PRD is the build contract for the downstream architecture, epic, story, and dev agents producing
the Windows-first (Linux-second) clean-room reimplementation of Palmier Pro — an AI-driven non-linear
video editor whose differentiator is **agent-controlled timeline editing via a local MCP server**.

It is grounded in three authoritative sources, in priority order: (1) `docs/FOUNDATION.md` — the locked
specification (the §2 stack is non-negotiable); (2) `docs/phase0-reconciliation.md` — 24 **binding**
amendments where FOUNDATION contradicted the macOS reference, with the reference winning on behavior
parity; (3) `docs/reference/*.md` — 15 subsystem port-notes extracted from the macOS source at
`../palmier-pro`. **Where this PRD and `phase0-reconciliation.md` disagree, the reconciliation doc
wins.** There is no human in the loop for this build; every decision below is made from the grounding
docs and the reference, and tensions are surfaced as Open Questions or `[NOTE FOR PM]` callouts rather
than smoothed over.

The PRD is structured Glossary-anchored: §3 defines vocabulary that the Epic List (§10), FRs, and
Success Metrics use verbatim. The spine is the five primary user workflows from FOUNDATION §1.3,
expressed as User Journeys (§2.3); every epic and FR traces back to one of them. The Epic List (§10) is
ordered by the FOUNDATION §14 dependency chain and is the primary handoff to the epic/story agents.

---

## 1. Vision

Most short-form social content is produced on Windows, and most AI-tooling users already live in the
Claude Code / Claude Desktop / Cursor / Codex ecosystem — none of which the macOS-only reference reaches.
Palmier Pro for Windows/Linux is a native non-linear video editor (NLE) that an LLM can *operate*, not
merely a tool a human operates with assistance. A local MCP server on `127.0.0.1:19789` exposes the
editor's timeline, media library, and generation pipeline as 30 tools and 2 resources, so any
MCP-capable agent can transcribe a recording, propose and apply cuts, generate B-roll, and assemble a
directed edit — while the human reviews, refines, and stays in control with a full Premiere-class manual
editor.

The product is a Tauri 2 shell (native WebView2 on Windows, WebKitGTK on Linux) over a Rust core of 17
crates, with the timeline canvas and preview composited in real time through `wgpu` (D3D12 / Vulkan) and
media decoded/encoded through FFmpeg. It is a behavior-parity port: the same data model, the same MCP
tool surface, the same export fidelity, the same agent prompt as the reference — but on a new substrate,
on the platforms the reference abandoned.

It matters because the strategic bet is **the editor as an agent surface**. The reference proved the
concept on macOS; this port takes it to where the content and the agents actually are, and does so
without sharing a line of code with the GPLv3 Swift reference.

---

## 2. Target User

### 2.1 Jobs To Be Done

- **Turn long-form into shorts without manual scrubbing.** "I recorded 25 minutes; give me a feed of
  vertical clips with the dead air and filler removed."
- **Direct a cut by intent, not by dragging.** "Here's a folder of clips and a script — assemble a
  rough directed edit and let me refine it."
- **Fill gaps with generation in-flow.** "I need a 5-second transition / a title card / a VO line right
  here" — without leaving the editor or holding provider API keys.
- **Edit by hand when I want to, with the agent on standby.** "I drive like I'm in Premiere; the agent
  only acts when I invite it, and its edits never tangle with mine."
- **Keep my existing agent stack.** "My Claude Desktop / Cursor / Codex already talk to Palmier on
  macOS — point them at the same MCP surface on Windows with no client changes."

### 2.2 Non-Users (v1)

- Cinema / broadcast colorists needing 8K, scopes, or cinema-grade grading (FOUNDATION §1.4).
- Teams wanting multi-user collaborative or cloud-stored projects — this is single-user, single-machine,
  local-first.
- macOS users — Mac is explicitly out of scope; they have the reference.

### 2.3 Key User Journeys

*The five FOUNDATION §1.3 workflows are the product spine. Every epic in §10 traces to at least one.*

- **UJ-1. Maya turns a 25-minute monologue into a feed of shorts.**
  - **Persona + context:** Maya is a solo creator on Windows 11 with an RTX 4060; she records talking-head
    monologues and needs vertical clips for TikTok/Reels.
  - **Entry state:** App open, new project created, her `.mp4` imported into the Media panel.
  - **Path:** Runs transcription on the clip (Whisper `small.en`) → opens the Agent panel → asks "cut the
    filler words and dead air" → agent calls `get_transcript`, identifies filler/silence ranges in source
    seconds, converts to project frames, calls `ripple_delete_ranges` → reviews the closed-gap timeline →
    asks "split this into 30-second vertical shorts with captions" → exports each as 9:16 MP4.
  - **Climax:** The timeline collapses dead air in one atomic, undoable operation; captions appear as a
    linked text track; the export feed lands as MP4s.
  - **Resolution:** Maya has a folder of captioned vertical shorts; the agent's edits are on a separate
    undo stack she can reverse without touching her own edits.
  - **Edge case:** If transcription hasn't run, the agent's `get_transcript` returns empty and it tells her
    to transcribe first rather than guessing cut points.

- **UJ-2. Dev assembles a directed cut from a folder of clips and a script.**
  - **Persona + context:** Dev is a small-agency editor who dumps a shoot's B-roll into a folder and has a
    voice-over script.
  - **Entry state:** Project open; a folder of clips imported (recursive, mirroring the directory tree as
    a Media-panel folder hierarchy); script pasted into the Agent panel.
  - **Path:** Asks the agent for "a 30-second cut about the product launch, B-roll under the VO" → agent
    runs `search_media` (visual + spoken) to find semantic moments → `add_clips` / `move_clips` to lay a
    rough sequence → Dev refines by hand (trim, ripple, snap).
  - **Climax:** A coherent rough cut exists in seconds; semantic B-roll is placed at the right beats.
  - **Resolution:** Dev hand-finishes; agent edits and hand edits stay independently undoable.

- **UJ-3. Sam fills a gap with a generated transition mid-edit.**
  - **Persona + context:** Sam, signed in with credits, is mid-edit and missing a connective beat.
  - **Entry state:** Editor open, playhead on the gap, Agent panel open, signed-in via Clerk with credits.
  - **Path:** Asks "generate a 5-second swirl transition here" → agent checks `can_generate`, proposes
    model + params, waits for confirmation (generations cost money and are not undoable) → on confirm,
    `generate_video` creates a placeholder `MediaAsset` (status Generating) → Convex job runs → asset
    downloads and Sam drops it on the timeline.
  - **Climax:** A pulsing placeholder appears immediately; a native toast fires on completion; the clip
    lands on the timeline.
  - **Resolution:** Sam continues editing; the generation is logged to `generation-log.json`.
  - **Edge case:** If `can_generate` is false (signed out / out of credits), the agent refuses and tells
    Sam to sign in or top off, and the generation UI is blocked.

- **UJ-4. Priya hand-edits like Premiere with the agent silent.**
  - **Persona + context:** Priya is an experienced editor who wants full manual control.
  - **Entry state:** Editor open; no agent session active.
  - **Path:** Drag-drop import → drag a clip across tracks → trim edges (with snap) → split at playhead
    (Ctrl+K) → ripple-delete a range → Undo (Ctrl+Z) / Redo (Ctrl+Shift+Z) → Save (Ctrl+S).
  - **Climax:** Every operation is immediate (<100 ms perceived), snaps cleanly, and is undoable on the
    **user** stack — entirely separate from any agent stack.
  - **Resolution:** Project saved as a `.palmier` bundle; no agent involvement at any point.

- **UJ-5. Maya exports a finished cut to the social platform.**
  - **Persona + context:** Maya (UJ-1) is done editing and wants to hand off to the external TypeScript
    social-media generation platform.
  - **Path:** Export → "Export to Social" → produces a standard MP4 plus a sidecar
    `<export>.palmier-meta.json` (transcript, chapter markers, AI-suggested captions, source project hash).
  - **Climax:** The MP4 + sidecar are written; the social platform can consume the sidecar (schema is
    OQ-7, jointly defined — **M1 emits a best-effort sidecar; the frozen schema and full handoff land at
    M5** once OQ-7 resolves with the social-platform team).
  - **Resolution:** Handoff complete; the social platform is out of this repo.

---

## 3. Glossary

*Downstream artifacts use these terms verbatim. Introducing a synonym anywhere is a discipline violation.*

- **Timeline** — Root edit document: `fps`, `width`, `height`, `tracks[]`. `fps`/resolution frozen after
  the first clip. Persisted as `project.json` (ruling #3). One per Project.
- **Track** — Ordered, non-overlapping `Clip`s of one `ClipType`. Has `muted`, `hidden`, `sync_locked`.
- **Clip** — Core editable entity: a placed segment of a `MediaAsset` with timeline placement
  (`start_frame`, `duration_frames`), source trims (`trim_start_frame`/`trim_end_frame` in source frames),
  speed, volume, fades, static visual props, and optional keyframe tracks.
- **Transform** — Clip geometry stored **center-based** (`centerX`, `centerY`, `width`, `height`,
  `rotation`, flips), with legacy top-left migration (ruling #7). Normalized 0..1 canvas space.
- **Keyframe / KeyframeTrack** — Animation samples for `opacity, position, scale, rotation, crop, volume`.
  Interpolation `Linear | Hold | Smooth`; **default Smooth (smoothstep)** (ruling #8).
- **MediaAsset** — A library item (imported or generated); has `generation_status`, folder, source
  metadata. Persisted in `media.json` (ruling #3).
- **Project** — A `.palmier` **directory bundle** presented as a single document: `project.json`,
  `media.json`, `generation-log.json`, `thumbnail.jpg`, `media/`, `chat/` (ruling #3).
- **Project Registry** — `project-registry.json` of `ProjectEntry` records (ruling #3).
- **Editing Engine** — Pure-function timeline transformers: **RippleEngine, OverwriteEngine, SnapEngine**
  only (no Slip/Slide — ruling #11).
- **User Undo Stack / Agent Undo Stack** — Two separate undo stacks. The agent `undo` tool refuses if the
  most recent change came from the user.
- **Preview Composition** — Per-frame wgpu render of stacked `LayerRender`s, sourced from FFmpeg decode;
  replaces the reference's AVFoundation pipeline. Stills/Lottie are first-class GPU textures (ruling #22).
  **Decode ownership (one-decode-owner contract):** the `DecoderThread` and the LRU `FrameCache`
  (FOUNDATION §6.5) live in **`palmier-media`** (FOUNDATION §4: "FFmpeg decode/encode, thumbnails,
  waveforms"); **`palmier-engine` never opens an `AVFormatContext` itself** — it consumes decoded frames
  via a handle from `palmier-media`. Composition (palmier-engine) and decode (palmier-media) are distinct
  crates; the FrameCache home is pinned to palmier-media so the two never both claim it.
- **MCP Server** — Local HTTP JSON-RPC server on `127.0.0.1:19789` exposing **30 tools + 2 resources**
  (ruling #1) via `rmcp`, dispatching into `palmier-tools`.
- **palmier-tools** — The single shared tool dispatcher. Exactly one implementation per tool name,
  invoked by **both** the MCP server and the in-app agent.
- **In-App Agent** — The Agent panel's Anthropic-Messages loop (BYOK or Convex-proxied) that streams text,
  calls `palmier-tools`, and feeds results back.
- **Agent Prompt** — The single shared `instructions` string ported **verbatim** from the reference,
  injected into both the MCP server `instructions` field and the in-app agent `system` (ruling #2).
- **Generation** — An AI media-creation job submitted through Convex (we never hold provider keys);
  lifecycle Generating → Downloading → None | Failed. **Not undoable; costs real money.**
- **Visual Search** — Frame-embedding search via **SigLIP2 base patch16-256, 768-dim** (ruling #13), not
  CLIP. Indexed to `<project>/.search/visual_index.bin`.
- **Transcript Search** — Keyword + semantic search over transcribed segments/words; keyword mode needs no
  model download.
- **Transcription** — Whisper (`whisper-rs`) speech-to-text producing `TranscriptionResult` (words +
  segments in source seconds). Cache key `sha256(content)+model+language` (ruling #19).
- **CaptionBuilder** — Splits transcript segments into screen-ready timed phrases and emits `TextClipSpec`
  records; 14 reference unit tests ported verbatim.
- **XMEML Export** — FCP7 XMEML 4 XML emitter for Premiere/DaVinci/FCP interchange; byte-for-byte
  golden-fixture fidelity required.
- **Convex** — The reference backend, accessed over HTTP (`reqwest` + WebSocket); hosts provider keys,
  billing, sample catalog, model catalog, generation queue, proxied agent stream.
- **Clerk** — Auth provider; React SDK in the webview, JWT forwarded to the Rust backend.
- **can_generate** — Advisory gate `signed_in && tier_allows && has_remaining_credits`; the real gate is
  the Convex server mutation (ruling #24).

---

## 4. Features (FR index)

*Features map 1:1 to the epics in §10; each FR is referenced by an epic. FRs are numbered globally so
downstream artifacts have stable references. This section is intentionally a summary — the
implementation-precise spec for each feature lives in the cited `docs/reference/*.md` and in FOUNDATION,
which the epic/story agents read directly. FRs state capability + testable consequence; the reference
docs carry the byte-level contract.*

### 4.1 App Shell & Project Lifecycle (governs Epic 1)

**Description:** Tauri 2 boot sequence, Home + Project windows, main menu with reference keyboard
shortcuts (Ctrl for Cmd; F11 fullscreen on Windows), settings + registry persistence, sample-project
materialization. Realizes UJ-4. Govern: `docs/reference/settings-account-app.md`, FOUNDATION §6.1/§6.15/§6.16.

- **FR-1: Boot to editable Home.** The app boots (crash handler → tracing → settings → Clerk/Convex config
  → model catalog → MCP server if enabled → Home window). *Consequences:* cold start to project window
  < 3 s on NVMe + RTX 4060 (SM-1); settings read from `%APPDATA%\PalmierProWin\settings.json` (Win) /
  `~/.config/palmier-pro/settings.json` (Linux); pref keys `io.palmier.pro.*.enabled`, absent ⇒ ON
  (ruling #6). The **model-catalog load (Convex `/v1/models`) is async and 24 h-cached** (FOUNDATION §6.1):
  it **must not block reaching the Home/Project window** — offline or slow-Convex cold start still meets
  SM-1, and the catalog populates lazily once it resolves. This decouples SM-1 from the R-4/OQ-9 Convex
  dependency (a failed catalog fetch degrades to the cached/empty catalog, never a boot stall).
- **FR-2: Project registry & windows.** One Project window per project; switching auto-saves the previous;
  registry in `project-registry.json` (ruling #3) sorted newest-first; delete moves to Recycle Bin/Trash.
- **FR-3: Main menu & shortcuts.** All menu items and shortcuts from FOUNDATION §6.1 are present with
  identical bindings (Ctrl substituted for Cmd). *Consequence:* every shortcut in the table is invokable
  and triggers the named action.
- **FR-4: Sample projects.** Fetch `/v1/samples`, resolve + materialize a `.palmier` bundle to
  `%APPDATA%\PalmierProWin\Samples\<slug>\` with download progress. *Consequence:* a resolved sample opens
  and round-trips (its bundle uses the reference filenames so import does not break — ruling #3).

### 4.2 Project I/O & Data Model (governs Epic 2)

**Description:** The `.palmier` directory-bundle read/write, the serde model, autosave, and the
directory-as-document UX on Windows. Realizes UJ-4. Govern: `docs/reference/project-io.md`,
`docs/reference/timeline-model.md`, FOUNDATION §5.

- **FR-5: Bundle round-trip.** Read/write `project.json`, `media.json`, `generation-log.json`,
  `thumbnail.jpg`, `media/`, `chat/` (ruling #3). *Consequences:* import media → edit → save → reopen
  yields byte-identical model state (SM-7); Date encoding matches the reference per field (chat =
  iso8601+pretty+sortedKeys; project/media/log = Apple reference-epoch doubles — confirmed against the
  Convex sample payload by **Spike S-1b (M1, before the serde lock)**; serde is provisional until S-1b with
  a per-field-codec fallback and a round-trip regression gate, carry-forward note R-6).
- **FR-6: Center-based Transform with legacy migration.** Transform is stored center-based; legacy
  top-left projects migrate on load (`centerX = oldX + w − 0.5`) (ruling #7). *Consequence:* a
  reference-authored project opens with clips positioned identically.
- **FR-7: Deterministic model semantics.** Keyframe sampling, frame↔source rounding, and computed
  properties match the reference exactly. *Consequences:* keyframe interpolation default is **Smooth**
  (ruling #8); all source↔timeline conversions use `f64::round` ties-away-from-zero, never
  `round_ties_even` (carry-forward note); serde round-trip unit tests pass for every shape.
- **FR-8: Directory-as-document UX.** On Windows the file picker presents `.palmier` directories as a
  single document via a Tauri custom dialog; on Linux it behaves as a directory naturally.

### 4.3 Timeline Editor & Editing Engines (governs Epic 3)

**Description:** The timeline canvas (geometry, ruler, playhead, per-type clip visuals, rubber bands,
fades, range selection), input (tool modes, selection, drag/trim/split, snap), and the pure-function
editing engines. Realizes UJ-4, UJ-1. Govern: `docs/reference/timeline-model.md`,
`docs/reference/edit-engines.md`, FOUNDATION §6.3/§6.4/§6.8.

- **FR-9: Pointer & Razor tools, selection.** Pointer (V) select/move/trim; Razor (C) split at click.
  Single/Shift/Ctrl click + marquee selection persisting across re-renders by clip ID.
- **FR-10: Drag, trim, split with snap.** Move (cross-track for compatible types — **all visual types
  interchangeable**, ruling #12), trim-left/right, split. *Consequences:* SnapEngine snaps to every clip
  edge + playhead; base threshold 8 px, playhead ×1.5, trim handle 4 px, **sticky multiplier 1.5×**
  (ruling #10); **Slip and Slide are not implemented** (ruling #11). Add-clip-via-drag < 100 ms perceived
  (SM-3).
- **FR-11: RippleEngine.** Compute ripple shifts for deletes (single + multi-range, merged), pushes for
  inserts, sync-locked propagation across tracks; linked clips ride along. *Consequence:* deleting clips
  closes gaps and shifts only clips whose start ≥ removed-range end; unit-tested against the reference
  algorithm.
- **FR-12: OverwriteEngine.** Insertion at `(track, start, duration)` returns clips to delete + trim for
  all cases (inside / overlap-start / overlap-end / cover-multi). Used by drag-drop, paste, agent
  `add_clips`.
- **FR-13: Split & linked clips.** `split_clip` migrates keyframes into the new clip with recomputed
  offsets; linked clips (shared `link_group_id`) move together; timing props propagate, volume/opacity/
  transform/text do not.

### 4.4 Media Import & Panel (governs Epic 4)

**Description:** The left-dock asset browser: drag-drop/file-picker import (recursive folder mirroring),
sort/filter/view modes, thumbnails + waveforms, folder hierarchy, and the three tabs (Media, Captions,
Music = a **generation form**, ruling #14). Realizes UJ-2, UJ-1. Govern: `docs/reference/media-panel.md`,
FOUNDATION §6.2.

- **FR-14: Import.** Native Tauri drop + file picker, multi-file, recursive multi-folder mirroring the
  directory tree as folder hierarchy; supported extensions per FOUNDATION §6.2.
- **FR-15: Browse, sort, filter, view.** **4 sort modes** (dateAdded = insertion order, name, duration,
  type — ruling #15); filter chips; folder/flat/grouped views; thumbnail-size slider; inline folder
  create/rename; marquee select; name search.
- **FR-16: Thumbnails & waveforms.** Video thumb = single JPEG **sprite-sheet + JSON sidecar**; waveform =
  **150 samples/s capped 20000**; cache key `sha256(path|size|mtime).prefix16`; gates waveform=2,
  image-thumb=4, video-thumb ungated (ruling #16). *Note:* mtime key may false-hit on coarse Windows FS —
  watch (carry-forward note).

### 4.5 Preview Composition & Playback (governs Epic 5)

**Description:** The largest replacement of Apple APIs — a Rust composition graph rendered via wgpu from
FFmpeg-decoded frames, with audio mixing via symphonia/cpal/rubato, transport, multiple preview tabs, and
viewport overlays (transform/crop). Realizes all UJs (it is the editor's eyes). Govern:
`docs/reference/preview-engine.md`, FOUNDATION §6.5/§6.6.

- **FR-17: Per-frame composition.** For each visible frame, sample animated props, convert timeline→source
  frame, **fetch the decoded frame from `palmier-media`'s LRU `FrameCache` via a handle** (composition lives
  in `palmier-engine`; decode + FrameCache live in `palmier-media` — see Glossary "Preview Composition";
  palmier-engine never opens an `AVFormatContext`), build `LayerRender`s bottom→top, render textured quads +
  text (cosmic-text) via wgpu. *Consequences:* 4K scrub ≥ 30 fps and 1080p60 ≥ 60 fps on a mid-range GPU
  (SM-2); stills/Lottie are first-class GPU textures, **no `.mov` bake** (ruling #22).
- **FR-18: wgpu→WebView presentation.** The rendered texture is presented into the webview viewport. *This
  mechanism is unspecified in all grounding docs and is a MANDATORY SPIKE before the Epic 5 architecture
  commit* (ruling #23, Spike S-1). *Consequence:* the chosen mechanism hits the §10 FPS targets on both
  Windows (D3D12) and Linux (Vulkan), or the documented CPU-compositing fallback is used.
- **FR-19: Audio mixing & transport.** Decode (symphonia) → resample 48 kHz (rubato) → speed time-stretch
  → volume envelope + fades → sum → cpal out. Transport: play/pause/toggle/seek(mode)/step; `current_frame`
  reactive via Tauri events; SeekMode Exact vs InteractiveScrub with reference tolerance
  `min(0.75, 0.15*activeLayerCount)s` (carry-forward note). *Consequence (testable):* an
  InteractiveScrub-tolerance test asserts the displayed frame is within `min(0.75, 0.15*activeLayerCount)s`
  of the requested target for representative `activeLayerCount` values (1, 3, 6), and Exact mode lands on
  the exact frame; gated in Epic 5 acceptance.
- **FR-20: Preview tabs & overlays.** Closable per-asset tabs + the always-present `.timeline` tab;
  transform overlay (corner/edge/rotation handles, center snap guides) and crop overlay (rule-of-thirds,
  aspect lock), counter-rotated to clip-local axes.

### 4.6 Export (governs Epic 6)

**Description:** Three export modes — rendered video (H.264/H.265/ProRes), FCP7 XMEML 4 XML, and
self-contained `.palmier` bundle — plus the social-platform sidecar handoff. Realizes UJ-5, UJ-1. Govern:
`docs/reference/export.md`, FOUNDATION §6.12.

- **FR-21: Video export.** Per output frame, build composition (same path as preview) → render to wgpu
  texture → read back (or NVENC zero-copy) → FFmpeg encode; mix audio → AAC; mux. Codecs H.264, H.265,
  **ProRes 422 LPCM** (ruling #17 — 4444+alpha deferred, OQ-1). *Consequence:* 1 min 1080p H.264 exports
  faster than real time on an RTX 4060 (NVENC) (SM-5); progress + cancellation via Tauri events.
- **FR-22: XMEML export (golden fidelity).** Emit FCP7 XMEML 4 byte-for-byte against committed golden
  fixtures: 2-space indent, `\n` joins, self-closing tags, exact escape order, TRUE/FALSE literals, exact
  float formats, drop-frame `round(fps*0.066666)`, `file://localhost//` rewrite, rotation negated, center
  as normalized offset-from-0.5 (carry-forward note). *Consequence:* CI diff against goldens is exact;
  text overlays / flips / custom easing are documented as XML-unsupported.
- **FR-23: Self-contained bundle export.** Rewrite `External` media refs → `Project { relative_path }`,
  copy into `media/`, copy log/chat/thumbnail; report `collected, copied_internal, missing, total_bytes`.
- **FR-24: Social-platform sidecar.** "Export to Social" emits MP4 + `<export>.palmier-meta.json`
  (transcript, chapter markers, AI-suggested captions, source project hash). **The exact sidecar SCHEMA is
  OQ-7, a cross-team dependency not finalized until M5.** M1 ships the MP4 + a **best-effort sidecar** with
  the FOUNDATION §6.12 fields under a provisional schema; the **frozen schema (and full UJ-5 realization)
  lands at M5** once OQ-7 resolves jointly with the TypeScript social-platform team. The export epic is
  **not** held to a frozen schema at M1 (see §12 M1/M5 UJ-5 boundary).

### 4.7 MCP Server (governs Epic 7 — the strategic centerpiece)

**Description:** Local HTTP JSON-RPC server (`rmcp` + `axum`) on `127.0.0.1:19789`, exposing **30 tools +
2 resources** dispatching into the shared `palmier-tools`, with origin/content-type/protocol validators,
the verbatim Agent Prompt, ShortId prefixing, the agent undo stack, and the `.mcpb` bundle for Claude
Desktop. Realizes UJ-1, UJ-2, UJ-3. Govern: `docs/reference/mcp-tools.md`,
`docs/reference/agent-instructions.md`, FOUNDATION §6.14/§7.

- **FR-25: Loopback JSON-RPC surface.** `POST /mcp` (single + batched) and
  `GET /.well-known/oauth-protected-resource`; TCP bound to `Ipv4Addr::LOCALHOST` only; validators reject
  non-localhost Origin, non-`application/json`, and bad protocol version. *Consequence:* MCP round trip
  < 100 ms p50 / 300 ms p99 on a 200-clip `get_timeline` (SM-6).
- **FR-26: The 30 tools.** Implement the **complete 30-tool catalogue** (FOUNDATION §6.14 table) with
  identical names, parameters, and semantics; **no missing-6 set** (ruling #1, §13.12 void). Tool
  descriptions are **contract text** ported verbatim (all-or-none `track_index`, ripple_delete units,
  source-vs-timeline frame math, ShortId ≥8-char unique-prefix, get_timeline default-omission + 200-row
  captionGroup cap, **transcript pagination caps 400 segments / 10000 words**, and the distinct
  **image-frame sampling ceiling `maxFrames ≤ 12` (default 6) on `inspect_media`/`inspect_timeline`** —
  these are two different classes of cap, not one; do not encode `12` as a pagination page-size
  (`docs/reference/mcp-tools.md` lines 53-55, 157-158 — carry-forward note).
- **FR-27: ShortId & agent undo.** Outputs use minimum unique ID prefix ≥ 8 chars; inputs accept any
  unambiguous prefix (ambiguous → tool error), via one `IdUniverse` snapshot per call. The `undo` tool
  pops the **agent** stack and reverses one action, refusing if the editor's current undo-action name
  doesn't match the pushed name (carry-forward note) — i.e. refusing after an interleaved user edit.
- **FR-28: Client compatibility.** Re-emit `palmier-pro.mcpb` (`manifest_version 0.4`, name `palmier-pro`)
  with the Node stdio→HTTP shim; Help → MCP Instructions exposes copy URL + Cursor/Claude Code/Codex/Claude
  Desktop install. *Consequence:* existing reference MCP clients connect with only the server URL changed
  (SM-8).

### 4.8 In-App Agent Panel (governs Epic 8)

**Description:** The right-side chat: Anthropic-Messages streaming loop (BYOK or Convex-proxied), tool
execution into the same `palmier-tools`, sessions, mentions/context-hints, and the verbatim Agent Prompt.
Realizes UJ-1, UJ-2, UJ-3. Govern: `docs/reference/agent-panel.md`,
`docs/reference/agent-instructions.md`, FOUNDATION §6.13/§7/§8.3.

- **FR-29: Streaming tool loop.** SSE parse (`message_start` usage, `text_delta`, `tool_use_complete`,
  `message_stop`); on `tool_use`, dispatch every ToolUse to `palmier-tools` synchronously, append
  ToolResult user message, resume; clean cancellation drops the in-flight assistant turn with no
  half-written ToolUse. *Consequence:* exactly 2 ephemeral cache breakpoints (system+tools, conversation
  tail); orphan-tool_use repair injects synthetic Cancelled results (carry-forward note).
- **FR-30: Client selection & key storage.** Anthropic key in OS keyring (account `anthropic-api-key`,
  ruling #5) → `AnthropicClient`; else Clerk-signed-in → Convex-proxied `PalmierClient`; else inline "sign
  in or add key". Image inline limits longest-edge 1568px / 3.5 MB / JPEG q-ladder (carry-forward note).
- **FR-31: Sessions & mentions.** Sessions persisted to `<project>/chat/<uuid>.json` **on document save**
  (ruling #4), loaded sorted by `updated_at` desc; `@`-mentions emit JSON context-hint blocks (mediaAsset
  with base64-inlined images, timelineClip, timelineRange).
- **FR-32: Model availability.** BYOK = all three (Sonnet 4.6 / Opus 4.8 / Haiku 4.5); signed-in **free
  tier = Haiku 4.5**; signed-in **paid tier = catalog-driven**, default Sonnet 4.6, Convex catalog may
  enable Opus (ruling #20). The BYOK-all and free=Haiku / paid-default=Sonnet trio is the reference's
  `availableModels` rule verbatim (`docs/reference/agent-panel.md` lines 33-34, 53: BYOK → all three;
  signed-in → `isPaid ? [.sonnet46] : [.haiku45]`); ruling #20 only layers the catalog-driven Opus
  enablement onto the paid tier (the reference hard-coded paid → `[.sonnet46]`). Not PRD inference.

### 4.9 AI Generation (governs Epic 9)

**Description:** The Convex-proxied generation lifecycle (catalog fetch + per-model validation,
placeholders, upload, submit, subscribe, download, credit gating) and its UI (generation panel, Music tab
form, completion toast). Realizes UJ-3. Govern: `docs/reference/generation.md`, FOUNDATION §6.11/§8.1.

- **FR-33: Generation lifecycle.** Fetch `/v1/models` catalog (24 h cache), validate per-model, create
  placeholder `MediaAsset`(s) (Generating), optionally upload references via Convex tickets, submit
  `generations:submit`, subscribe `generations:by_id`, on success download to `<project>/media/<id>.{ext}`
  (auto-correct extension), notify. *Consequence:* status transitions Generating → Downloading → None |
  Failed are reflected in the UI; a native toast fires on completion.
- **FR-34: Credit gating.** `can_generate = signed_in && tier_allows && has_remaining_credits` is advisory;
  the real gate is the Convex mutation (ruling #24). When false, the generation UI is blocked with "Sign
  in" / "Out of credits".
- **FR-35: Cancellation (v1).** Cancel tears down the client subscription only; the Convex job keeps
  running/billing (ruling #24). A server cancel mutation is deferred (backend out of repo).

### 4.10 Transcription & Captions (governs Epic 10)

**Description:** Whisper transcription (`whisper-rs`, CUDA/Vulkan/DirectML/CPU) and the CaptionBuilder that
splits segments into timed phrases, plus the transcript-driven cut. Realizes UJ-1. Govern:
`docs/reference/transcription.md`, FOUNDATION §6.9.

- **FR-36: Transcription.** Extract audio (FFmpeg → 16 kHz mono PCM) → Whisper → word + segment
  timestamps; bundle `small.en`, offer `medium.en`/`large-v3` downloads (OQ-4); cache key
  `sha256(content)+model+language` (ruling #19, hash first N MB if 25-min hashing is slow). *Consequence:*
  25-min recording transcribes < 2 min on RTX 4060 CUDA with `small.en` (SM-9).
- **FR-37: CaptionBuilder.** Split → distribute time by character count → enforce min duration 0.7 s
  (cascade) → map to timeline frames through trim/speed → emit `TextClipSpec`. **Port the 14 reference
  unit tests verbatim** (grapheme-aware counts; enforceMinDuration may push final phrase past segment end;
  breakOn delimiter+space) (carry-forward note). Caption text case **auto/upper/lower** (ruling #18 — no
  title-case).
- **FR-38: Transcript-driven cut.** Agent `get_transcript` → identify dead-air/filler in source seconds →
  convert to project frames via placement/trim/speed → `ripple_delete_ranges`. Realizes UJ-1's climax.

### 4.11 Visual & Transcript Search (governs Epic 11)

**Description:** Two local search subsystems — visual (SigLIP2 frame embeddings) and transcript (keyword +
semantic) — surfaced as Media-panel "Moments"/"Spoken" sections and the `search_media` tool. Realizes
UJ-2. Govern: `docs/reference/search.md`, FOUNDATION §6.10.

- **FR-39: Visual search (SigLIP2).** Embed sampled frames via **SigLIP2 base patch16-256, 768-dim**
  (ruling #13), index to `<project>/.search/visual_index.bin`; query via text encoder, cosine similarity,
  top-K. Preprocessing: 256×256 squash (no crop, black fill, sRGB BGRA), tokenizer pad-to-64 id 0 no mask,
  raw dot-product on L2-normalized output, cosine floor 0.05, relative cutoff 0.85 (carry-forward note).
  *Consequence:* embeddings reproduce the `.embed` magic format or adopt a new magic and re-index (not
  interchangeable with OpenAI CLIP).
- **FR-40: Transcript search.** Index transcribed segments/words; exact keyword (always available, no
  model) + semantic (BGE-small / all-MiniLM via candle). Click hit → jump preview + select asset; "Use as
  B-roll" → drop at playhead.

### 4.12 Polish, Settings, Telemetry & Release (governs Epic 12)

**Description:** Inspector panel, toolbar, settings (5 tabs), account/billing, Help/MCP-instructions,
feedback, telemetry/logging, updater, and packaging. Realizes UJ-4. Govern:
`docs/reference/inspector.md`, `docs/reference/settings-account-app.md`, FOUNDATION §6.7/§6.8/§6.15/§6.16/§8.4.

- **FR-41: Inspector.** Selection-driven tabs (Text/Video/Audio/AI-Edit/Details) with scrubbable number
  fields, color/font pickers, keyframes side panel + per-property lanes. Volume field range **−60…+15 dB**
  (ruling #9 — verify keyframe-storage floor in code before locking).
- **FR-42: Settings & account.** 5 tabs (Account/General/Models/Agent/Storage); Clerk sign-in + Convex
  billing; General toggles use `io.palmier.pro.{notifications,telemetry}.enabled` (ruling #6), telemetry
  restart-required.
- **FR-43: Telemetry, logging, updater.** `tracing` to platform log dirs (rotated daily, 7 days), Sentry
  (DSN build-injected, PII off, opt-out default — OQ-2), Tauri Ed25519 updater (manifest URL OQ-9 backend,
  channel strategy OQ-1-update).
- **FR-44: Packaging.** Build `.msi` (Windows) and `.AppImage` + `.deb` + `.rpm` (Linux) — Flatpak is OQ-8.

**Cross-cutting NFRs** (apply across all features):

- **Performance:** All FOUNDATION §10 targets are hard acceptance criteria (see §7 SMs).
- **Strict layering:** The frontend never touches FFmpeg/wgpu/filesystem directly — all side effects go
  through Tauri commands; reactive state flows back via Tauri events (FOUNDATION §4).
- **Single tool implementation:** Exactly one `palmier-tools` implementation per tool name, shared by the
  MCP server and the in-app agent — no duplication (FOUNDATION §4).
- **Atomicity:** Every AI-mutated timeline operation is atomically undoable on the agent stack, separate
  from the user stack (FOUNDATION §1.5).
- **GPU floor & fallback:** D3D12 12_0 / Vulkan 1.2 with 4 GB VRAM; below that, CPU compositing via
  FFmpeg libavfilter with degraded frame-stepped preview (FOUNDATION §3).

**Test-strategy traceability (FOUNDATION §11 → owning epics / milestone gates).** Every §11 subsection is
mapped to the build so no test class is orphaned:

| §11 subsection | Owning epic(s) | Milestone gate |
|---|---|---|
| §11.1 Unit (per-crate mandatory coverage) | All crate epics (2,3,5,6,7,8,9,10,11) | Each epic's crate |
| §11.2 Integration (bundle round-trip, tool dispatcher, MCP server, generation lifecycle) | Epics 2, 7, 9 | M1 (round-trip), M2 (dispatcher/MCP), M3 (generation) |
| §11.3 e2e (`tauri-driver`+Playwright, the four §1.3 workflows) | hand-edit→Epics 3/5/6; agent-cut→Epics 7/8/10; generative→Epic 9; B-roll→Epics 8/11 | **hand-edit e2e exits M1; agent-cut + generative e2e exit M2/M3; B-roll e2e exits M4** |
| §11.4 Criterion perf (composition 50/200/1000-clip; per-frame eval; tool dispatch; search index 1k/10k/100k) | Composition→Epic 5; dispatch→Epics 7/8; **search index→Epic 11** | M1 (composition, incl. **1000-clip**), M2 (dispatch), M4 (**search-index query**) |
| §11.5 Golden assets (project/keyframes/text/XMEML + rendered-frame for SM-C1) | Epics 2, 6, 5 | M1 |
| §11.6 MCP compatibility suite | Epic 7 | M2 |

The §11.3 e2e workflows are **named milestone exit gates** (not just deliverables); the §11.4 **1000-clip
composition benchmark** is an explicit Epic 5 acceptance item and the **search-index-query benchmark** an
explicit Epic 11 acceptance item (both previously unmapped).

---

## 5. Non-Goals (Explicit)

- No code or runtime shared with the Swift reference. No Mac, iOS, or iPadOS support.
- No 8K / cinema-grade color grading, scopes, or node-based grading. Target 1080p–4K social formats.
- No multi-user / collaborative editing; no cloud project storage. Single-user, single-machine,
  local-first `.palmier` bundles.
- No bundled provider API keys — all paid generation flows through Convex.
- No mpv-embedded playback, no Electron/CEF outside Tauri, no Qt/GTK-direct/WPF/WinUI/.NET, no mixed-language
  agent code (FOUNDATION §2.3).
- **No Slip / Slide edit gestures in v1** — neither exists in the reference (ruling #11); revisit post-v1.
- **No ProRes 4444 + alpha in v1** — ProRes 422 LPCM only (ruling #17); alpha export deferred (OQ-1).
- **No `/v1/music` built-in library** — the Music tab is a generation form (ruling #14).
- **No server-side generation cancel in v1** — client teardown only (ruling #24).
- No XML export of text overlays, flips, or custom keyframe easing (XMEML limitation).

---

## 6. MVP Scope

### 6.1 In Scope

The complete behavior-parity port across all 12 epics (§10), milestoned M1–M5 (§12): hand-edit MVP →
MCP+agent → generation+transcription → visual search+captions → export polish + release. The full 30-tool
MCP surface, the verbatim agent prompt, golden-fidelity XMEML, and all FOUNDATION §10 performance targets
are in scope for v1.

### 6.2 Out of Scope for MVP

- ProRes 4444 + alpha (OQ-1) — threading alpha through the whole pipeline; defer unless needed sooner.
  `[NOTE FOR PM]` revisit if the social platform requires alpha mattes.
- Slip / Slide gestures (ruling #11) — net-new design, not a port.
- Server-side generation cancel (ruling #24) — backend out of repo.
- Flatpak Linux packaging (OQ-8) — AppImage/.deb/.rpm cover v1.
- `large-v3` / `medium.en` Whisper as bundled defaults — offered as downloads only (OQ-4).
- Distinct branding/fork name (OQ-10) — ship under working title, resolve before public launch.
  `[NOTE FOR PM]` "Palmier Pro Windows" risks confusion with the OSS Mac project.

---

## 7. Success Metrics

*Each SM cross-references the FR(s) and FOUNDATION §1.5/§10 targets it validates.*

**Primary**

- **SM-1: Cold start.** < 3 s to project window on NVMe + RTX 4060-class GPU. Validates FR-1. (§10, §1.5)
- **SM-1b: Open existing project.** Open an existing 30-clip 1080p `.palmier` project < 1 s on the §10
  reference HW (RTX 4060 / Radeon 7600 / Intel A380, NVMe). Validates FR-5, FR-1. (§10) — *closes the
  previously-unmapped §10 "Open existing 30-clip 1080p project" row.*
- **SM-2: Preview FPS.** 4K scrub ≥ 30 fps (5 clips/2 layers, no keyframe motion); 1080p60 ≥ 60 fps.
  Validates FR-17, FR-18. (§10, §1.5) — *gated by Spike S-1.*
- **SM-6: MCP round trip.** `get_timeline` on a 200-clip project < 100 ms p50 / 300 ms p99 over loopback.
  Validates FR-25. (§10)
- **SM-8: Client compatibility.** Claude Desktop, Claude Code, Cursor, and Codex connect to the MCP server
  with **only the server URL changed** from the reference install; the reference MCP test prompts ("what's
  on my timeline?", "cut the filler words", "add a title", "generate B-roll") run with no protocol errors.
  Validates FR-26, FR-28. (§1.5, §11.6)

**Secondary**

- **SM-3: Edit latency.** Add-clip-via-drag < 100 ms perceived; agent tool dispatch < 50 ms p50 / 150 ms
  p99 in-process. Validates FR-10, FR-29. (§10)
- **SM-4: Atomic undo.** Every AI-mutated operation is reversible on the agent stack without affecting the
  user stack; the `undo` tool refuses after an interleaved user edit. Validates FR-27. (§1.5)
- **SM-5: Export speed.** 1 min 1080p H.264 exports faster than real time on RTX 4060 (NVENC, balanced).
  Validates FR-21. (§10)
- **SM-7: Round-trip fidelity.** Import → edit → save → reopen yields byte-identical model state; XMEML
  exports diff exactly against golden fixtures. Validates FR-5, FR-22. (§11.1/§11.5)
- **SM-9: Transcription speed.** 25-min recording < 2 min on RTX 4060 CUDA with `small.en`. Validates
  FR-36. (§10)
- **SM-10: Memory.** < 800 MB RSS idle / < 2.5 GB RSS editor+preview on a 200-clip project. Validates
  cross-cutting NFR. (§10)
- **SM-11: Generation lifecycle (demonstrable exit).** On a `generate_video` request, a placeholder
  `MediaAsset` (status Generating) appears in < 2 s; status transitions Generating → Downloading →
  None|Failed are observed in the UI; a native completion toast fires; the asset downloads to
  `<project>/media/<id>.{ext}`. Verified on the §11.3 "generative augment" e2e (mock Convex via §11.2
  generation-lifecycle integration). Validates FR-33, FR-34. (§11.2/§11.3)
- **SM-12: Search correctness.** `search_media` returns the planted B-roll frame in top-K on the
  `golden_search` fixture (visual scope), and transcript exact-keyword recall = 100% on the
  `golden_project_text`/transcript fixture (spoken scope). Search-index query benchmark runs at 1k/10k/100k
  frames (§11.4). Validates FR-39, FR-40. (§11.4)
- **SM-13: CaptionBuilder golden.** All **14** reference CaptionBuilder unit tests pass byte/timing-exact
  against committed expected `TextClipSpec` fixtures (grapheme-aware counts; min-duration 0.7 s cascade;
  breakOn delimiter+space); fixture regeneration is gated behind `--update-golden` review, and **any diff
  in CI blocks merge** (mirrors SM-7's XMEML treatment). Validates FR-37. (§11.1/§11.5)

**Counter-metrics (do not optimize)**

- **SM-C1: Don't trade fidelity for FPS.** Preview FPS must not be bought by silently dropping keyframe
  interpolation, BT.709 color, or layer accuracy below the reference. Counterbalances SM-2.
  **Verification (otherwise this guardrail is unenforceable):** per-frame **golden rendered-image
  comparison** (SSIM ≥ threshold or exact-within-tolerance) at known frames on `golden_project_keyframes`
  and `golden_project_text`, run on **both** the wgpu path and the CPU fallback. A fidelity regression
  (interpolation/color/layer drift) fails the gate. **Sanctioned exception:** if Spike S-1 forces the
  FOUNDATION §3 CPU-compositing fallback, that path's preview degrades to **frame-stepped, with live
  keyframe interpolation off** per FOUNDATION §3 — this is the *one* waiver of SM-C1's "interpolation"
  clause, and SM-C1's interpolation requirement applies only to the GPU path; color/layer accuracy still
  bind on both paths. (See R-1, S-1, Epic 5.)
- **SM-C2: Don't trade parity for tool count.** Do not add tools beyond the 30 to "improve" the agent
  surface; the surface is exactly 30 + 2 resources for client compatibility. Counterbalances SM-8.
- **SM-C3: Don't trade safety for convenience.** Do not relax the MCP origin/localhost validators to ease
  client setup; loopback-only binding is a security boundary. Counterbalances SM-8.

---

## 8. Decisions on FOUNDATION §13 Open Questions

*Most §13 items are resolved by `phase0-reconciliation.md`; those are restated with the ruling cited. The
genuinely-open ones carry a working decision and are marked `[Wren-visible]`.*

- **OQ-1-update (§13.1 update channel):** **Decision — single `stable` channel for v1**; add `beta` only
  if a tester pool materializes. Tauri updater supports per-platform manifests already (FR-43). Working
  decision, low stakes.
- **OQ-2 (§13.2 telemetry opt-in/out):** **Decision — opt-out, matching the reference** (`io.palmier.pro.
  telemetry.enabled` absent ⇒ ON, ruling #6), Send-default-PII false, restart-required toggle in General.
  Resolved.
- **§13.3 (visual model bundled vs downloadable):** Two parts. **(a) Model identity — resolved by ruling
  #13:** the model is SigLIP2 base patch16-256, not CLIP. **(b) Packaging (bundled vs downloadable) — PRD
  working-decision, NOT settled by ruling #13** (which fixes only the model identity, not its packaging):
  **ship it downloadable, not bundled** (states `model_not_installed → downloading_model`), matching the
  reference's on-demand acquisition and keeping the installer small — SigLIP2 base patch16-256 weights are
  ≈0.8–1 GB depending on ONNX/candle precision (fp16/fp32), large enough that bundling would materially
  bloat the `.msi`/AppImage; downloading on first visual-search use avoids that. The weights must still be
  sourced/converted (ONNX `ort` or candle) — **see Risk R-3** (sourcing is the open unknown; size figure
  is approximate until S-3 confirms the converted artifact). `[Wren-visible]` working-decision.
- **§13.4 (Whisper default):** **Decision — bundle `small.en`** (FOUNDATION §6.9 default); offer
  `medium.en` + `large-v3` as optional downloads. `base.en` is not a default. Resolved.
- **§13.5 (Lottie priority):** **Decision — Lottie is in v1** (the reference ships it; the data model has a
  `lottie` ClipType; preview treats it as a first-class GPU texture, ruling #22). Pre-render Lottie to
  texture per FOUNDATION §6.5. Resolved.
- **§13.6 (tutorial content):** **Decision — port the reference tutorial content** as the starting text;
  light edits only for platform shortcut names (Ctrl/F11). Low stakes; revisit copy before launch.
- **§13.7 (social sidecar schema):** **Open — OQ-7.** Working decision: emit transcript + chapter markers +
  AI-suggested captions + source project hash (FOUNDATION §6.12); finalize the exact schema jointly with
  the TypeScript social-platform team. `[Wren-visible]` — cross-team dependency.
- **§13.8 (Linux distribution):** **Decision — AppImage + .deb + .rpm for v1** (FOUNDATION §3/§8.4);
  Flatpak deferred (OQ-8). Resolved for v1.
- **§13.9 (Convex backend access):** **Open — OQ-9.** Working decision: target the **existing Convex
  deployment** via HTTP with a Clerk JWT; if it rejects the Windows client, stand up a parallel
  Windows-port backend. Gates generation, samples, billing, proxied agent. `[Wren-visible]` — external
  dependency, confirm before M3.
- **§13.10 (branding):** **Open — OQ-10.** Working decision: ship under the working title "Palmier Pro
  Windows"; pick a distinct fork name before public launch to avoid OSS-Mac confusion. `[Wren-visible]`.
- **§13.11 (license compatibility):** **Open — OQ-11, the GPL clean-room contradiction.** Porting the
  agent prompt verbatim (ruling #2) and bundling the reference fonts is **not clean-room** — the result
  inherits GPLv3. Working decision: **accept GPLv3 for the distributed app** (FFmpeg LGPL/GPL, Whisper
  MIT, Tauri MIT/Apache are all GPL-compatible), and keep the prompt/fonts as the GPL-tainted boundary.
  Recorded in `signals/gpl-cleanroom-contradiction`. `[Wren-visible]` — see Risk R-2; this is a
  legal/strategy call, not a technical one.
- **§13.12 (exact MCP tool surface):** **Resolved by ruling #1 — exactly 30 tools + 2 resources; there is
  no missing-6 set.** §13.12 and the "36" figure are void.
- **ProRes 422 vs 4444+alpha (reconciliation Open Item 1 / OQ-1):** **Decision — ProRes 422 LPCM for v1**
  (ruling #17); 4444 + alpha deferred. `[Wren-visible]` — confirm if alpha export is needed sooner.
- **wgpu→WebView spike (reconciliation Open Item 3 / OQ-23):** **Decision — MANDATORY SPIKE S-1 before the
  Epic 5 / Phase 2 architecture commit** (ruling #23). `[Wren-visible]` — see Risk R-1 and §11.

---

## 9. Risk Register

*Severity = impact on shipping. R-1 leads as the #1 architecture risk.*

- **R-1 [Critical] — wgpu→WebView texture presentation is unspecified.** No grounding doc defines how the
  wgpu-composited frame reaches the webview viewport (ruling #23). This gates the entire `palmier-engine`
  preview crate (Epic 5) and SM-2. *Mitigation:* **Spike S-1 first**, before any Epic 5 architecture
  commit. Candidates: (a) native transparent child surface (D3D11/DXGI swapchain via DirectComposition on
  Windows; GTK native child on Linux); (b) DXGI shared-handle into a `<canvas>`; (c) IPC readback
  (fallback, slow, but proves the FPS floor with CPU compositing). *Trigger:* if no candidate hits SM-2 on
  both platforms, fall back to CPU compositing + frame-stepped preview (FOUNDATION §3) and re-scope SM-2.
  *SM-C1 interaction:* the CPU fallback is the **one sanctioned exception to SM-C1** — in fallback mode live
  keyframe interpolation degrades to frame-stepped per FOUNDATION §3, so SM-C1's interpolation clause binds
  only the GPU path (color/layer accuracy still bind on both). An architect choosing the fallback must treat
  SM-C1-interpolation as waived for that branch, not still-binding.

- **R-2 [High] — GPLv3 clean-room contradiction.** The build is specified as "clean-room" (FOUNDATION §1.1)
  yet ports the agent prompt verbatim (ruling #2) and bundles reference fonts — which makes the result a
  GPLv3 derivative, not clean-room. *Mitigation:* accept GPLv3 distribution (OQ-11 working decision);
  confirm all dependency licenses are GPL-compatible (FFmpeg LGPL build, Whisper MIT, Tauri MIT/Apache);
  isolate the GPL-tainted prompt/font boundary so a future clean-room re-authoring is possible if legal
  requires it. *Trigger:* if GPLv3 distribution is unacceptable, the prompt must be re-authored
  clean-room and the SM-8 client-compat bar re-validated against a non-identical prompt. `[Wren-visible]`.

- **R-3 [High] — SigLIP2 weights sourcing.** Visual search requires SigLIP2 base patch16-256 768-dim
  weights as ONNX or candle (ruling #13); the reference ships CoreML, which we cannot use. *Mitigation:*
  source/convert SigLIP2 weights to ONNX (`ort`) or candle; reproduce the `.embed` magic format
  (`PALMEMB1`) or adopt a new magic and re-index; lock preprocessing exactly (256×256 squash, pad-to-64,
  L2-normalized dot product — carry-forward note). *Trigger:* if weights can't be sourced/converted with
  matching output, visual search slips to a later milestone (M4) without blocking M1–M3.

- **R-4 [High] — Convex Rust client maturity.** No native Rust Convex SDK; FOUNDATION §8.1 hedges
  ("`convex-rs` if stable, otherwise raw HTTP via `reqwest`"). Subscriptions need WebSocket live queries
  via `tokio-tungstenite`. This underpins generation (Epic 9), samples (Epic 1), billing, and the proxied
  agent (Epic 8). *Mitigation:* spike the Convex HTTP + WebSocket path early (Spike S-2); prefer raw HTTP
  for determinism; **the sample-payload Date-encoding confirmation is split out into Spike S-1b (M1-critical)
  so the Epic 2 serde lock does not wait on S-2's M3 WebSocket work** (carry-forward note). *Trigger:*
  coupled with OQ-9 — if the existing deployment rejects our client, stand up a parallel backend.

- **R-5 [Medium] — Golden-fidelity XMEML / CaptionBuilder.** Byte-exact XMEML and the 14 CaptionBuilder
  tests are unforgiving; subtle Rust float-formatting or grapheme-counting differences break parity
  silently. *Mitigation:* port the 14 CaptionBuilder tests and the XMEML golden fixtures verbatim into CI;
  gate golden updates behind `--update-golden` review (FOUNDATION §11.5). *Trigger:* any golden diff in CI
  blocks merge.

- **R-6 [Medium] — serde Date round-trip corruption.** Project/media/log use Apple reference-epoch
  (seconds since 2001-01-01) doubles; chat uses iso8601+pretty+sortedKeys. A single wrong serde Date
  format silently corrupts round-trips and breaks sample import (carry-forward note, ruling #3).
  *Mitigation:* **Spike S-1b confirms the Convex sample payload's Date format in M1, before the Epic 2 serde
  lock** (decoupled from S-2's M3 WebSocket slice); per-field Date strategy with a round-trip regression
  gate and a per-field-codec fallback (FR-5/FR-7).

- **R-7 [Medium] — Windows mtime cache false-hits.** Thumbnail/waveform cache key uses
  `sha256(path|size|mtime)` (ruling #16); coarse Windows FS mtime granularity can false-hit and serve
  stale thumbnails. *Mitigation:* watch in QA; fall back to content hashing if false-hits surface.

- **R-8 [Low] — HW-encoder/decoder matrix breadth.** NVENC/QSV/AMF/VAAPI across Windows + Linux + the CPU
  fallback is a wide test matrix (FOUNDATION §11). *Mitigation:* CI GPU + CPU-fallback lanes; degrade
  gracefully to libavfilter CPU export below the GPU floor.

---

## 10. Epic List (prioritized, dependency-ordered)

*Order follows the FOUNDATION §14 dependency chain. Each epic names its goal, the crates it touches
(FOUNDATION §4/§12), concrete acceptance criteria, and the governing `docs/reference/*.md`. Crates are the
**17 core crates of FOUNDATION §4** (palmier-model, -project, -media, -engine, -text, -edit, -history,
-export, -transcribe, -search, -gen, -agent, -mcp, -tools, -auth, -update, -telemetry) plus the
`palmier-tauri` binary (§12 `crates/`) and the `src-ui` frontend — 18 crates + frontend in the workspace.*

### Epic 1 — App Shell & Project Lifecycle
- **Goal:** Boot to a working Home + Project window with menu, settings, registry, and sample
  materialization. (FR-1..FR-4)
- **Crates:** `palmier-tauri`, `palmier-auth`, `palmier-update`, `palmier-telemetry`, `src-ui/app`,
  `src-ui/home`, `src-ui/settings`.
- **Acceptance:** App boots in the §6.1 sequence; **cold start < 3 s to the project window on NVMe +
  RTX 4060-class HW (SM-1)**, with the **model-catalog fetch async/24 h-cached and non-blocking** (offline
  cold start still meets SM-1; FR-1); all §6.1 menu shortcuts fire; registry round-trips; a resolved sample
  opens; pref keys `io.palmier.pro.*` (ruling #6).
- **Governed by:** `docs/reference/settings-account-app.md`; FOUNDATION §6.1/§6.15/§6.16.

### Epic 2 — Project I/O & Data Model
- **Goal:** `.palmier` bundle read/write + the serde model + autosave + directory-as-document. (FR-5..FR-8)
- **Crates:** `palmier-model`, `palmier-project`.
- **Acceptance:** Bundle round-trip byte-identical for every shape (SM-7); **open existing 30-clip 1080p
  project < 1 s on §10 HW (SM-1b)**; reference filenames `project.json`/`media.json`/`generation-log.json`/
  `chat/`/`project-registry.json` (ruling #3); center-based Transform + legacy migration (ruling #7);
  Smooth keyframe default (ruling #8). **Golden-fidelity model gates (FOUNDATION §11.1):**
  (a) **keyframe sampling unit-tested at segment boundaries** — `t=0`, `t=end`, exact-on-key, and
  between-keys — against reference values for **Smooth / Linear / Hold**; (b) a **frame-rounding parity
  test** asserting `f64::round` ties-**away**-from-zero (never `round_ties_even`) on the known divergence
  cases (x.5 source frames) for both source↔timeline directions. Per-field Date encoding confirmed against
  the **M1 sample-payload check (Spike S-1b)** (R-6 — serde Dates are provisional until S-1b lands, with a
  per-field-codec fallback and a round-trip regression gate).
- **Governed by:** `docs/reference/project-io.md`, `docs/reference/timeline-model.md`; FOUNDATION §5.

### Epic 3 — Timeline Editor & Editing Engines
- **Goal:** The timeline canvas, input, and pure-function ripple/overwrite/snap/split engines. (FR-9..FR-13)
- **Crates:** `palmier-edit`, `palmier-history`, `palmier-model`, `src-ui/editor` (timeline).
- **Acceptance:** Pointer/Razor tools; selection persists by ID; cross-track move for all visual types
  (ruling #12); snap thresholds 8/×1.5/4 px, sticky ×1.5 (ruling #10); **no Slip/Slide** (ruling #11);
  ripple/overwrite/split unit-tested against reference algorithms; user undo/redo; add-clip < 100 ms (SM-3).
- **Governed by:** `docs/reference/edit-engines.md`, `docs/reference/timeline-model.md`; FOUNDATION §6.3/§6.4/§6.8.

### Epic 4 — Media Import & Panel
- **Goal:** Asset import, browse, thumbnails/waveforms, folder hierarchy, three tabs. (FR-14..FR-16)
- **Crates:** `palmier-media`, `palmier-model`, `src-ui/media-panel`.
- **Acceptance:** Native drop + recursive folder import; 4 sort modes (ruling #15); sprite-sheet thumbs +
  150 samples/s waveforms with reference cache key + gates (ruling #16); Music tab is a generation form
  (ruling #14, wired in Epic 9); name search.
- **Governed by:** `docs/reference/media-panel.md`; FOUNDATION §6.2.

### Epic 5 — Preview Composition & Playback **(spike-gated)**
- **Goal:** wgpu+FFmpeg per-frame composition, audio mix, transport, preview tabs, overlays. (FR-17..FR-20)
- **Crates:** `palmier-engine`, `palmier-media`, `palmier-text`, `src-ui/editor` (preview).
- **Acceptance:** **Spike S-1 resolved first** (ruling #23, R-1); 4K scrub ≥ 30 fps (5 clips/2 layers, no
  keyframe motion) and 1080p60 ≥ 60 fps on §10 GPU (SM-2); **DecoderThread + FrameCache owned by
  `palmier-media`, palmier-engine consumes frames via handle and never opens an `AVFormatContext`**
  (Glossary boundary); stills/Lottie first-class textures, no `.mov` bake (ruling #22); audio mix with
  speed/volume/fades; **InteractiveScrub-tolerance test** (displayed frame within
  `min(0.75, 0.15*activeLayerCount)s` of target for layer counts 1/3/6) + Exact-mode exact-frame test;
  transform/crop overlays counter-rotated; **§11.4 composition-graph benchmark at 50/200/1000 clips**;
  **SM-C1 golden rendered-frame comparison on `golden_project_keyframes`+`golden_project_text`, run on both
  the wgpu and CPU-fallback paths**; CPU fallback below GPU floor (frame-stepped, interpolation off per
  FOUNDATION §3 — the sanctioned SM-C1 waiver).
- **Governed by:** `docs/reference/preview-engine.md`; FOUNDATION §6.5/§6.6.

### Epic 6 — Export
- **Goal:** Video (H.264/H.265/ProRes 422), XMEML 4 (golden), self-contained bundle, social sidecar.
  (FR-21..FR-24)
- **Crates:** `palmier-export`, `palmier-engine`, `palmier-media`, `palmier-text`.
- **Acceptance:** ProRes **422 LPCM** only (ruling #17); XMEML byte-exact vs goldens (SM-7, R-5); 1 min
  1080p H.264 faster than real time (SM-5); bundle export reports collected/copied/missing/bytes; social
  sidecar emits transcript/markers/captions/hash (schema OQ-7).
- **Governed by:** `docs/reference/export.md`; FOUNDATION §6.12.

### Epic 7 — MCP Server **(strategic centerpiece)**
- **Goal:** Local JSON-RPC server, 30 tools + 2 resources, validators, ShortId, agent undo, `.mcpb`.
  (FR-25..FR-28)
- **Crates:** `palmier-mcp`, `palmier-tools`, `palmier-history`, `palmier-model`, `palmier-edit`.
- **Acceptance:** Loopback-only bind; origin/content-type/protocol validators; **exactly 30 tools**
  (ruling #1) with verbatim contract descriptions; ShortId ≥8-char prefixing; agent undo stack refuses
  after user edits (R-5 carry-forward); **MCP round trip < 100 ms p50 / 300 ms p99 on a 200-clip
  `get_timeline` over loopback (SM-6)**; reference clients connect with only the server URL changed (SM-8);
  the §11.6 MCP compatibility suite passes. **Cap/ambiguity test gates (FOUNDATION §11.1):** ShortId
  expand/shorten **ambiguity returns a tool error, unit-tested both directions**; `get_timeline` enforces
  the **200-row captionGroup cap** and the **400-segment / 10000-word transcript pagination caps**, and
  `inspect_media`/`inspect_timeline` enforce the **`maxFrames ≤ 12` (default 6) image-frame ceiling**, each
  asserted on an over-cap fixture.
- **Governed by:** `docs/reference/mcp-tools.md`, `docs/reference/agent-instructions.md`; FOUNDATION §6.14/§7.

### Epic 8 — In-App Agent Panel
- **Goal:** Anthropic-Messages streaming tool loop, sessions, mentions, model gating. (FR-29..FR-32)
- **Crates:** `palmier-agent`, `palmier-tools`, `palmier-auth`, `src-ui/agent-panel`.
- **Acceptance:** SSE streaming loop dispatches into the **same** `palmier-tools` (no duplication); 2
  ephemeral cache breakpoints + orphan-tool repair (carry-forward); keyring account `anthropic-api-key`
  (ruling #5); sessions persist to `chat/` **on save** (ruling #4); mentions emit context-hints; model
  availability per ruling #20; dispatch < 50 ms p50 (SM-3).
- **Governed by:** `docs/reference/agent-panel.md`, `docs/reference/agent-instructions.md`; FOUNDATION §6.13/§7/§8.3.

### Epic 9 — AI Generation
- **Goal:** Convex-proxied generation lifecycle + UI + credit gating. (FR-33..FR-35)
- **Crates:** `palmier-gen`, `palmier-auth`, `palmier-tools`, `src-ui/media-panel` (generation panel + Music tab).
- **Acceptance:** Catalog fetch + per-model validation; placeholders → submit → subscribe → download;
  native toast on completion; `can_generate` advisory gate, real gate is Convex (ruling #24); cancel =
  client teardown only (ruling #24); Convex HTTP+WS path proven (Spike S-2, R-4).
  **Demonstrable exit (SM-11):** placeholder appears < 2 s, Generating → Downloading → None|Failed observed
  in UI, completion toast fires, asset lands in `<project>/media/`; verified by the **§11.3 "generative
  augment" e2e** (over the §11.2 mock-Convex generation-lifecycle integration). The generation tools
  (`generate_video`, `can_generate`) are **surfaced in M2 (Epic 7)** but return "backend not available" /
  advisory-false until **this epic wires Convex in M3** (ruling #24; Spike S-2 gating) — so UJ-3 is
  end-to-end testable only at M3.
- **Governed by:** `docs/reference/generation.md`; FOUNDATION §6.11/§8.1.

### Epic 10 — Transcription & Captions
- **Goal:** Whisper transcription, CaptionBuilder, transcript-driven cut. (FR-36..FR-38)
- **Crates:** `palmier-transcribe`, `palmier-text`, `palmier-media`, `palmier-tools` (`add_captions`,
  `get_transcript`), `src-ui/media-panel` (Captions tab).
- **Acceptance:** `small.en` bundled; 25-min < 2 min on RTX 4060 CUDA (SM-9); cache key content+model+lang
  (ruling #19); **all 14 CaptionBuilder tests pass byte/timing-exact against committed expected
  `TextClipSpec` fixtures (SM-13)** — regeneration gated behind `--update-golden` review and **any diff in
  CI blocks merge** (R-5, mirrors SM-7); caption case auto/upper/lower (ruling #18); transcript-driven cut
  via `ripple_delete_ranges` (UJ-1).
- **Governed by:** `docs/reference/transcription.md`; FOUNDATION §6.9.

### Epic 11 — Visual & Transcript Search
- **Goal:** SigLIP2 visual index + transcript keyword/semantic search, surfaced in panel + `search_media`.
  (FR-39..FR-40)
- **Crates:** `palmier-search`, `palmier-media`, `palmier-tools` (`search_media`), `src-ui/media-panel`.
- **Acceptance:** **SigLIP2 base patch16-256 768-dim** (ruling #13, R-3) with exact preprocessing
  (carry-forward); `.embed` magic reproduced or new magic + re-index; transcript keyword always available;
  click-to-jump + "Use as B-roll" (UJ-2). **Correctness exit (SM-12):** `search_media` returns the planted
  B-roll frame in top-K on the `golden_search` fixture (visual) and transcript exact-keyword recall = 100%
  (spoken); **§11.4 search-index-query benchmark at 1k / 10k / 100k frames** runs as an explicit acceptance
  item. Weights downloadable, not bundled (≈0.8–1 GB; §8 §13.3 working-decision).
- **Governed by:** `docs/reference/search.md`; FOUNDATION §6.10.

### Epic 12 — Polish, Settings, Telemetry & Release
- **Goal:** Inspector, toolbar, settings/account/help/feedback, telemetry/logging/updater, packaging.
  (FR-41..FR-44)
- **Crates:** `palmier-telemetry`, `palmier-update`, `palmier-auth`, `palmier-model`, `src-ui/settings`,
  `src-ui/editor` (inspector + toolbar), `palmier-tauri`.
- **Acceptance:** Selection-driven Inspector; volume field −60…+15 dB (ruling #9, verify keyframe floor);
  5 settings tabs; `io.palmier.pro.*` pref keys (ruling #6); Sentry opt-out (OQ-2); Ed25519 updater
  (channel OQ-1-update, URL OQ-9); `.msi` + `.AppImage`/`.deb`/`.rpm` artifacts (Flatpak OQ-8); memory
  ceilings met (SM-10).
- **Governed by:** `docs/reference/inspector.md`, `docs/reference/settings-account-app.md`; FOUNDATION §6.7/§6.8/§6.15/§6.16/§8.4.

---

## 11. Spikes Required Before Build

*Time-boxed investigations that gate architecture commits. Run S-1 first.*

- **S-1 [BLOCKER, before Epic 5] — wgpu→WebView presentation.** Resolve how a wgpu-composited frame is
  presented into the Tauri webview on both Windows (D3D12) and Linux (Vulkan), hitting SM-2. Evaluate the
  three candidates in R-1; deliver a working prototype + a chosen mechanism + a measured FPS number per
  platform, or a documented decision to use the CPU-compositing fallback. **Pass bar:** a measured per-platform
  FPS ≥ the SM-2 floors (4K ≥ 30 / 1080p60 ≥ 60), OR an explicit fallback decision. **Fallback note:** if the
  CPU-compositing fallback is chosen, live keyframe interpolation degrades to frame-stepped per FOUNDATION §3
  — that path is the **one sanctioned SM-C1 waiver** (interpolation clause only; see SM-C1, R-1, Epic 5).
  **No Epic 5 architecture commit until S-1 lands** (ruling #23). `[Wren-visible]`.
- **S-1b [M1-CRITICAL, before Epic 2 serde lock] — Convex sample-payload Date encoding.** *Decoupled from
  S-2's WebSocket slice because Epic 2 (M1) locks the `palmier-model`/`palmier-project` serde long before
  S-2's live-query work (M3).* Fetch one real `/v1/samples` + `/resolve` payload from the target deployment
  and **document the exact Date encoding per field** (chat = iso8601+pretty+sortedKeys; project/media/log =
  Apple reference-epoch doubles, ruling #3). **Pass bar:** a recorded sample payload + a round-trip unit
  test proving the chosen per-field serde re-encodes it identically. Until S-1b lands, Epic 2 serde uses the
  documented per-field strategy **provisionally** with a named fallback (switch the field's `Date` codec)
  and a round-trip regression gate (R-6). If Convex access is blocked at M1, use a captured fixture payload.
- **S-2 [before Epic 9] — Convex Rust HTTP + WebSocket client.** Prove the `/v1/*` HTTP calls and a
  `generations:by_id` WebSocket live-query subscription against the target deployment (OQ-9); decide
  `convex-rs` vs raw `reqwest`+`tokio-tungstenite`. **Pass bar (concrete exit):** a passing integration test
  that (a) issues `/v1/models` over HTTP and (b) completes a `generations:by_id` WebSocket round-trip against
  the target deployment, **with the sample-payload Date format already documented by S-1b**. Gates
  generation, samples, billing, proxied agent (R-4, R-6).
- **S-3 [before Epic 11] — SigLIP2 weight conversion.** Source/convert SigLIP2 base patch16-256 768-dim to
  ONNX (`ort`) or candle and reproduce L2-normalized embeddings matching the reference preprocessing
  (R-3). If output can't be matched, re-index with a new `.embed` magic. Can run in parallel with M1–M2.
  **Pass bar:** converted weights produce embeddings whose cosine similarity to reference embeddings on a
  fixture frame set is within tolerance (or a documented re-index decision), and the converted artifact
  size is recorded (confirms the ≈0.8–1 GB §13.3 estimate).
- **S-4 [before Epic 6 export, low-risk confirm] — wgpu texture readback / NVENC zero-copy.** Confirm the
  export read-back path (or NVENC zero-copy) feeds FFmpeg at faster-than-real-time for SM-5. **Pass bar
  (concrete number):** the readback+encode path sustains **≥ the SM-5 throughput (1 min 1080p H.264 < real
  time, NVENC balanced) on the §10 reference GPU**, measured end-to-end. Small spike; de-risks the export
  encoder boundary.

---

## 12. Milestone Plan

*Five milestones mapping the 12 epics; M1 ships a usable hand-edit MVP, the MCP centerpiece lands in M2.*

- **M1 — Hand-Edit MVP.** **Epics 1–6.** Spike **S-1 first**; Spike **S-1b before Epic 2 serde lock**.
  Deliverable: a Premiere-class manual editor — boot, open/save `.palmier`, import media, hand-edit the
  timeline (ripple/overwrite/snap/split, undo/redo), live wgpu preview + audio, and export (video + XMEML +
  bundle). Realizes **UJ-4 fully; UJ-5 partial** (export emits MP4 + best-effort sidecar; the **frozen
  sidecar schema and full UJ-5 land at M5** with OQ-7). **Validates SM-1, SM-1b, SM-2, SM-3, SM-5, SM-7,
  SM-10**, the SM-C1 golden rendered-frame gate, and the **§11.3 hand-editing e2e** (drag/trim/split/undo/
  redo/save) as an exit gate.
  - *Note:* S-1's outcome may re-scope SM-2 to CPU fallback; if so, that is decided at M1, not deferred, and
    SM-C1's interpolation clause is waived on the fallback path (sanctioned exception).

- **M2 — MCP Server + Agent.** **Epics 7–8.** The strategic centerpiece. Deliverable: the 30-tool MCP
  server with reference-client compatibility, and the in-app agent driving the same `palmier-tools`.
  Realizes UJ-2 (hand-assisted), UJ-4 (agent-on-standby). **Validates SM-4, SM-6, SM-8** (incl. §11.6
  suite) and the **§11.3 agent-cut e2e** (transcription-gated cut deferred to M3) as a gate.
  - *Note:* the generation tools (`generate_video`, `can_generate`) are part of the 30-tool surface and so
    are **wired into MCP/dispatch in M2**, but return "backend not available" / advisory-false until M3
    connects Convex (ruling #24; Spike S-2). M2 acceptance is therefore **not** held to a UJ-3 end-to-end
    bar — **UJ-3 is end-to-end testable only at M3**.

- **M3 — Generation + Transcription.** **Epics 9–10.** Spike **S-2 first** (Convex; S-1b already landed in
  M1). Deliverable: Convex-proxied generation (with credit gating + toasts) and Whisper transcription +
  CaptionBuilder + transcript-driven cut. Realizes **UJ-1** (cut + captions), **UJ-3** (generation).
  **Validates SM-9, SM-11** (generation lifecycle), **SM-13** (CaptionBuilder golden), and the **§11.3
  generative-augment e2e**. Gated by OQ-9 (Convex access).

- **M4 — Visual Search + Captions polish.** **Epic 11** (+ caption refinements). Spike **S-3** (SigLIP2)
  resolved by here, runnable in parallel from M1. Deliverable: visual + transcript search in the panel and
  via `search_media`, completing the B-roll-directed workflow. Realizes UJ-2 fully. **Validates SM-12**
  (search correctness + §11.4 search-index benchmark) and the **§11.3 B-roll-directed e2e**.

- **M5 — Export Polish + Release.** **Epic 12** (+ UJ-5 schema finalization). Deliverable:
  Inspector/toolbar/settings/account/help polish, telemetry/updater, packaging (`.msi` + Linux artifacts),
  the **frozen social sidecar schema (OQ-7) completing UJ-5**, and release. Resolve OQ-10 (branding) and
  OQ-1 (ProRes alpha) before public launch. **Release gate (replaces "overall parity"):** re-run the **full
  §11.6 MCP compatibility suite** + the **complete SM regression set (SM-1, SM-1b, SM-2..SM-13, SM-C1..C3)**
  green, plus the four §11.3 e2e workflows. Validates SM-10 and the SM regression set as the concrete
  release bar.

---

## 13. Open Questions

1. **OQ-1 — ProRes 422 vs 4444+alpha.** Shipping 422 LPCM for v1 (ruling #17); confirm if alpha export is
   needed sooner. `[Wren-visible]`
2. **OQ-7 — Social-platform sidecar schema.** Finalize `<export>.palmier-meta.json` jointly with the
   TypeScript social-platform team. **M1 emits a best-effort sidecar under a provisional schema; the frozen
   schema (and full UJ-5 realization) lands at M5** — the export epic is not held to a frozen schema at M1
   (FR-24, §12 M1/M5). `[Wren-visible]`
3. **OQ-8 — Flatpak.** Add Flatpak alongside AppImage/.deb/.rpm? Deferred from v1.
4. **OQ-9 — Convex backend access.** Does the existing deployment accept the Windows client, or do we stand
   up a parallel backend? Gates M3; confirm before M3. `[Wren-visible]`
5. **OQ-10 — Branding / fork name.** "Palmier Pro Windows" risks confusion with the OSS Mac project; pick a
   distinct name before public launch. `[Wren-visible]`
6. **OQ-11 — GPLv3 clean-room contradiction.** Verbatim prompt + bundled fonts make the app a GPLv3
   derivative, not clean-room. Accepting GPLv3 distribution as the working decision; confirm legal/strategy
   acceptance. See Risk R-2. `[Wren-visible]`
7. **OQ-23 — wgpu→WebView mechanism.** Resolved-in-method by Spike S-1, but the chosen mechanism + FPS
   outcome is Wren-visible because it may re-scope SM-2 to the CPU fallback. `[Wren-visible]`
8. **OQ — Keyframe-storage dB floor.** Three distinct dB constants exist in the reference; verify the
   keyframe-storage volume floor in code before locking the field (ruling #9). Engineering confirm in Epic 12.

---

## 14. Assumptions Index

- §4.2 / FR-5 — `[ASSUMPTION]` the Convex sample payload uses Apple reference-epoch doubles for
  project/media/log Dates and iso8601 for chat; **confirmed by Spike S-1b (M1-critical, before the Epic 2
  serde lock) — NOT S-2**, since Epic 2 locks serde in M1 while S-2's WebSocket work is M3. Until S-1b
  lands, Epic 2 serde is provisional with a per-field-codec fallback and a round-trip regression gate (R-6).
- §4.9 / FR-33 — `[ASSUMPTION]` the existing Convex deployment will accept the Windows client with a Clerk
  JWT (OQ-9); if not, a parallel backend is stood up.
- §8 / OQ-11 — `[ASSUMPTION]` GPLv3 distribution is acceptable for the shipped app, accepting that the
  verbatim prompt + bundled fonts forfeit clean-room status (R-2).
- §11 / S-1 — `[ASSUMPTION]` at least one of the three wgpu→WebView candidates hits SM-2 on both platforms;
  if none do, SM-2 is re-scoped to the documented CPU-compositing fallback at M1.
- §4.11 / FR-39 — `[ASSUMPTION]` SigLIP2 base patch16-256 768-dim weights can be sourced/converted to ONNX
  or candle with output matching the reference preprocessing (R-3); else re-index with a new magic.
- §10 Epic 3 / FR-10 — `[ASSUMPTION]` deferring Slip/Slide (ruling #11) does not block any §1.3 workflow —
  none of the five primary workflows requires them.

---

_End of PRD. Downstream: epic/story agents consume §10 + §4; the architecture agent consumes §9 + §11 +
the stack lock (FOUNDATION §2). Where this PRD and `phase0-reconciliation.md` disagree, the reconciliation
doc wins._
