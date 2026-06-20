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
