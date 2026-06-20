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

**Wave 0 + Wave 1: COMPLETE** — all 5 workers merged green on main:
- **S-1 RESOLVED** — wgpu→WebView = native surface composited under a transparent webview (zero-copy,
  SM-2 met); wgpu 27.x pinned; WRY-integration sub-spike deferred to E5-S8 start. [[phase0-reconciliation]] #23.
- **E2-S1** palmier-model · **E1-S1** palmier-tauri (real Tauri 2.11 runtime, clean Windows build) ·
  **E3-S8** palmier-history · **E4-S2** palmier-media cache.

**Wave 2: COMPLETE** — all 5 merged green on main (37d8637): E2-S2/E2-S4/E3-S1 (model: center Transform #7,
VolumeScale #9, edit types), E1-S6 (palmier-auth), E1-S2 (palmier-telemetry), E5-S6 (palmier-engine audio
mixer), **S-1b** (Convex Date codec decided — [[phase0-reconciliation]] Date entry, unblocks E2-S8).

**Carry-forwards to honor in later stories:**
- **E2-S5** must implement Clip frame derivations with `f64::round` ties-away + the rounding-parity test (E3-S1 dep).
- **Telemetry boot seam:** boot stub installs its own tracing subscriber → file logging won't attach until the
  integration removes it and holds the `TelemetryHandle` from `palmier_telemetry::init`. (palmier-tauri touch.)
- **E5-S6** local `AudioClip`/`VolumeKeyframe` → convert to `From<&Clip>` adapter once E2-S5 lands.
- **palmier-auth** Convex HTTP path strings inferred; confirm against the live deployment (S-2 window).
- **E2-S8** implements `palmier-model/src/serde_date.rs` per `spikes/s1b-convex-date/FINDINGS.md`.

**Wave 2b: DISPATCHING** (disjoint crates): palmier-model **E2-S3+E2-S5+E2-S8** · palmier-edit
**E3-S2+E3-S3+E3-S4+E3-S5** (pure engines) · palmier-media **E4-S1+E5-S2** · palmier-tauri **E1-S3 + telemetry/auth boot wiring**.

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
