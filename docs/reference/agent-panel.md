---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# agent-panel — reference port notes

## Purpose
The in-app AI chat: an agentic Anthropic-Messages loop that streams text, calls editor
tools, and feeds results back until the model ends its turn. This doc covers backend
selection, SSE parsing, the tool-execution loop, the message/session model, mentions and
context-hints, and on-disk session persistence. **Excludes** the tool catalogue (see
mcp-tools) and system prompt text (see agent-instructions).

## Key types & files (cite paths under Sources/PalmierPro/Agent/...)
- `AgentService.swift` — `@MainActor @Observable` orchestrator. Holds `sessions`, `messages`,
  `draft`, `mentions`, `isStreaming`, `streamError`, `model`. Owns the streaming loop.
- `Clients/AgentClientTypes.swift` — shared value types + the `AgentClient` protocol +
  `AnthropicSSE.parse` (single SSE parser, used by both clients) + `AnthropicRequestBody.build`
  (single body builder) + `AgentUsageLog`.
- `Clients/AnthropicClient.swift` — BYOK client + `AnthropicKeychain` (key load/save/delete).
- `Clients/PalmierClient.swift` — Convex-proxied client + `PalmierClientError`.
- `ChatSessionStore.swift` — `ChatSession` model + JSON load/encode (dir name = **`"chat"`**).
- `AgentMentionContext.swift` — `AgentMention`, `AgentTimelineRangeMention`, context-hint JSON.
- Persistence writer lives outside Agent/: `Project/VideoProject.swift` (`captureSaveSnapshot`,
  `chatDirWrapper`, `makeWindowControllers` wires `loadSessions` + `onSessionsChanged`).
- `Utilities/ImageEncoder.swift` — image inlining (downscale + JPEG, memoized).
- Panel UI: `Panel/AgentPanelView.swift` (7 starter prompts, tab bar, send/cancel gating).

### Core value types
- `AnthropicModel`: `sonnet46="claude-sonnet-4-6"`, `opus48="claude-opus-4-8"`,
  `haiku45="claude-haiku-4-5-20251001"`.
- `AnthropicStopReason`: end_turn / tool_use / max_tokens / stop_sequence / pause_turn /
  refusal / other.
- `AnthropicStreamEvent`: `.textDelta(String)`, `.toolUseComplete(id,name,inputJSON)`,
  `.messageStop(stopReason)`. (Only these three reach the loop; usage is logged internally.)
- `AgentMessage { id:UUID, role:user|assistant, blocks:[AgentContentBlock], mentions, contextHint }`.
- `AgentContentBlock`: `.text(String)` | `.toolUse(id,name,inputJSON:String)` |
  `.toolResult(toolUseId, content:[ToolResult.Block], isError)`. **inputJSON is stored as a raw
  JSON string**, parsed lazily. Codable with a `kind` discriminator (text/toolUse/toolResult).
- `ToolResult.Block`: `.text(String)` | `.image(base64, mediaType)`.

## Core behaviors & algorithms (concrete — downstream story/dev agents implement from this)

### Backend selection (`AgentService.selectClient`)
1. If keychain has a non-empty Anthropic key → `AnthropicClient(apiKey, effectiveModel)`.
2. Else if `AccountService.shared.isSignedIn` → `PalmierClient(effectiveModel)`.
3. Else → nil → `streamError = .upstream("No backend available.")`.
- `canStream` = `hasApiKey || (isSignedIn && hasCredits)`. `send()` guards on `canStream`; if
  false sets `.upstream("Sign in to a paid plan or add an Anthropic API key to start.")`.
- `availableModels`: BYOK → all three. Signed-in: `isPaid ? [.sonnet46] : [.haiku45]`.
- `effectiveModel`: persisted `model` if in `availableModels`, else first available, else sonnet46.
  `model` persisted to `UserDefaults` key `"agentModel"`.
- Key reload: keychain read off-main (utility task); `NotificationCenter` `.anthropicAPIKeyChanged`
  re-triggers `reloadAPIKey`. In DEBUG, `AnthropicKeychain.load()` honors env `ANTHROPIC_API_KEY`.

### Request body (`AnthropicRequestBody.build`) — IDENTICAL for both clients
- `max_tokens = 8192`, `stream = true`.
- `system`: single block `[{type:text, text:system, cache_control:{type:ephemeral}}]`.
- `tools`: `[{name, description, input_schema}]`; **only the LAST tool** gets
  `cache_control:{type:ephemeral}` (cache boundary covers system + entire tool list).
