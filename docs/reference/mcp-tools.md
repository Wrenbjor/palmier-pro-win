---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# mcp-tools — reference port notes

## Purpose
Authoritative enumeration of EVERY MCP tool the macOS reference exposes, with exact tool
name, JSON-Schema parameters, mutation/async classification, and output shape — so the
`palmier-mcp` + `palmier-tools` crates can be built to behavior parity. **Resolves the
FOUNDATION §6.14 / §13.12 "36 vs 30" delta below.**

## TOOL COUNT — authoritative finding (resolves §13.12 open item #12)
The reference registers tools from exactly **one** registry: `ToolDefinitions.all`
(`Tools/ToolDefinitions.swift:44`), enumerated by the `ToolName` enum (lines 5–34). Both the
MCP server (`MCP/MCPService.swift:73`, `.map { Tool(name: def.name.rawValue, …) }`) and the
in-app agent (`Agent/AgentService.swift:346`) iterate that same array — there is no second tool
source anywhere under `Agent/`.

**The count is 30, not 36.** Verified two ways: `grep -c "case … = \""` on the enum = 30;
`grep -c "name: \."` in `all` = 30. The dispatch `switch` in `ToolExecutor.run`
(`ToolExecutor.swift:47-78`) has exactly 30 arms, one per enum case, and is exhaustive (no
`default`). **FOUNDATION §6.14's "36 tools" is incorrect; its 30-row catalogue IS the complete
surface. There is no hidden set of 6. Flag for FOUNDATION fix: replace all "36" with "30".**
The port should implement **30** tools with the names below.

## Key types & files (cite paths under Sources/PalmierPro/Agent/Tools/...)
- `ToolDefinitions.swift` — `enum ToolName: String` (snake_case raw values = wire names),
  `struct AgentTool { name, description, inputSchema: [String:Any] }`, `static let all`, and
  `objectSchema(properties:required:)` helper. `mcpSchemaValue` converts `[String:Any]`→`Value`.
- `ToolExecutor.swift` — `@MainActor` class; `execute(name:args:)` is the single entry point;
  `run(_:_:_:)` is the 30-arm dispatch; holds `agentUndoStack: [String]`. Shared by MCP + agent.
- `ToolExecutor+*.swift` — one extension file per category holds the implementations
  (Captions, Clips, Folders, Generate, Import, InspectTimeline, Search, ShortId, Texts, Timeline).
- `ToolResult.swift` — `ToolResult { content: [Block], isError }`; `Block = .text(String) |
  .image(base64,mediaType)`; `toMCPResult()` maps to `CallTool.Result`.
- `ToolExecutor+ShortId.swift` — UUID prefix shortening / expansion (see algorithm below).
- `../MCP/MCPService.swift` + `../MCP/MCPHTTPServer.swift` — HTTP/JSON-RPC adapter + 2 resources.
- `AgentInstructions.swift` — `serverInstructions` (Initialize `instructions` field) — port verbatim.

## Core behaviors & algorithms (concrete — downstream story/dev agents implement from this)

### The 30 tools (name | mutation | async | required inputs | output)
Mutation = changes timeline/library (pushes agent-undo). Async = kicks off background work and
returns a placeholder/immediately. `image` output means a `ToolResult.Block.image` is returned.

