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
