---
kind: doc
domain: [build-orchestration]
type: sprint-plan
status: ready
links: [[PRD]]
---

# Palmier Pro — Sprint Plan

Derived from the 12 epic files (`epic-01`…`epic-12`), the machine-readable story
index, PRD §12 (milestones M1–M5), and PRD §11 (spikes S-1, S-1b, S-2, S-3, S-4).

**Scope:** 124 stories across 12 epics. This plan gives (1) the story dependency DAG,
(2) milestone assignment in dependency order, (3) the explicit spike gates, and
(4) the ordered parallel batches the Phase-4 orchestrator delegates to worktrees.

**How to read it:** a story is *ready* when every id in its `depends_on` is `done`.
Epic-level deps (`E2`, `Epic2`, `Epic-3`, `E2-model`, `E7-S*`, `E8-S*`, `palmier-auth`)
resolve to "all of that epic's / glob's stories landed" and are expanded below.
Spike ids (`S-1`, `S-1b`, `S-2`) are hard gates, not normal stories.

---

## 1. Spike Gates (hard blockers)

| Spike | Gate | Blocks | Pass bar (PRD §11) |
|-------|------|--------|--------------------|
| **S-1** | BLOCKER, run **first** in M1 | every Epic 5 *build* story (`E5-S8`, `E5-S9`, `E5-S10`, `E5-S11`, and transitively `E6` export) | per-platform FPS ≥ SM-2 floors (4K≥30 / 1080p60≥60) **or** explicit CPU-compositing fallback decision. Carried as story **E5-S1**. No Epic 5 architecture commit until it lands (ruling #23). |
| **S-1b** | M1-CRITICAL, before the Epic 2 **serde lock** | **E2-S8** (per-field Date codec) and therefore `E2-S9`/`E2-S10` and `E1-S8` | recorded `/v1/samples`+`/resolve` payload + round-trip unit test re-encoding it identically. Until it lands, Epic 2 serde is provisional with a named codec-swap fallback (R-6). Captured fixture allowed if Convex access is blocked. |
| **S-2** | before Epic 9 (M3) | `E9-S1` (and all of Epic 9 transitively), `E8-S6` (PalmierClient) | passing integration test: `/v1/models` over HTTP **and** a `generations:by_id` WebSocket round-trip against the deployment, with S-1b's Date format already documented. |
| **S-3** | before Epic 11 (M4), runnable in parallel from M1 | `E11-S1` (and the visual-embedding chain `E11-S2/S4/S5/S6`) | converted SigLIP2 weights reproduce reference embeddings within cosine tolerance (or documented re-index), artifact size recorded. Carried as story **E11-S1**. |
| **S-4** | before Epic 6 export (low-risk confirm) | `E6-S5` (video export pipeline) | readback+encode sustains ≥ SM-5 throughput on the §10 reference GPU. Carried as story **E6-S0**. |

> **S-1 and S-1b are the two gates the orchestrator must clear before any heavy
> M1 build fan-out.** S-1 unblocks the preview/export spine; S-1b unblocks the
> serde lock that the whole project-I/O write path depends on.

---

## 2. Story Dependency DAG

Notation: `A → B` means A blocks B (B depends on A). Spikes in **bold**. Epic-level
deps expanded to the concrete stories they imply.

### Epic 1 — App Shell
```
E1-S1 ──┬─ E1-S2 ─── E1-S9
        ├─ E1-S3 ─── E1-S10
        ├─ E1-S4 ─── E1-S9
        ├─ E1-S5
        ├─ E1-S6 ─┬─ E1-S8, E1-S9
        │         └─ (E8-S6 needs palmier-auth = E1-S6)
        ├─ E1-S7   (also needs Epic 2 complete)
        └─ E1-S10  (also needs E1-S3)
E1-S8  needs E1-S1, E1-S6, Epic2(write path: E2-S9), **S-1b**
```

### Epic 2 — Project I/O & Data Model
```
E2-S1 ─┬─ E2-S2 ─┬─ E2-S3 ─┐
       │         └─────────┤
       ├─ E2-S4 ───────────┤
       │                   ▼
       │                 E2-S5 ─── E2-S6 ─┐
       ├─ E2-S8 (gated by **S-1b**) ──────┤
       └─ E2-S7 (needs E2-S8) ────────────┤
                                          ▼
                                 E2-S9 ─┬─ E2-S10
                                        ├─ E2-S11 ─── E2-S12
                                        └─ E2-S12 (also needs E2-S7, E2-S11)
```

### Epic 3 — Timeline Editor
```
E3-S1 ─┬─ E3-S2 ─┐
       ├─ E3-S3 ─┤
       ├─ E3-S4 ─┼─ E3-S6 ─┐ (E3-S6 also needs E3-S8)
       ├─ E3-S5 ─┤         │
       │         ├─ E3-S7 ─┤ (E3-S7 needs E3-S1,S4,S5)
       │         └─ E3-S9 ─┤ (E3-S9 needs E3-S1,S5)
E3-S8 ─(indep)────┴────────┼─ E3-S10 (needs E3-S5,S6,S7,S8,S9)
```

### Epic 4 — Media Import & Panel
```
Epic2 ─── E4-S1 ─┬─ E4-S3 ─┐
E4-S2 ───────────┼─ E4-S4  │
                 ├─ E4-S5 ─┤
Epic2+Epic3 ── E4-S6 ─┬─ E4-S7   │
                      └─ E4-S12  │
E4-S8 ─┬─ E4-S9 ─┬─ E4-S10 (also needs E4-S3,S5)
       │         ├─ E4-S11
       │         ├─ E4-S12 (also needs E4-S6)
       │         └─ E4-S13
       └─ E4-S14
```

### Epic 5 — Preview (spike-gated)
```
**S-1**=E5-S1 ──┬───────────────┐
E2-model ── E5-S2 ─┬─ E5-S3      │
                   ├─ E5-S4 ─┬─ E5-S5 ─┐
                   │         │         │
E2-model ── E5-S6 ─┴─ E5-S7  │         │
                             ▼         ▼
                    E5-S8 (S-1 + E5-S4 + E5-S5) ─┬─ E5-S9 (E5-S5,S8)
                                                 ├─ E5-S10 (S-1,E5-S7,S8)
                                                 └─ E5-S11 (E5-S8,S9)
```

### Epic 6 — Export
```
**S-4**=E6-S0 ──┐
E2 ── E6-S1 ─── E6-S2 ─── E6-S3 ─── E6-S4
E2 ── E6-S7 (indep)
                E6-S0 + Epic5 ── E6-S5 ─┬─ E6-S6 (also Epic5)
                                        └─ E6-S8
```

### Epic 7 — MCP Server
```
E7-S1 ─── E7-S2 ─┬─ E7-S3
                 ├─ E7-S4 ─┐
                 ├─ E7-S12 ─┬─ E7-S6, E7-S7, E7-S8, E7-S10
                 ├─ E7-S5 (E7-S4)
                 ├─ E7-S9 (E7-S4)
                 └─ E7-S11 (E7-S13) ─── E7-S13
```

### Epic 8 — Agent Panel
```
E8-S1 ─┬─ E8-S2 ─┬─ E8-S3 ─┬─ E8-S4 (E8-S2,S3,S5, Epic7) ─┐
       │         │         └─ E8-S6 (palmier-auth, **S-2**) │
       ├─ E8-S5 ─┘                                          │
       └─ E8-S7 (Epic2 stories)                             │
                  E8-S8 (E8-S4,S6,S7) ── E8-S9 (E8-S3,S4,S5,S7,S8, Epic7)
```

### Epic 9 — Generation (gated by **S-2**)
```
**S-2** ── E9-S1 ─┬─ E9-S2 ─┬─ E9-S3 ─── E9-S9
                  │         ├─ E9-S4 ─── E9-S5 ─┬─ E9-S6 ─── E9-S7
                  │         ├─ E9-S8 (E9-S2,S3,S4)│
                  │         └─ E9-S10 ────────────┘ (E9-S7 needs S5,S6,S10)
                  └─ E9-S11 (E9-S7,S8,S1)
```

### Epic 10 — Transcription & Captions
```
E10-S1 ─┬─ E10-S2 (Epic4) ─┐
        ├─ E10-S3          │
        └─ E10-S4 ─────────┤
E10-S5 ───────────────────┤
                  E10-S6 (E10-S5,S2,S4 + Epic2,3,4) ── E10-S7 (E10-S2,S4 + Epic7)
                                                          └─ E10-S8 (E10-S6 + Epic3,7)
```

### Epic 11 — Search (gated by **S-3**)
```
**S-3**=E11-S1 ─┬─ E11-S2 ─┬─ E11-S4 ─┐
                ├─ E11-S5 ─┤          │
                └──────────┤          │
E11-S3 ────────────────────┘          │
E11-S7 ─┬─ E11-S8 ─┬─ E11-S9          │
        └──────────┤                  │
                   ▼                  ▼
        E11-S6 (E11-S1,S4,S5,S8) ── E11-S10 (E11-S5,S6,S8) ── E11-S11 (E11-S6,S8,S10)
E11-S12 (E11-S2,S5)
```

### Epic 12 — Polish & Release
```
E12-S1 (E2) ─┐
E12-S2 (E3,E1) ─┬─ E12-S3 ─┬─ E12-S5 (E12-S1,S2,S3)
                ├─ E12-S4 ─┴─ E12-S6 (E12-S2,S3,S4)
                ├─ E12-S7 (E9,E1)
                └─ E12-S8 (E12-S1,S2,S3,E3)
E12-S9 (E3,E5)
E12-S10 (E1,E4,E10,E11)
E12-S11 (E1,E8,E9,E7)
E12-S12 (E1)   E12-S13 (E1) ─── E12-S14 ─── E12-S15 (E5,E4,E10,E11,E12-S14)
                                                       └── E12-S16 (ALL epics + E12-S14,S15)
```

`E12-S16` is the M5 release gate and the single terminal node of the whole DAG.

---

## 3. Milestone Assignment (dependency-ordered)

### M1 — Hand-Edit MVP (Epics 1–6)
Spike **S-1 first**, **S-1b before the Epic 2 serde lock**. Foundation + usable
hand-edit editor + export. Stories in a valid topological order:

1. **E5-S1 (Spike S-1)** — run first, in parallel with foundation scaffolds.
2. **S-1b** (Convex Date encoding) — run alongside S-1; gates E2-S8.
3. E2-S1 · E3-S1 · E1-S1 · E4-S2 · E4-S8 · E3-S8 · E6-S0(S-4) · E11-S1(S-3, parallel) — root scaffolds.
4. E2-S2 · E2-S4 · E2-S8(after S-1b) · E1-S2 · E1-S3 · E1-S4 · E1-S5 · E1-S6 · E3-S2 · E3-S3 · E3-S4 · E3-S5 · E5-S2 · E5-S6.
5. E2-S3 · E2-S7 · E5-S3 · E5-S4 · E3-S9 · E3-S7 · E6-S1 · E6-S7 · E4-S1 · E1-S10.
6. E2-S5 · E5-S5 · E5-S7 · E3-S6 · E6-S2 · E4-S3 · E4-S4 · E4-S5 · E4-S9 · E1-S9.
7. E2-S6 · E5-S8 · E3-S10 · E6-S3 · E4-S6 · E4-S10 · E4-S11 · E4-S13 · E4-S14.
8. E2-S9 · E5-S9 · E5-S10 · E6-S4 · E4-S7 · E4-S12.
9. E2-S10 · E2-S11 · E5-S11 · E6-S5.
10. E2-S12 · E1-S7 · E1-S8 · E6-S6 · E6-S8.

**M1 exit:** §11.3 hand-edit e2e; SM-1/1b/2/3/5/7/C1; §11.5 golden assets; §11.4 composition bench incl. 1000-clip.

### M2 — MCP Server + Agent (Epics 7–8)
1. E7-S1 · E8-S1.
2. E7-S2 · E8-S2 · E8-S5 · E8-S7.
3. E7-S3 · E7-S4 · E7-S12 · E8-S3.
4. E7-S5 · E7-S9 · E7-S11 · E8-S6 (needs **S-2**? — no; PalmierClient model-availability uses S-2; see note).
5. E7-S6 · E7-S7 · E7-S8 · E7-S10 · E7-S13.
6. E8-S4 (needs Epic 7 tool bodies).
7. E8-S8.
8. E8-S9 (agent-cut e2e gate).

> **Note on E8-S6/S-2:** `E8-S6` lists `S-2` in `depends_on`. S-2 lands in M3. In
> M2, PalmierClient is built against the documented S-2 contract and exercised with
> a stub/recorded transport; live model-availability is end-to-end only at M3
> (ruling #24 — generation tools return "backend not available" until M3). M2
> acceptance is **not** held to a UJ-3 end-to-end bar.

**M2 exit:** §11.6 MCP compatibility suite; §11.2 dispatcher/MCP integration; §11.3 agent-cut e2e (transcription-gated cut deferred to M3).

### M3 — Generation + Transcription (Epics 9–10)
Spike **S-2 first** (S-1b already landed in M1).
1. **S-2** → E9-S1 · E10-S1 · E10-S5.
2. E9-S2 · E10-S2 · E10-S3 · E10-S4.
3. E9-S3 · E9-S4 · E9-S10 · E10-S6.
4. E9-S5 · E9-S8 · E9-S9 · E10-S7.
5. E9-S6 · E10-S8.
6. E9-S7.
7. E9-S11.

**M3 exit:** Convex-proxied generation (credit gating + toasts); Whisper transcription + caption generation; UJ-3 + transcription-gated agent cut e2e.

### M4 — Visual Search + Captions polish (Epic 11)
Spike **S-3** resolved (runnable in parallel from M1). `E11-S1`=S-3, `E11-S7/S8/S9`
have no model dep and can start as early as M1.
1. **E11-S1 (S-3)** · E11-S3 · E11-S7.
2. E11-S2 · E11-S8.
3. E11-S4 · E11-S5 · E11-S9 · E11-S12.
4. E11-S6.
5. E11-S10.
6. E11-S11.

**M4 exit:** §11.4 search-index query bench (1k/10k/100k, SM-12); visual+transcript search in panel; B-roll e2e.

### M5 — Export Polish + Release (Epic 12)
1. E12-S1 · E12-S2 · E12-S9 · E12-S10 · E12-S11 · E12-S12 · E12-S13.
2. E12-S3 · E12-S4 · E12-S7 · E12-S14.
3. E12-S5 · E12-S6 · E12-S8 · E12-S15.
4. **E12-S16** — M5 release gate (SM regression + §11.6 MCP suite + e2e). Terminal node.

---

## 4. Parallel Batches for Phase-4 Delegation

Each **wave** below is a set of stories with **no shared crate or `src-ui/*` package**,
so they can run concurrently in separate worktrees without file collisions. The
orchestrator delegates a whole wave, waits for all PRs to merge, then advances.
Crate ownership is listed per story so the orchestrator can confirm disjointness.

> **Wave gating note:** S-1 (E5-S1) and S-1b run as their own first wave because they
> are decision spikes, not parallel-safe build work — their outcome can re-scope
> downstream stories. Do not fan out M1 build until both report.

### M1

**Wave 0 — SPIKES (run first, serially-decided, low parallelism)**
- `E5-S1` (S-1: palmier-engine/palmier-tauri/src-ui/editor) — **BLOCKER**
- `S-1b` (Convex Date encoding investigation; feeds E2-S8)
- `E11-S1` (S-3 SigLIP2; palmier-search) — *may run in background through M1–M2*
- `E6-S0` (S-4 readback; palmier-media/palmier-export) — low-risk confirm

**Wave 1 — root scaffolds (the orchestrator's FIRST delegation batch)**
Disjoint crates, all `depends_on: []`:
- `E2-S1` — palmier-model
- `E3-S1` — palmier-model **CONFLICT with E2-S1 (same crate)** → sequence E3-S1 after E2-S1, OR merge into one worktree. *(see §5)*
- `E1-S1` — palmier-tauri
- `E4-S2` — palmier-media
- `E4-S8` — src-ui/media-panel
- `E3-S8` — palmier-history

> **First-wave to actually delegate (fully disjoint, zero shared files):**
> **`E2-S1`, `E1-S1`, `E4-S2`, `E4-S8`, `E3-S8`.**
> `E3-S1` touches palmier-model too — run it in the *same* worktree as E2-S1 (one
> agent owns palmier-model scaffolding) or as Wave 1b right after.

**Wave 1b**
- `E3-S1` (palmier-model — after E2-S1) · `E1-S5` is M1 but touches palmier-text+tauri (defer to Wave 2).

**Wave 2**
- `E2-S2` · `E2-S4` · `E2-S8`(needs S-1b) — palmier-model (serialize within one worktree; same crate)
- `E1-S2` — palmier-telemetry (+tauri touch — coordinate with E1-S* owner)
- `E1-S6` — palmier-auth/src-ui/settings
- `E3-S2` · `E3-S3` · `E3-S4` · `E3-S5` — palmier-edit (one worktree; same crate)
- `E5-S2` — palmier-media
- `E5-S6` — palmier-engine
- `E4-S1` — palmier-media/palmier-model (coordinate model writes)

**Wave 3**
- `E2-S3` · `E2-S5` · `E2-S6` · `E2-S7` — palmier-model (serialized)
- `E5-S3` · `E5-S4` — palmier-media/palmier-engine
- `E3-S9` — src-ui/editor
- `E6-S1` · `E6-S7` — palmier-export
- `E1-S3` · `E1-S4` · `E1-S9` · `E1-S10` — palmier-tauri/src-ui (serialize tauri+app touches)

**Wave 4**
- `E2-S9` → `E2-S10` · `E2-S11` · `E2-S12` — palmier-project (serialized)
- `E5-S5` · `E5-S7` · `E5-S8` — palmier-engine (serialized after S-1)
- `E3-S6` · `E3-S7` · `E3-S10` — palmier-edit/history/src-ui-editor
- `E6-S2` → `E6-S3` → `E6-S4` — palmier-export (serialized)
- `E4-S3` · `E4-S4` · `E4-S5` — palmier-media
- `E4-S6` · `E4-S7` — palmier-project/model/media

**Wave 5**
- `E5-S9` · `E5-S10` · `E5-S11` — palmier-engine/text/editor
- `E6-S5` → `E6-S6` · `E6-S8` — palmier-export/engine (serialized, needs Epic 5)
- `E4-S9`…`E4-S14` — src-ui/media-panel (serialized; same package)
- `E1-S7` · `E1-S8` — palmier-project (after Epic 2)

### M2

**Wave M2-1**
- `E7-S1` — palmier-tools
- `E8-S1` — palmier-agent
*(disjoint; this is M2's first delegation batch)*

**Wave M2-2**
- `E7-S2` → `E7-S3` · `E7-S4` · `E7-S12` — palmier-tools (serialized)
- `E8-S2` · `E8-S5` · `E8-S7` — palmier-agent (serialized; same crate)

**Wave M2-3**
- `E7-S5`–`E7-S11` — palmier-tools/mcp (serialize tools-crate writers; mcp transport separate)
- `E8-S3` · `E8-S6` — palmier-agent

**Wave M2-4**
- `E7-S13` — palmier-mcp/tauri
- `E8-S4` → `E8-S8` → `E8-S9` — palmier-agent (serialized; needs Epic 7 done)

### M3

**Wave M3-1 (after S-2)**: `E9-S1` (palmier-gen) ‖ `E10-S1` (palmier-transcribe) ‖ `E10-S5` (palmier-text).
**Wave M3-2**: `E9-S2`,`E9-S10` (gen/model) ‖ `E10-S2`,`E10-S3`,`E10-S4` (transcribe/media).
**Wave M3-3**: `E9-S3`,`E9-S4`,`E9-S8`,`E9-S9` (gen) ‖ `E10-S6` (edit/text/transcribe).
**Wave M3-4**: `E9-S5`,`E9-S6`,`E9-S7`,`E9-S11` (gen, serialized) ‖ `E10-S7`,`E10-S8` (tools/edit, serialized).

### M4

**Wave M4-1 (S-3 + no-model deps, startable from M1)**: `E11-S1`(S-3) ‖ `E11-S3` ‖ `E11-S7`.
**Wave M4-2**: `E11-S2` ‖ `E11-S8` ‖ `E11-S12`.
**Wave M4-3**: `E11-S4` ‖ `E11-S5` ‖ `E11-S9`.
**Wave M4-4**: `E11-S6` → `E11-S10` → `E11-S11`. (all palmier-search + final media-panel UI; serialized.)

### M5

**Wave M5-1**: `E12-S2`,`E12-S9` (src-ui/editor — serialize) ‖ `E12-S10`,`E12-S11` (src-ui/settings — serialize) ‖ `E12-S12` (telemetry) ‖ `E12-S13` (update) ‖ `E12-S1` (model).
**Wave M5-2**: `E12-S3`,`E12-S4`,`E12-S5`,`E12-S6`,`E12-S7`,`E12-S8` (src-ui/editor — serialize within editor package) ‖ `E12-S14` (tauri packaging).
**Wave M5-3**: `E12-S15` (memory profiling, cross-crate).
**Wave M5-4**: `E12-S16` (release gate — terminal).

---

## 5. Cross-Epic Constraints on Parallelism (read before fan-out)

1. **`palmier-model` is the contention hub.** E1-S5, E2-S1–S8, E3-S1, E3-S4, E4-S1,
   E4-S6, E5-S4, E6-S1–S3, E9-S10, E12-S1/S5/S6/S8 all write it. **E2-S1 and E3-S1
   both scaffold model with `depends_on: []`** — they collide on the same crate.
   Resolution: one worktree owns palmier-model scaffolding (do E2-S1 then E3-S1
   first), then model-touching stories serialize through that crate per wave.

2. **The Epic 2 serde lock is the spine of M1's write path.** `E2-S9` (bundle
   reader/writer) is the join node for `E2-S10/S11/S12`, `E1-S7/S8`, and everything
   that persists a project. It cannot start until `E2-S6`+`E2-S7`+`E2-S8` land, and
   `E2-S8` is **gated by S-1b**. → S-1b is on the critical path of M1, not a side quest.

3. **S-1 (E5-S1) gates the entire preview→export spine.** `E5-S8` (compositor
   present) blocks `E5-S9/S10/S11` and, via Epic 5 completion, `E6-S5/S6`. If S-1
   chooses the CPU fallback, SM-C1 gets its one sanctioned interpolation waiver and
   E5 stories re-scope — decided at M1, not deferred.

4. **Epic 7 ↔ Epic 8 ordering (M2).** `E8-S4` and `E8-S9` depend on `E7-S*` (all of
   Epic 7's tool bodies). Agent run-loop work cannot finish until the MCP tool
   dispatch exists. Run all of Epic 7 to "tools land" before E8-S4.

5. **`E7-S12` (agent undo stack) is an internal Epic-7 bottleneck.** `E7-S6/S7/S8/S10`
   (the mutation tools) all depend on it. Sequence it right after `E7-S2`.

6. **S-2 splits across M2/M3.** `E8-S6` references S-2 but ships in M2 against the
   documented contract (stubbed transport); S-2's live WebSocket proof lands M3 and
   gates Epic 9. Do not block M2 on S-2.

7. **`src-ui/media-panel` is one package** shared by E4-S8–S14, E9-S8, E10-S6,
   E11-S11. Stories touching it within a wave must serialize (one worktree at a time)
   even when their crate-level deps are satisfied.

8. **`palmier-search` chains are mostly serial.** Despite many `parallel_safe: true`
   flags, `E11-S4`, `E11-S6`, `E11-S10`, `E11-S11` form a serial spine; only the
   independent roots (`E11-S3`, `E11-S7`, and S-3 itself) genuinely parallelize.

9. **E12-S16 is the only true terminal node** — it depends on every epic plus
   E12-S14/S15. It is the release gate and cannot be parallelized with anything.
