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

**Phase 1 — PRD: NEXT.** Drive BMAD (party-mode / `bmad-prd`) to produce `docs/PRD.md` from
FOUNDATION + the reconciliation rulings + `docs/reference/*`. Decide the remaining §13 open questions.
Gate: PRD validated; all open questions decided.

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