- `messages`: `[{role, content:[...]}]`; the **last content block of the last message** gets
  `cache_control:{type:ephemeral}` (caches the conversation prefix). 2 cache breakpoints total
  per request (system+tools, and conversation tail) — note Anthropic's 4-breakpoint max is fine.
- JSON serialized with `.sortedKeys` (deterministic ordering, important for cache hits).

### HTTP & SSE
- **AnthropicClient:** `POST https://api.anthropic.com/v1/messages`. Headers: `x-api-key: <key>`,
  `anthropic-version: 2023-06-01`, `content-type: application/json`, `accept: text/event-stream`.
- **PalmierClient:** `POST {BackendConfig.convexHttpURL}/v1/agent/stream`,
  `Authorization: Bearer <clerk_jwt>` (`Clerk.shared.session` must be `.active`, then
  `session.getToken()`). Same content-type/accept. Same body builder.
- Both: `URLSession.shared.bytes(for:)`. If HTTP status >= 400, drain the body and throw
  (`AnthropicClientError.httpError(status,body)` / `PalmierClientError.from(status,body)`).
- `PalmierClientError.from`: parse `{error:{code,message}}` envelope; `code=="unauthenticated"`
  or status 401 → `.unauthenticated`; `code=="insufficient_credits"` or status 402 →
  `.insufficientCredits`; else `.upstream`.
- Stream shape: `AsyncThrowingStream`; on `continuation.onTermination` the inner `Task.cancel()`.

### SSE parser (`AnthropicSSE.parse`) — line-oriented over `bytes.lines`
Keep `pendingTools: [blockIndex -> (id,name,jsonAccumulator)]`. For each line: require prefix
`data:`, strip it, JSON-decode the object, switch on `type`:
- `message_start`: read `message.usage`, log token counts (`AgentUsageLog`, DEBUG print only).
- `content_block_start`: if `content_block.type=="tool_use"`, record `pendingTools[index]=(id,name,"")`.
- `content_block_delta`: `delta.type=="text_delta"` & non-empty `text` → yield `.textDelta`.
  `delta.type=="input_json_delta"` → append `partial_json` to `pendingTools[index].json`.
- `content_block_stop`: pop `pendingTools[index]`; json defaults to `"{}"` if empty; yield
  `.toolUseComplete(id,name,inputJSON)`.
- `message_delta`: read `delta.stop_reason` → yield `.messageStop(reason)`.
- `error`: read `error.message` → `continuation.finish(throwing: .streamError(msg))`.
- Other types (`ping`, `content_block_start` for text, `message_stop`) ignored. `Task.checkCancellation()`
  each line.

### Agentic loop (`AgentService.runLoop` / `kickOffStream`)
1. `send(text,mentions)`: trim; compute `referencedMentions` (mentions whose `@displayName`
   literally appears in text); build a `contextHint` snapshot; `resolveOrphanToolUses()`; append
   user `AgentMessage`; `kickOffStream()`.
2. `kickOffStream`: cancel prior task, `isStreaming=true`, spawn `Task` running `runLoop`;
   `defer` sets `isStreaming=false`, `syncMessagesIntoCurrentSession()`, `onSessionsChanged?()`.
3. `runLoop` `while !Task.isCancelled`:
   a. `resolveOrphanToolUses()`, build `apiMessages()`, append an empty assistant `AgentMessage`,
      remember its `id`.
   b. `client.stream(system, tools, messages)`; for each event: text_delta → append/extend the
      last `.text` block; toolUseComplete → append `.toolUse` block; messageStop → record reason.
   c. After stream ends: if `stopReason==.toolUse` → `runPendingToolUses(assistantID)` then
      `continue loop`. Else `break loop`.
   d. On `CancellationError` → `dropEmptyAssistantTurn` (remove assistant msg if blocks empty),
      break. On `PalmierClientError`/other → drop empty turn, set `streamError`, break.
4. `runPendingToolUses`: collect `.toolUse` blocks from the assistant msg; skip ids already
   resolved in the immediately-following user msg; for each: if cancelled append error
   toolResult "Cancelled"; else `executor.execute(name, parseJSONObject(input))` →
   `.toolResult(content, isError)`. Append one user `AgentMessage` of all result blocks.
   If `toolExecutor==nil`, append a user text "Tool executor unavailable."
5. `resolveOrphanToolUses(reason="Cancelled")`: for every assistant msg with unresolved
   toolUse ids (no matching toolResult in next user msg), inject synthetic
   `.toolResult(content:[.text("Cancelled")], isError:true)` blocks — prepended into the next
   user msg if it already has results, else inserted as a new user msg. **Required so the
   Anthropic API never sees a tool_use without a matching tool_result.**

