# Work log

Append-only journal of finished work bulks, so anyone (human or agent) can catch up fast.
Newest at the BOTTOM. Append an entry whenever a bulk of work wraps (ideally right before
the commit that ships it). Keep entries SHORT: header line + What + Refs, nothing else.

**Entry grammar** (strict, one header line per entry):
```
## YYYY-MM-DD · Short title · #tag1 #tag2
What: 1-2 lines, outcome first.
Refs: [doc](path) (new|updated), repo PR/commit links.
```

**Tags** (reuse before inventing): add your own as loops emerge, e.g.
#analysis #product #content #infra #skill #research #ops #revenue #growth

**Retrieval recipes** (macOS; entry headers always start `## 20`):
```bash
# index of all entries (one line each)
grep '^## 20' LOG.md
# last 5 entries, full
tail -r LOG.md | awk '{print} /^## 20/{c++; if(c==5) exit}' | tail -r
# all entries about a topic
awk '/^## 20/{p=/#product/} p' LOG.md
# entries from a month
awk '/^## 20/{p=/^## 2026-06/} p' LOG.md
```

---

## 2026-06-20 · Environment prep for palmier-pro Win port · #setup #infra #ops
What: Made the loop-engineer + BMAD harness Windows-ready and laid the orchestration spine for the
palmier-pro Mac→Windows port (this repo is app + KB + planning). Fixed the party-mode blocker
(PYTHONUTF8), wrote CLAUDE.md operating context, the phase pipeline, and the master build loop.
Refs: [build-orchestration](docs/build-orchestration.md) (new), [windows-harness-notes](docs/windows-harness-notes.md) (new),
[build loop](domains/build-orchestration/README.md) (new), CLAUDE.md (updated), .claude/settings.json (new). Awaiting Mac source path + kickoff task.

## 2026-06-20 · Kickoff input filed: Foundation Spec + verified macOS reference · #setup #product #spec
What: Received the Palmier-Pro-Windows Foundation Specification (locked stack: Tauri 2 / Rust / React /
wgpu / FFmpeg / Whisper / Convex+Clerk+Anthropic; agent-controlled NLE via local MCP). Verified the
GPLv3 macOS Swift reference at ../palmier-pro/ matches the spec's citations. Filed spec as the source
of truth; product identity + source path now wired into CLAUDE.md and the build loop. Ready to launch.
Refs: [FOUNDATION](docs/FOUNDATION.md) (new), CLAUDE.md (updated), [build loop](domains/build-orchestration/README.md) (updated),
[build-orchestration](docs/build-orchestration.md) (updated). Next: on `go` → Phase 0 (document ../palmier-pro) → Phase 1 party-mode → PRD.

