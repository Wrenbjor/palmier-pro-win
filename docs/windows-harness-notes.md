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

## FFmpeg toolchain (ffmpeg-next) — RESOLVED
`ffmpeg-next` 7.1 builds on this box. Setup (see `spikes/ffmpeg-setup/FINDINGS.md`):
- **FFmpeg 7.1 LGPL shared** (BtbN `win64-lgpl-shared`) extracted to `C:\ffmpeg` (headers + import `.lib`s + 7
  runtime DLLs avcodec-61/avformat-61/avutil-59/avfilter-10/avdevice-61/swresample-5/swscale-8).
- **libclang 18.1.1** (PyPI wheel; the `LLVM.LLVM` winget installer needs admin and fails non-interactively)
  at `%LOCALAPPDATA%\Programs\libclang\bin`.
- Env lives in **`scripts/ffmpeg-env.ps1`** (sets `FFMPEG_DIR`, `LIBCLANG_PATH`, PATH), **auto-sourced by
  `scripts/with-msvc.ps1`** — so any worker building Rust through the wrapper inherits it, no per-worker action.
  Verified: the probe runs from a cleared env.
- **⚠ Encode licensing:** the LGPL build EXCLUDES GPL encoders (libx264/libx265/xvid). **Software H.264/H.265
  encode is unavailable** — the video-export story (E6-S5) must use HW encoders (NVENC/QSV/AMF/MediaFoundation).
  **ProRes (`prores_ks`) and all decode are LGPL-fine.** Aligns with the ProRes-422-for-v1 ruling (#17).
- **Packaging:** the app bundle must ship the 7 FFmpeg DLLs next to the `.exe` (Tauri `externalBin`/resources)
  + FFmpeg LGPL attribution in third-party notices. On a fresh machine, re-run the two installs (FINDINGS).

## tmux is not native
`bmad-story-automator` (the autonomous BMAD story build cycle) uses tmux for resumable orchestration.
tmux isn't native on Windows. For the autonomous inner loop, drive it with the **`/loop`** skill +
`ship-change.js`, or a `CronCreate` schedule, instead of story-automator. See [[build-orchestration]].

## Shells
- **Bash tool** = Git Bash (POSIX sh): use for the template's `.sh`/POSIX recipes, `find`/`grep` idioms.
- **PowerShell** = Windows-native: use for Windows build/packaging, path ops with backslashes.
- `LOG.md`'s retrieval recipes are written for macOS (`tail -r`); use `tac` or `Get-Content` on Windows.

## Build toolchain status (for Phase 4 dev)
Checked 2026-06-20 on this box:
- **Rust 1.94.1** + cargo + rustup — present (supports the 2024 edition FOUNDATION §2 wants). ✅
- **Node v22.14.0** (nvm-managed: `C:\Users\Wren\AppData\Local\nvm`) + npm 11.2.0. ✅
- **pnpm 11.8.0** — installed via the standalone Windows installer to `PNPM_HOME=%LOCALAPPDATA%\pnpm`
  (corepack failed with EPERM into the nvm dir; standalone installer worked). On PATH for **new** shells. ✅
- **winget** present; **scoop / gh NOT installed.**
- **Tauri CLI** — not installed globally; add per-project as a dev dependency (`@tauri-apps/cli` via pnpm)
  in the scaffold story rather than globally.
- **No workspace scaffold yet** — `Cargo.toml`/`crates/`/`src-ui/`/`tauri.conf.json` do not exist; the
  first M1 story scaffolds the 17-crate Cargo workspace + Vite/React `src-ui` + Tauri.
- **MSVC build — RESOLVED, but builds MUST use the wrapper.** VS 2022 Community + MSVC 14.29 + Windows SDK
  10.0.22621 are present on disk, but `vswhere` does **not** register the install, so rustc/cc can't
  auto-detect MSVC and `cargo build` fails at link ("ensure the Visual C++ option was installed") from a
  plain shell. **Fix:** all Rust/Tauri builds run inside the `vcvars64.bat` env via
  **`pwsh -File scripts/with-msvc.ps1 <cargo|pnpm tauri ...>`** (verified: a trivial crate links cleanly
  through it). Do NOT run bare `cargo build`/`cargo test` for this repo — they'll fail at link.
  (Don't run Rust builds from Git Bash either: its coreutils `link` shadows MSVC `link.exe`.)
  **Gotcha:** PowerShell eats `-p` as an ambiguous param to the wrapper — use the long form
  `--package <crate>` (not `cargo build -p <crate>`) when building a single crate through it.
- Install **gh** (`winget install GitHub.cli`) when the PR/merge phase needs it (not yet installed).
- **Frontend verify after merging a `src-ui` story:** run `corepack pnpm install` in `src-ui/` FIRST
  (worktrees carry their own `node_modules`; the main checkout's can be stale → pnpm's deps-status check
  fails before build). Then `corepack pnpm build`. `pnpm` isn't on PATH for shims here — use `corepack pnpm …`.

## Timeline
2026-06-20 | setup — captured during environment prep; PYTHONUTF8 fix applied and verified.
2026-06-20 | toolchain — verified Rust/Node present; installed pnpm 11.8.0; noted Tauri-CLI/gh/MSVC-linker as Phase 4 pre-flight.
