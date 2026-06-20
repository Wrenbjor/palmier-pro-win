---
kind: doc
domain: [build-orchestration]
type: decision
status: adopted
links: []
---

# Build orchestration — palmier-pro Mac → Windows

The master plan: how we drive the port from idea to a working, spec-compliant Windows app using
**compounding loops** over **BMAD** skills, instead of prompting task-by-task. This doc is the spine
the `build-orchestration` loop reads to know what phase we're in and what advances it.

## The macro loop

```
0 Port-analysis ─▶ 1 Product (PRD) ─▶ 2 Architecture + UX ─▶ 3 Epics + Stories
                                                                      │
                        ┌─────────────────────────────────────────────┘
                        ▼
        ┌─▶ 4 Parallel dev ─▶ 5 Review + merge ─▶ 6 UI + integration test ─┐
        │                                                                   │
        └────────────  7 Validate vs spec  ◀── 8 Docs  ◀────────────────────┘
                                 │
                    meets spec? ─┴─ no ─▶ back to 4 (next epic/story / fixes)
                                 └─ yes ─▶ DONE
```

Phases 0–3 run once to establish the plan. Phases **4–8 are the inner convergence loop**, repeated
per epic/story (and for fix cycles) until Phase 7 confirms the app meets the PRD. Each run reads
`domains/build-orchestration/README.md` for state, advances the frontier, and logs.

## Phases, skills, gates

Every phase names the **BMAD skill(s)** that do the work and the **gate** that must be true before
advancing. Don't skip gates — that's the whole point of the harness.

> **Authoritative input:** `docs/FOUNDATION.md` ([[FOUNDATION]]) is the source of truth for Phases 0–3.
> The stack (§2) is locked — don't relitigate. The §13 open questions are the PRD's decision list.

### Phase 0 — Port-analysis (run once)
- **Goal:** understand what Palmier Pro *is* on Mac and what the clean-room Windows reimpl entails.
- **Inputs:** the macOS reference at `../palmier-pro/` (recorded in `CLAUDE.md`); `docs/FOUNDATION.md`.
- **Skills:** `bmad-document-project` (against `../palmier-pro/`) → `docs/`; `bmad-technical-research`
  for platform-gap questions (AppKit/AVFoundation/Apple Speech → wgpu/FFmpeg/Whisper equivalents).
- **Output:** `docs/` analysis — feature inventory, reference architecture map, Mac→Win/Linux porting
  risks, and **the MCP tool delta** (FOUNDATION §13.12: spec lists 30 tools, reference exposes 36 —
  enumerate the missing 6 from `Sources/PalmierPro/Agent/Tools/`).
- **Gate:** `docs/` states the feature set, top porting risks, and the full 36-tool MCP surface.

### Phase 1 — Product (PRD) (run once)
- **Goal:** turn FOUNDATION into a decided, validated PRD with acceptance criteria.
- **Skills:** **`bmad-party-mode`** kickoff roundtable (the launch point) → `bmad-product-brief`
  → `bmad-prd`. The roundtable decides every §13 open question (or escalates to Wren).
- **Output:** PRD draft in `_bmad-output/planning-artifacts/`; promote finalized to `docs/PRD.md` (§12).
- **Gate:** `bmad-prd` validate passes **and** `bmad-check-implementation-readiness` is green; every
  §13 open question has a recorded decision.

### Phase 2 — Architecture + UX (run once)
- **Skills:** `bmad-create-architecture` (Winston) for the Windows technical design (toolkit choice,
  project layout, build/packaging); `bmad-ux` (Sally) for screen/interaction specs where the port
  changes UX.
- **Output:** architecture + UX specs in `_bmad-output/planning-artifacts/`.
- **Gate:** architecture decisions cover every PRD capability; no open toolkit/packaging unknowns.

### Phase 3 — Epics + Stories (run once, then re-sliced as needed)
- **Skills:** `bmad-create-epics-and-stories` → `bmad-sprint-planning` → `bmad-create-story` per story
  (each story file carries full implementation context).
- **Output:** epics + story files in `_bmad-output/implementation-artifacts/`.
- **Gate:** every epic decomposed into stories with acceptance criteria; dependency order known so
  independent stories can run in parallel.

### Phase 4 — Parallel dev (inner loop)
- **Goal:** implement stories — independent ones concurrently.
- **Mechanism:** for each ready story with no file-overlap, spawn a dev agent in its **own worktree**
  via `ship-change.js` (worktree → implement → simplify → review → verify → PR). The per-story
  implementation follows `bmad-dev-story`.
- **Output:** one PR per story; worker returns PR URL + summary; worktree removed after push.
- **Gate:** story's acceptance criteria implemented; local verify passed before PR opens.

### Phase 5 — Review + merge (inner loop)
- **Skills:** `bmad-code-review` (adversarial parallel layers) + the `/code-review` skill; a
  **read-only verifier sub-agent** (never let the author self-verify). Auto-fix blocking findings,
  re-review, merge on green.
- **Gate:** no blocking findings; verifier sign-off; CI/build green.

### Phase 6 — UI + integration test (inner loop)
- **Skills:** `e2e-setup` (stand up the gate, once) → `bmad-qa-generate-e2e-tests` per feature;
  Playwright MCP for UI flows with video evidence; the `verify` / `pr` skill's verifier drives the
  real app.
- **Gate:** critical-flow e2e tests pass on Windows; UI evidence attached.

### Phase 7 — Validate vs spec (inner loop, convergence check)
- **Skills:** `bmad-check-implementation-readiness` against the PRD; `bmad-retrospective` per epic.
- **Decision:** does the built app satisfy the PRD acceptance criteria?
  - **No** → file gaps as `signals/` + backlog items, return to Phase 4.
  - **Yes** → done (for this epic / overall).
- **Gate:** PRD acceptance criteria demonstrably met by passing tests + verifier evidence.

### Phase 8 — Docs (inner loop)
- **Skills:** `bmad-document-project`, `bmad-index-docs`, tech-writer (Paige) — keep `docs/` and any
  app-level docs current with what shipped.
- **Gate:** shipped changes reflected in docs; `LOG.md` entry appended.

## Triggers (how the loop wakes)
- **Now:** **manual** — Wren kicks off each phase; the kickoff task launches Phase 1 party-mode.
- **Later (autonomous):** drive the inner loop (4–8) on a cadence. **Note:** `bmad-story-automator`
  is built for exactly this but assumes **tmux**, which isn't native on Windows. On this box, prefer
  the **`/loop`** skill calling `ship-change.js` per ready story, or a `CronCreate` schedule. Earn
  the automation once the manual pipeline runs clean end-to-end.

## Why one domain, not eight
`ARCHITECTURE.md` says earn structure. Review / testing / validation / docs are **phases of the build
loop**, not separately-cadenced workstreams — they share the build loop's trigger and state. So there
is **one** domain, `build-orchestration`. Spin up a second domain only if a workstream gets its own
independent cadence (e.g. a standing "Windows-platform research" loop that runs regardless of the build).

## Where things land
- Planning artifacts → `_bmad-output/planning-artifacts/` (PRD, architecture, UX).
- Implementation artifacts (epics, stories) → `_bmad-output/implementation-artifacts/`.
- Durable analysis / decisions / learnings → `docs/`.
- Feedback / frictions / ideas surfaced mid-build → `signals/` (feed back into the backlog).
- App code → this repo (per-story PRs via worktrees).
