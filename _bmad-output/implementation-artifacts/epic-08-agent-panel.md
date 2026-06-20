---
kind: doc
domain: [build-orchestration]
type: epic
status: ready
links: [[PRD]] [[FOUNDATION]] [[phase0-reconciliation]]
---

# Epic 8 — In-App Agent Panel

## Epic goal

Port the macOS in-app AI chat to Windows/Linux: a streaming Anthropic-Messages agentic loop
(BYOK direct or Convex-proxied) that streams text, dispatches every tool call into the **same**
`palmier-tools` implementation as the MCP server (no duplication), feeds results back until the
model ends its turn, persists multi-tab chat sessions, and injects `@`-mention context hints —
all driven by the verbatim shared agent system prompt.

**PRD acceptance this epic must satisfy (PRD §4.8 FR-29..FR-32, §10 Epic 8):**

- **FR-29 Streaming tool loop.** One SSE parser (`message_start` usage logged, `text_delta`,
  `tool_use_complete`, `message_stop`) over two transports; on `tool_use` stop, dispatch every
  ToolUse to `palmier-tools` synchronously, append a ToolResult user message, resume; clean
  cancellation drops the in-flight assistant turn with no half-written ToolUse. Exactly **2
  ephemeral cache breakpoints** (system+tools, conversation tail); **orphan-tool_use repair**
  injects synthetic Cancelled results (carry-forward note).
