---
kind: signal
category: observation
frequency: 1
sources: ["docs/FOUNDATION.md §1, §2.3, §7, §13.11", "docs/reference/agent-instructions.md", "../palmier-pro LICENSE (GPLv3)"]
domain: [build-orchestration]
status: open
---

# GPLv3 clean-room vs verbatim-port contradiction

FOUNDATION's charter says **clean-room, no code/assets shared** with the GPLv3 macOS reference
(§1.3 out-of-scope, §2.3). But the same spec also requires porting the agent system prompt
**verbatim** (§7), translating design tokens **from** `AppTheme.swift` (§9), and **bundling the same
font files** (§6.6). Copying copyrightable text/assets from a GPLv3 work is **not** clean-room — the
result is a derivative work and inherits GPLv3. This is a genuine legal contradiction, not a detail.

**Why it matters:** affects distribution licensing of the whole product, and whether "Palmier Pro
Windows" can be closed-source or differently licensed. Touches §13.10 (naming/branding) and §13.11
(license compatibility), which flag licensing generically but don't name this contradiction.

**How to apply / proposed resolution (for PRD to ratify):**
- Treating this as a behavior-parity port that *reuses* GPLv3 prompt text + fonts ⇒ **accept GPLv3**
  for the port and comply (publish source, GPLv3 LICENSE). Simplest, lowest-risk; matches "port".
- OR re-derive the agent prompt from scratch and source independently-licensed fonts (e.g. Anton is
  OFL, not GPL) to keep clean-room ⇒ enables non-GPL licensing but is more work and risks behavior drift.
- Fonts: audit each file in `../palmier-pro/Sources/PalmierPro/Resources/Fonts/` for its own license
  (many are OFL/Apache and freely bundlable regardless of the app license).

Recorded by the orchestrator during Phase 0. Surfaced to Wren in `docs/phase0-reconciliation.md`
(open item #2). Proceeding under the **accept-GPLv3** assumption unless Wren rules otherwise.

## Timeline
2026-06-20 | Phase 0 — identified during reference documentation; logged for PRD to ratify.
