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

**IN FLIGHT (3 workers):**
- **E5-S8** (a98b18d9) — the wgpu compositor present (the M1 keystone; build on proven A1). **M1's sole remaining
  critical-path story** — E5-S9/S10/S11 + E6-S5 all depend on or conflict with it, so they wait.
- **M2 foundation, started in parallel** (disjoint crates, independent of the preview, safe regardless of E5-S8):
  **E7-S1** (af837a44 — palmier-tools: MCP 30-tool registry + dispatch + ShortId) · **E8-S1** (a8c6511f —
  palmier-agent: message/session model + client trait + StreamEvent).

**Remaining for M1:** E5-S8 → E5-S9/S10/S11 (overlays/aspect-quality/perf gate SM-2) → E6-S5 (video export, HW
encoders) → hand-edit e2e gate → **M1 EXIT**. Then M2 proceeds (E7 tool bodies + E8 streaming loop).

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
