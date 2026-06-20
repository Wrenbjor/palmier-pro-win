---
kind: doc
domain: [build-orchestration]
type: learning
status: adopted
links: [[build-orchestration]]
---

# Windows harness notes

This template + BMAD were authored POSIX-first. What had to change to run them on Windows 11.

## PYTHONUTF8 — mandatory (fixes party-mode)
**Symptom:** `_bmad/scripts/resolve_config.py` crashes with
`UnicodeEncodeError: 'charmap' codec can't encode character '\U0001f4ca'`. Party-mode calls this
resolver on activation to build the agent roster, so party-mode dies at step 3.
**Cause:** Python on Windows defaults stdout to cp1252; the agent `icon` fields are emoji (📊 🎨 …).
**Fix:** `PYTHONUTF8=1` (and `PYTHONIOENCODING=utf-8`) — set durably in `.claude/settings.json` `env`,
so every Python invocation in the session inherits it. Verified: resolver emits clean JSON with it set.

## python3 vs python
Both exist on this box (`python3` → 3.12, `python` → 3.13). BMAD skills call `python3`; that resolves
fine. No shim needed.

## tmux is not native
`bmad-story-automator` (the autonomous BMAD story build cycle) uses tmux for resumable orchestration.
tmux isn't native on Windows. For the autonomous inner loop, drive it with the **`/loop`** skill +
`ship-change.js`, or a `CronCreate` schedule, instead of story-automator. See [[build-orchestration]].

## Shells
- **Bash tool** = Git Bash (POSIX sh): use for the template's `.sh`/POSIX recipes, `find`/`grep` idioms.
- **PowerShell** = Windows-native: use for Windows build/packaging, path ops with backslashes.
- `LOG.md`'s retrieval recipes are written for macOS (`tail -r`); use `tac` or `Get-Content` on Windows.

## Timeline
2026-06-20 | setup — captured during environment prep; PYTHONUTF8 fix applied and verified.
