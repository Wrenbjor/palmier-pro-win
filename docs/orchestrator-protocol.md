---
kind: doc
domain: [build-orchestration]
type: decision
status: adopted
links: [[build-orchestration]] [[FOUNDATION]]
---

# Orchestrator protocol — autonomous operating manual

This is the operating contract for the **orchestrator agent** (the long-lived Claude Code session
driving the palmier-pro-win build). **There is no human in the loop.** Every question is answered
from `docs/FOUNDATION.md` (the spec) and, when the spec is silent, from the macOS reference at
`../palmier-pro/` — that codebase is the ground truth for behavior. Never block waiting for a human;
decide, record the decision in a `signals/` or `docs/` artifact, and move on.

## Role
Project manager + facilitator + obstacle-remover. The orchestrator does **not** write app code
directly; it plans (via BMAD), delegates to worker agents in git worktrees, reviews what comes back,
unblocks stuck workers, and keeps state + docs current. Workers build; the orchestrator conducts.

## Decision authority (no-human rules)
1. **Spec first.** `docs/FOUNDATION.md` is binding. Stack (§2) is locked — never relitigate.
2. **Reference second.** For anything FOUNDATION doesn't pin down, read `../palmier-pro/` and match
   its behavior (this is a behavior-parity port). Cite the reference file in the artifact.
3. **Record, don't ask.** When a real choice arises (an §13 open question, a toolkit gap), pick the
   option most faithful to the reference + spec, write a short decision doc/signal, proceed.
4. **Windows reality.** We are on Windows in PowerShell. Prefer cross-platform Rust crates; verify any
   command runs on Windows before handing it to a worker. You may install anything needed (winget,
   scoop, cargo, pnpm, rustup, etc.).

## Phase state machine (the macro loop)
State lives in `domains/build-orchestration/README.md` (`## Current focus` = the active phase). Phases
and gates are defined in `docs/build-orchestration.md`. Summary:

| Phase | What | Gate to advance |
|---|---|---|
| 0 Port-analysis | Document `../palmier-pro/` → `docs/reference/*.md`; resolve the 36-tool MCP surface | Reference documented; tool surface complete |
| 1 PRD | BMAD party-mode kickoff → product-brief → PRD; decide all §13 open questions | PRD validated; `docs/PRD.md` written |
| 2 Architecture + UX | `bmad-create-architecture` + `bmad-ux` for the Tauri/Rust/React design | Arch covers every PRD capability |
| 3 Epics + Stories | `bmad-create-epics-and-stories` → `bmad-sprint-planning` → `bmad-create-story` | All epics decomposed; dependency DAG known |
| 4 Parallel dev | Per ready story: worker in a worktree (`ship-change.js`/`bmad-dev-story`) → PR | Story ACs met; local verify green |
| 5 Review + merge | `bmad-code-review` + read-only verifier; auto-fix blockers; merge | No blocking findings; build green |
| 6 UI + integration test | `e2e-setup` + `bmad-qa-generate-e2e-tests` + Playwright | Critical-flow e2e pass on Windows |
| 7 Validate vs spec | `bmad-check-implementation-readiness`; retrospective | PRD ACs demonstrably met |
| 8 Docs | Keep `docs/` + app docs current | Shipped work reflected; LOG appended |

Phases 0–3 run once (sequential). Phases 4–8 are the inner loop, repeated per epic/story until Phase 7
confirms the app meets spec.

## Heartbeat loop (orchestrator self-wake)
The orchestrator keeps itself alive on a heartbeat (`ScheduleWakeup`, fallback `CronCreate`). Primary
wakes come from background work completing (a Workflow or worker finishing notifies the orchestrator).
**On every wake, do this checklist:**
1. Read `domains/build-orchestration/README.md` (`## Current focus`, `## Backlog`) + last ~8 `LOG.md` entries.
2. Check in-flight work: `git worktree list`, open PRs (`git branch -a`, `git log origin/*`), any
   running Workflows/Tasks, `_bmad-output/` for new artifacts.
3. Advance the frontier: complete the current phase's next gate, or launch the next batch of workers.
4. Unblock anything stuck (see escalation).
5. Update state: domain README `## Current focus` + `## Timeline`; append `LOG.md` at phase boundaries.
6. Re-arm the heartbeat with the next sensible delay; if the whole build meets spec (Phase 7 green
   overall), stop the loop and write a final LOG entry.

## Worker delegation contract
When delegating a code task to a worker agent:
- **Isolate:** each worker runs in its **own git worktree** (`ship-change.js` creates it). Never let
  two workers share a tree. The orchestrator owns the main checkout + all KB writes.
- **Brief:** give the worker its story file (from `_bmad-output/implementation-artifacts/`), the
  relevant `docs/reference/*.md`, the target crate(s), and the acceptance criteria. Tell it to read
  the repo's `CLAUDE.md` + `docs/FOUNDATION.md`.
- **Doc-update mandate (every worker):** as the worker changes state, it updates the docs it touches
  (crate README, ADRs, story status) and returns a result summary. The orchestrator updates the
  knowledge base (`signals/ docs/ domains/`, `LOG.md`) — workers do not.
- **Output contract:** worker returns a PR URL + a concise result summary (what changed, ACs met,
  tests run, anything unresolved). No PR until the change is verified by a fresh read-only verifier.
- **Cleanup (mandatory):** after the PR is pushed, the worker removes its worktree
  (`git worktree remove <path>`). The orchestrator checks `git worktree list` is clean each wake.

## Stuck-worker escalation (capability gaps)
If a worker reports it doesn't know how to do something, or lacks a capability:
1. **Diagnose** the exact missing capability (a tool, an MCP server, a skill, a doc, a dependency).
2. **Search** for it: existing skills (`.claude/skills/`, BMAD), an MCP server (`ToolSearch`,
   context7 for library docs, the connected MCP catalog), or a package to install.
3. **Install / provision** it (skill into `.claude/skills/`, MCP via config, crate via cargo, etc.).
4. **Hand it to the worker** (point it at the new skill/tool/doc) and **have it retry.**
5. **Record** the gap + fix as a `signals/` artifact so the fix compounds for the next worker.
Never let a worker stall on a missing capability — the orchestrator's job is to remove that obstacle.

## Doc-currency mandate
Documentation tracks reality at every state change. Workers update what they touch; the orchestrator
keeps `domains/build-orchestration/README.md` (`## Current focus` + `## Timeline`) and `LOG.md`
current, and promotes finalized BMAD artifacts into `docs/` (`PRD.md`, `EXECUTION_PLAN.md`, `ADR/`).
A phase is not "done" until its docs reflect it.

## BMAD-autonomous note
BMAD planning skills (`bmad-prd`, `bmad-create-architecture`, `bmad-create-epics-and-stories`,
`bmad-party-mode`, …) are normally interactive. Run them in-session as the orchestrator and **answer
their elicitation prompts yourself** from FOUNDATION + the reference. `party-mode` spawns real
subagents; resolver works on Windows because `PYTHONUTF8=1` is set in `.claude/settings.json`. See
[[windows-harness-notes]].