## 2026-06-20 · Autonomous orchestrator launched; repo attached; Phase 0 running · #ops #infra #orchestration
What: Wren handed off full autonomy (no human in the loop). Attached the repo to github.com/Wrenbjor/palmier-pro-win
(main pushed). Wrote the orchestrator operating manual and launched Phase 0 — a 15-agent workflow documenting
the 21-subsystem / 42K-LOC macOS reference into docs/reference/*.md (incl. the full 36-tool MCP surface,
verbatim AgentInstructions, and AppTheme token verification). Orchestrator self-heartbeat armed.
Refs: [orchestrator-protocol](docs/orchestrator-protocol.md) (new), [build loop](domains/build-orchestration/README.md) (updated),
workflow run wf_2cdb63a7-e48. Next: on Phase 0 completion → synthesize → Phase 1 PRD via BMAD party-mode.

## 2026-06-20 · Phase 0 complete — reference documented; 24 discrepancies reconciled · #analysis #reference #decision
What: 15-agent workflow documented the macOS reference into docs/reference/*.md (3,100+ lines). Synthesized
the binding decision record docs/phase0-reconciliation.md ruling on 24 FOUNDATION↔reference contradictions
(reference = parity authority). Headlines: MCP surface is 30 tools not 36 (FOUNDATION corrected in place);
clip Transform is center-based; bundle files are project.json/media.json/chat/; visual model is SigLIP2 not
CLIP; Slip/Slide don't exist (deferred); ProRes 422 (not 4444+alpha) for v1. Logged the GPLv3 clean-room
contradiction as a signal. Top architecture risk flagged: wgpu→WebView texture presentation is unspecified —
mandatory spike before Phase 2.
Refs: [phase0-reconciliation](docs/phase0-reconciliation.md) (new), docs/reference/*.md (15 new), FOUNDATION.md
(corrected), [signal](signals/gpl-cleanroom-contradiction.md) (new), [build loop](domains/build-orchestration/README.md) (updated).
Next: Phase 1 — drive BMAD to produce docs/PRD.md from FOUNDATION + reconciliation + reference docs.

## 2026-06-20 · Phase 1 complete — PRD validated · #product #prd #planning
What: docs/PRD.md (1,008 lines, status: validated) produced via a BMAD-aligned pipeline — PM author draft →
3 adversarial critics (PM pass / architect pass / QA revise) → reviser. Majors fixed: crate count 16→17 core
(+tauri=18), restored the dropped "open 30-clip 1080p <1s" perf target (SM-1b), decoupled the Convex
Date-encoding lock into Spike S-1b (M1) ahead of the Epic 2 serde commit. 12 dependency-ordered epics each
with crates + acceptance + governing reference doc; milestones M1–M5; spikes S-1 (wgpu→WebView, gates Epic 5).
Refs: [PRD](docs/PRD.md) (new), [build loop](domains/build-orchestration/README.md) (updated). Next: Phase 3 —
decompose the 12 epics into story files in _bmad-output/implementation-artifacts/.

## 2026-06-20 · Phase 3 complete (135 stories) + toolchain unblocked · #planning #stories #infra
What: 12-agent workflow decomposed all epics into 135 implementable stories (crates, acceptance, deps,
milestone, parallel-safe flag) + sprint-plan.md (dependency DAG, M1–M5, parallel-batch waves) in
_bmad-output/implementation-artifacts/. Then pre-flighted the build toolchain and caught a blocker that
would have failed every dev worker: rustc couldn't link because vswhere doesn't register this VS install.
Fixed by routing all Rust/Tauri builds through scripts/with-msvc.ps1 (calls vcvars64.bat; verified a crate
links cleanly). Installed pnpm 11.8. Windows SDK 10.0.22621 + MSVC 14.29 confirmed present.
Refs: _bmad-output/implementation-artifacts/* (13 new), scripts/with-msvc.ps1 (new), windows-harness-notes.md
+ CLAUDE.md (updated). Next: Phase 4 — scaffold the workspace, run Spike S-1 (wgpu→WebView), delegate M1 first wave.

## 2026-06-20 · Phase 4 M1: workspace scaffold + Wave-1 (5 stories) + S-1 resolved · #build #m1 #milestone
What: Scaffolded the 18-crate Cargo + Vite/React/Tauri workspace (merged, verified green). Delegated Wave-0/1 to
5 isolated-worktree workers, each verified independently + merged to main: E2-S1 (palmier-model enums), E1-S1
(real Tauri 2.11 runtime — first Tauri build on Windows, clean), E3-S8 (palmier-history undo), E4-S2
(palmier-media cache), and Spike S-1. **S-1 resolved the #1 architecture risk**: wgpu renders to a native GPU
surface composited UNDER a transparent webview (zero-copy, SM-2 FPS floors met, wgpu 27.x pinned; one
WRY-integration sub-spike deferred to E5-S8). Fixed a Cargo.lock merge conflict by regeneration.
Refs: github.com/Wrenbjor/palmier-pro-win main @0612ef0; crates/* (18), spikes/s1-wgpu-webview/FINDINGS.md,
phase0-reconciliation.md #23 (resolved). Next: Wave 2 (model E2-S2/S4 + E3-S1, auth E1-S6, telemetry E1-S2,
engine E5-S6, spike S-1b).

## 2026-06-20 · Phase 4 M1 Wave-2: 5 stories merged (model/auth/telemetry/engine) + S-1b · #build #m1
What: Wave-2's 5 workers all landed verified-green on main (37d8637): model trio (E2-S2 center Transform #7,
E2-S4 VolumeScale #9, E3-S1 edit value types; 38 tests), E1-S6 palmier-auth (Clerk/Convex/keyring #5/account
machine; 28 tests), E1-S2 palmier-telemetry (Sentry+tracing+crash+categorized logging; 30 tests), E5-S6
palmier-engine audio mixer (8-segment smoothstep envelope parity; 26 tests), and Spike S-1b which decided the
per-field project-bundle Date codec (Apple-epoch doubles vs ISO-8601). Resolved two Cargo.lock merge conflicts
by regeneration. Carry-forwards recorded (E2-S5 f64::round derivations, telemetry boot-subscriber seam, E5-S6
From<&Clip> adapter, auth Convex path confirmation).
Refs: main @37d8637; crates/palmier-{model,auth,telemetry,engine}; spikes/s1b-convex-date/FINDINGS.md;
phase0-reconciliation.md (Date entry resolved). Next: Wave 2b — model E2-S3/S5/S8, edit engines E3-S2..S5,
media E4-S1/E5-S2, tauri E1-S3 + telemetry/auth boot wiring.

## 2026-06-20 · Phase 4 M1 Wave-2b complete: model/edit/media/menu (6 stories) · #build #m1
What: Wave-2b's 4 workers landed verified-green on main (b69f057): E2-S3/S5/S8 (keyframes+Clip+serde_date;
the f64::round ties-away parity test is LOCKED — the keystone frame-math guarantee; 75+4 tests), E3-S2..S5
(the four pure edit engines ripple/overwrite/split/snap; 57 tests; #10 sticky-1.5× guarded vs a 2.5×
regression), E4-S1 (pure-Rust media metadata, no ffmpeg; 30 tests), E1-S3 (full menu + telemetry/auth boot
integration — resolved the subscriber-ownership seam so file logging attaches; 15 tests). A worker caught +
I corrected a 1-day arithmetic slip in the S-1b FINDINGS worked example (synthetic fixture, round-trips fine).
M1 now ~20 stories + 2 spikes merged. Dispatched Wave 3 (model E2-S6/S7, timeline-canvas E3-S9, app
E1-S4/S9/S10). Next infra: provision the FFmpeg-on-Windows toolchain before the decode/export stories.
Refs: main @b69f057; crates/palmier-{model,edit,media,tauri}, src-ui. Next: Wave 3 merges → FFmpeg → Wave 4.

## 2026-06-20 · Phase 4 M1 Wave-3 complete: model done + app shell + timeline canvas · #build #m1
What: Wave-3's 3 workers landed verified-green on main (5bc0494): E2-S6/S7 (Track/Timeline + MediaAsset/
Manifest — Epic-2 model layer COMPLETE; 103+4 tests), E3-S9 (timeline canvas in src-ui/editor — immediate-mode
draw of tracks/clips/ruler/playhead/rubber-bands, mocked until the get_timeline command; pnpm build green),
E1-S4/S9/S10 (window management, the 5 settings tabs + Help + Feedback, Tauri updater behind an optional
feature, and a fix to an empty capabilities file that would have denied all frontend invoke/listen). ~23
stories + 2 spikes merged. Dispatched Wave 4 (palmier-project E2-S9 save/load, palmier-export E6-S1/S7 XMEML,
palmier-edit E3-S6/S7 orchestration) + a dedicated FFmpeg-on-Windows toolchain infra worker (unblocks decode/
export). Recorded the frontend-verify lesson (pnpm install in main checkout first).
Refs: main @5bc0494; crates/palmier-{model,tauri,update}, src-ui/{app,home,settings,editor}. Next: Wave 4 merges
+ FFmpeg toolchain → Wave 5 (decode/thumbnail/waveform + video export).

## 2026-06-20 · 🎯 M1 COMPLETE — the hand-edit MVP (Epics 1–6) · #milestone #m1 #build
What: Every M1 story is merged + green on main (bb3eb4a). The native Windows Palmier Pro editing core ships:
app shell + auth/telemetry, the full model + crash-safe project I/O, the edit engines + interactive timeline,
media cache/metadata/thumbnails/panel, the complete preview pipeline (decode → composition → transport → audio →
**wgpu compositor present, GPU-proven** → text), XMEML + video export. ~47 stories + 5 spikes, 18 crates.
**Headline results:** the wgpu→WebView risk is solved + hardware-proven; **SM-2 crushed** (1080p60=602fps,
4K30=529fps on AMD RX6600XT, floors 60/30 — GPU path ships, no fallback); a real ProRes encode ran end-to-end.
Verified: cargo default + wgpu-compositor + gpu-export builds + the SM-2 GPU tests + goldens + pnpm, all green.
Obstacles cleared autonomously: MSVC linker, FFmpeg/libclang, wgpu→WebView, gpu-allocator/windows pin, a burst
rate-limit, a stalled worker, a date-math slip. Parked (reversible): ProRes 422, accept-GPLv3.
Refs: [retro-m1](docs/retro-m1.md) (new), main @bb3eb4a. Next: drive M2 (MCP server + agent — the centerpiece) to completion, then M3–M5.

## 2026-06-20 · Phase 4 M1 Wave-4 complete: project I/O + XMEML export + edit orchestration + FFmpeg unblocked · #build #m1 #infra
What: Wave-4's 4 workers all landed verified-green on main (25eed3c): E2-S9 (palmier-project bundle reader/
writer — crash-safe atomic temp-dir-swap save, reference filenames #3; the save/load spine; 16 tests), E6-S1/S7
(pure XMEML 4 emitter with 3 byte-exact golden fixtures = the SM-7 gate, + self-contained bundle export; 27
tests), E3-S6/S7 (edit orchestration — Clip↔view adapter wiring the pure engines to real Timeline/Clip with
ATOMIC validate-before-mutate + one-undo-step-per-edit, + the drag-state machine; 90 tests), and the FFmpeg-on-
Windows toolchain (ffmpeg-next 7.1 builds via the wrapper — FFmpeg 7.1 LGPL shared + libclang wheel, env auto-
sourced from scripts/ffmpeg-env.ps1; independently re-verified PROBE_SUCCESS from a cleared env). LGPL note:
software x264/x265 excluded → HW encoders for H.264/H.265 in E6-S5; ProRes/decode fine. M1 backend largely
complete (~26 stories + 2 spikes). Dispatched Wave 5 (media thumbnails E4-S3/S4/S5, project E2-S10/S11/S12,
timeline input E3-S10).
Refs: main @25eed3c; crates/palmier-{project,export,edit}, scripts/ffmpeg-env.ps1, spikes/ffmpeg-setup/FINDINGS.md.
Next: Wave 5 merges → Wave 6 (the preview stack E5-S2..S8 + WRY sub-spike + E6-S5 video export) → M1 exit.

## 2026-06-20 · M2 COMPLETE — the agentic NLE (MCP server + in-app agent, one EditorState) · #build #m2 #agent #mcp
What: Epics 7–8 functionally done and verified green on main (af28901). An MCP HTTP server (127.0.0.1:19789,
loopback-only, 3 validators, verbatim 8694-byte AgentInstructions identity, tools/list = exactly 30) lets external
Claude Desktop/Code/Cursor/Codex clients drive the editor; 26/30 tool bodies are live (generate→M3, search→M4 remain
stubbed). The in-app agent has a real BYOK Anthropic SSE client + run-loop (orphan-tool_use repair, clean cancel) +
chat panel + session persistence to `<project>/chat/` + mentions/context-hints + image inlining, and now (E8-S8) an
EXPLICIT verbatim system prompt = palmier_prompt::AGENT_INSTRUCTIONS (byte-identical to what the server advertises —
no-drift test at AgentState/AgentLoop/AgentRequest). MCP server and in-app agent share ONE EditorState via a single
Arc<ToolExecutor> (proven by `mcp_and_agent_share_one_editor_state`), so external and in-panel edits land on one
timeline + undo stack. Panel tabs round-trip create/switch/close/delete to the backend. Live-access-gated remainders
(need Wren §13.9, the only non-code blockers): E8-S6 PalmierClient (builds vs the `convex` crate; live needs the Convex
URL + test Clerk account) and E8-S9 agent-cut e2e (needs a real ANTHROPIC_API_KEY + a window/tauri-driver).
M3 already underway: E9 generation lifecycle (palmier-gen) merged; whisper-rs 0.16 toolchain proven end-to-end
(JFK sample transcribed; portable CMake + whisper-env.ps1, models MIT); S-3 SigLIP2 runtime resolved (ort+ONNX) for M4.
Refs: main @af28901; crates/palmier-{mcp,agent,tools,tauri,prompt,gen}; [phase0-reconciliation](docs/phase0-reconciliation.md) (updated: S-3 + convex preserve_order); scripts/whisper-env.ps1 (new). Next: M3 E10 (palmier-transcribe — whisper wrapper + verbatim CaptionBuilder + add_captions/get_transcript bodies); M4 E11 (SigLIP2 search).
