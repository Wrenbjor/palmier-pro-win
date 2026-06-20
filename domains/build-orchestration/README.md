---
kind: domain
domain: build-orchestration
status: active
goal: Ship a working native Windows palmier-pro that meets the product spec, via compounding loops over BMAD.
cadence: manual (autonomous inner loop earned once the pipeline runs clean end-to-end)
---

# build-orchestration — the master build loop

Drives the palmier-pro Mac→Windows port end to end. Reads the phase pipeline in
[[build-orchestration]] (`docs/build-orchestration.md`), figures out the current phase from this
README's state, advances the frontier one step, and logs the run. Consumes: the existing Mac source,
the PRD/architecture/stories in `_bmad-output/`, and `signals/` raised mid-build. Produces: planning
artifacts, app-code PRs, passing tests, and a spec-compliant Windows app.

## Current focus
**Phase 0 COMPLETE → Phase 1 (PRD) starting.** Orchestrator operating per [[orchestrator-protocol]],
autonomous, no human in the loop.

**Phase 0 done:** 15 reference docs in `docs/reference/*.md` + the binding decision record
[[phase0-reconciliation]] resolving 24 FOUNDATION↔reference discrepancies (reference = parity
authority). Key results: MCP surface is **30 tools** (not 36 — FOUNDATION corrected); clip Transform
center-based; bundle files `project.json`/`media.json`/`chat/`; visual model **SigLIP2** not CLIP;
Slip/Slide deferred (don't exist in reference). Top risk surfaced: **wgpu→WebView texture presentation
is unspecified — mandatory spike before Phase 2 architecture commit.**

**Phase 1 — PRD: COMPLETE.** `docs/PRD.md` (1,008 lines, `status: validated`) produced via BMAD-aligned
draft → 3 adversarial critics (PM/architect/QA) → revise. 12 dependency-ordered epics, each with crates +
acceptance + governing reference doc; milestones M1–M5; spikes S-1 (wgpu→WebView, gates Epic 5) + S-1b
(Convex Date encoding, M1). All §13 open questions decided.

**Phase 3 — Epics + Stories: COMPLETE.** 12 epic files + `sprint-plan.md` in
`_bmad-output/implementation-artifacts/` — **135 stories**, each with crates, acceptance, dependencies,
milestone, and a parallel-safe flag; sprint plan has the dependency DAG + M1–M5 + parallel-batch waves.

**Phase 4 — Build: IN PROGRESS (M1).** Workspace **scaffold merged to main** (`d7b36c0`) — 18 crates
compile + test green, `src-ui` builds (independently verified). Toolchain via `scripts/with-msvc.ps1`.

_(Per-wave history → `## Timeline` below. This block = concise current state.)_

**M1 build — ~36 stories + 3 spikes merged & green on main (`a4c7ae3`).** What's in:
- **Epic 2 (model + project I/O): COMPLETE** — Timeline/Track/Clip/keyframes/MediaAsset/dates; save/load
  (atomic), registry, autosave, 3 golden `.palmier` bundles (SM-7/SM-1b gates). `f64::round` parity locked.
- **Epic 3 (edit): COMPLETE** — pure engines (ripple/overwrite/split/snap) + orchestration (atomic apply,
  undo grouping) + interactive timeline input controller (E7 command seam).
- **Epic 4 (media):** cache + metadata + ffmpeg thumbnails + waveform.
- **Epic 6 (export):** XMEML emitter + golden fixtures (video export E6-S5 pending).
- **Epic 1 (app shell):** runtime, menu, windows, settings, updater, telemetry+auth wired.
- **Epic 5 (preview):** only the audio mixer (E5-S6) so far.
- **Infra:** MSVC build wrapper, **FFmpeg-on-Windows toolchain** (ffmpeg-next 7.1, env auto-sourced).
- **Decided:** wgpu→WebView mechanism (S-1), Convex Date codec (S-1b).

**Open carry-forward:** palmier-auth Convex HTTP paths inferred — confirm vs the live deployment (S-2 window).

**Epic 5 (preview):** decode/frame source (E5-S2) in; audio mixer (E5-S6) in; **GPU-present mechanism proven**
(E5-S8 sub-spike — Plan A1, [[phase0-reconciliation]] #23).

**Wave 7: COMPLETE** — E5-S3/S4 (composition graph + sampling), E4-S8..S11 (media-panel UI), E4-S6/S7 (folder
model + import; re-dispatched after a stall). All green.

**Wave 8: COMPLETE** — E5-S5/S7 (transport), E4-S12..S14 (panel polish). **Epics 1-4 done; Epic 6 XMEML done.**

**E5-S8 MERGED (93c44a3) — the wgpu compositor present is in** (GPU-proven on real HW; A1 mechanism; windows-0.58
dep fix preserved through the merge). Epic 5 now has decode + composition + transport + audio + PRESENT.

**M2 foundation MERGED** (E7-S1 30-tool registry, E8-S1 agent scaffold).

## 🎯 M1 COMPLETE (bb3eb4a) — the hand-edit MVP. See [[retro-m1]].
Epics 1–6 all merged + green (cargo default + wgpu-compositor + gpu-export builds + SM-2 GPU tests + goldens +
pnpm). ~47 stories + 5 spikes, 18 crates. wgpu→WebView solved + HW-proven; **SM-2 crushed** (1080p60=602fps /
4K30=529fps); ProRes export proven. QA follow-ups (need a display/NVIDIA): live-window composite confirm,
§11.3 driven e2e, H.264/H.265 HW encode. Parked: ProRes 422, accept-GPLv3.

**M2 IN PROGRESS (Epics 7–8 — the strategic centerpiece). In:** the MCP 30-tool registry + executor + ALL
non-generation tool bodies (read/edit/library/text/inspect — 24 of 30 functional; generate=M3, search=M4 stubbed);
the **MCP HTTP server** (E7-S11, 127.0.0.1:19789, verbatim AgentInstructions, external clients connect); the agent
message model + request/SSE + real AnthropicClient + **run loop** (E8-S1..S4) + the **chat panel UI**.

**IN FLIGHT (3 workers):**
- **M2 integration** (a26c5fcc — wire MCP server boot + the agent command/event surface + the ToolDispatcher adapter
  over ToolExecutor + mount the panel; MCP server & agent share ONE EditorState; palmier-tauri+src-ui) — the keystone.
- **E8-S5** (a3524880 — agent mentions/context-hints + image inlining, palmier-agent)
- **Spike S-2** (afb04362 — Convex WS live-query for generations:by_id; M3 prep, isolated)

**M2 REMAINING after integration:** E7-S13 (.mcpb + shared prompt module + Help MCP-instructions) · E8-S6 (PalmierClient,
needs S-2) · E8-S7 (tab orchestration/save) · E8-S9 (agent-cut e2e). Then **M3** (gen+transcription) · **M4** (search) · **M5** (polish+release).

## 🎯 M2 COMPLETE (af28901) — the agentic NLE. MCP server + in-app agent live, sharing one EditorState.
Epics 7–8 functionally done: 30-tool MCP HTTP server (127.0.0.1:19789, verbatim 8694-byte AgentInstructions, external
clients connect) + 26/30 tool bodies live (generate=M3, search=M4 remain); agent run-loop + real BYOK Anthropic SSE
client + chat panel + session persistence (`chat/`) + **explicit verbatim system prompt** (E8-S8, palmier-prompt direct
dep) + mentions/image-inline; panel tab create/switch/close/delete round-trips to the backend. **Live-access-gated
remainders (need Wren §13.9):** E8-S6 PalmierClient (builds vs the `convex` crate; live round-trip needs the Convex URL +
test Clerk account), E8-S9 agent-cut e2e (needs a real ANTHROPIC_API_KEY + a window/tauri-driver). These are the only
non-code blockers; everything testable-without-secrets is green.

**M3 IN PROGRESS (Epics 9–10 — generation + transcription). In:** E9 generation lifecycle (palmier-gen: transport /
catalog / cost / validate / params / upload / gating / service + the 4 generate tool bodies; `convex-transport` feature
**OFF by default** until palmier-agent canonicalizes request JSON — [[phase0-reconciliation]] preserve_order ruling).
**Toolchain prep done:** whisper-rs 0.16 builds + runs end-to-end (JFK sample transcribed; portable CMake + `whisper-env.ps1`
auto-sourced by `with-msvc.ps1`; CPU baseline, MIT models). S-3 SigLIP2 runtime resolved (ort+ONNX, M4 prep). **Next:**
E10 (palmier-transcribe — whisper wrapper + verbatim 14-test CaptionBuilder + add_captions/get_transcript bodies).

## Backlog
- [x] Record the macOS source path (`../palmier-pro/`) in `CLAUDE.md`. ✓ 2026-06-20
- [x] File the Foundation Spec as `docs/FOUNDATION.md`. ✓ 2026-06-20
- [ ] **Phase 0** — `bmad-document-project` on `../palmier-pro/` → feature inventory + porting risks in `docs/`; resolve the 6-tool MCP delta (§13.12).
- [ ] **Phase 1** — party-mode kickoff → `bmad-product-brief` → `bmad-prd`; decide the §13 open questions; gate on PRD validation. Promote PRD to `docs/PRD.md`.
- [ ] **Phase 2** — `bmad-create-architecture` + `bmad-ux` for the Windows design.
- [ ] **Phase 3** — `bmad-create-epics-and-stories` → `bmad-sprint-planning` → `bmad-create-story`.
- [ ] **Phase 4–8** (inner loop) — parallel dev → review+merge → UI/integration test → validate → docs; repeat until spec met.
- [ ] Decide & wire the autonomous trigger for the inner loop (`/loop` + `ship-change.js`, not tmux/story-automator).
- [ ] Repoint `origin` off the upstream template to our own remote before pushing port work.

## Evidence & analysis
[[FOUNDATION]] · [[phase0-reconciliation]] · [[orchestrator-protocol]] · `docs/reference/*` (15 docs) · [[build-orchestration]] · [[windows-harness-notes]]

## Metrics
`metrics/` — TBD. Candidate once dev starts: stories shipped, PRs merged, e2e pass rate, PRD criteria met.

## Timeline
2026-06-20 | setup — environment prepared: Windows harness fixes (PYTHONUTF8), CLAUDE.md operating context, orchestration pipeline + this loop contract written. Awaiting Mac source path + kickoff task.
2026-06-20 | kickoff-input — macOS reference located + verified at `../palmier-pro/`; Foundation Spec filed as `docs/FOUNDATION.md` (source of truth). Both Phase 0 inputs in hand. Ready to launch on `go`.
2026-06-20 | launch — repo attached to github.com/Wrenbjor/palmier-pro-win; orchestrator machinery written; Phase 0 docs workflow launched (after fixing a burst rate-limit via throttled batches).
2026-06-20 | Phase 0 COMPLETE — 15/15 reference docs written; [[phase0-reconciliation]] rules 24 discrepancies; FOUNDATION corrected (30 tools). Advancing to Phase 1 (PRD). Top risk: wgpu→WebView spike.
2026-06-20 | Phase 1 COMPLETE — docs/PRD.md validated (draft→3 adversarial critics→revise; majors fixed: 17 crates, open-30-clip perf, Convex Date sequencing S-1b). 12 epics, M1–M5, spikes S-1/S-1b. Advancing to Phase 3 (epics+stories).
2026-06-20 | Phase 3 COMPLETE — 135 stories across 12 epic files + sprint-plan.md (DAG + M1–M5 + parallel waves). Advancing to Phase 4 (build).
2026-06-20 | toolchain-unblocked — diagnosed + fixed the MSVC link failure (vswhere blind to the VS install); all builds now go through scripts/with-msvc.ps1 (verified). Removed a blocker that would have killed every dev worker.
2026-06-20 | scaffold merged (d7b36c0) — 18-crate workspace + src-ui, independently re-verified green. M1 build delegation started.
2026-06-20 | E2-S1 merged (ccc9de4) — palmier-model core enums (ClipType/Interpolation/AnimatableProperty; rulings #8 Smooth-default, #12 all-visual-compatible). Build+tests green on main. Workers S-1/E1-S1/E3-S8/E4-S2 still in flight.
2026-06-20 | E4-S2 merged (431cd83) — palmier-media disk cache + SHA256 key (#16) + concurrency gates (2/4/ungated) + in-flight dedup; 18 tests green on main. Wave-1 remaining: E1-S1, E3-S8, S-1.
2026-06-20 | E3-S8 merged (6940f8b) — palmier-history generic 2-stack undo (user/agent), agent-refusal rule, nested coalescing; 13 tests green on main. Wave-1 remaining: E1-S1, S-1.
2026-06-20 | E1-S1 merged (0612ef0) — real Tauri 2.11.3 runtime + boot; FIRST Tauri build on Windows compiled clean (wry/tao/webview2-com, no missing deps); asInvoker manifest; 10 tests green. Cargo.lock conflict resolved by regenerate.
2026-06-20 | S-1 RESOLVED + merged — wgpu→WebView decided (native composited surface, zero-copy, SM-2 met; wgpu 27.x). [[phase0-reconciliation]] #23 updated. Wave 0+1 complete; dispatching Wave 2.
2026-06-20 | Wave 2 COMPLETE (37d8637) — model trio (E2-S2/S4/E3-S1, 38 tests), E1-S6 auth (28 tests), E1-S2 telemetry (30 tests), E5-S6 engine audio mixer (26 tests), S-1b Date codec. All verified green on main. Dispatching Wave 2b.
2026-06-20 | Wave 2b partial (2e32a4d) — E4-S1 media metadata (pure-Rust, no ffmpeg; 30 tests) + E3-S2..S5 palmier-edit pure engines (ripple/overwrite/split/snap; 57 tests, #10 sticky-1.5× guarded). Green on main. Wave-2b remaining: model E2-S3/S5/S8, tauri E1-S3+boot-wiring.
2026-06-20 | E2-S3/S5/S8 merged (8c6ba8c) — keyframes+sampling, Clip core entity (f64::round ties-away parity test LOCKED), serde_date (apple-epoch/iso8601); 75+4 tests green. Worker caught a 1-day arithmetic slip in the S-1b FINDINGS worked example (corrected; fixture synthetic, round-trips fine). Wave-2b remaining: E1-S3 (tauri).
2026-06-20 | E1-S3 merged (b69f057) — full menu + exact shortcuts; telemetry subscriber seam RESOLVED (file logging attaches); auth/telemetry wired into Tauri managed state; 15 tests green. Wave 2b COMPLETE. Dispatching Wave 3 (E2-S6/S7, E3-S9, E1-S4/S9/S10); FFmpeg toolchain queued next.
2026-06-20 | E2-S6/S7 merged (0e61518) — Track/Timeline (fps-freeze, total_frames, displayHeight reset) + MediaAsset/Manifest/MediaSource/GenerationLog (legacy cost fallback); 103+4 tests green. Epic-2 model layer COMPLETE (E2-S1..S8). Wave-3 remaining: E3-S9 (canvas), E1-S4/S9/S10 (app shell).
2026-06-20 | E3-S9 merged (e1c660a) — timeline canvas (src-ui/editor; immediate-mode draw, per-type clip visuals, ruler/playhead/rubber-bands; #10/#9/#21; mocked data until get_timeline). pnpm build green (note: pnpm install first in main checkout). Wave-3 remaining: E1-S4/S9/S10.
2026-06-20 | E1-S4/S9/S10 merged (5bc0494) — windows (per-label state, sizes), settings 5 tabs + Help + Feedback, updater (behind optional feature; Ed25519 pubkey needed for release), capabilities-file fix (was empty → would've denied invoke/listen). cargo+pnpm green. Wave 3 COMPLETE. Dispatching Wave 4 (E2-S9, E6-S1/S7, E3-S6/S7) + FFmpeg infra.
2026-06-20 | E2-S9 merged (f85b37f) — palmier-project bundle reader/writer; atomic temp-dir-swap save (crash-safe), reference filenames (#3), severities ported exactly; round-trip test (SM-7 seed); 16 tests green. The save/load spine. Wave-4 remaining: E6-S1/S7, E3-S6/S7, FFmpeg infra.
2026-06-20 | E6-S1/S7 + E3-S6/S7 merged (e4ee262) — XMEML emitter + 3 golden fixtures (SM-7 byte gate; 27 tests) + bundle export; edit orchestration (Clip↔view adapter, ripple/split/move with ATOMIC validate-before-mutate, one-undo-per-edit) + drag-state machine (90 tests). Green on main. Wave-4 remaining: FFmpeg infra (then Wave 5 decode/export).
2026-06-20 | FFmpeg toolchain RESOLVED + merged (25eed3c) — ffmpeg-next 7.1 builds via the wrapper (independently verified PROBE_SUCCESS from clean env); FFmpeg 7.1 LGPL shared @C:\ffmpeg + libclang wheel; env auto-sourced. Note: LGPL excludes x264/x265 → HW encoders for H.264/H.265 (E6-S5), ProRes fine. Wave 4 COMPLETE. Dispatching Wave 5.
2026-06-20 | E3-S10 merged (40c83f6) — interactive timeline input controller (src-ui/editor): tools V/C, selection/marquee, drag-move/trim/split, sticky-snap 1.5×, transport, undo/redo; local-optimistic with an EditController.dispatch seam for E7 Tauri commands. pnpm build green. Wave-5 remaining: E4-S3/S4/S5, E2-S10/S11/S12.
2026-06-20 | E2-S10/S11/S12 merged (a463c23) — ProjectRegistry + media-path resolver + ProjectDocument autosave + 3 golden .palmier fixtures (SM-7/SM-1b gates); 43 tests. Epic-2 project I/O COMPLETE.
2026-06-20 | E4-S3/S4/S5 merged (c76f22d) — ffmpeg sprite thumbnails + waveform (150/s cap 20000) + image thumbnails; ffmpeg-next 7.1 linked first-try via the wrapper (toolchain validated); E4-S1 fps backfilled; 56 tests. Wave 5 COMPLETE (~29 stories). Dispatching Wave 6 (E5-S2 decode, E5-S8 WRY sub-spike, E1-S7/S8 home/registry/samples).
2026-06-20 | E5-S8 WRY sub-spike SUCCEEDED + merged (04e921d) — composited frame ACTUALLY APPEARED (wgpu 27 behind transparent WRY WebView2 child, D3D12/AMD, zero-copy, screenshot-proven). E5-S8 mechanism = Plan A1 (WRY build_as_child); pinned wgpu 27.0.1/winit 0.30.13/wry 0.55.1/rwh 0.6.2 + clip_children(false). [[phase0-reconciliation]] #23 updated. Last rendering unknown closed.
2026-06-20 | E5-S2 merged (ab1b947) — palmier-media decode/frame source (FrameSource/FrameCache distance-from-playhead/512MB ceiling/SeekMode; HW decode d3d11va/dxva2/vaapi + CPU fallback; CPU planes, GPU upload deferred to engine); 80 tests. Preview-pipeline ROOT in. Wave-6 remaining: E1-S7/S8.
2026-06-20 | E1-S7/S8 merged (199ba03) — Home registry lifecycle (create/open/delete-to-trash, autosave-on-switch) + sample materialization (offline-safe, reference filenames #3); 67 tests, pnpm green. Wave 6 COMPLETE; Epic 1 done (~33 stories). Dispatching Wave 7 (E5-S3/S4 composition, E4-S8..S11 panel UI, E4-S6/S7 folders+import).
2026-06-20 | ops — E4-S6/S7 worker stalled at 0 tool-uses (read CLAUDE.md worker-note, stopped); SendMessage unavailable in harness → re-dispatched fresh (aea08a8a). No stray worktree/branch left.
2026-06-20 | E5-S3/S4 merged (8a0b4d3) — palmier-engine composition graph (build_frame → CompositionFrame/LayerRender, z-order, source-frame mapping) + per-frame sampling (Mat3 affine, opacity/crop, 8-segment smooth parity); no wgpu dep (descriptors for E5-S8); 57 tests + criterion bench. Wave-7 remaining: E4-S8..S11, E4-S6/S7.
2026-06-20 | E4-S8..S11 merged (cfa24be) — media-panel UI (src-ui/media-panel: shell + Media/Captions/Music tabs, 4 sort modes, filters, folder/flat/grouped views, search panel, generation panel; mocked + E7/E9/E11 seams); pnpm green. Wave-7 remaining: E4-S6/S7.
2026-06-20 | E4-S6/S7 merged (a4c7ae3) — folder model + cycle-guarded moves + snapshot-undo (palmier-model/project/history) + import orchestration (one undo step, recursive folder→hierarchy, byte-exact drag payload; palmier-media); new dep edges (project→history, media→project), no cycle; 112+89+89 tests. Wave 7 COMPLETE (~36 stories). Dispatching Wave 8 (E5-S5/S7 transport, E4-S12..S14 panel polish).
2026-06-20 | E5-S5/S7 merged (698bb45) — preview transport (play/pause/seek/step/tick → TransportEvent Render/SeekDecode/CurrentFrameChanged; shared transport + per-tab state) + RenderFrame/PreviewTab model for E5-S8; SeekMode/throttle reused from E5-S2; 76 tests. E5-S8 unblocked → DISPATCHED EARLY (a98b18d9) the wgpu compositor present (build on proven A1).
2026-06-20 | E4-S12/S13/S14 merged (cbe7110) — panel drag-out/cycle-guarded moves + OS actions (reveal/copy/relink/clipboard via tauri-plugin-opener/clipboard-manager) + Captions/Music forms (#18 case, #14 Music=gen form). cargo+pnpm green. **Epic 4 (media) COMPLETE** (~38 stories). Only E5-S8 in flight (watch palmier-tauri merge conflict).
2026-06-20 | heartbeat — M1 critical path fully serialized behind E5-S8 (everything else depends on/conflicts with it). To use idle capacity, dispatched M2 FOUNDATION in parallel (disjoint crates, safe regardless of E5-S8): E7-S1 (palmier-tools MCP registry/dispatch/ShortId, af837a44) + E8-S1 (palmier-agent message/session/client scaffold, a8c6511f).
2026-06-20 | E8-S7 merged (fff2590) — agent session persistence: load <project>/chat/*.json on open (sorted desc + fresh tab), save non-empty sessions on document-save (#4) + dirty-mark after each turn; tab commands (list/new/open/close/delete); scoped to palmier-tauri reusing palmier-agent session_store + palmier-project bundle; 50+9 tests. Frontend tab wiring = E8-S8. (M2)
2026-06-20 | E7-S13 merged (ee144d5) — palmier-prompt shared crate (AGENT_INSTRUCTIONS one source, byte-fidelity gate 8694 bytes; palmier-mcp re-exports it) + the .mcpb bundle (manifest 0.4 + node stdio→HTTP shim → mcp-remote@127.0.0.1:19789) so Claude Desktop can install. 51 suites. Follow-up: wire palmier-agent's system prompt to palmier_prompt (ruling #2); the Help "Install for Claude Desktop" UX. (M2)
2026-06-20 | ⭐ **M2 INTEGRATION merged (721ff74)** — the agent + MCP server are LIVE in the app. MCP server (boot step 6) + the in-app agent (agent_send/cancel/status/set_pref commands + agent://event streaming + ExecutorDispatcher adapter) **share ONE EditorState** via a single Arc<ToolExecutor> (test `mcp_and_agent_share_one_editor_state` passes) — external Claude-Desktop edits and in-panel edits land on one timeline+undo. AgentPanel mounted in Project shell; read+edit tools run live through the agent. cargo+pnpm green. Needs a real Anthropic key for the live round-trip + a window for the visual run (tauri-driver e2e = E8-S9). M2 core functionally COMPLETE.
2026-06-20 | E8-S5 merged (dc5f9eb) — agent mentions/context-hints (mediaAsset/timelineClip/timelineRange verbatim JSON, prepended to user msg) + image inlining (downscale 1568px, JPEG q-ladder [85,70,55,40] to ≤3.5MB, bounded cache); MentionEnricher seam (wired live in E8-S9); 120+5 tests. (M2)
2026-06-20 | Spike S-2 RESOLVED + merged (71d10af) — Convex WS live-query. **Found FOUNDATION §2.2/§8.1 wrong**: the official `convex` crate (0.10.4, Apache-2.0) exists + does the full WS sync protocol 1:1 with the reference. Ruling #25: ADOPT it (don't hand-roll). E9 contract locked. ⚑ E9 gating (Wren §13.9): live Convex URL + test Clerk account still secret — E9 builds vs the crate+shapes, live round-trip needs that access.
2026-06-20 | E7-S5/S8/S10 merged (69fb90e) — MCP tool bodies: add_texts/add_captions, library tools (import/folders, dual-shape, MediaLibrary agent-undo stack), inspect_media/inspect_timeline (gpu-inspect feature, real GPU frames); 136 tests. 24 of 30 tools functional (generate=M3, search=M4 stubbed). Dispatching M2 integration + E8-S5 + Spike S-2.
2026-06-20 | **E7-S11 MCP SERVER merged (2e92360)** — palmier-mcp axum HTTP server on 127.0.0.1:19789 (loopback-only), the 3 validators (Origin/content-type/protocol), Initialize identity (name palmier-pro, **instructions = verbatim AgentInstructions 8694 bytes**, .gitattributes LF-pinned for byte-fidelity), tools/list = exactly 30, 2 resources, single+batched JSON-RPC, well-known endpoint; McpServer::start/stop boot seam; 36+9 tests. **External MCP clients (Claude Desktop/Code/Cursor/Codex) can now connect.** (M2 centerpiece)
2026-06-20 | agent-panel-ui merged (e2e4b34) — the in-app agent chat panel (src-ui/agent-panel: collapsible shell, session tabs+history, message blocks [text/toolUse/toolResult], @mention autocomplete, model picker #20, 7 starter prompts; MockAgentStream + AgentPanelController integration seam); pnpm green. (M2)
2026-06-20 | E8-S4 merged (587f866) — agent tool-execution loop (AgentLoop::run_turn drives a turn across tool rounds: stream→accumulate→dispatch tool_use→resume→end_turn) + orphan-tool_use repair (prepend/insert Cancelled) + clean cancellation; ToolDispatcher trait seam (mock-testable); 96+5 tests. The agent's run loop works end-to-end vs a mock; real ToolExecutor wiring = integration. (M2)
2026-06-20 | 🎯 **M1 COMPLETE** (bb3eb4a) — last wave merged + verified: E5-S11 (perf gate — **SM-2 met 10-17×**: 1080p60=602fps/4K30=529fps on real GPU + SM-C1 golden + 1000-clip bench), E6-S5 (video export — real ProRes encode end-to-end; HW-encoder chain for H.264/H.265), E7-S4/S12 (8 MCP edit tools via palmier-edit + agent undo stack), E8-S3 (real AnthropicClient SSE transport). Full M1-exit verify green. Retrospective: [[retro-m1]]. Driving M2 next.
2026-06-20 | E8-S3 merged (124a4b8) — concrete BYOK AnthropicClient (reqwest+rustls → SSE → StreamEvents; CancellationToken + drop cancellation; HTTP≥400 → typed terminal Error; wired select_client/build_client); 82 unit + 5 wiremock tests (no live API in CI). The agent can now stream from Claude. E8-S4 (run loop) next. (M2)
2026-06-20 | wave merged (f9f0ed1) — E5-S9 (compositor TEXT rendering: palmier-text cosmic-text layout + GPU glyph atlas pass, 18 reference fonts bundled w/ OFL licenses, real-HW text smoke; 41 suites), E5-S10 (preview viewport UI + overlays + transport command/event wiring → **preview functional end-to-end**; cargo+pnpm green), E7-S2/S3 (MCP executor: single-owner EditorState + Mutex serialization + read-tool bodies w/ exact get_timeline shaping + arg validation; +justified palmier-history Send bounds for cross-thread executor; 42 suites). ~45 stories. Dispatching M1-finish (E5-S11 perf, E6-S5 export) + M2 (E7-S4/S12 edit tools+undo, E8-S3 reqwest SSE).
2026-06-20 | E8-S2 merged (93686a1) — palmier-agent Anthropic request builder (exact wire body, 3 cache_control markers = 2 logical breakpoints per reference byte-parity, sorted keys, headers) + stateful partial-line SSE parser (all event types, tool_use input_json_delta accumulation); model ids verified vs claude-api docs; 70 tests. E8-S3 (reqwest transport + tool loop) next. (M2)
2026-06-20 | E5-S8 MERGED (93c44a3) — **M1 KEYSTONE: the wgpu compositor present**. GPU-proven on real HW (device/pipeline/texture-upload/premult-alpha-quad/readback smoke tests); A1 mechanism via Tauri WebviewWindow HasWindowHandle (rwh 0.6.2); video/image/lottie real, text→E5-S9. Worker diagnosed+fixed a gpu-allocator/windows 0.56-vs-0.58 conflict; the engine's feature-gated windows=0.58 dep preserved it through the 3-way merge (default+featured builds both green, verified). Resolved the palmier-tauri main.rs conflict (combined E4-S12 media cmds + E5-S8 preview cmds).
2026-06-20 | E10 WAVE-2 dispatched (3 workers, all palmier-transcribe, file-partitioned) — E10-S2 (engine.rs: FFmpeg 16kHz/mono/s16le extraction + whisper-rs 0.16 run via the auto-sourced whisper toolchain, real ggml-small.en transcription test, CPU baseline), E10-S3 (locale.rs+profanity.rs: sys-locale resolver + etiquette censoring), E10-S4 (cache.rs: TranscriptCache sha256(content)+model+lang key #19, windowed-filter no-re-transcribe, 4-entry clear-all). Each makes minimal additive lib.rs/Cargo.toml edits; orchestrator resolves any mod/dep merge overlap. All depend on the merged S1 model.
2026-06-20 | M3/M4 WAVE-1 MERGED + green (154065b) — E10-S1 (palmier-transcribe model: TranscriptionWord start/end Option<f64> per reference, offsetting no-op@0, error enum verbatim msgs, serde round-trip; 4 tests), E10-S5 (CaptionBuilder phrase algo in palmier-text: grapheme-aware via unicode-segmentation, min-dur cascade uncapped, "U.S."/"3.14" intact; 28 tests). **Parity catch:** the reference's "14 CaptionBuilderTests" = **8 phrase-algo oracles (S5) + 6 specs(...) tests (E10-S6 scope)** — all 8 ported verbatim+passing; the 6 specs tests carried to E10-S6. E11-S2 (palmier-search PALMEMB1 .embed store: byte-exact vs S-3 golden incl f16 0x3800/0xBC00 LE, modelVersion=2, sha256(path|mtime|size)[:32] key, atomic write, save does NOT normalize — indexer feeds pre-normalized vecs; 13 tests). All 3 disjoint crates, clean merges (no Cargo.lock conflict). Worktrees cleaned.
2026-06-20 | M3/M4 WAVE-1 dispatched (3 disjoint workers, worktree-isolated) — E10-S1 (palmier-transcribe TranscriptionResult model + offsetting + error enum), E10-S5 (CaptionBuilder phrase algo in palmier-text — 14 verbatim parity tests, SM-13), E11-S2 (palmier-search .embed PALMEMB1 byte-exact store + cache key, reusing the S-3-proven reader/writer, modelVersion=2). Disjoint crates (transcribe/text/search), no shared files. E11-S1 spike already satisfied by S-3. Awaiting completion+verify+merge.
2026-06-20 | E9 GENERATION merged+pushed (76ce7bb) — palmier-gen generation lifecycle (transport/convex_ws/http_poll/catalog/cost/validate/params/upload/gating/service) + the 4 generate tool bodies (palmier-tools/src/generate.rs); 40+71 lib+74 integ tests. ⚑ Cross-crate hazard: the `convex` crate enables serde_json preserve_order (workspace-contagious) which would break palmier-agent's sorted-key Anthropic goldens → `convex-transport` defaulted OFF; verified 52 default suites green + featured build compiles. BINDING follow-up recorded in [[phase0-reconciliation]]: palmier-agent must canonicalize request JSON before convex-transport is ever default. (M3)
2026-06-20 | E8-S8 merged (af28901) — agent-panel tab wiring (newChat/select/close/delete round-trip to agent_* commands + new agent_get_session restores a switched-to tab's conversation; FrontendMessage projection) + **the in-app agent's system prompt now explicitly = palmier_prompt::AGENT_INSTRUCTIONS verbatim** (ruling #2; palmier-prompt direct dep of palmier-tauri; new test asserts byte-identity at AgentState/AgentLoop/AgentRequest + vs the MCP server's advertised instructions — the no-drift property). cargo+pnpm green. **M2 functionally COMPLETE.**
2026-06-20 | Spike S-3 SigLIP2 RESOLVED + merged (isolated under spikes/) — runtime = **ort (ONNX Runtime 2.0)** + DirectML/CPU (candle ships SigLIP1 only); weights = onnx-community/siglip2-base-patch16-256-ONNX (Apache-2.0, split vision/text encoders, Gemma tokenizer). **Load-bearing:** ONNX pooler_output is NOT L2-normalized → port must add explicit L2-normalize (then 0.05/0.85 cutoffs hold). `.embed` keeps PALMEMB1 bytes but bumps modelVersion→2 (re-index). 19 tests pass; ort path type-checks. NOT yet proven: live encode/cosine (blocked on ~750MB weights DL). Decisions for Epic 11 recorded in [[phase0-reconciliation]]. (M4 prep)
2026-06-20 | whisper toolchain merged (infra/whisper-toolchain) — **whisper-rs 0.16 builds + RUNS end-to-end** (JFK sample transcribed correctly on CPU via the MSVC wrapper). Installed portable CMake 4.3.3 (no admin) + new `scripts/whisper-env.ps1` (PATH + CMAKE_GENERATOR=Ninja); `with-msvc.ps1` now dot-sources it so every Rust build inherits cmake — **verified the modified wrapper still builds the whole workspace green (52 suites)**. CPU baseline (AMD box, no CUDA), Vulkan opt-in; **models are MIT** (clean, unlike LGPL FFmpeg). E10 unblocked. Gotcha for E10: cargo clean -p palmier-transcribe if switching CMake generators (stale CMakeCache). (M3 prep)
2026-06-20 | E8-S1 + E7-S1 merged (5f2b5e5) — M2 FOUNDATION in. palmier-agent scaffold (message/session model, StreamEvent, AgentClient trait+selection, tier availability, chat/ session store; 47 tests). palmier-tools MCP scaffold: ToolName/registry = EXACTLY 30 (verified 3 ways), all 30 descriptions byte-for-byte verbatim from ToolDefinitions.swift, ToolDispatch seam + exhaustive 30-arm match + ShortId expand/shorten/ambiguity, 2 resources; 34 tests. ~40 stories + 3 spikes. Only E5-S8 (M1 keystone) in flight.
