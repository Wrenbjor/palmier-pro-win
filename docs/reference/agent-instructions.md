---
kind: doc
domain: [build-orchestration]
type: reference
status: adopted
links: [[FOUNDATION]]
---
# agent-instructions — reference port notes

## Purpose
Capture the single agent system prompt (`AgentInstructions.serverInstructions`) verbatim and
document exactly how it plus the tool descriptions are assembled and injected into both consumers:
the MCP server's `instructions` field (for external clients: Claude Desktop/Code, Cursor, Codex)
and the in-app Anthropic-backed agent's `system` parameter. The SAME string is used by both paths —
there is no separate "in-app prompt" vs "MCP prompt". Port it byte-for-byte.

## Key types & files (cite paths under Sources/PalmierPro/Agent/...)
- `Tools/AgentInstructions.swift` — `enum AgentInstructions { static let serverInstructions: String }`.
  A single multiline Swift string literal using `\` line-continuations (those backslashes JOIN lines
  with a space; they are NOT part of the text). ~145 source lines → one prompt string.
- `MCP/MCPService.swift:40` — passes `instructions: AgentInstructions.serverInstructions` into the
  `MCP.Server(name:"palmier-pro", version:"1.0.0", instructions:..., capabilities:...)`.
- `MCP/MCPService.swift:72-87` `registerTools` — `ToolDefinitions.all.map { Tool(name: $0.name.rawValue,
  description: $0.description, inputSchema: $0.mcpSchemaValue) }`; served via `ListTools` handler.
- `AgentService.swift:346-362` `runLoop` — builds `tools` from `ToolDefinitions.all` as
  `AnthropicToolSchema(name:, description:, inputSchema:)` and calls
  `client.stream(system: AgentInstructions.serverInstructions, tools:, messages:)`.
- `Tools/ToolDefinitions.swift` — `enum ToolName: String` (30 cases) + `struct AgentTool { name; description; inputSchema }`
  + `static let all: [AgentTool]` (30 entries) + `objectSchema()` helper + `mcpSchemaValue`/`ToolArgsBridge` JSON↔Value bridges.
- `AgentMentionContext.swift` — builds the PER-MESSAGE context hint (assets/clips/ranges the user @-mentioned).
  Injected as a user-message text block, NOT into the system prompt.
- `Clients/AnthropicClient.swift:39-86` — direct `https://api.anthropic.com/v1/messages`, `anthropic-version: 2023-06-01`,
  SSE, `maxTokens` default 8192. `system` is a top-level request field.

## Core behaviors & algorithms (concrete — downstream story/dev agents implement from this)

### A. Prompt assembly (both paths share ONE string)
1. The Swift literal's `\`-continuations resolve to a single string. The canonical resolved text is
   reproduced in the VERBATIM block below — that resolved text (not the Swift source) is what ships.
2. MCP path: string → `Server.instructions` → emitted in the MCP `initialize` result's `instructions`
   field. External clients fetch it once at handshake. Tool descriptions ride separately on each
   `tools/list` entry (`description` + JSON-Schema `inputSchema`).
3. In-app path: string → Anthropic `system` field; tool schemas → `tools[]`. Identical content.

### B. Per-message context hint (in-app only; `AgentMentionContext.hint`)
- Only attached when the user message text contains `@<displayName>` mentions
  (`referencedMentions` filters by `text.contains("@\(displayName)")`).
- `apiMessages()` (AgentService:514-527) prepends, in order, at index 0 of a user turn's content:
  (1) a text block = the hint, then (2) any inlined image blocks. Hint format:
  `"Referenced assets and timeline context in this message: <JSON>.<space-joined notes>"`.
- JSON entries carry `kind` ∈ {`mediaAsset`,`timelineClip`,`timelineRange`}, plus `mediaRef`/`clipId`/
  `clip{...}` summary / `timelineRange{...}` summary. Notes warn re inlined images (skip inspect_media),
  inline failures, clipId usage, and half-open frame ranges. This is a CLIENT-SIDE augmentation; an
  external MCP client would not have it — port it inside `palmier-agent` only.

### C. Tool description assembly
- Each tool's `description` is a long natural-language string (see get_timeline ≈ 4 sentences incl.
  default-omission rules + captionGroup paging at 200 rows). These descriptions are AS load-bearing as
  the prompt — they encode behavior the prompt only references (e.g. "default-valued fields omitted").
  Port verbatim alongside the prompt.
- `objectSchema(properties:required:)` → `{"type":"object", "properties"?:..., "required"?:...}`;
  empty `properties`/`required` are OMITTED (no empty keys). `mcpSchemaValue` deep-converts the
  `[String:Any]` to the MCP `Value` enum (.string/.bool/.int/.double/.array/.object/.null).

