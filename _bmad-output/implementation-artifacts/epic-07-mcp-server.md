---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
title: "Epic 7 — MCP Server (implementation stories)"
governing_reference: [docs/reference/mcp-tools.md, docs/reference/agent-instructions.md]
milestone: M2
crates: [palmier-mcp, palmier-tools, palmier-history, palmier-model, palmier-edit]
---

# Epic 7 — MCP Server (the strategic centerpiece)

## Epic goal

Build the two crates that make Palmier Pro an **agent surface**: `palmier-tools` (the single shared
30-tool dispatcher — exactly one implementation per tool, invoked by *both* this MCP server and the
Epic 8 in-app agent) and `palmier-mcp` (the local loopback HTTP JSON-RPC server on
`127.0.0.1:19789`, `rmcp` + `axum`, with origin/content-type/protocol validators, the two
`palmier://models/*` resources, the verbatim agent `instructions` string, and the `palmier-pro.mcpb`
Claude-Desktop bundle). Behavior parity with the macOS reference is the contract: the same 30 tool
names, the same JSON-Schema parameters, the same **verbatim** tool descriptions and system prompt, the
same ShortId prefixing, the same agent undo semantics, the same error shape — so any existing reference
MCP client (Claude Desktop / Claude Code / Cursor / Codex) connects with **only the server URL changed**.

`palmier-tools` is the spine of the whole product: it is built here in M2 and then *consumed* by Epic 8
(in-app agent) and extended by Epics 9/10/11 (generation / transcription+captions / search) — those
later epics wire their real subsystems into tool implementations that ship as **registered-but-stubbed**
here (schema present, returns "backend not available" / advisory-false until the owning epic lands). The
tool *surface* (all 30 names, schemas, descriptions, dispatch) is complete and client-compatible at M2.

## PRD acceptance this epic must satisfy (PRD §4.7 / §10 Epic 7)

- **FR-25 Loopback JSON-RPC surface** — `POST /mcp` (single + batched) and
  `GET /.well-known/oauth-protected-resource`; TCP bound to `Ipv4Addr::LOCALHOST` **only**; validators
  reject non-localhost Origin, non-`application/json` content-type, and bad protocol version.
  *Consequence:* **MCP round trip < 100 ms p50 / 300 ms p99 on a 200-clip `get_timeline`** over loopback
  (SM-6).
- **FR-26 The 30 tools** — implement the **complete 30-tool catalogue** (FOUNDATION §6.14 table /
  mcp-tools.md) with identical names, parameters, and semantics; **no missing-6 set** (ruling #1, §13.12
  void). Tool descriptions are **contract text** ported verbatim — all-or-none `track_index`,
  ripple_delete units, source-vs-timeline frame math, ShortId ≥8-char unique-prefix, get_timeline
  default-omission + **200-row captionGroup cap**, **transcript pagination caps 400 segments / 10000
  words**, and the distinct **image-frame sampling ceiling `max_frames ≤ 12` (default 6)** on
  `inspect_media` / `inspect_timeline` (two different classes of cap; do not encode `12` as a
  pagination page-size — mcp-tools.md lines 53–55, 157–158, carry-forward note).
- **FR-27 ShortId & agent undo** — outputs use minimum unique ID prefix ≥ 8 chars; inputs accept any
  unambiguous prefix (ambiguous → tool error), via **one `IdUniverse` snapshot per call**. The `undo`
  tool pops the **agent** stack and reverses one action, refusing if the editor's current undo-action
  name doesn't match the pushed name (i.e. refusing after an interleaved user edit) (carry-forward note).
- **FR-28 Client compatibility** — re-emit `palmier-pro.mcpb` (`manifest_version 0.4`, `name`
  `palmier-pro`, `display_name` `Palmier Pro Windows`) with the Node stdio→HTTP shim; Help → MCP
  Instructions exposes copy-URL + Cursor / Claude Code / Codex / Claude Desktop install. *Consequence:*
  existing reference MCP clients connect with only the server URL changed (SM-8).

**PRD §10 acceptance, additionally:** loopback-only bind; origin/content-type/protocol validators;
**exactly 30 tools** (ruling #1) with verbatim contract descriptions; ShortId ≥8-char prefixing; agent
undo refuses after user edits (R-5 carry-forward); **SM-6** round-trip latency; **SM-8** reference-client
compat; the **§11.6 MCP compatibility suite** passes. **Cap/ambiguity test gates (FOUNDATION §11.1):**
ShortId expand/shorten **ambiguity returns a tool error, unit-tested both directions**; `get_timeline`
enforces the **200-row captionGroup cap** and the **400-segment / 10000-word transcript pagination
caps**; `inspect_media` / `inspect_timeline` enforce the **`max_frames ≤ 12` (default 6) image-frame
ceiling** — each asserted on an over-cap fixture.

**Success metrics:** SM-4 (atomic agent undo, separate from user stack; `undo` refuses after interleaved
user edit), SM-6 (MCP round trip), SM-8 (client compatibility), SM-C2 (do **not** add tools beyond 30 —
the surface is exactly 30 + 2 resources), SM-C3 (do **not** relax the origin/localhost validators).

**Milestone (PRD §12): M2 — MCP Server + Agent** (Epics 7–8, the strategic centerpiece). This epic
**validates SM-4, SM-6, SM-8 (incl. the §11.6 suite)** and feeds the **§11.3 agent-cut e2e** gate
(the transcription-gated cut itself is deferred to M3). The **§11.4 tool-dispatch Criterion benchmark**
exits at M2 (shared with Epic 8). The §11.2 MCP-server + tool-dispatcher integration tests exit at M2.

---

## Spike / risk gate

**This epic is NOT spike-gated** (no S-1 dependency). It depends on data/edit subsystems, not on the
unresolved wgpu→WebView presentation mechanism (S-1, which gates Epic 5). `palmier-tools` mutates the
timeline model and calls the pure-function edit engines — none of which need the preview pipeline.

However, several tools' *real* implementations belong to later epics, and the sequencing matters:

- **Generation tools** (`generate_video`, `generate_image`, `generate_audio`, `upscale_media`) and the
  `list_models` catalog + the two `palmier://models/*` resources are **surfaced in M2 (this epic)** but
  return **"backend not available" / advisory-false** until **Epic 9 wires Convex in M3** (ruling #24,
  Spike S-2). M2 acceptance is **not** held to a UJ-3 end-to-end bar. The schemas, descriptions, dispatch
  arms, and ShortId expansion are complete and client-compatible at M2.
- **Transcription-backed tools** (`inspect_media` transcript/word output, `get_transcript`,
  `add_captions`, `search_media` spoken scope) get their real Whisper/CaptionBuilder backing in **Epic 10
  (M3)** and visual `search_media` its SigLIP2 backing in **Epic 11 (M4)**. They are registered with full
  schema in M2 and return an empty/"not-yet-indexed" result until the owning epic lands. `get_transcript`
  on an already-transcribed clip should return real word data if Epic 10's transcription store is present;
  if not, empty (the reference returns empty → agent tells user to transcribe first; UJ-1 edge case).
- **`import_media`** needs `palmier-media` import (Epic 4, lands M1) for path/bytes and the Convex/HTTP
  fetch (Epic 9 plumbing) for url; in M2 path/bytes are live, url-async may stub until M3.

**Risk note (R-5 carry-forward — golden parity):** tool descriptions and the agent prompt are
**load-bearing contract text** the LLM was tuned against; paraphrasing them silently breaks SM-8. Port
**byte-for-byte** from `Sources/PalmierPro/Agent/Tools/AgentInstructions.swift` and the per-tool
`description` strings in `ToolDefinitions.swift`. Any drift fails the §11.6 compatibility suite.

**Binding rulings consumed here:** **#1** (exactly 30 tools, §13.12 void — SM-C2), **#2** (agent prompt
ported **verbatim**, single shared constant, both injection sites), **#3** (reference filenames — the
`.mcpb` and resource bodies use reference identity), plus carry-forward notes: tool descriptions are
contract text; **`f64::round` ties-away** for any frame math a tool performs; **agent undo refuses on
undo-action-name mismatch**; ShortId ≥8-char unique-prefix.