READ (8, none mutate):
1. `get_timeline` | no | no | — (opt `startFrame`,`endFrame`) | text: fps, width/height, totalFrames, tracks[{type,clips[…]}], canGenerate. Default-valued clip/track fields OMITTED. Caption clips collapse into per-track `captionGroups` (shared props hoisted; rows `[clipId,startFrame,durationFrames,text]`, capped 200, page via start/endFrame).
2. `get_media` | no | no | — | text: assets[{id,name,type,duration,generationStatus∈{generating,downloading,failed,none},folderId}].
3. `inspect_media` | no | **yes** | `mediaRef` (opt clipId,maxFrames≤12 dflt6,startSeconds,endSeconds,wordTimestamps,overview) | image frames + transcript segments `[text,start,end]` capped 400 (page via nextStartSeconds); words capped 10000; Lottie frames over gray.
4. `get_transcript` | no | no | — (opt startFrame,endFrame,clipId) | text: clips in timeline order, words `[text,startFrame,endFrame]` capped 10000, page via nextStartFrame. Walks every audio/video clip, maps words through trim/speed/position.
5. `inspect_timeline` | no | **yes** | — (opt startFrame dflt0, endFrame, maxFrames≤12 dflt6) | image: composited frames (downscaled) + frameNumbers sampled.
6. `search_media` | no | **yes** | `query` (opt scope∈{visual,spoken,both}dflt both, mediaRef, limit≤50 dflt10) | text+image: hits as source-second ranges + score (ordering only) + visual-index `status`.
7. `list_folders` | no | no | — | text: folders[{id,name,parentFolderId?}].
8. `list_models` | no | no | — (opt type∈{video,image,audio,upscale}) | text: `{ models, loaded }`. If loaded=false catalog not synced (not signed in).

EDIT (10, all mutate, all synchronous):
9. `add_clips` | yes | no | `entries[{mediaRef,startFrame,durationFrames}]` (opt trackIndex per-entry, all-or-none) | text: new clip ids. Whole batch one undo; video-with-audio auto-creates linked audio clip; same-track overlap → trim/split/overwrite.
10. `remove_clips` | yes | no | `clipIds[]` | text. Removes whole link group of any clip.
11. `remove_tracks` | yes | no | `trackIndexes[]` | text. Remaining indexes shift down.
12. `move_clips` | yes | no | `moves[{clipId}]` (opt toTrack,toFrame; ≥1 required) | text. Linked partners follow (startFrame delta preserves l/j-cut).
13. `set_clip_properties` | yes | no | `clipIds[]` + any of durationFrames,trimStartFrame,trimEndFrame,speed,volume,opacity,transform{centerX,centerY,width,height,flipH,flipV},content,fontName,fontSize,color,alignment | text. Values applied to ALL clips. trim = SOURCE offsets. Setting volume/opacity clears that keyframe track. Text-only fields on non-text clip → reject.
14. `set_keyframes` | yes | no | `clipId`,`property`∈{volume,opacity,rotation,position,scale,crop},`keyframes[[frame,…values,interp?]]` | text. Replaces track (empty = clear). Frames CLIP-RELATIVE. interp∈{linear,hold,smooth}dflt smooth. Row layout per property (position=topLeft xy; scale=normalized wh; crop=top,right,bottom,left).
15. `split_clip` | yes | no | `clipId`,`atFrame` (strictly between start/end) | text.
16. `ripple_delete_ranges` | yes | no | `ranges[[start,end]]` + exactly one of `trackIndex`(project frames, units must be 'frames') or `clipId`(clamped, units∈{seconds,frames}dflt frames) | text: anchor track post-cut layout. Overlaps merge; linked partners cut on same span; sync-locked tracks shift to preserve alignment (refuse if would cross frame 0).
17. `undo` | yes | no | — | text. Pops `agentUndoStack`; reverses ONE agent edit. Refuses if latest undo-action name ≠ expected (user edit interleaved) or stack empty.
18. `add_texts` | yes | no | `entries[{startFrame,durationFrames,content}]` (opt trackIndex all-or-none, transform, fontName dflt 'Helvetica-Bold', fontSize dflt 96, color dflt '#FFFFFF', alignment dflt center) | text: new clip ids. Omit trackIndex → auto new top video track.
19. `add_captions` | yes | **yes** | — (opt clipIds, language BCP-47, fontName, fontSize dflt 48, color, centerX dflt .5, centerY dflt .9, textCase∈{auto,upper,lower}, censorProfanity) | text. On-device transcribe + styled caption clips on new track.

