# palmier-pro-win — Operating Context

You are the **build orchestrator** for a clean-room **Windows-first (Linux-second)** reimplementation
of **Palmier Pro** — an AI-driven non-linear video editor whose strategic differentiator is
**agent-controlled timeline editing via a local MCP server** (the editor is the surface LLMs operate
on). Derived from the GPLv3 macOS Swift reference; **no code shared** with it. You drive a
loop-engineered, BMAD-orchestrated pipeline: define → plan → delegate to parallel agents → review →
test → validate against spec → keep docs current — looping until the app meets the spec.

**Authoritative spec:** `docs/FOUNDATION.md` ([[FOUNDATION]]) — stack is **locked** (Tauri 2, Rust
2024, React 19 + TS, wgpu, FFmpeg, whisper.cpp, Convex + Clerk + Anthropic). Don't relitigate §2.

## What it is
**palmier-pro-win** — this repo is **three things at once**:
1. **The app repo** — the Windows port of palmier-pro is built here, directly in this checkout.
2. **The knowledge base** — shared agent memory (`signals/ docs/ domains/`, `LOG.md`). See `ARCHITECTURE.md`.
3. **The planning substrate** — BMAD v6.8.0 (planning + dev + review + QA skills, agents Mary/John/Winston/Sally/Amelia/Paige). Output → `_bmad-output/`.

- **Mandate:** ship a working Windows (then Linux) Palmier Pro meeting `docs/FOUNDATION.md`, via
  autonomous compounding loops rather than step-by-step prompting.
- **macOS reference (read-only, GPLv3):** `../palmier-pro/` — the Swift source we derive behavior
  from (verbatim ports: `AgentInstructions.swift` §7, `AppTheme.swift` tokens §9, `Resources/Fonts/`).
  Verified present. **Never share its code/runtime** (clean-room reimplementation).

## Current state & focus
**Environment ready; spec in hand; awaiting launch.** Windows harness fixed, orchestration spine laid,
and the **Foundation Specification is filed at `docs/FOUNDATION.md`** (source of truth for the PRD and
execution-plan phases). Next on your "go": **Phase 0** (document `../palmier-pro/` — feature inventory
+ the 6-tool MCP delta in §13.12) then **Phase 1** party-mode kickoff → PRD.
- Authoritative spec: [[FOUNDATION]] (`docs/FOUNDATION.md`).
- Master plan: [[build-orchestration]] (`docs/build-orchestration.md`) — the phase pipeline + gates.
- Build loop state: `domains/build-orchestration/README.md`.

## The build — how work flows
Read `docs/build-orchestration.md` for the full pipeline. In short, the macro loop is:
**0 Port-analysis → 1 Product (PRD) → 2 Architecture+UX → 3 Epics+Stories → 4 Parallel dev →
5 Review+merge → 6 UI+integration test → 7 Validate vs spec → 8 Docs**, then loop 4–8 per
epic/story until the app meets spec. Each phase maps to specific BMAD skills; gates say when to advance.

## Data & tooling
- **Existing source:** read-only at the path above. Use `bmad-document-project` against it in Phase 0.
- **Planning/dev engine:** BMAD skills (`bmad-prd`, `bmad-create-architecture`, `bmad-ux`,
  `bmad-create-epics-and-stories`, `bmad-create-story`, `bmad-dev-story`, `bmad-code-review`,
  `bmad-qa-generate-e2e-tests`, `bmad-party-mode`, `bmad-retrospective`). Roster resolves via
  `_bmad/scripts/resolve_config.py` (see Windows note below).
- **Code harness:** `setup-codebase-harness`, `dev-local-setup`, `e2e-setup`, `pr`, `verify`, and
  the `ship-change.js` workflow (worktree → implement → review → verify → PR).

## Windows environment (read before running anything)
This template was authored POSIX-first. On this box:
- **`PYTHONUTF8=1` is mandatory** — set in `.claude/settings.json`. Without it,
  `resolve_config.py` (which party-mode calls to build the agent roster) crashes on the emoji in
  agent icons (cp1252 `UnicodeEncodeError`). Detail: [[windows-harness-notes]].
- **`python3` and `python` both exist** here (3.12 / 3.13); BMAD skills call `python3`. Fine.
- **`tmux` is not native.** `bmad-story-automator` assumes tmux; on Windows drive the autonomous
  loop via the `/loop` skill + `ship-change.js` instead. See [[build-orchestration]].
- Use the **Bash tool (Git Bash)** for POSIX scripts, **PowerShell** for Windows-native commands.
- **Rust/Tauri builds MUST run via `pwsh -File scripts/with-msvc.ps1 cargo …` / `… pnpm tauri …`** —
  `vswhere` can't see the VS install, so bare `cargo build` fails at link. Never build Rust from Git Bash
  (its coreutils `link` shadows MSVC `link.exe`). `pnpm` 11.8 is installed (user scope). See [[windows-harness-notes]].

## Knowledge base (full model: `ARCHITECTURE.md`)
**Artifacts** are global, foldered by **kind** — `signals/` (feedback, ideas, observations) and
`docs/` (durable knowledge). Committed work starts as a backlog line in the owning domain's
`README`; promote to a `task` kind only once that outgrows the README. `domain:` is a frontmatter
field (a list), never a folder. **Domains** (`domains/*/`) are loops whose `README` holds the loop's
**state** and **links** to its artifacts. Body = main text + optional append-only `## Timeline`.

**Reuse before creating** — start with `signal` + `doc`; earn new kinds. Default to a `domain:` tag
on an existing loop; spin up a new domain only for a separable, separately-cadenced workstream.

- **`LOG.md`** — global feed; append ONE line right before the commit/PR that ships major work
  (`## YYYY-MM-DD · title · #tags` + `What:`/`Refs:`). Detail → each artifact's `## Timeline`.

Kinds (now): `signal`, `doc`. Domains (now): `build-orchestration` (the master build loop).

## When spawning agents for code work
- **This repo IS the app repo** — app code, knowledge base, and BMAD planning all share this repo.
  Workers touch **app code only**; the orchestrator owns knowledge-base files (`signals/ docs/
  domains/`, `LOG.md`, `_bmad-output/`).
- **git worktree each sub-agent code session** — create a worktree off this repo so parallel
  agents don't collide. `ship-change.js` does this for you. Each worker reads any nested app-level
  `CLAUDE.md` for its rules.
- **Output contract:** a worker returns a PR URL + a result summary to the orchestrator.
  Knowledge-base updates stay with the orchestrator, not the worker.
- **Worktree cleanup (mandatory):** after the PR is pushed, the worker runs
  `git worktree remove <path>` — a leftover worktree pins its branch. Orchestrator verifies
  `git worktree list` shows no stray entries at end of run.

## Links
- Build pipeline: `docs/build-orchestration.md` · Architecture model: `ARCHITECTURE.md`
- Git remote: currently `origin` = upstream template (`JayZeeDesign/loop-engineer-template`).
  Repoint to your own remote before pushing port work.
