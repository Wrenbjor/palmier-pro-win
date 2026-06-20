---
kind: signal
category: observation
frequency: 1
sources: ["scaffold/workspace worker (commit b8e1a41)", "crates/palmier-update/build.rs"]
domain: [build-orchestration]
status: triaged
---

# Windows UAC installer-detection breaks cargo test for "update"/"setup"/"install" binaries

Windows' installer-detection heuristic auto-flags executables whose name contains `update`,
`setup`, `install`, `patch`, etc. as requiring elevation. A test binary like `palmier_update-*.exe`
therefore fails to launch under `cargo test` with **`os error 740` (The requested operation requires
elevation)** — even for a plain unit test.

**Fix applied (scaffold):** `crates/palmier-update/build.rs` embeds an `asInvoker` application
manifest on `windows-msvc` (`/MANIFEST:EMBED` + `/MANIFESTINPUT`), so the binary declares it does NOT
need elevation. `cargo test` then passes.

**How to apply:** any crate that produces an executable whose name trips the heuristic needs an
embedded `asInvoker` (or `requireAdministrator` if truly an installer) manifest. **The real
`palmier-tauri` bundle (E1-S1) must carry its own app manifest** — Tauri handles this via its bundler
config, but verify the requestedExecutionLevel is `asInvoker`. Watch for this on `palmier-update`,
any installer/updater helper, and the final MSI.

## Timeline
2026-06-20 | scaffold — discovered + fixed during workspace scaffolding; kept the build.rs manifest.
