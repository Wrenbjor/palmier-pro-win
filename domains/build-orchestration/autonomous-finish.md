---
kind: doc
domain: build-orchestration
status: active
mode: autonomous
goal: Finish the human-usable + agent-controlled NLE to spec, self-verifying, surfacing to Wren only at review-readiness or a hard external blocker.
---

# Autonomous finish loop

Operating mode (set 2026-06-21 by Wren's directive): stop per-stage human validation.
Decompose → delegate to parallel worktree agents → self-verify (no human) → merge → loop
until the product meets spec. Surface to Wren ONLY at a reviewable milestone, on a hard
external blocker, or when done. Use Playwright + cargo + the MCP server as the gates.

## The gap the story-count hid
M1–M4 "complete" delivered tested **components** + the **agent/MCP** editing path (proven:
MCP drove import_media→add_clips→get_timeline live on the running app). It did **not** deliver
the **human UI ↔ backend bridge** — the editor panels' controllers are stubbed (`TODO(E7/E9)`),
and no Tauri commands exist for the UI to read/edit the timeline. That bridge is **net-new
scope** beyond the 138 planned stories, and it is the bulk of "finish the app".

## Self-verification gates (no human in the loop)
1. **cargo** — `pwsh -File scripts/with-msvc.ps1 cargo build -p <crate> --no-default-features`
   + `cargo test` (touched crates). Deterministic. MANDATORY before any merge.
2. **tsc** — `cmd /c "cd src-ui && node_modules\.bin\tsc --noEmit"`. Deterministic. MANDATORY for UI.
3. **MCP oracle** — launch app, drive the 30 MCP tools on 127.0.0.1:19789, assert editor state
   (the backend truth). Covers agentic/backend e2e with zero UI.
4. **UI render** — Playwright (vs vite :5173 with seeded/mocked invoke) asserts each surface
   renders; (later) tauri-driver / tauri-plugin-playwright drives the REAL WebView2 for full e2e.

## Backlog (dependency-ordered)
- **EDIT-BRIDGE** (critical path): Tauri `editor_get_media` + `editor_edit(name,args)` over the
  shared `Arc<ToolExecutor>`; emit `timeline://changed` on every mutation (UI + agent paths).
- **EDITOR-COMPOSE**: NLE layout in Project.tsx (media L / preview T / inspector R / timeline B /
  agent far-R); stores+controllers loaded from the commands; event-driven refetch (drop the poll);
  selection/playhead sync; drag media→timeline = add_clips.
- **FULL-SERIALIZER**: EditorState→full TimelineView (real volume/trim/keyframes), replacing the
  default-filled adapter.
- **E2E-HARNESS**: automated UI verification (tauri-driver/WebdriverIO or tauri-plugin-playwright)
  + `scripts/mcp-smoke.ps1` backend oracle; wire into `scripts/test.ps1`.
- **M5 deferred** (were the interrupted worktrees): E12-S1 VolumeScale (palmier-model),
  E12-S10/S11 Settings UI, E12-S12 telemetry, E12-S13 updater, E12-S3..S9 Inspector tabs,
  E12-S14 packaging, E12-S15 memory, E12-S16 release gate.

## Hard external blockers (will need Wren — named up front)
- Ed25519 updater signing keypair (E12-S13/S14 real release).
- Convex deployment URL + Clerk key (account/generation backend, S-2).
- Anthropic API key OR a LiteLLM Anthropic↔OpenAI bridge to the local nemotron endpoint
  (http://10.20.1.141:8000/v1) for the **in-app** agent panel. NOTE: the MCP path needs none of this.
- Code-signing cert + branding/ProRes-alpha decisions for a shippable installer.

## Verification division of labor (learned)
The app uses wgpu + WebView2, so it only launches with a real GPU/display. **Headless
sub-agents cannot run the live app** — they verify via cargo/tsc/mock-Playwright only.
**Live-app gates (mcp-smoke, real-webview render) run in the orchestrator's desktop
session**, after each merge. Need a `--open-project <id>` boot affordance so the
orchestrator can open a project window non-interactively for screenshot verification.

## Timeline
- 2026-06-21 — Wave-A merged to `main` (`3b2c46e`): editor UI↔backend integration
  (editor_get_timeline/get_media/edit + timeline://changed + NLE composition). Gates green:
  cargo build, 54/54 tauri tests, tsc, mcp-smoke. Live WebView2 render NOT yet confirmed
  (needs the open-project affordance). Harness merged earlier (`40c47ef`).
- 2026-06-21 — Autonomous mode engaged. Baseline `50fb409` (manifest + window-threading runtime
  fixes, editor read-bridge slice 1, harness). Launched wave-A: EDIT-BRIDGE+EDITOR-COMPOSE (one
  agent, coherent vertical) ∥ E2E-HARNESS (one agent). Self-paced continuation scheduled.
- 2026-06-21 — Wren chose D (keep grinding). Wave running: E12-S9 toolbar ∥ configurable
  agent base URL. Orchestrator stood up the **LiteLLM nemotron bridge** (scripts/litellm-nemotron.yaml,
  proxy on :4000) — PROVEN: app's Anthropic /v1/messages → nemotron-super-49b → Anthropic-format 200.
  Once the base-URL override lands, PALMIER_ANTHROPIC_BASE_URL=http://127.0.0.1:4000/v1/messages makes
  the in-app agent use the local model (clears the "Anthropic key OR bridge" blocker). Bridge tool-use
  (Anthropic tool_use ↔ OpenAI function-calling through LiteLLM) still to verify for full agentic chat.
- 2026-06-21 — Merged: E12-S9 toolbar (849cb4a), configurable agent base URL (2c73c95,
  PALMIER_ANTHROPIC_BASE_URL→with_base_url), ui-smoke mock fix (unregisterListener). Bridge live on
  :4000. In-app agent→nemotron is wired+component-verified (bridge proven E2E; override unit-tested);
  full live panel send needs the real-webview harness (UI driving). Next: complete clip-property
  editing backend (rotation/fades in set_clip_properties); then the tauri-driver real-webview harness
  to verify human-click editing + the agent panel end-to-end. Remaining gated: signing key, Convex/Clerk,
  packaging .sig, deeper polish.