---

## Cross-epic dependencies (must land or be stubbable first)

- **Epic 2 (palmier-model, palmier-project)** — the serde timeline/media/folder model the tools read and
  mutate. **Hard prerequisite** (lands M1).
- **Epic 3 (palmier-edit, palmier-history)** — RippleEngine / OverwriteEngine / SnapEngine and the
  history/undo machinery the mutating tools call. **Hard prerequisite for the EDIT tools** (lands M1).
  The **agent undo stack** is a distinct stack but shares the `palmier-history` undo-action-name
  mechanism — Epic 3 must expose named undo groups (carry-forward: "Move 3 clips").
- **Epic 4 (palmier-media)** — `get_media`, `import_media` (path/bytes), thumbnails. Lands M1.
- **Epic 1 (palmier-tauri)** — the MCP server is started/stopped by the app boot sequence behind the
  `io.palmier.pro.mcp.enabled` pref (ruling #6, absent ⇒ ON); Help → MCP Instructions tab lives in the
  settings/help UI. Lands M1.
- **Epic 8 (palmier-agent)** — *consumes* `palmier-tools` and *shares* the agent-prompt constant; built
  immediately after in M2. The shared prompt constant must live in a place both crates import.
- **Epics 9 / 10 / 11** — wire real backends into the stubbed tools at M3 / M3 / M4 (see Spike/risk gate).

---

## Story decomposition

13 stories. E7-S1 (tool schema catalogue) and E7-S2 (ToolResult + dispatch skeleton) are the
foundation everything else builds on; the per-category implementation stories (E7-S5..S10) are largely
parallel-safe because the reference splits them into one `ToolExecutor+<Category>.swift` extension file
per category. The MCP transport stories (E7-S11..S13) run in parallel with the tool stories once the
dispatch entry point (E7-S2) exists.

---

### E7-S1 — Tool schema catalogue: `ToolName` + `AgentTool` + the 30 verbatim definitions

**Intent.** As a porter of the agent surface, I want the 30 tool names, descriptions, and JSON-Schemas
defined verbatim in `palmier-tools` so that both the MCP server and the in-app agent advertise an
identical, client-compatible tool list.

**Acceptance criteria.**
- A `ToolName` enum with **exactly 30** snake_case string variants matching the reference wire names
  (mcp-tools.md §"The 30 tools" / FOUNDATION §6.14 catalogue): `get_timeline, get_media, inspect_media,
  get_transcript, inspect_timeline, search_media, list_models, list_folders, add_clips, remove_clips,
  remove_tracks, move_clips, set_clip_properties, set_keyframes, split_clip, ripple_delete_ranges, undo,
  add_texts, add_captions, generate_video, generate_image, generate_audio, upscale_media, import_media,
  create_folder, move_to_folder, rename_media, rename_folder, delete_media, delete_folder`. A compile-time
  or unit assertion proves `ToolName` has 30 variants and the `all` definition list has 30 entries
  (mirror the reference's two-way grep verification). **SM-C2: adding a 31st tool fails this gate.**
- An `AgentTool { name: ToolName, description: &'static str, input_schema: serde_json::Value }` struct and
  a `pub fn all() -> Vec<AgentTool>` returning 30 entries.
- Each tool's `description` is the **byte-for-byte** reference string from `ToolDefinitions.swift` (and
  its category extension). **No paraphrase.** Store descriptions as `include_str!` or `const &str`;
  preserve Unicode (`×` U+00D7, `–` U+2013, `•` U+2022, `…`) as UTF-8 (do not ASCII-fold).
- Each `input_schema` is a JSON-Schema object built by an `object_schema(properties, required)` helper
  that **omits empty `properties`/`required`** (matches reference `objectSchema`). Schemas match the
  reference's required/optional fields exactly. **Pin the easy-to-miss cases:** `generate_audio` has
  `required: []` (NO required field — prompt is optional, for video-to-music); `create_folder` /
  `move_to_folder` / `rename_media` / `rename_folder` are **dual-shape** (direct fields **XOR**
  `entries[]`, "not both"); `set_keyframes` `interp` default is **smooth** (ruling #8), enum
  `{linear, hold, smooth}`; `inspect_media`/`inspect_timeline` `max_frames` is capped 12 default 6 in the
  *description*; `ripple_delete_ranges` requires **exactly one** of `track_index` / `clip_id`.
- A unit test asserts each tool name's schema deserializes as valid JSON-Schema and that `rmcp` /
  `serde_json::Value` round-trips it unchanged.

**Implementation context.**
- **Crate:** `palmier-tools`. New module `tools/schema.rs` (or `definitions.rs`).
- **Key types:** `enum ToolName`, `struct AgentTool`, `fn all() -> Vec<AgentTool>`,
  `fn object_schema(props, required) -> serde_json::Value`. The `[String:Any]→Value` bridge
  (`mcpSchemaValue`/`ToolArgsBridge`) collapses to using `serde_json::Value` directly (rmcp takes it).
- **Reference files:** `Sources/PalmierPro/Agent/Tools/ToolDefinitions.swift` (enum lines 5–34, `all`
  line 44, `objectSchema`); the per-category description strings live across `ToolExecutor+*.swift`.
- **Docs:** mcp-tools.md §"The 30 tools" (per-tool required inputs + output shape), §"Mapping to
  FOUNDATION crates"; agent-instructions.md §C (objectSchema/empty-omission). Reconciliation #1 (count),
  #8 (smooth default).

**Dependencies.** None (pure data; can start day 1 of M2). Names cross-checked against Epic 2 model.
**Parallel-safe?** Yes — sole owner of `tools/schema.rs`.

---

### E7-S2 — `ToolResult`, the dispatch entry point, and the 30-arm `run` skeleton

**Intent.** As the shared tool dispatcher, I want a single `execute(name, args)` entry point with a
30-arm exhaustive dispatch and a `ToolResult` output type so that both the MCP server and the in-app
agent invoke tools through exactly one code path with one error shape.

**Acceptance criteria.**
- `ToolResult { content: Vec<Block>, is_error: bool }` where `Block = Text(String) |
  Image { base64: String, media_type: String }`. A `to_mcp_result()` maps to the rmcp `CallTool.Result`
  shape. Error shape is **exactly** `{ "isError": true, "content": [{ "type": "text", "text": <msg> }] }`
  (FOUNDATION §6.14 / mcp-tools.md §"Error shape").
- A single `pub async fn execute(&self, name: &str, args: serde_json::Value) -> ToolResult` entry point
  that: (1) resolves `name` → `ToolName` (unknown name → error), (2) runs **arg validation** (E7-S3),
  (3) runs **ShortId input expansion** (E7-S4), (4) dispatches to the `run` arm, (5) runs **ShortId output
  shortening** (E7-S4), (6) updates the **agent undo stack** (E7-S2b/E7-S12).
- `run` is a **30-arm match, exhaustive, no `default`/`_` arm** (mirrors the reference's exhaustive
  switch). In this story the 24 not-yet-implemented arms return a typed `todo!`-style "not implemented"
  ToolResult so the surface compiles and dispatches; later stories fill real bodies.
- The dispatcher is single-owner-serialized: tool calls run through **one** owner of `EditorState`
  (`Mutex<EditorState>` or a single-threaded command actor) — replicating the reference's `@MainActor`
  serialization. A test issues two concurrent `execute` calls and asserts they serialize (no data race).
- Unit test: dispatching every one of the 30 names reaches its arm (no "unknown tool" for any of the 30);
  an unknown name returns the error shape with `is_error: true`.

**Implementation context.**
- **Crate:** `palmier-tools`. Module `tools/executor.rs`. Owns the `EditorState` handle (a
  `palmier-model` `Timeline`/`MediaLibrary` + `palmier-edit` engines + `palmier-history`).
- **Key types:** `struct ToolExecutor`, `enum Block`, `struct ToolResult`, `fn execute`, `fn run`.
- **Reference files:** `ToolExecutor.swift` (`execute(name:args:)` single entry, `run(_:_:_:)` 30-arm
  dispatch at lines 47–78, `agentUndoStack` lines 33–36), `ToolResult.swift` (`Block`, `toMCPResult()`).
- **Docs:** mcp-tools.md §"Key types & files", §"Error shape", §"Mapping to FOUNDATION crates".
  Replace `@MainActor`/`@Observable` with `Mutex<EditorState>`/single command actor (mcp-tools.md
  §macOS APIs to replace).

**Dependencies.** E7-S1 (`ToolName`); Epic 2 (`palmier-model` `EditorState`/`Timeline`); Epic 3
(`palmier-edit`, `palmier-history` for the editor handle type). **Parallel-safe?** Partly — owns
`tools/executor.rs`; must land before any per-category impl story (E7-S5..S10) can fill its arm.

---

### E7-S3 — Argument validation: unknown-key rejection, non-finite guard, JSON-path errors, colors

**Intent.** As the dispatcher, I want strict argument validation matching the reference so that malformed
tool calls fail with the same human-readable, JSON-path-anchored errors clients expect.

**Acceptance criteria.**
- **Unknown-key rejection:** each tool's args are validated against an allowed-keys set
  (`DecodableToolArgs.allowedKeys` equivalent); an unexpected key → ToolError naming the key. (Note the
  dual-shape XOR tools accept the union of both shapes' keys but reject "both shapes at once".)
- **Non-finite guard:** any `NaN`/`±Inf` number anywhere in the args (recursively) → error reporting the
  **JSON path** to the offending value, **before** decode (`firstNonFiniteNumberPath` equivalent).
- **Decode errors** are formatted with the JSON path to the failing field (not a bare serde message).
- **Color parsing** accepts `#RRGGBB` and `#RRGGBBAA` (the `TextStyle.RGBA(hex:)` equivalent); invalid
  hex → error. Used by `add_texts`, `add_captions`, `set_clip_properties`.
- Unit tests: unknown key rejected; `NaN` in a nested array rejected with correct path; `#RRGGBBAA`
  parses; `#GGG` rejected; a dual-shape tool given both `name` and `entries[]` is rejected.

**Implementation context.**
- **Crate:** `palmier-tools`. Module `tools/validate.rs`.
- **Reference files:** `ToolExecutor.swift` lines 134–198 (`validateUnknownKeys`,
  `firstNonFiniteNumberPath`, decode-error formatting), `TextStyle.RGBA(hex:)`.
- **Docs:** mcp-tools.md §"Arg validation". Colors via reconciliation #9 context (dB ranges elsewhere).

**Dependencies.** E7-S1, E7-S2. **Parallel-safe?** Yes — owns `tools/validate.rs`; E7-S2 calls into it.

---

### E7-S4 — ShortId: `IdUniverse` snapshot, ≥8-char unique-prefix shorten, key-allowlist expand

**Intent.** As the dispatcher, I want UUID prefix shortening on outputs and prefix expansion on inputs so
that the agent passes back short stable ids and the system accepts any unambiguous prefix — and **rejects
ambiguous ones with a tool error**.

**Acceptance criteria.**
- **`IdUniverse` snapshot:** one snapshot per tool call collecting **all** ids — track ids, clip ids,
  `caption_group_id`, `link_group_id`, asset ids, folder ids — into one `HashSet<String>`
  (`currentIdUniverse`).
- **Output shortening:** `idPrefixFloor = 8`; each full id → the **shortest prefix ≥ 8 chars unique**
  across the universe. A regex (`[0-9A-Fa-f]{8}-…` UUID pattern) replaces every **known** UUID in result
  text with its short prefix; **unknown UUIDs** (e.g. embedded in filenames) **pass through untouched**.
- **Input expansion (runs BEFORE the tool):** only keys in the **scalar allowlist**
  `{clip_id, source_clip_id, media_ref, start_frame_media_ref, end_frame_media_ref, source_video_media_ref,
  video_source_media_ref, folder_id, parent_folder_id}` and the **array allowlist**
  `{clip_ids, asset_ids, folder_ids, reference_media_refs, reference_image_media_refs,
  reference_video_media_refs, reference_audio_media_refs}` are expanded, recursing into nested
  dicts/arrays. Resolution: exact match → keep; exactly 1 prefix match → expand; **>1 → ToolError
  ("Ambiguous id …")**; 0 → pass through (tool emits its own not-found).
- **Test gates (FOUNDATION §11.1, PRD §10):** ShortId expand/shorten **ambiguity returns a tool error,
  unit-tested both directions** — (a) two ids sharing an 8-char prefix force a ≥9-char shorten on output;
  (b) an input prefix matching two universe ids returns the ambiguity error; (c) a < floor / non-existent
  prefix passes through; (d) a UUID inside a filename in result text is NOT rewritten.
- **Carry-forward gotcha:** a new id-bearing input field must be added to **both** allowlists or it won't
  accept prefixes — document this at the allowlist definition site.

**Implementation context.**
- **Crate:** `palmier-tools`. Module `tools/short_id.rs`.
- **Key types:** `struct IdUniverse`, `fn shorten_ids(text, universe)`, `fn expand_id_prefixes(args,
  universe)`, the two key allowlists as `const`. Use the `uuid` + `regex` crates (mcp-tools.md §macOS
  APIs to replace: `Foundation.UUID` → `uuid`, `Regex` literal → `regex` same pattern).
- **Reference files:** `ToolExecutor+ShortId.swift` (`idPrefixFloor`, `currentIdUniverse`, `shortIdMap`,
  `shorteningIds`, `expandingIdPrefixes`, `scalarIdKeys`, `arrayIdKeys`, `uuidRegex`).
- **Docs:** mcp-tools.md §"ID prefix shortening"; FOUNDATION §6.14 "ID prefix shortening"; FR-27.

**Dependencies.** E7-S1, E7-S2; Epic 2 model (id-bearing types for the universe). **Parallel-safe?** Yes
— owns `tools/short_id.rs`.

---

### E7-S5 — READ tools (timeline/media): `get_timeline`, `get_media`, `list_folders`, `get_transcript`

**Intent.** As an agent, I want to read the timeline, library, folder tree, and transcript so I can plan
edits — with the reference's exact output shaping (default-omission, captionGroup collapse, pagination).

**Acceptance criteria.**
- **`get_timeline`** (`start_frame?`, `end_frame?`): returns `fps`, `width`/`height`, `total_frames`,
  `tracks[{type, clips[…]}]`, `can_generate`. **Default-valued clip/track fields are OMITTED.** Caption
  clips on a track **collapse into per-track `caption_groups`** — shared style props hoisted; rows are
  `[clip_id, start_frame, duration_frames, text]`, **capped at 200 rows**, paged via `start_frame`/
  `end_frame`. **Test gate:** an over-200-caption fixture asserts the 200-row cap + paging; a clip with all
  default props asserts those fields are absent from the JSON.
- **`get_transcript`** (`start_frame?`, `end_frame?`, `clip_id?`): clips in timeline order, words
  `[text, start_frame, end_frame]` **capped 10000**, paged via `next_start_frame`. Walks every audio/video
  clip, mapping words through **trim / speed / position** using **`f64::round` ties-away** for source↔
  timeline conversion. Returns empty if no transcription exists (UJ-1 edge case → agent tells user to
  transcribe). **Real word data depends on Epic 10's transcription store** (M3); in M2 returns
  empty/placeholder if the store is absent.
- **`get_media`**: `assets[{id, name, type, duration, generation_status ∈ {generating, downloading,
  failed, none}, folder_id}]`.
- **`list_folders`**: `folders[{id, name, parent_folder_id?}]`.
- All four are **non-mutating** (do not touch the agent undo stack). Output strings pass through ShortId
  shortening (E7-S4).

**Implementation context.**
- **Crate:** `palmier-tools`, module `tools/read.rs` (or split `inspect_timeline.rs` reference layout:
  `ToolExecutor+InspectTimeline.swift` covers `get_timeline` shaping). Reads `palmier-model` types only.
- **Reference files:** `ToolExecutor+InspectTimeline.swift` (get_timeline shaping, captionGroup collapse),
  `ToolExecutor+Texts.swift`/`+Folders.swift` for folder/transcript helpers, `ToolExecutor.swift` walk.
- **Docs:** mcp-tools.md tools #1, #2, #4, #7; FOUNDATION §6.14 rows; FR-26 caps. Carry-forward
  `f64::round` ties-away for the word frame-mapping.

**Dependencies.** E7-S1, E7-S2, E7-S4; Epic 2 model. `get_transcript` real data: Epic 10 (M3, stubbed
empty in M2). **Parallel-safe?** Yes — owns `tools/read.rs`.

---

### E7-S6 — EDIT tools (clip mutations): `add_clips`, `remove_clips`, `remove_tracks`, `move_clips`, `split_clip`

**Intent.** As an agent, I want to add, remove, move, and split clips/tracks as atomic, undoable
operations so I can build and restructure the timeline the way the reference does.

**Acceptance criteria.**
- **`add_clips`** (`entries[{media_ref, start_frame, duration_frames, track_index?}]`, **all-or-none**
  `track_index`): returns new clip ids; **whole batch is ONE undo**; video-with-audio **auto-creates a
  linked audio clip**; same-track overlap → trim/split/overwrite via **OverwriteEngine** (Epic 3).
- **`remove_clips`** (`clip_ids[]`): removes the **whole link group** of any referenced clip.
- **`remove_tracks`** (`track_indexes[]`): remaining indexes shift down.
- **`move_clips`** (`moves[{clip_id, to_track?, to_frame?}]`, ≥1 move): **linked partners follow the
  `start_frame` delta** (preserving l/j-cut); track changes do **not** propagate to partners.
- **`split_clip`** (`clip_id`, `at_frame` strictly between start/end): migrates keyframes into the new
  clip with recomputed offsets (Epic 3 `split` semantics, FR-13).
- Every one of these is **mutating**: wraps work in a named undo group (e.g. "Move 3 clips") via
  `palmier-history`, and after a non-error run that changed the timeline, pushes that name to the **agent
  undo stack** (E7-S12). Unit tests against reference algorithm outcomes for overlap/trim/split/link cases.

**Implementation context.**
- **Crate:** `palmier-tools`, module `tools/clips.rs`. Calls `palmier-edit` (Overwrite/Ripple/Snap) and
  `palmier-model` mutators; **does not** re-implement edit math (reuse Epic 3 engines).
- **Reference files:** `ToolExecutor+Clips.swift`.
- **Docs:** mcp-tools.md tools #9–12, #15; FOUNDATION §6.14; FR-26/FR-27; Epic 3 FR-11/FR-12/FR-13.

**Dependencies.** E7-S1, E7-S2, E7-S12 (agent-undo push); **Epic 3 (`palmier-edit`, `palmier-history`)**
hard prerequisite. **Parallel-safe?** Yes — owns `tools/clips.rs`.

---

### E7-S7 — EDIT tools (properties/keyframes/ripple): `set_clip_properties`, `set_keyframes`, `ripple_delete_ranges`

**Intent.** As an agent, I want to set clip properties, replace keyframe tracks, and ripple-delete ranges
so I can style clips and close gaps with the reference's exact semantics.

**Acceptance criteria.**
- **`set_clip_properties`** (`clip_ids[]` + any of `duration_frames, trim_start_frame, trim_end_frame,
  speed, volume, opacity, transform{center_x, center_y, width, height, flip_h, flip_v}, content,
  font_name, font_size, color, alignment`): values applied to **ALL** listed clips; **`trim_*` are SOURCE
  offsets**; setting `volume`/`opacity` **clears that property's keyframe track**; text-only fields on a
  non-text clip → **reject**. Transform is **center-based** (ruling #7).
- **`set_keyframes`** (`clip_id`, `property ∈ {volume, opacity, rotation, position, scale, crop}`,
  `keyframes[[frame, …values, interp?]]`): **replaces** the track (empty array clears); frames are
  **CLIP-RELATIVE**; `interp ∈ {linear, hold, smooth}` **default smooth** (ruling #8); per-property row
  layout — `position` = topLeft xy, `scale` = normalized wh, `crop` = top,right,bottom,left.
- **`ripple_delete_ranges`** (`ranges[[start, end]]` + **exactly one** of `track_index` (project frames,
  `units` must be `'frames'`) **or** `clip_id` (clamped, `units ∈ {seconds, frames}` default frames)):
  returns the anchor track's post-cut layout. **Overlaps merge**; **linked partners cut on the same span**;
  **sync-locked tracks shift to preserve alignment** (refuse if it would cross frame 0). Uses
  **RippleEngine** (Epic 3, FR-11).
- All three mutating: named undo group + agent-undo push. **Carry-forward gotcha:** the `units` semantics
  and `track_index` XOR `clip_id` are **contract text** in the tool description — the dispatch must enforce
  them exactly. Frame math uses **`f64::round` ties-away**.

**Implementation context.**
- **Crate:** `palmier-tools`, module `tools/properties.rs` + `tools/ripple.rs`. Calls Epic 3 engines and
  Epic 2 keyframe model (Smooth default lives in the model serde, ruling #8).
- **Reference files:** `ToolExecutor+Clips.swift` / `+Timeline.swift` (property + keyframe + ripple).
- **Docs:** mcp-tools.md tools #13, #14, #16; FOUNDATION §6.14; reconciliation #7, #8; FR-26.

**Dependencies.** E7-S1, E7-S2, E7-S12; **Epic 3 (RippleEngine), Epic 2 (keyframe model)**. **Parallel-
safe?** Yes — owns `tools/properties.rs` + `tools/ripple.rs` (distinct from E7-S6's `clips.rs`).

---

### E7-S8 — TEXT + CAPTION tools: `add_texts`, `add_captions`

**Intent.** As an agent, I want to add text overlays and auto-generated captions so I can title and
caption a cut with the reference's defaults.

**Acceptance criteria.**
- **`add_texts`** (`entries[{start_frame, duration_frames, content}]`, **all-or-none** `track_index`,
  optional `transform`, `font_name` **default 'Helvetica-Bold'**, `font_size` **default 96**, `color`
  **default '#FFFFFF'**, `alignment` **default center`): returns new clip ids; **omitting `track_index`
  auto-creates a new top video track**. Mutating (named undo + agent-undo push).
- **`add_captions`** (optional `clip_ids`, `language` BCP-47, `font_name`, `font_size` **default 48**,
  `color`, `center_x` **default .5**, `center_y` **default .9**, `text_case ∈ {auto, upper, lower}`
  (ruling #18 — **no title-case**), `censor_profanity`): on-device transcribe + styled caption clips on a
  **new track**. **Async** tool. Its real Whisper + CaptionBuilder backing is **Epic 10 (M3)** — in M2 the
  schema is registered and it returns "transcription not available" until Epic 10 lands. When Epic 10 is
  present, the CaptionBuilder path must satisfy the **14 verbatim CaptionBuilder tests (SM-13)** owned by
  Epic 10 — this tool is the dispatch seam, not the builder.
- Mutating-tool undo semantics as E7-S6/S7. Colors via E7-S3 hex parser.

**Implementation context.**
- **Crate:** `palmier-tools`, module `tools/texts.rs`. `add_captions` calls into `palmier-transcribe` +
  `palmier-text` CaptionBuilder (Epic 10) behind a trait so M2 can stub it.
- **Reference files:** `ToolExecutor+Texts.swift`, `ToolExecutor+Captions.swift`.
- **Docs:** mcp-tools.md tools #18, #19; FOUNDATION §6.14; reconciliation #18; FR-37 (CaptionBuilder,
  Epic 10).

**Dependencies.** E7-S1, E7-S2, E7-S3 (colors), E7-S12; `add_captions` real path: **Epic 10 (M3)**.
**Parallel-safe?** Yes — owns `tools/texts.rs`.

---

### E7-S9 — GENERATE + INSPECT tools (stubbed backends): `generate_*`, `upscale_media`, `list_models`, `inspect_media`, `inspect_timeline`, `search_media`

**Intent.** As an agent, I want the generation, model-listing, inspection, and search tools present in the
surface so clients see the full 30-tool catalogue at M2 — with real backends wired by later epics.

**Acceptance criteria.**
- **`generate_video` / `generate_image` / `generate_audio` / `upscale_media`** (Generate, mutating,
  **async**, cost money, **NOT undoable** — do **not** push the agent undo stack): full schemas per
  FOUNDATION §6.14 / mcp-tools.md (note `generate_audio` **has no required field**). In M2 they return
  **"backend not available"** until **Epic 9 (M3, ruling #24, Spike S-2)** wires Convex. The dispatch +
  ShortId expansion of their reference-media-ref fields must be complete now (clients negotiate these).
- **`list_models`** (`type? ∈ {video, image, audio, upscale}`): returns `{ models, loaded }`; `loaded =
  false` ⇒ catalog not synced (not signed in). Stubbed `loaded: false` until Epic 9.
- **`inspect_media`** (Read, **async**: `media_ref`, opt `clip_id, max_frames ≤ 12 default 6,
  start_seconds, end_seconds, word_timestamps, overview`): image frames + transcript segments
  `[text, start, end]` **capped 400** (page via `next_start_seconds`); words **capped 10000**; Lottie
  frames over gray; **`overview=true` ignores `max_frames`**. Frame sampling backed by `palmier-media`
  (Epic 4/5) and transcript by Epic 10; in M2 returns frames if decode is available, empty transcript
  otherwise. **Test gate:** `max_frames` requested > 12 is **clamped to 12**; the 400-segment cap holds on
  an over-cap fixture.
- **`inspect_timeline`** (Read, **async**: opt `start_frame` default 0, `end_frame`, `max_frames ≤ 12
  default 6`): composited frames (downscaled) + sampled `frame_numbers`. Compositing depends on Epic 5
  (M1 preview); the **`max_frames ≤ 12` ceiling** is enforced here too (distinct from pagination caps).
- **`search_media`** (Read, **async**: `query`, opt `scope ∈ {visual, spoken, both} default both`,
  `media_ref`, `limit ≤ 50 default 10`): hits as source-second ranges + score (ordering only) + visual
  index `status`. Spoken backing = Epic 10 (M3); visual = **Epic 11 (M4, SigLIP2)**; in M2 returns
  empty with `status: not_indexed`.
- **Carry-forward (FR-26):** the **`max_frames ≤ 12` image-frame ceiling** and the **400-segment / 10000-
  word pagination caps** are **two different classes of cap** — do **not** encode `12` as a page size.

**Implementation context.**
- **Crate:** `palmier-tools`, modules `tools/generate.rs`, `tools/inspect.rs`, `tools/search.rs`. Each
  real backend hidden behind a trait (`GenerationBackend`, `TranscriptionBackend`, `SearchBackend`,
  `CompositorHandle`) so M2 stubs satisfy the trait and M3/M4 epics implement it.
- **Reference files:** `ToolExecutor+Generate.swift` (incl. `videoModelInfo`/`imageModelInfo` at lines
  398/421 for the resources), `ToolExecutor+InspectTimeline.swift`, `ToolExecutor+Search.swift`,
  `ToolExecutor+Import.swift`.
- **Docs:** mcp-tools.md tools #3, #5, #6, #8, #20–23; FOUNDATION §6.14; reconciliation #13, #24; FR-26.

**Dependencies.** E7-S1, E7-S2, E7-S4 (ref-id expansion); real backends: **Epic 9 (M3), Epic 10 (M3),
Epic 11 (M4)**. **Parallel-safe?** Yes — owns `tools/generate.rs` + `tools/inspect.rs` + `tools/search.rs`.

---

### E7-S10 — LIBRARY tools: `import_media`, `create_folder`, `move_to_folder`, `rename_media`, `rename_folder`, `delete_media`, `delete_folder`

**Intent.** As an agent, I want to import assets and organize the library (folders, renames, deletes) so I
can manage media — including the dual-shape direct-vs-`entries[]` tools.

**Acceptance criteria.**
- **`import_media`** (`source { exactly one of url | path | bytes; mime_type required for bytes }`,
  optional `name`, `folder_id`): returns a new `media_ref`. **`path` (recursive dir OK) and `bytes`
  (≤ ~11 MB) are synchronous**; **`url` (≤ 1 GB) is async**. path/bytes use `palmier-media` import
  (Epic 4, M1); url-fetch may stub until M3 plumbing.
- **`create_folder`** (`name` + `parent_folder_id?` **XOR** `entries[]`): returns folder id(s). Output
  shape differs (direct → single id vs `{ folders }`) — **validate the XOR** (not both); reject both.
- **`move_to_folder`** (`asset_ids[]` + `folder_id?` **XOR** `entries[]`): omitting `folder_id` → root.
- **`rename_media`** (`media_ref`, `name` **XOR** `entries[]`).
- **`rename_folder`** (`folder_id`, `name` **XOR** `entries[]`).
- **`delete_media`** (`asset_ids[]`): referencing clips removed **in the same undo**.
- **`delete_folder`** (`folder_ids[]`): **recursive**; referencing clips removed.
- All mutating (named undo + agent-undo push, except none are async besides `import_media` url). The four
  dual-shape tools' XOR validation is shared with E7-S3. ShortId expands `folder_id`/`parent_folder_id`/
  `asset_ids` inputs (E7-S4).

**Implementation context.**
- **Crate:** `palmier-tools`, module `tools/library.rs` + `tools/import.rs`. Calls `palmier-media`
  import + `palmier-model` library/folder mutators.
- **Reference files:** `ToolExecutor+Import.swift`, `ToolExecutor+Folders.swift`.
- **Docs:** mcp-tools.md tools #24–30; FOUNDATION §6.14; FR-26. Carry-forward: dual-shape XOR gotcha.

**Dependencies.** E7-S1, E7-S2, E7-S3 (XOR), E7-S4, E7-S12; **Epic 4 (`palmier-media`)** for path/bytes
import. **Parallel-safe?** Yes — owns `tools/library.rs` + `tools/import.rs`.

---

### E7-S11 — `axum` loopback transport, validators, JSON-RPC dispatch, `.well-known`

**Intent.** As a porter, I want the `rmcp` + `axum` HTTP server bound to loopback with the three request
validators and JSON-RPC dispatch into `palmier-tools` so MCP clients reach the tools securely and fast.

**Acceptance criteria.**
- TCP listener bound to **`IpAddr::V4(Ipv4Addr::LOCALHOST)` only** on default port **19789**
  (configurable via settings; SM-C3 — do **not** make it bindable to non-localhost).
- **`POST /mcp`** handles JSON-RPC **single-shot AND batched** requests, dispatching `tools/call` into the
  `palmier-tools` `execute` entry point and `tools/list` into the schema catalogue (E7-S1).
- **`GET /.well-known/oauth-protected-resource`** returns the literal body
  `{"resource":"http://127.0.0.1:19789"}` (no trailing path) for the Claude Desktop one-click handshake.
- **Three validators (request middleware), each rejecting with the proper MCP error (SM-C3):**
  (1) **Origin** — allow Origin header **missing** OR `Origin: null` OR `http://127.0.0.1:19789`; reject
  anything else; (2) **Content-type** — require `application/json`; (3) **Protocol version** — enforce the
  MCP spec version in the `mcp-protocol-version` header.
- **Initialize response identity:** `name: "palmier-pro"`, `version: "1.0.0"`,
  `instructions: <E7-S13 constant>`, capabilities `{ resources: { subscribe: false, listChanged: false },
  tools: { listChanged: false } }`.
- **SM-6 latency:** a `get_timeline` over loopback on a **200-clip** project is **< 100 ms p50 / 300 ms
  p99** (Criterion/integration benchmark).
- Server start/stop is gated by `io.palmier.pro.mcp.enabled` (ruling #6, absent ⇒ ON), wired by Epic 1's
  boot sequence.

**Implementation context.**
- **Crate:** `palmier-mcp`. Modules `server.rs`, `validators.rs`, `well_known.rs`. Uses `rmcp` (FOUNDATION
  §6.14 — no protocol re-implementation) + `axum`.
- **Reference files:** `MCP/MCPService.swift` (server identity, registration), `MCP/MCPHTTPServer.swift`
  (HTTP adapter, `OriginValidator.localhost`, `requiredLocalEndpoint host:127.0.0.1`).
- **Docs:** mcp-tools.md §"Resources"/§"Mapping to FOUNDATION crates"/§"Port risks" (`.well-known` body);
  FOUNDATION §6.14 (binding, endpoints, validators, identity); FR-25; SM-6, SM-C3.

**Dependencies.** E7-S1 (`tools/list`), E7-S2 (`execute`), E7-S13 (instructions constant). Epic 1 boot
for start/stop wiring. **Parallel-safe?** Yes — owns `palmier-mcp/server.rs` + validators (separate crate
from `palmier-tools`); can develop against the E7-S2 dispatch stub.

---

### E7-S12 — Agent undo stack (separate from user stack), undo-action-name matching, `undo` tool

**Intent.** As an agent, I want my edits on a **separate** undo stack that the `undo` tool reverses one at
a time, refusing once the user has interleaved a manual edit — so agent and human edits never tangle (UJ-4,
SM-4).

**Acceptance criteria.**
- A `agent_undo_stack: Vec<String>` of action names **distinct from the user undo stack** (FR-27, SM-4).
- **After** any non-`undo`, non-error tool run: **if** the timeline changed (`editor.timeline != before`)
  **and** the history layer reports a current `undo_action_name`, **push that name** onto the agent stack
  (the mutating tools wrap their work in `with_undo_group(action_name:)`, e.g. "Move 3 clips").
- **`undo` tool** (mutating, sync): pops one agent-stack entry and reverses **one** agent edit. **Refuses**
  (tool error) if the history layer's **current `undo_action_name` ≠ the expected popped name** (a user
  edit interleaved) **or** the stack is empty.
- **Test gates (SM-4):** (a) agent edit → `undo` reverses it, user stack untouched; (b) agent edit, then a
  **user** edit, then `undo` → **refused** with the interleaved-edit error; (c) empty stack → refused.
- **Carry-forward:** undo-group **action names must match the reference exactly** — replicate the
  reference's `undoActionName` strings so the mismatch check behaves identically. `UndoManager` →
  custom named-action undo in `palmier-history` (mcp-tools.md §macOS APIs to replace).

**Implementation context.**
- **Crate:** `palmier-tools` (`agent_undo_stack` on `ToolExecutor`) + `palmier-history` (named undo
  groups, `undo_action_name`). Module `tools/undo.rs`.
- **Reference files:** `ToolExecutor.swift` lines 33–36, 82–96 (`agentUndoStack`, push/pop, name-match
  refusal); `withUndoGroup(actionName:)` usages across the mutating tools.
- **Docs:** mcp-tools.md §"Agent undo stack"; FOUNDATION §6.14 "Agent undo stack"; reconciliation
  carry-forward "Agent undo refuses unless current undoActionName equals pushed name"; FR-27, SM-4.

**Dependencies.** E7-S2 (executor hook), Epic 3 (`palmier-history` named undo groups). The mutating tool
stories (E7-S6/S7/S8/S10) call its push. **Parallel-safe?** Partly — owns `tools/undo.rs` but adds the
post-run push hook in E7-S2's `execute`; coordinate that one insertion point.

---

### E7-S13 — Verbatim agent prompt constant, the two `palmier://models/*` resources, the `.mcpb` bundle + Help install UX

**Intent.** As a porter, I want the agent `instructions` string ported byte-for-byte, the two model
resources, and the `.mcpb` bundle + install instructions so existing reference clients connect with only
the URL changed (FR-28, SM-8).

**Acceptance criteria.**
- **Agent prompt:** the resolved `AgentInstructions.serverInstructions` text (agent-instructions.md
  §VERBATIM block) stored as **`include_str!("agent_instructions.txt")`** in **one shared module both
  `palmier-mcp` and `palmier-agent` import** (ruling #2 — single constant, no drift between injection
  sites). The Swift `\`-continuations are **already resolved** in the verbatim block — bake that resolved
  text; do **not** re-author the continuation style. Preserve Unicode (`×`, `–`, `•`, `…`, curly quotes) as
  UTF-8; do **not** ASCII-fold. A byte-diff test asserts the file equals the agent-instructions.md verbatim
  block. The literal product token **`palmier-pro`** stays as written.
- **Two resources** (`MCPService.swift:96-133`): `palmier://models/video` and `palmier://models/image`,
  each a JSON array from `videoModelInfo` / `imageModelInfo` (`ToolExecutor+Generate.swift:398,421`).
  `listChanged: false`, `subscribe: false`. Until Epic 9 supplies the catalog, return an empty/cached
  array (clients tolerate empty). **SM-C2:** these are **resources, not tools** — they do not count toward
  the 30 and must not be registered as tools.
- **`.mcpb` bundle:** re-emit `palmier-pro.mcpb` (`manifest_version: 0.4`, `name: palmier-pro`,
  `display_name: Palmier Pro Windows`, version, server block) into app resources, bundling a Node.js
  stdio→HTTP shim (`server/index.js`) running `mcp-remote` against `http://127.0.0.1:19789/mcp`.
- **Help → MCP Instructions tab:** copy-URL `http://127.0.0.1:19789/mcp`; Cursor deeplink install button
  (`cursor://…/mcp/install?name=palmier-pro&config=…`) + manual JSON; Claude Code
  `claude mcp add --transport http palmier-pro http://127.0.0.1:19789/mcp`; Codex
  `codex mcp add palmier-pro --url http://127.0.0.1:19789/mcp`; Claude Desktop install (extract `.mcpb`
  to `%APPDATA%\Claude\Extensions\` or platform equiv) + manual JSON.
- **SM-8 / §11.6:** Claude Desktop, Claude Code, Cursor, Codex connect with **only the server URL
  changed**; the reference test prompts ("what's on my timeline?", "cut the filler words", "add a title",
  "generate B-roll") run with no protocol errors. The **§11.6 MCP compatibility suite** passes.

**Implementation context.**
- **Crates:** a small shared `palmier-prompt` (or module) for the constant; `palmier-mcp` for the
  resources + `.mcpb` emission; Help UI lives in `src-ui/settings`/Help (Epic 1/12) — this story provides
  the strings/bundle, the UI wiring is shared with those epics.
- **Reference files:** `Tools/AgentInstructions.swift` (resolved text), `MCP/MCPService.swift:40,72-87,
  96-133`, `ToolExecutor+Generate.swift:398-440` (model info field sets), `palmier-pro.mcpb` source.
- **Docs:** agent-instructions.md (VERBATIM block, assembly, `include_str!` recommendation), FOUNDATION
  §6.14 (MCPB, Help tab, identity, resources), §7 (prompt); reconciliation #2, #3; FR-28, SM-8, SM-C2.

**Dependencies.** E7-S11 (server registers resources + instructions); shared with Epic 8 (consumes the
prompt constant) and Epic 9 (supplies real model catalog for the resources). **Parallel-safe?** Yes —
owns the prompt constant + `.mcpb` emission; coordinates the one shared-module location with Epic 8.

---

## Story dependency graph (within-epic)

```
E7-S1 ─┬─ E7-S2 ─┬─ E7-S3 ─┐
       │         ├─ E7-S4 ─┤
       │         ├─ E7-S5  │ (read)
       │         ├─ E7-S6  │ (clips)      ← Epic 3
       │         ├─ E7-S7  │ (props/ripple) ← Epic 3, Epic 2
       │         ├─ E7-S8  │ (texts/captions) ← Epic 10 (M3 backend)
       │         ├─ E7-S9  │ (generate/inspect/search) ← Epics 9/10/11
       │         ├─ E7-S10 │ (library)     ← Epic 4
       │         └─ E7-S12 │ (agent undo)  ← Epic 3 history
       └─ E7-S11 (axum transport) ─ E7-S13 (prompt/resources/.mcpb) ─ §11.6 suite + SM-8
```

E7-S1 and E7-S2 are the gate; E7-S3/S4/S12 are dispatcher infrastructure; E7-S5..S10 are the per-category
tool bodies (one reference extension file each, parallel-safe); E7-S11/S13 are the transport + client
surface (parallel with the tool stories once E7-S2 exists). The §11.6 compatibility suite + SM-6/SM-8
benchmarks are the M2 exit gate (shared with Epic 8 for the §11.3 agent-cut e2e and §11.4 dispatch bench).

## Out of scope for this epic (parity guardrails)

- **No 31st tool (SM-C2).** The surface is exactly **30 tools + 2 resources**. Resources do not count as
  tools. Do not "improve" the agent surface with extra tools.
- **No relaxed validators (SM-C3).** Loopback-only bind and the Origin/content-type/protocol validators
  are a security boundary — do not loosen them to ease client setup.
- **No paraphrased descriptions or prompt.** Tool descriptions and the system prompt are contract text;
  port byte-for-byte (ruling #2, R-5).
- **No real generation/transcription/visual-search backends here** — those land in Epics 9/10/11
  (M3/M4); this epic ships the surface with stubbed/trait-gated backends.
- The **in-app agent loop** (SSE, sessions, mentions, model gating) is **Epic 8**, not this epic; this
  epic only builds the shared `palmier-tools` + the MCP transport it shares with Epic 8.