### apiMessages() — wire-format projection
For each stored `AgentMessage`: map blocks via `contentBlockJSON` (drops empty text). For
**user** messages with mentions: compute inlined image blocks + context-hint text, then prepend
(in order) the hint text block, then the image blocks, at index 0. Skip messages whose content
ends up empty. `.toolUse` re-parses inputJSON into an object; `.toolResult` emits
`{type:tool_result, tool_use_id, content:[text|image], is_error}`.

### Mentions & context hints (`AgentMentionContext`)
- Three mention kinds: `mediaAsset` (mediaRef + ClipType), `timelineClip` (adds clipId +
  full clip summary), `timelineRange` (`AgentTimelineRangeMention`, frame range half-open:
  startFrame inclusive / endFrame exclusive, `rangeSemantics="startInclusiveEndExclusive"`).
- `attachMention*` insert a single-word `@displayName ` token into `draft` (collapsing spaces and
  `-` to one `-` via `makeDisplayName`), de-dupe, disambiguate collisions by appending
  `#<first6 of id>`. `pruneDetachedMentions` drops mentions whose token was deleted from `draft`.
- Hint text: `"Referenced assets and timeline context in this message: <JSON array>.<notes>"`.
  Each entry: `{mention:"@name", kind, ...}`. Notes appended when images are inlined / failed /
  clips present / ranges present (exact strings in `mentionNotes`).
- Image inlining (`inlineImageBlocks`): for `type==.image` mentions, `ImageEncoder.encode(url)` →
  base64 image block; entry marked `inlined:true`. Failures recorded as `inlineError` reason
  (e.g. "asset not in media library", "could not read or decode image file").

### Image encoding (`ImageEncoder`)
- Target: longest edge ≤ **1568 px**, file ≤ **3,500,000 bytes**. Passthrough if already small
  enough & sniffed mime is image; else downscale via ImageIO thumbnail and JPEG-encode trying
  qualities [0.85, 0.7, 0.55, 0.4] until under maxBytes. Memoized by path+size+mtime.

### Sessions & persistence
- `ChatSession { id:UUID, title:String="New chat", updatedAt:Date, messages:[AgentMessage],
  isOpen:Bool=true }`. `isOpen` decodes-if-present defaulting true.
- `loadSessions(projectURL)`: read `<project>/chat/*.json`, drop empty-message sessions, set
  `isOpen=false`, sort `updatedAt` desc, then prepend a fresh empty open session as current.
- `newChat`/`selectSession`/`closeTab`/`deleteSession` manage tabs; `selectSession` cancels the
  in-flight task and `syncMessagesIntoCurrentSession()` first. Closing the last open tab →
  `newChat()`.
- `syncMessagesIntoCurrentSession`: copy `messages` into the session, bump `updatedAt`, and if
  title is still "New chat" derive it from first 40 chars of first user text.
- **Write path:** NSDocument save. `captureSaveSnapshot` encodes each non-empty session to
  `<uuid>.json` (`JSONEncoder` prettyPrinted + sortedKeys + iso8601 dates); `chatDirWrapper`
  builds a `chat/` FileWrapper. `onSessionsChanged` → `updateChangeCount(.changeDone)` marks the
  doc dirty so a save is scheduled (sessions are NOT written eagerly on every change).
- Export copies the `chat/` dir (`PalmierProjectExporter`).

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- `URLSession.bytes(for:)` SSE stream → `reqwest` + `reqwest-eventsource` (or manual
  byte-stream line split). Preserve `data:` line semantics and `[DONE]`-free Anthropic protocol.
- `AsyncThrowingStream` → `tokio::sync::mpsc` / `async_stream` / `futures::Stream`.
- Keychain (`KeychainStore`, Security.framework) → `keyring` crate (Windows Credential Manager /
  Linux Secret Service). Account name in reference: **`"anthropic-api-key"`** (see discrepancy).
- `NotificationCenter` key-changed observer → `tokio::sync::watch` or an event bus.
- `JSONSerialization` (`[String:Any]`, `.sortedKeys`) → `serde_json::Value` with a
  BTreeMap-backed map or `serde_json::to_string` (note: serde sorts via `#[serde]`? — use a
  canonical serializer to keep key order deterministic for cache hits).
- `ImageIO`/`CGImageDestination` downscale+JPEG → `image` crate (resize + `jpeg` encoder).
- `UserDefaults("agentModel")` → app config store (e.g. `confy`/JSON in app data dir).
- `NSDocument` FileWrapper save → Tauri/Rust file IO writing `<project>/chat/<uuid>.json`.
- `@Observable @MainActor` state → Tauri state (Rust struct behind `Mutex`/`tokio::Mutex`) with
  events emitted to the webview for streaming deltas.