- **FR-30 Client selection & key storage.** Anthropic key in OS keyring (account
  **`anthropic-api-key`**, ruling #5) → `AnthropicClient`; else Clerk-signed-in →
  Convex-proxied `PalmierClient`; else inline "sign in or add key". Image inline limits
  longest-edge **1568 px** / **3,500,000 bytes** / JPEG q-ladder **[0.85, 0.7, 0.55, 0.4]**.
- **FR-31 Sessions & mentions.** Sessions persisted to `<project>/chat/<uuid>.json` **on
  document save** (ruling #4), loaded sorted by `updated_at` desc; `@`-mentions emit JSON
  context-hint blocks (mediaAsset with base64-inlined images, timelineClip, timelineRange).
- **FR-32 Model availability.** BYOK = all three (Sonnet 4.6 / Opus 4.8 / Haiku 4.5); signed-in
  **free = Haiku 4.5**; signed-in **paid = catalog-driven**, default Sonnet 4.6, Convex catalog
  may enable Opus (ruling #20). Reference rule: `isPaid ? [.sonnet46] : [.haiku45]`.
- **§10 cross-cutting:** SSE loop dispatches into the **same** `palmier-tools` (no duplication);
  dispatch **< 50 ms p50 (SM-3)**; agent mutations are atomically undoable on the **separate
  agent undo stack**.

**Milestone (PRD §12):** **M2 — MCP Server + Agent** (Epics 7–8, the strategic centerpiece).
Exit gates: SM-3 (dispatch < 50 ms p50), SM-4/SM-6/SM-8 shared with Epic 7, and the **§11.3
agent-cut e2e** (transcription-gated cut deferred to M3; the agent loop + tool dispatch portion
is the M2 gate).

## Spike / gating note

Epic 8 is **NOT gated by Spike S-1** (wgpu→WebView — that gates Epic 5 only). The relevant spike
is **S-2 (Convex Rust HTTP + WebSocket client, before Epic 9, M3)**. Therefore:

- The **BYOK path (`AnthropicClient`, direct `https://api.anthropic.com/v1/messages`)** is fully
  buildable and e2e-testable in M2 with **zero spike dependency** — it is the M2-testable agent
  path. All loop/SSE/session/mention stories build and verify against the direct Anthropic API.
- The **`PalmierClient` (Convex-proxied) transport** depends on the Convex HTTP path proven by
  **S-2** and on `palmier-auth`'s Clerk JWT (Epic 9/M3 territory). Per PRD §10 Epic 9 + §12 M2
  note, the proxied path is **wired in M2 behind the auth/Convex boundary but exercised live at
  M3**. Story **E8-S6** isolates the `PalmierClient` transport so the rest of the epic does not
  block on S-2; it is the one story whose live verification slips to M3.
- **Dependency on Epic 7 is hard:** Epic 8 consumes the 30-tool catalogue, the verbatim shared
  agent prompt constant, the `palmier-tools::execute(name, args)` dispatcher, and the agent undo
  stack — all owned/landed by Epic 7. Do **not** re-implement tools or the prompt here.

---

## Stories

### E8-S1 — `palmier-agent` core value types + Codable wire models

As the agent crate, I want the message/content/session data model and its JSON (de)serialization,
so that every other story has stable types to build the loop, sessions, and mentions on.

**Acceptance criteria:**
- Define `AnthropicModel { sonnet46="claude-sonnet-4-6", opus48="claude-opus-4-8",
  haiku45="claude-haiku-4-5-20251001" }`; `AnthropicStopReason { end_turn, tool_use, max_tokens,
  stop_sequence, pause_turn, refusal, other }`; `AnthropicStreamEvent { TextDelta(String),
  ToolUseComplete{id,name,input_json:String}, MessageStop{stop_reason} }` — **only these three
  events reach the loop** (`agent-panel.md` lines 33-38).
- `AgentMessage { id:Uuid, role:User|Assistant, blocks:Vec<AgentContentBlock>, mentions,
  context_hint }`; `AgentContentBlock` enum `Text(String)` | `ToolUse{id,name,input_json:String}`
  | `ToolResult{tool_use_id, content:Vec<ToolResultBlock>, is_error}` with a **`kind`
  discriminator** (`text`/`toolUse`/`toolResult`); `ToolResultBlock` = `Text(String)` |
  `Image{base64, media_type}` (`agent-panel.md` lines 39-43).
- **`input_json` is stored as a raw JSON string and forwarded verbatim** — empty → `"{}"`; the
  serializer MUST NOT round-trip/normalize it (would change bytes, break cache determinism)
  (`agent-panel.md` lines 41-42, 201-203; reconciliation carry-forward "store and forward
  verbatim").
- `ChatSession { id:Uuid, title:String (default "New chat"), updated_at:DateTime, messages,
  is_open:bool (decode-if-present default true) }` (`agent-panel.md` line 149).
- **Date encoding for chat = iso8601 + pretty-print + sorted-keys** (distinct from project/media
  which use Apple reference-epoch doubles — reconciliation carry-forward "Project I/O Date
  encoding"; a single shared Date codec corrupts round-trips). Unit test: encode→decode a session
  is byte-stable and key order is deterministic.
- Unit tests: round-trip each `AgentContentBlock` variant; assert `kind` discriminator values;
  assert `input_json` passes through unmodified including whitespace.

**Implementation context:**
- Crate: **`palmier-agent`** (FOUNDATION §6.13; `agent-panel.md` "Mapping to FOUNDATION crates").
- Reference: `Sources/PalmierPro/Agent/Clients/AgentClientTypes.swift` (value types),
  `Sources/PalmierPro/Agent/ChatSessionStore.swift` (`ChatSession`). Doc: `agent-panel.md`
  §"Core value types", §"Sessions & persistence".
- Use `serde` with a canonical serializer (BTreeMap-backed object / `sortedKeys` equivalent) for
  any block that participates in the request body.

**Dependencies:** none.
**Parallel-safe?** Yes — net-new files in `palmier-agent`.

---

### E8-S2 — Request body builder + shared SSE parser

As the agent crate, I want one `AnthropicRequestBody::build` and one `AnthropicSSE::parse`,
so that both transports produce identical wire bytes and consume identical event streams.

**Acceptance criteria:**
- `build(system, tools, messages)` — **identical for both clients**: `max_tokens = 8192`,
  `stream = true`; `system` = single block `[{type:text, text, cache_control:{type:ephemeral}}]`;
  `tools` = `[{name, description, input_schema}]` with **`cache_control:{type:ephemeral}` on ONLY
  the LAST tool**; `messages` with **`cache_control:{type:ephemeral}` on ONLY the last content
  block of the last message** → **exactly 2 cache breakpoints total** (`agent-panel.md` lines
  59-66; reconciliation carry-forward "exactly 2 ephemeral cache breakpoints").
- JSON serialized with **sorted keys** (deterministic ordering for cache hits) (`agent-panel.md`
  line 67, lines 199-200). Unit test: byte-compare built body for a fixed (system, 30-tool,
  2-message) fixture against a committed golden; assert cache_control appears exactly twice and
  on the correct nodes.
- `parse(byte_stream)` — line-oriented over `bytes.lines`, keeps
  `pending_tools: Map<block_index → (id, name, json_accumulator)>`; per `data:`-prefixed line,
  JSON-decode and switch on `type`:
  - `message_start` → read `message.usage`, log token counts (`AgentUsageLog`, DEBUG-only).
  - `content_block_start` w/ `content_block.type=="tool_use"` → record
    `pending_tools[index]=(id,name,"")`.
  - `content_block_delta`: `text_delta` non-empty `text` → yield `TextDelta`;
    `input_json_delta` → append `partial_json` to `pending_tools[index].json`.
  - `content_block_stop` → pop `pending_tools[index]`; json **defaults to `"{}"` if empty**;
    yield `ToolUseComplete`.
  - `message_delta` → read `delta.stop_reason` → yield `MessageStop`.
  - `error` → finish stream with `StreamError(message)`.
  - `ping` / text `content_block_start` / `message_stop` ignored; **check cancellation each line**
    (`agent-panel.md` lines 82-94).
- Unit test: drive `parse` over a recorded SSE fixture (text deltas + one streamed tool_use with
  chunked `input_json_delta`) and assert the exact ordered event sequence, including empty-json
  → `"{}"` defaulting.

**Implementation context:**
- Crate: **`palmier-agent`** (`Clients/AgentClientTypes.swift`: `AnthropicRequestBody.build`,
  `AnthropicSSE.parse`, `AgentUsageLog`). Doc: `agent-panel.md` §"Request body", §"SSE parser".
- Use a canonical serializer (BTreeMap) so `.sortedKeys` parity holds; do not let serde reorder
  the `input_json` passthrough.

**Dependencies:** E8-S1.
**Parallel-safe?** Yes — distinct files from S1's models (same crate; coordinate the shared
`AgentClientTypes` module boundary, but no overlapping symbols).

---

### E8-S3 — `AgentClient` trait + `AnthropicClient` (BYOK transport) + keyring

As a BYOK user, I want my Anthropic key stored in the OS keyring and used to stream directly from
api.anthropic.com, so that I can run the agent without signing in.

**Acceptance criteria:**
- `AgentClient` trait: `async fn stream(system, tools, messages) -> Stream<AnthropicStreamEvent>`
  (futures `Stream` / `async_stream`; on termination, inner task is cancelled — `agent-panel.md`
  line 80).
- `AnthropicClient`: `POST https://api.anthropic.com/v1/messages`; headers `x-api-key:<key>`,
  `anthropic-version: 2023-06-01`, `content-type: application/json`,
  `accept: text/event-stream`; body from `AnthropicRequestBody::build`; stream via `reqwest`
  byte stream (+ `eventsource-stream`/manual line split — preserve `data:` semantics, no `[DONE]`
  sentinel). On HTTP status **≥ 400**, drain body and throw `AnthropicClientError::HttpError{
  status, body }` (`agent-panel.md` lines 70-76).
- `AnthropicKeychain` load/save/delete via the **`keyring`** crate (Windows Credential Manager /
  Linux Secret Service), account name **`anthropic-api-key`** (**ruling #5** — `palmier-pro-…`
  is wrong and silently loses keys). In DEBUG, `load()` honors env `ANTHROPIC_API_KEY`
  (`agent-panel.md` lines 168-169, line 57).
- Keychain read happens **off the main/UI path**; a key-changed event
  (`.anthropicAPIKeyChanged` equivalent — `tokio::sync::watch` or event bus) re-triggers a key
  reload (`agent-panel.md` line 56, line 170).
- Unit test (mock HTTP): a 401/429 body → typed `HttpError`; integration test (gated, needs real
  key via env): a 1-line "say hi" prompt streams ≥ 1 `TextDelta` then `MessageStop(end_turn)`.

**Implementation context:**
- Crate: **`palmier-agent`** (`Clients/AnthropicClient.swift`, `AnthropicKeychain`). Doc:
  `agent-panel.md` §"HTTP & SSE", §"macOS/Apple APIs to replace" (keyring row). FOUNDATION §656
  (keyring) amended by ruling #5.

**Dependencies:** E8-S1, E8-S2.
**Parallel-safe?** Yes (own files); shares the `AgentClient` trait definition with E8-S6 — land
the trait here.

---

### E8-S4 — Agentic run loop + tool dispatch into `palmier-tools` + orphan repair

As the agent, I want to run the streaming tool loop until end-of-turn, dispatching every ToolUse
into the shared `palmier-tools`, so that the model can read and mutate the project.

**Acceptance criteria:**
- `send(text, mentions)`: trim; compute `referenced_mentions` (mentions whose `@displayName`
  literally appears in text — `text.contains("@\(displayName)")`); build a `context_hint`
  snapshot; **`resolve_orphan_tool_uses()`**; append user `AgentMessage`; `kick_off_stream()`
  (`agent-panel.md` lines 98-99).
- `send()` guards on **`can_stream`** = `has_api_key || (is_signed_in && has_credits)`; if false
  → `streamError = upstream("Sign in to a paid plan or add an Anthropic API key to start.")`
  (`agent-panel.md` lines 51-52).
- `run_loop` `while !cancelled`: `resolve_orphan_tool_uses()`, build `api_messages()`, append an
  empty assistant `AgentMessage` (remember its id); stream events: `text_delta` → append/extend
  the **last `.text` block in place**; `tool_use_complete` → append a `.toolUse` block;
  `message_stop` → record reason. After stream: if `stop_reason == tool_use` →
  `run_pending_tool_uses(assistant_id)` then **continue**; else **break** (`agent-panel.md`
  lines 102-108).
- `run_pending_tool_uses`: collect `.toolUse` blocks of the assistant msg; **skip ids already
  resolved** in the immediately-following user msg; for each, if cancelled append error
  toolResult `"Cancelled"`, else call **`palmier_tools::execute(name, parse_json_object(input))`**
  → `.toolResult(content, is_error)`; append **one** user `AgentMessage` of all result blocks. If
  no executor present, append user text `"Tool executor unavailable."` (`agent-panel.md`
  lines 111-114).
- **`resolve_orphan_tool_uses(reason="Cancelled")`** — for every assistant msg with an unresolved
  toolUse id (no matching toolResult in the next user msg), inject synthetic
  `.toolResult(content:[.text("Cancelled")], is_error:true)` — **prepended** into the next user
  msg if it already has results, else **inserted** as a new user msg. **Load-bearing: Anthropic
  rejects any `tool_use` without a matching `tool_result`** (`agent-panel.md` lines 115-120,
  193-197; reconciliation carry-forward). Runs **before every send AND every loop iteration**.
- Cancellation: on `CancellationError` → `drop_empty_assistant_turn` (remove assistant msg if its
  blocks are empty), break. On client/other error → drop empty turn, set `streamError`, break
  (`agent-panel.md` lines 109-110, 211).
- **`input_json` is re-parsed in exactly two places** (`content_block_json` for the wire,
  `parse_json_object` for execution) — never normalized (`agent-panel.md` lines 201-203).
- Tool dispatch is the **same single `palmier-tools` implementation** as the MCP server (no
  duplication — PRD §10, FOUNDATION §4); agent mutations push to the **separate agent undo stack**
  with undo-group names matching the reference exactly (reconciliation carry-forward "Agent
  undo"). Acceptance: **dispatch < 50 ms p50 (SM-3)** measured on a `get_timeline` over a 200-clip
  fixture.
- Unit tests: (a) orphan repair injects exactly one synthetic Cancelled result for one dangling
  tool_use, prepend-vs-insert branch covered; (b) a two-round loop (tool_use → results →
  end_turn) terminates with the expected message sequence; (c) mid-tool cancel yields a
  `"Cancelled"` `is_error` toolResult, not an aborted message.

**Implementation context:**
- Crate: **`palmier-agent`** (`AgentService.swift` `send`/`kickOffStream`/`runLoop`/
  `runPendingToolUses`/`resolveOrphanToolUses`/`dropEmptyAssistantTurn`). Dispatch boundary:
  **`palmier-tools::execute(name, args)`** (FOUNDATION §6.13 / line 666). Doc: `agent-panel.md`
  §"Agentic loop". State = Rust struct behind `Mutex`/`tokio::Mutex`, deltas emitted to the
  webview as Tauri events (`agent-panel.md` lines 177-178, 204).

**Dependencies:** E8-S2 (parser), E8-S3 (a client to stream from), E8-S5 (`api_messages()`
projection + mentions), **Epic 7** (`palmier-tools::execute` dispatcher, agent undo stack, 30-tool
catalogue, shared prompt constant).
**Parallel-safe?** No — central orchestrator; serialize against S5 (it calls `api_messages`).

---

### E8-S5 — `api_messages()` wire projection + mentions + context hints + image inlining

As the agent, I want user turns projected to wire format with `@`-mention context hints and
inlined images, so that the model receives the referenced assets and timeline context.

**Acceptance criteria:**
- `api_messages()`: map each stored `AgentMessage`'s blocks via `content_block_json` (**drops
  empty text**); for **user** messages with mentions, compute inlined image blocks + context-hint
  text, then **prepend at index 0 in order: (1) the hint text block, (2) the image blocks**; skip
  messages whose content ends up empty. `.toolUse` re-parses `input_json` into an object;
  `.toolResult` emits `{type:tool_result, tool_use_id, content:[text|image], is_error}`
  (`agent-panel.md` lines 122-127; `agent-instructions.md` lines 46-52).
- Three mention kinds: **`mediaAsset`** (mediaRef + ClipType), **`timelineClip`** (clipId + full
  clip summary), **`timelineRange`** (`AgentTimelineRangeMention`, **frame range half-open:
  startFrame inclusive / endFrame exclusive**, `rangeSemantics="startInclusiveEndExclusive"`)
  (`agent-panel.md` lines 129-132).
- `attach_mention*` inserts a single-word `@displayName ` token into `draft`
  (`make_display_name` collapses spaces and `-` to a single `-`), de-dupes, disambiguates
  collisions by appending **`#<first6 of id>`**; `prune_detached_mentions` drops mentions whose
  token was deleted from `draft` (`agent-panel.md` lines 133-135).
- Hint text format (exact): `"Referenced assets and timeline context in this message:
  <JSON array>.<notes>"`; each entry `{mention:"@name", kind, ...}`; notes appended for
  inlined/failed images, clips present, ranges present — port the exact `mentionNotes` strings
  (`agent-panel.md` lines 136-138; `agent-instructions.md` lines 50-52).
- **Image inlining** (`inline_image_blocks`): for `image` mentions, `ImageEncoder::encode(url)`
  → base64 image block; entry marked `inlined:true`. Failures recorded as `inlineError` reason
  (exact strings: `"asset not in media library"`, `"could not read or decode image file"`)
  (`agent-panel.md` lines 139-141).
- `ImageEncoder`: longest edge ≤ **1568 px**, file ≤ **3,500,000 bytes**; passthrough if already
  small enough & sniffed mime is image; else downscale (ImageIO-equivalent via the **`image`
  crate**) and JPEG-encode trying qualities **[0.85, 0.7, 0.55, 0.4]** until under maxBytes;
  memoized by **path+size+mtime** (`agent-panel.md` lines 143-146, 174). NOTE: the mtime key may
  false-hit on coarse Windows FS — acceptable; port as a bounded LRU clearing whole cache at
  `maxCacheEntries` (open question, line 230).
- Context hint is **client-side only** — never pushed into MCP `instructions`
  (`agent-instructions.md` lines 189-190).
- Unit tests: hint JSON for one of each mention kind matches the exact format incl. half-open
  range semantics; image inlining produces a base64 block ≤ limits for an oversized fixture and
  records the exact failure string for a missing asset.

**Implementation context:**
- Crate: **`palmier-agent`** (`AgentMentionContext.swift`, `Utilities/ImageEncoder.swift`,
  `AgentService.apiMessages`). Doc: `agent-panel.md` §"Mentions & context hints", §"Image
  encoding", §"apiMessages()"; `agent-instructions.md` §B.

**Dependencies:** E8-S1.
**Parallel-safe?** Yes (own files); E8-S4 consumes `api_messages()`, so land before/with S4.

---

### E8-S6 — `PalmierClient` (Convex-proxied transport) + model availability

As a signed-in user without a BYOK key, I want the agent to stream through the Convex proxy with
my plan's models, so that I can use the agent on my subscription.

**Acceptance criteria:**
- `PalmierClient`: `POST {BackendConfig.convex_http_url}/v1/agent/stream`,
  `Authorization: Bearer <clerk_jwt>` (Clerk session must be `.active`, then `get_token()` via
  **`palmier-auth`**); same `content-type`/`accept`; **same `AnthropicRequestBody::build`** and
  **same `AnthropicSSE::parse`** — do **not** fork the parser/body (`agent-panel.md` lines 73-74,
  207-209).
- `PalmierClientError::from(status, body)`: parse `{error:{code,message}}`; `code=="unauthenticated"`
  or status 401 → `Unauthenticated`; `code=="insufficient_credits"` or status 402 →
  `InsufficientCredits`; else `Upstream` (`agent-panel.md` lines 77-79).
- **Backend selection** (`select_client`): keyring has non-empty key → `AnthropicClient`; else
  `AccountService.is_signed_in` → `PalmierClient`; else nil →
  `streamError = upstream("No backend available.")` (`agent-panel.md` lines 47-50).
- **Model availability:** BYOK → all three (`sonnet46`, `opus48`, `haiku45`); signed-in
  **`is_paid ? catalog_default(sonnet46) : [haiku45]`** — **ruling #20**: paid is
  **catalog-driven** (default Sonnet 4.6; Convex catalog may enable Opus), NOT the reference's
  hard-coded `[sonnet46]` (`agent-panel.md` lines 53-54; PRD FR-32; reconciliation #20).
  `effective_model` = persisted `model` if in `available_models`, else first available, else
  `sonnet46`; persisted to config key **`"agentModel"`** (`UserDefaults` → app config store)
  (`agent-panel.md` lines 54, 175).
- Unit tests: error-envelope mapping (401/402/code strings) → correct typed errors;
  `available_models` matrix for BYOK / signed-paid / signed-free; `effective_model` fallback
  chain.
- **Live verification gated on Spike S-2** (Convex HTTP path) + `palmier-auth` Clerk JWT — see
  spike note. The transport/types/selection logic build and unit-test in M2; the live
  end-to-end round trip is exercised at **M3**.

**Implementation context:**
- Crate: **`palmier-agent`** (`Clients/PalmierClient.swift`, `PalmierClientError`,
  `AgentService.selectClient`/`availableModels`/`effectiveModel`); auth token from
  **`palmier-auth`** (`agent-panel.md` line 179). Doc: `agent-panel.md` §"Backend selection",
  §"HTTP & SSE" (PalmierClient).

**Dependencies:** E8-S2, E8-S3 (trait), **palmier-auth** (Clerk JWT), **Spike S-2** (for live
verification only). Selection logic also feeds E8-S4's `can_stream`.
**Parallel-safe?** Yes — own client file; the `select_client`/model bits touch `AgentService`,
coordinate with S4.

---

### E8-S7 — Session store: load, tabs, sync, and save-on-document-save persistence

As a user, I want my chats kept as tabs and saved with the project, so that I can reopen prior
conversations.

**Acceptance criteria:**
- `load_sessions(project_url)`: read `<project>/chat/*.json`, **drop empty-message sessions**, set
  `is_open=false`, sort `updated_at` **desc**, then **prepend a fresh empty open session as
  current** (`agent-panel.md` lines 151-152). Session dir name is **`chat/`** (ruling #3 / FR-31;
  FOUNDATION's `chatsessions/` is void).
- `new_chat`/`select_session`/`close_tab`/`delete_session` manage tabs; `select_session` first
  cancels the in-flight task and `sync_messages_into_current_session()`; closing the last open
  tab → `new_chat()` (`agent-panel.md` lines 153-155).
- `sync_messages_into_current_session`: copy `messages` into the session, bump `updated_at`, and
  if title is still `"New chat"` derive it from the **first 40 chars of the first user text**
  (`agent-panel.md` line 156).
- **Write path = on document save (ruling #4):** `capture_save_snapshot` encodes each
  **non-empty** session to `<uuid>.json` (**iso8601 dates + pretty + sorted-keys**);
  `chat_dir_wrapper` builds the `chat/` directory; `on_sessions_changed` marks the document
  **dirty** so a save is scheduled — **sessions are NOT written eagerly on every change**
  (`agent-panel.md` lines 157-162, 206; ruling #4). Empty-message sessions filtered on **both
  load and save**.
- Export copies the `chat/` dir (`PalmierProjectExporter` equivalent) (`agent-panel.md` line 162).
- Unit tests: load drops empty sessions, sorts desc, prepends a current; title auto-derive fires
  once and stops; round-trip a session dir (write → load) is content-stable; save emits no file
  for an empty-message session.

**Implementation context:**
- Crate: **`palmier-agent`** (`ChatSessionStore.swift`) + the persistence writer that lives in
  the project layer (`Project/VideoProject.swift`: `captureSaveSnapshot`, `chatDirWrapper`,
  `makeWindowControllers` wiring `loadSessions` + `onSessionsChanged`) → Tauri/Rust file IO into
  `palmier-project`/`palmier-tauri` save lifecycle. Doc: `agent-panel.md` §"Sessions &
  persistence".

**Dependencies:** E8-S1; integrates with the project save lifecycle (Epic 2 `palmier-project`).
**Parallel-safe?** Mostly — the in-`palmier-agent` store is independent; the save-hook wiring
touches the project save path (coordinate with the owning project-save story).

---

### E8-S8 — Agent panel UI (tabs, starter prompts, send gating, streaming render)

As a user, I want the right-side chat panel with tabs and live streaming, so that I can talk to
the agent and watch the timeline change.

**Acceptance criteria:**
- Reproduce the SwiftUI `AgentPanelView` in the webview (`src-ui/agent-panel`): **floating tab
  bar**, **7 starter prompts**, message list with streaming text appended in place, and a
  **jump-to-bottom** affordance (`agent-panel.md` lines 30, 232).
- **Send gating:** send enabled iff `!is_streaming && can_stream && non-empty draft`; cancel
  available while streaming (`agent-panel.md` line 232; `can_stream` from E8-S4/S6).
- Model picker bound to `available_models`/`effective_model`; persists selection to `"agentModel"`
  (E8-S6).
- Streaming deltas arrive as **Tauri events** from `palmier-agent` and render incrementally
  (text appends to the last assistant block; tool-use blocks render as tool activity). The
  frontend **never touches HTTP/keyring/filesystem directly** — all via Tauri commands/events
  (PRD cross-cutting "Strict layering", FOUNDATION §4).
- Mention entry inserts `@displayName ` tokens into the draft (binds E8-S5); detached tokens
  prune their mentions.
- e2e (`tauri-driver` + Playwright): part of the **§11.3 agent-cut e2e** M2 gate — type a prompt,
  observe streamed text + at least one tool call dispatched and a resulting timeline change
  (transcription-gated cut step deferred to M3).

**Implementation context:**
- Frontend: **`src-ui/agent-panel`** (`Panel/AgentPanelView.swift`). Tauri command/event surface
  in `palmier-tauri`/`palmier-agent`. Doc: `agent-panel.md` §"Open questions" (UI list).

**Dependencies:** E8-S4 (loop + events), E8-S6 (model availability + `can_stream`), E8-S7
(sessions/tabs).
**Parallel-safe?** No for final integration (depends on S4/S6/S7), but the static panel
scaffolding can start in parallel against mocked events.

---

### E8-S9 — `palmier-agent` integration + M2 agent-cut e2e gate

As the milestone owner, I want the full BYOK loop wired through Tauri with the §11.3 e2e passing,
so that M2's agent acceptance gate is satisfied.

**Acceptance criteria:**
- End-to-end through the **BYOK `AnthropicClient`** path (no spike dependency): user prompt →
  stream → tool dispatch into the **shared `palmier-tools`** → ToolResult → resume → end_turn,
  with the timeline mutated atomically on the **agent undo stack**.
- **§11.3 agent-cut e2e** (the agent loop + tool dispatch portion) passes as an M2 exit gate
  (PRD §12 M2; transcription-gated cut step deferred to M3).
- **Dispatch < 50 ms p50 (SM-3)** asserted via the §11.4 tool-dispatch benchmark on the shared
  dispatcher (co-owned with Epic 7).
- Orphan-tool repair, 2-cache-breakpoint body, and verbatim prompt injection verified end-to-end
  against a recorded/real Anthropic exchange (the shared prompt constant is owned by Epic 7 /
  `palmier-mcp`/shared module — assert this crate injects the **same** constant as `system`, no
  drift; `agent-instructions.md` lines 156-163).
- `palmier-agent` per-crate unit coverage (FOUNDATION §11.1) green.

**Implementation context:**
- Crates: **`palmier-agent`** + `palmier-tauri` (command/event wiring) + e2e harness
  (`tauri-driver`+Playwright). Doc: PRD §12 M2 note; §11.3/§11.4 traceability table.

**Dependencies:** E8-S3, E8-S4, E8-S5, E8-S7, E8-S8, **Epic 7** (dispatcher, prompt constant,
agent undo). (E8-S6 not required for the BYOK e2e.)
**Parallel-safe?** No — integration/gate story; runs last.

---

## Story dependency summary

- **Foundation:** E8-S1 (types) → E8-S2 (body/parser) → E8-S3 (BYOK client + keyring).
- **Loop:** E8-S5 (projection/mentions) feeds E8-S4 (run loop); E8-S4 needs **Epic 7**
  (`palmier-tools::execute`, agent undo, prompt constant).
- **Proxied path:** E8-S6 (PalmierClient) needs `palmier-auth` + **Spike S-2** for live use only.
- **Sessions/UI:** E8-S7 (store) and E8-S8 (panel) on top of the loop.
- **Gate:** E8-S9 closes the M2 agent-cut e2e via the BYOK path (spike-independent).

## Cross-epic dependencies

- **Epic 7 (hard):** 30-tool catalogue, verbatim shared agent prompt constant,
  `palmier-tools::execute(name, args)` dispatcher, separate agent undo stack.
- **palmier-auth:** Clerk JWT for `PalmierClient` (E8-S6).
- **Epic 2 (`palmier-project`):** document save lifecycle that triggers session persistence
  (E8-S7).
- **Spike S-2:** live verification of E8-S6's proxied transport (M3); does NOT block the rest of
  the epic.