GENERATE (4, all mutate, all **async**, cost money, NOT undoable):
20. `generate_video` | yes | async | `prompt` (+name,model,duration,aspectRatio,resolution,startFrameMediaRef,endFrameMediaRef,sourceVideoMediaRef,sourceClipId,referenceImage/Video/AudioMediaRefs[],folderId) | text: placeholder asset id.
21. `generate_image` | yes | async | `prompt` (+name,model,aspectRatio,resolution,quality,referenceMediaRefs[],folderId) | text: placeholder asset id(s).
22. `generate_audio` | yes | async | **no required field** (prompt opt) (+name,model,voice,lyrics,styleInstructions,instrumental,duration,videoSourceStartFrame,videoSourceEndFrame,videoSourceMediaRef,folderId) | text: placeholder id. Video-span source → auto-placed on timeline.
23. `upscale_media` | yes | async | `mediaRef` (+model,sourceClipId) | text: placeholder id.

LIBRARY (7, all mutate):
24. `import_media` | yes | **yes (url only)** | `source{exactly one of url|path|bytes; mimeType req for bytes}` (+name,folderId) | text: new media_ref. url≤1GB async; path(recursive dir ok)/bytes(≤~11MB) sync.
25. `create_folder` | yes | no | `name`+parentFolderId? OR `entries[]` (not both) | text: folder id(s).
26. `move_to_folder` | yes | no | `assetIds[]`+folderId? OR `entries[]` | text. Omit folderId → root.
27. `rename_media` | yes | no | `mediaRef`,`name` OR `entries[]` | text.
28. `rename_folder` | yes | no | `folderId`,`name` OR `entries[]` | text.
29. `delete_media` | yes | no | `assetIds[]` | text. Referencing clips removed in same undo.
30. `delete_folder` | yes | no | `folderIds[]` | text. Recursive; referencing clips removed.

### ID prefix shortening (`ToolExecutor+ShortId.swift`)
- `idPrefixFloor = 8`. `currentIdUniverse(editor)` = all track ids, clip ids, captionGroupId,
  linkGroupId, asset ids, folder ids in one `Set<String>`.
- OUTPUT: `shortIdMap` maps each id → shortest prefix ≥8 chars unique across the universe;
  `shorteningIds` regex-replaces (`uuidRegex`) every known UUID in result text with its prefix.
  Unknown UUIDs (e.g. in filenames) pass through untouched.
- INPUT: `expandingIdPrefixes` runs BEFORE the tool. Only keys in `scalarIdKeys`
  {clipId,sourceClipId,mediaRef,startFrameMediaRef,endFrameMediaRef,sourceVideoMediaRef,
  videoSourceMediaRef,folderId,parentFolderId} and `arrayIdKeys`
  {clipIds,assetIds,folderIds,referenceMediaRefs,referenceImage/Video/AudioMediaRefs} are
  expanded. Exact match → keep; 1 prefix match → expand; >1 → ToolError("Ambiguous id…"); 0 →
  pass through (tool emits its own not-found).

### Agent undo stack (`ToolExecutor.swift:33-36, 82-96`)
Separate from user undo. After a non-undo, non-error tool run, if `editor.timeline != before`
AND there's an `undoManager.undoActionName`, push that name. `undo` pops one; refuses if
`undoManager.undoActionName != expected` (user interleaved). Mutating tools wrap work in
`withUndoGroup(actionName:)`.

### Arg validation (`ToolExecutor.swift:134-198`)
Unknown keys rejected (`validateUnknownKeys` against `DecodableToolArgs.allowedKeys`);
non-finite numbers rejected pre-decode (`firstNonFiniteNumberPath`); decode errors formatted
with JSON path. Colors via `TextStyle.RGBA(hex:)` accept `#RRGGBB`/`#RRGGBBAA`.

### Error shape
`ToolResult.error` → `{ content:[{type:text,text:msg}], isError:true }` (FOUNDATION §6.14 match).

### Resources (2, `MCPService.swift:96-133`)
`palmier://models/video`, `palmier://models/image` — JSON arrays from
`VideoModelConfig.allModels`/`ImageModelConfig.allModels` via `videoModelInfo`/`imageModelInfo`
(`ToolExecutor+Generate.swift:398,421`). `listChanged:false`, `subscribe:false`.

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
The tool LAYER itself is largely Foundation-only and portable; Apple coupling is at the edges:
- `@MainActor` / `@Observable` (Swift concurrency + Observation) → Rust: single-threaded
  command actor or `Mutex<EditorState>`; tool calls serialized through one owner.