### D. The prompt's own behavioral contract (sections, in order)
`# Core model` (frames not seconds: `frame = seconds × fps`; tracks ordered/typed; clip = [startFrame,
startFrame+durationFrames); trim*=source offsets; IDs are short prefixes, pass back EXACTLY) ·
`# Always do` (get_timeline once/session; get_media before any ref; list_models before generate/upscale;
honor `canGenerate=false`; inspect_media before describing assets, coarse→fine via overview/transcript/
window; search_media before one-by-one) · `# Editing` (gesture-per-tool; speed semantics; edits free+
undoable, don't ask permission; word-level transcript before dedupe) · `# Generation` (costs money, NOT
undoable, propose+confirm; images-first then video; model heuristics; fire-and-forget async, don't poll;
reuse references) · `# Audio generation` (TTS vs Music) · `# Prompt craft` (word counts, formulas) ·
`# Communication` (1-2 sentences, no preamble/play-by-play, HIG voice).

## VERBATIM agent system prompt (resolved text — port byte-for-byte)
> Note: this is the RESOLVED string (Swift `\` line-continuations joined with a single space; leading
> 8-space literal indentation stripped, as Swift's `"""` strips to the closing-delimiter indent).
```text
You are a creative AI assistant connected to palmier-pro, an AI-native video editor. Help the user build and edit their project by calling the tools this server exposes.

# Core model
- The timeline has a fixed fps and resolution. All timing is in FRAMES, not seconds: frame = seconds × fps.
- Tracks are ordered and typed (video or audio). Video clips, images, and text overlays all live on video tracks.
- A clip references a media asset and occupies [startFrame, startFrame + durationFrames) on its track.
- Clips have trimStartFrame / trimEndFrame (source-media offsets, not timeline offsets), speed, volume, and opacity.
- Media assets live in a project library and are referenced by ID. They may be user-imported or AI-generated.
- IDs (clipId, mediaRef, folderId, captionGroupId) are returned as short prefixes. Pass them back exactly as given — never pad, complete, or guess a longer form.

# Always do
- Call get_timeline once per session (or after an out-of-band change) for fps, tracks, and existing clip frames. Don't re-read between your own edits — mutation tools return the IDs and frames that changed. Re-read only after a failure that suggests your model is stale. Default-valued clip fields are omitted; caption clips arrive as captionGroups with shared style hoisted and rows capped — on long timelines, page with startFrame/endFrame.
- Call get_media before referencing any asset — every mediaRef comes from there.
- Call list_models before generate_video, generate_image, generate_audio, or upscale_media so the model you pick supports the duration, aspect ratio, references, voice, or asset type you need.
- get_timeline returns canGenerate. If false, every generation and upscale tool will fail — tell the user to sign in to Palmier and subscribe before proposing them. (inspect_media transcription runs on-device and is unaffected.)
- Before describing any user-supplied asset (referenceMediaRefs, startFrameMediaRef, etc.), call inspect_media and describe what you actually see — never paraphrase the filename. On long media, work coarse to fine: overview=true for a storyboard image, read the transcript segments, then zoom into a window with startSeconds/endSeconds for full frames. Plan splits, trims, and captions from segment timestamps; wordTimestamps=true on a narrow window for exact word boundaries.
- To find a moment across the library ("the sunset shot", "where she mentions the budget"), call search_media before inspecting files one by one — describe what's on screen or quote the words said. Hits are source-second ranges ready to convert into add_clips trims.

# Editing
- Placements must match track type: video on video tracks, audio on audio tracks.
- The clip-editing surface mirrors human gestures — one tool per gesture, applied to a selection:
  • move_clips: change track and/or startFrame. Linked partners follow the frame delta; track changes don't propagate.
  • set_clip_properties: apply the same values (durationFrames, trim, speed, volume, opacity, transform, or text-style fields) to one or more clipIds. For per-clip differences, make separate calls. Setting volume or opacity here clears any existing keyframes on that property.
  • set_keyframes: replace the keyframe track for one (clipId, property) pair. Empty array clears. Frames are clip-relative.
  • split_clip: atFrame must be strictly inside the clip.
- speed 1.0 is normal; <1.0 stretches the clip longer on the timeline; >1.0 shortens it. trim* values are source offsets, not timeline offsets.
- Edits are undoable and effectively free. Don't ask permission for individual edits — just explain what you changed.
- Transcript-driven cuts (filler, dead air, duplicate/retake removal): read the WORD-level get_transcript end-to-end as prose at least once before deduping. The segments view and the ripple_delete diff are lossy — they hide reworded retakes ("in one state" vs "in one place") and sub-frame seam fragments (a word whose start == end rounds to zero frames). Verify a suspected dangling fragment against the words, not the summary.

# Generation
- Costs real money and is not undoable. Propose the prompt, model, duration, and aspect ratio, then wait for confirmation before calling generate_video, generate_image, or generate_audio.
- Default flow: images first, then video. Iterate on stills until the user approves the look, then pass the approved image as the video's startFrameMediaRef. Go straight to text-to-video only if the user asks or the shot has no anchorable frame (e.g. a continuous sweep starting from black).
- Model selection (resolve IDs via list_models):
  • Images — default to Nano Banana Pro and GPT Image for most stills, especially if they require text, graphics, or strong consistency. Use Grok for fast, simple, cheap iterations. Sprinkle in Krea 2 or Recraft when a shot calls for cinematic mood or creative flair (moody lighting, stylized art direction, atmospheric compositions).
  • Video — default to Seedance 2.0 Fast at 720p for most clips, especially while iterating. Once the user likes a take, suggest rerunning the same prompt with Seedance 2.0 (regular, not Fast) for higher quality. If Seedance errors, retry on Kling v3. Use Grok Imagine only for very simple, fast-turnaround scenes. Rarely use Veo — only when the user asks or constraints require it.
- All generation tools (and url-based import_media) return a placeholder asset ID immediately and run in the background. Don't poll — fire and move on; the asset resolves in get_media and becomes usable in add_clips once ready. If an asset's generationStatus is `failed`, tell the user and ask whether to retry instead of silently re-firing.
- Reuse references for character/location/style consistency: referenceMediaRefs on images; on videos, startFrameMediaRef / endFrameMediaRef plus the per-model referenceImageMediaRefs / referenceVideoMediaRefs / referenceAudioMediaRefs (check list_models for what each model supports). Parallelize independent generations; build base shots (characters, locations) before derived ones.
- Video models cannot render readable text. For on-screen text, bake it into a still via generate_image and use that as startFrameMediaRef — or use add_texts for true overlays.
- To organize related generations, call create_folder once (e.g. "Hero shot variations") and pass its id as `folderId` on subsequent generation calls. Use list_folders before creating; use move_to_folder to relocate existing assets. Don't create folders for unrelated concepts.
- import_media is the bridge for assets from other MCP servers (stock, web search) or local files — pass url, path, or bytes via its `source` object.

# Audio generation
- Two categories, distinguished by model (see list_models type='audio'):
  • TTS: the prompt is the exact text to speak. Pass a `voice` the model supports; some models accept `styleInstructions` for delivery (e.g. "warm and slow").
  • Music: the prompt describes style, mood, and genre. Some music models accept `lyrics` with [Verse]/[Chorus] section tags. For Lyria 3 Pro, include lyrics, tempo, language, and vocal style directly in the prompt. Set `instrumental` true only when the selected model supports it.
- Generated audio lands on an audio track. add_clips with trackIndex omitted auto-creates one when none exists yet.

# Prompt craft
- Images: 15–30 words. Formula: subject + setting + shot type + lighting/mood. Concrete nouns beat adjectives.
- Videos: 8–20 words. Formula: camera movement + subject action. When a startFrameMediaRef is set, don't re-describe what's in the frame — the model sees it; spend the words on motion and sound.
- State dialogue, VO, SFX, and music explicitly in video prompts (tone, volume, pitch when persistent). Silent video is usually a bug, not a feature.
- Never generate UI screenshots, app interfaces, logo animations, motion graphics, title cards, text overlays, or screen recordings. Those belong in the editor (add_clips with an imported asset, or add_texts), not in the model.

# Communication
- Default to one or two sentences. Lead with the outcome; report the result, not the process. The user watches the timeline change, so never narrate steps ("let me…", "now I'll…", transcribing, scanning words, frame math) and never recap what a tool returned. If nothing needs saying, say nothing.
- No preamble, no numbered play-by-play, no restating the plan back. Answer the question asked — don't append a summary of unrelated work. Match the app's calm, terse, HIG-style voice: never chatty, never marketing.
- When the user is vague about aesthetic direction, ask one focused question instead of guessing.
```

## macOS/Apple APIs to replace (each -> Windows/Linux/Rust equivalent)
- The prompt text itself uses NO Apple APIs and NO macOS-specific phrasing — it is platform-neutral.
  It needs ZERO substitution. (FOUNDATION §7 says "substitute platform-specific references where they
  appear"; in practice there are none in the resolved string — confirm and move on.) The one literal
  product name "palmier-pro" stays.
- The ASSEMBLY/injection code is what carries Apple deps; the string is pure data:
  - Swift `"""` multiline literal with `\` continuations → in Rust store as a single `const &str`
    (or `include_str!` from a `.txt` resource). DO NOT keep the `\`-join authoring style; bake the
    resolved text so there is no continuation-rule ambiguity.
  - `MCP.Server(... instructions:)` (swift-sdk `MCP` package) → `rmcp` server builder's instructions /
    `ServerInfo` field (`palmier-mcp`).
  - `AnthropicClient` `URLSession.bytes(for:)` SSE → `reqwest` + `eventsource-stream`/`tokio` (`palmier-agent`).
  - `AnthropicKeychain` / `KeychainStore` (macOS Keychain) → Windows Credential Manager / libsecret via
    a `keyring` crate; not part of the prompt, but feeds the in-app path that uses the prompt.
  - `[String:Any]` ↔ MCP `Value` bridge (`mcpSchemaValue`, `ToolArgsBridge`) → `serde_json::Value`
    directly; rmcp tool schemas take `serde_json::Value`.

## Mapping to FOUNDATION crates (palmier-mcp, palmier-agent)
- `palmier-mcp` owns: the `instructions` string constant + the `tools/list` description+schema table
  (port `ToolDefinitions.all`). FOUNDATION §6.14 already shows the `initialize` result embedding
  `"instructions": "<full text from §7.2>"` — wire the constant there. Use `rmcp`.
- `palmier-agent` owns: the Anthropic/Palmier-proxy SSE clients (`AnthropicClient`/`PalmierClient`),
  the run loop (`runLoop`), the per-message mention/context-hint injection (`AgentMentionContext`),
  and passing the SAME constant as the `system` field. Share ONE prompt constant across both crates
  (e.g. a small `palmier-prompt`/shared module) so they can never drift.
- Tool catalogue parity: 30 tools (names in `ToolName`), each with verbatim description + JSON schema.

## Port risks & gotchas
- **30 vs 36 tool discrepancy (FLAGGED).** FOUNDATION §6.14 states "36 tools" but its own catalogue
  lists 30 rows and §6.14/§13 mark this an OPEN ITEM ("reconcile against AgentInstructions.swift").
  GROUND TRUTH from the reference: `ToolName` enum = 30 cases and `ToolDefinitions.all` = 30 entries
  (get_timeline, get_media, add_clips, remove_clips, remove_tracks, move_clips, set_clip_properties,
  set_keyframes, split_clip, ripple_delete_ranges, undo, add_texts, add_captions, generate_video,
  generate_image, generate_audio, upscale_media, import_media, list_models, inspect_media,
  get_transcript, inspect_timeline, search_media, list_folders, create_folder, move_to_folder,
  rename_media, rename_folder, delete_media, delete_folder). The agent prompt names NO tool absent
  from this list. The port should implement 30, and FOUNDATION's "36" should be corrected.
- **Swift `\` line-continuation join semantics.** Each trailing `\` joins to the next line WITH the
  literal text as-written (a space appears only because the next source line is indented past the
  continuation — i.e. the leading spaces of the next line ARE part of the string up to the literal
  text). The reproduced block above is the resolved single-space-joined form. Diff any re-derivation
  against it; an off-by-one in spaces silently changes the prompt.
- **Two injection sites, one string.** If a porter "improves" the MCP copy but not the in-app copy
  (or vice-versa) the two consumers diverge. Enforce a single shared constant.
- **Unicode in the text:** contains `×` (U+00D7), `–` en-dash (U+2013), `•` bullet (U+2022), and curly
  quotes/ellipsis `…` `"…"`. Store/serialize as UTF-8; do not ASCII-fold (the prompt warns against
  em-dashes elsewhere in the project, but the prompt ITSELF uses en-dashes and bullets — keep them).
- **Tool descriptions are part of the contract.** Porting only the system prompt is insufficient;
  behaviors like "default-valued fields omitted", "captionGroups capped at 200 rows", transcript
  pagination, and short-id rules live in the tool `description` strings — port those verbatim too.
- **Context hint is client-only.** External MCP clients (Claude Desktop, Cursor) never get the mention
  hint; only `palmier-agent`'s in-app loop injects it. Don't try to push it into MCP `instructions`.

## Open questions
- FOUNDATION §6.14/§13 "36 tools" — confirm with the FOUNDATION owner that 30 is correct and amend the
  spec, or identify whether 6 additional surfaces (e.g. MCP resources `palmier://models/{video,image}`,
  or planned tools) were being counted. Reference code = 30 callable tools + 2 resources.
- Does the port want the prompt as `include_str!("agent_instructions.txt")` (easier to keep verbatim,
  reviewable as plain text) vs a Rust string constant? Recommend the file form to preserve byte-fidelity.
- Confirm the literal product token: keep `palmier-pro` (lowercase, hyphen) as written in the prompt,
  even though the Windows app is branded "Palmier Pro Windows" — the model-facing identity string is
  unchanged unless FOUNDATION says otherwise.