- ClerkKit `Clerk.shared.session.getToken()` → `palmier-auth` JWT cache (see auth reference).

## Mapping to FOUNDATION crates (palmier-agent)
- `palmier-agent` owns: `AgentClient` trait, `AnthropicClient`, `PalmierClient`, the shared SSE
  parser, request-body builder, `AgentService` loop, `AgentMessage`/`AgentContentBlock`/
  `ChatSession` models, mentions/context-hint logic, session JSON load/save.
- Tool dispatch (`executor.execute(name,args)`) delegates to `palmier-tools` (FOUNDATION §6.13/
  line 666 names `palmier_tools::execute(name, args)`); tool catalogue lives there + `palmier-mcp`.
- Auth token retrieval (Clerk JWT) → `palmier-auth`. API-key storage → `keyring` (FOUNDATION §656).
- Image inlining likely shared with `palmier-media`/a util crate; FOUNDATION doesn't name a crate
  — keep in `palmier-agent` unless reused.
- FOUNDATION lines 88, 142, 637–666, 797, 812–815 corroborate this subsystem; data structures at
  lines 640–648 match the reference 1:1.

## Port risks & gotchas
- **Orphan tool_use repair is load-bearing.** Anthropic rejects any `tool_use` block lacking a
  matching `tool_result` in the next user turn. `resolveOrphanToolUses` runs before EVERY send
  and EVERY loop iteration. Replicate exactly (synthetic "Cancelled" error results), including the
  "prepend into existing next-user-msg vs insert new msg" branch.
- **Cache-control placement is exact:** ephemeral on (a) the system block, (b) the last tool, (c)
  the last content block of the last message. Wrong placement → cache misses → cost/latency. Keep
  `.sortedKeys` canonical JSON or cache hashing breaks.
- **inputJSON stored as raw string**, re-parsed in two places (`contentBlockJSON` for the wire,
  `parseJSONObject` for execution). Empty → `"{}"`. Don't normalize/round-trip it (would change
  bytes and break determinism); store and forward verbatim.
- **Streaming UI state lives on the main actor.** The Rust port must marshal deltas back to the
  webview without data races; the loop appends/extends the last `.text` block in place.
- **Sessions are not written eagerly** — only on NSDocument save (triggered via dirty flag).
  Empty-message sessions are filtered out on both load and save. Title auto-derives once.
- **Convex stream identical to Anthropic** — implement one SSE parser, two transports. Do not
  fork the parser.
- **Cancellation drops the empty assistant turn** (no half-written tool_use committed). Mid-tool
  cancellation yields toolResult "Cancelled" (isError) rather than aborting the message.
- **DISCREPANCY — session dir name:** reference uses **`chat/`** (`ChatSessionStore.dirName="chat"`).
  FOUNDATION line 650 says `<project>/chatsessions/<uuid>.json`. Use `chat/` for behavior parity
  unless intentionally renaming; flag in port.
- **DISCREPANCY — keychain account name:** reference uses **`"anthropic-api-key"`**. FOUNDATION
  line 656 specifies `palmier-pro-anthropic-api-key`. Pick one and document; affects key
  migration from any earlier build.
- **DISCREPANCY — write trigger:** FOUNDATION line 650 says "written on tab close + new-session
  creation". Reference actually writes on NSDocument save; `onSessionsChanged` only marks dirty.
  Port should persist on a save/flush, not on every tab op, to match observed behavior.

## Open questions
- Does the Convex `/v1/agent/stream` proxy emit the byte-identical Anthropic SSE event set, or a
  re-encoded subset? Reference assumes identical (shared parser). Verify against the backend.
- Opus 4.8 availability for paid tier is gated by "Convex catalog" per FOUNDATION line 662 but the
  reference hard-codes paid → `[.sonnet46]` only. Which is authoritative for the port?
- `AgentUsageLog` is DEBUG-only print. Does the port need real usage telemetry surfaced to
  `palmier-telemetry`, or keep it debug-only?
- Image inline cache is a process-global static keyed by path+size+mtime — acceptable to port as a
  bounded LRU? (reference clears whole cache at `maxCacheEntries`.)
- Tab/session UI (`AgentPanelView`) is SwiftUI; the webview port must reproduce: floating tab bar,
  7 starter prompts, send gating (`!isStreaming && canStream && non-empty draft`), jump-to-bottom.