- `JSONSerialization` / `JSONDecoder` → `serde_json`.
- `Foundation.UUID` ids → `uuid` crate (string compare for prefix logic — identical algorithm).
- `UndoManager` (Foundation, drives `agentUndoStack` via `undoActionName`) → custom undo stack
  in `palmier-core`; the named-action-string mechanism must be replicated exactly (undo refuses
  on name mismatch).
- `Regex` literal `uuidRegex` → `regex` crate `[0-9A-Fa-f]{8}-…` same pattern.
- `MCP` Swift SDK (`Server`, `Tool`, `CallTool`, `ReadResource`) → `rmcp` per FOUNDATION §6.14.
- On-device transcription (inspect_media/get_transcript/add_captions/search spoken) →
  Whisper (FOUNDATION); visual search → CLIP. These live in `palmier-tools` impls, not the schema.
- `OriginValidator.localhost` + `requiredLocalEndpoint` (MCP-Swift) → axum middleware (FOUNDATION
  §6.14: Origin/content-type/protocol-version validators; bind `127.0.0.1:19789`).

## Mapping to FOUNDATION crates (palmier-mcp, palmier-tools)
- `palmier-mcp` ← `MCP/MCPService.swift` + `MCPHTTPServer.swift`: rmcp server identity
  (name `palmier-pro`, version `1.0.0`, instructions from AgentInstructions), tool/resource
  registration, axum HTTP + validators, `.well-known/oauth-protected-resource`, JSON-RPC dispatch.
- `palmier-tools` ← `Tools/*`: the 30 `AgentTool` schema definitions (port `ToolDefinitions.all`
  verbatim — names, descriptions, JSON-Schema), the `execute`→`run` dispatch, ShortId
  universe/expand/shorten, agent undo stack, arg validation, `ToolResult`. The actual editor
  mutations call into `palmier-core` (timeline/editor model). The Tools layer is the parity
  contract; do not paraphrase tool descriptions (the LLM behavior depends on them).

## Port risks & gotchas
- **FOUNDATION says 36, truth is 30.** Do not invent 6 phantom tools. Update §6.14/§13.12.
- Tool descriptions are load-bearing prompt text (e.g. add_texts trackIndex all-or-none rule,
  ripple_delete units semantics). Port byte-for-byte; only swap macOS phrasing where required.
- `generate_audio` has `required: []` — NO required field (prompt optional for video-to-music).
  Easy to wrongly mark prompt required.
- `create_folder`/`move_to_folder`/`rename_media`/`rename_folder` are dual-shape (direct OR
  `entries[]`, "not both"); output shape differs (direct vs `{ folders }`). Validate the XOR.
- ShortId expansion runs on a fixed key allowlist, recursing into nested dicts/arrays — but only
  expands those specific key names. A new id-bearing field must be added to both key sets or it
  won't accept prefixes.
- `inspect_media` `overview=true` ignores `maxFrames`; maxFrames hard-capped 12; transcript
  pagination cap 400 segments / 10000 words. Replicate caps or token budgets blow up.
- `undo` refuses unless the editor's *current* undo-action name equals the pushed one — tightly
  coupled to how mutations name their undo groups; get the names identical to the reference.
- Caption clips' get_timeline collapsing (captionGroups, 200-row cap) is non-obvious output
  shaping — a naive flat clips array would diverge from reference output.
- HTTP server uses `requiredLocalEndpoint host:127.0.0.1` and Origin localhost validator — the
  `.well-known` body is literally `{"resource":"http://127.0.0.1:<port>"}` (no trailing path).

## Open questions
- Exact `AgentInstructions.serverInstructions` text (the Initialize `instructions` field) lives
  in `AgentInstructions.swift` — not read here; port verbatim. FOUNDATION §7.2 should mirror it.
- FOUNDATION catalogue omits `list_folders`/`list_models` from some prose but they ARE in the
  table; with 30 confirmed there is no remaining delta to resolve — confirm FOUNDATION edit.
- `videoModelInfo`/`imageModelInfo` field set (for the two resources) not fully enumerated here;
  read `ToolExecutor+Generate.swift:398-440` when implementing the model resources.
