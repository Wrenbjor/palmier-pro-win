---
kind: doc
domain: [build-orchestration]
type: learning
status: adopted
links: [[FOUNDATION]], [[windows-harness-notes]]
---

# whisper.cpp toolchain - Windows setup findings

Provisioning the whisper-rs toolchain so M3 Epic 10 (transcription, palmier-transcribe) can build
on this box. Same proactive-infra pattern as the FFmpeg toolchain (spikes/ffmpeg-setup/FINDINGS.md).
Probed + verified 2026-06-20.

## TL;DR

whisper-rs 0.16.0 builds, links, AND transcribes end-to-end on this box. A throwaway probe
(spikes/whisper-probe/) compiles whisper.cpp + ggml from source through the existing MSVC wrapper,
loads ggml-small.en, and transcribes the JFK sample correctly:

```
--- transcription (2 segments) ---
[0.00 -> 8.00]  And so, my fellow Americans, ask not what your country can do for you.
[8.00 -> 11.00]  Ask what you can do for your country.
```

Backend: CPU (default features = none). This box is AMD (no CUDA); Vulkan is opt-in.

## Build requirements (what whisper-rs needs)

whisper-rs 0.16.0 -> whisper-rs-sys 0.15.0 builds whisper.cpp FROM SOURCE in its build script.
It needs, at build time:

| Need | Provided by | Status |
|---|---|---|
| C/C++ compiler (MSVC cl.exe) | scripts/with-msvc.ps1 (vcvars64) | already in place (FFmpeg precedent) |
| CMake (configures + builds whisper.cpp/ggml) | NEW - installed by this spike | CMake 4.3.3, portable |
| libclang (bindgen -> Rust FFI from headers) | scripts/ffmpeg-env.ps1 sets LIBCLANG_PATH | reused, no new work |
| A build tool for CMake (Ninja) | ships inside VS 2022 CMake component; vcvars64 puts it on PATH | no install needed |

Locked crate versions (probe Cargo.lock): whisper-rs 0.16.0, whisper-rs-sys 0.15.0,
bindgen 0.72.1, cmake 0.1.58.

## What was installed / changed

1. CMake 4.3.3 - portable (no admin, no MSI). winget Kitware.CMake MSI install WEDGED: the
   detached install process hung, leaving a SYSTEM-owned msiexec.exe holding the global
   _MSIExecute mutex (exit 1618 "another installation in progress" on every retry). That instance
   runs as SYSTEM and cannot be killed without elevation/reboot - both out of scope for a
   non-interactive worker. Pivoted (same reasoning as the libclang PyPI-wheel choice): downloaded
   the official portable ZIP cmake-4.3.3-windows-x86_64.zip -> extracted to
   %LOCALAPPDATA%\Programs\cmake (per-user, no elevation). cmake --version -> 4.3.3.
   - Fresh-machine note: if winget MSI works on another box, winget install Kitware.CMake is fine;
     otherwise re-run the portable-ZIP step.

2. scripts/whisper-env.ps1 - NEW. The wrapper already sources ffmpeg-env.ps1 (LIBCLANG_PATH), but
   that does NOT provide CMake. whisper-env.ps1:
   - prepends %LOCALAPPDATA%\Programs\cmake\bin (the portable CMake) to PATH, and
   - sets CMAKE_GENERATOR=Ninja and clears CMAKE_GENERATOR_PLATFORM / CMAKE_GENERATOR_TOOLSET.
   Why the generator override is mandatory (two distinct gotchas, both solved here):
   - (a) VS-generator discovery fails the same way bare cargo does. The cmake Rust crate defaults
     to the "Visual Studio 17 2022" generator, which runs its OWN vswhere discovery and dies with
     "could not find any instance of Visual Studio" - the exact blind-spot that forces
     with-msvc.ps1 to exist (see [[windows-harness-notes]]). Ninja builds with whatever cl.exe is
     on PATH (from vcvars64) and skips VS discovery entirely.
   - (b) Ninja rejects a platform spec. With Ninja, CMake errors "Ninja does not support platform
     specification, but platform x64 was specified" if CMAKE_GENERATOR_PLATFORM is defined - so
     the script clears it from the environment (an empty value still trips it; it must be fully
     unset, which the script does via the Env: drive).

3. scripts/with-msvc.ps1 - one line added to dot-source whisper-env.ps1 right after ffmpeg-env.ps1,
   so EVERY Rust build through the wrapper finds CMake automatically - no per-worker action,
   exactly like the FFmpeg env. Absent file = no-op (idempotent, optional).

## Gotchas hit (and how they were solved)

- Stale CMakeCache.txt poisoned later runs. The first (VS-generator) configure wrote
  CMAKE_GENERATOR_PLATFORM:INTERNAL=x64 into the build dir CMakeCache.txt. Every subsequent Ninja
  configure re-read that cache and kept failing with the platform error even after the env was
  fixed. Fix: wipe spikes/whisper-probe/target once so CMake reconfigures clean. (For E10 real
  crate: if you ever switch generators, cargo clean -p palmier-transcribe first.)
- whisper-rs-sys forwards every CMAKE_* env var as a -D cache define. Its build.rs loops
  env::vars() and re-injects anything starting with WHISPER_/GGML_/CMAKE_. Harmless here
  (CMAKE_BIN, CMAKE_GENERATOR pass through fine) but worth knowing before setting CMAKE_* env vars.
- PowerShell eats cargo --quiet / -p as wrapper params (same as the documented -p gotcha). For a
  single-crate build use --package <crate>; to run the built exe just invoke it directly through
  the wrapper (pwsh -File scripts/with-msvc.ps1 <path-to-exe> <args>).

## Backend decision

- Baseline: CPU (whisper-rs default features = none). Verified working; no GPU SDK needed. This
  box is AMD, so CUDA is N/A (FOUNDATION 6.9 lists CUDA for NVIDIA Windows boxes only).
- Optional: Vulkan (whisper-rs/vulkan feature) for GPU accel on AMD/Windows + Linux. The probe
  exposes it behind a vulkan cargo feature (off by default) but it was NOT built here (needs the
  Vulkan SDK present; CPU is the parity-safe baseline). FOUNDATION 6.9: "CUDA + Vulkan on Windows,
  Vulkan on Linux; CPU fallback." -> E10 ship plan: CPU always; Vulkan as an opt-in acceleration
  feature; CUDA only on NVIDIA builds.
- DirectML (2.2 mentions it) is NOT a whisper-rs backend - ignore unless we move to ONNX.

## Model bundle plan (FOUNDATION 6.9)

- Bundle ggml-small.en (~466 MB) with the app. Downloaded + verified working from HuggingFace
  ggerganov/whisper.cpp -> ggml-small.en.bin. For this spike it lives at
  %LOCALAPPDATA%\palmier-pro\models\ggml-small.en.bin (NOT committed - too large, per task scope).
  For the shipped app, place it under the Tauri resources/bundle and resolve via a known app-data
  path; offer medium.en + large-v3 as optional in-app downloads.
- Download URL pattern: https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-<name>.bin
- Licensing - clean. whisper.cpp is MIT (not GPL); GGML models are MIT/open. Compatible with our
  distribution; on-device, no-cloud transcription. (Contrast: the LGPL FFmpeg build excludes GPL
  encoders - that constraint does NOT apply to Whisper.) Add a whisper.cpp + ggml MIT attribution
  line to the app third-party notices.

## What E10 (palmier-transcribe) must do

Per docs/reference/transcription.md + FOUNDATION 6.9, palmier-transcribe (and palmier-text for
CaptionBuilder) implements, on top of this now-verified toolchain:

1. Whisper wrapper: load model (bundled small.en; user-selectable medium/large), run
   WhisperState::full(...), read full_n_segments() + get_segment(i) -> to_str_lossy() /
   start_timestamp() / end_timestamp() (centiseconds -> seconds). Enable word-level timestamps
   (token timestamps / --max-len) for TranscriptionWord start/end - confirm precision is enough
   for the dominant-track midpoint logic.
2. Audio extraction: FFmpeg decode -> 16 kHz mono s16le PCM temp file (the format the probe
   already feeds Whisper), windowed by range then offsetting(by: range.lower).
3. Result model + cache: TranscriptionResult { text, language, words[], segments[] }, offsetting,
   and TranscriptCache keyed on sha256(file_content) + model_id + language (FOUNDATION key, not
   the reference path+mtime+size - flagged in the reference doc).
4. Locale + profanity: prefer user locale else Whisper auto-detect else error; profanity ->
   bracketed replacement or token suppression.
5. CaptionBuilder (palmier-text): port the unit-tested phrase-split/distribute/min-duration
   algorithm verbatim (parity oracle) - independent of the engine.
6. Build wiring: the workspace build already inherits CMake + libclang via with-msvc.ps1 ->
   ffmpeg-env.ps1 + whisper-env.ps1. E10 only build concern: pick CPU (default) and gate Vulkan
   behind a cargo feature. No per-crate env work.

## Blockers

None for build/run. The toolchain is verified end-to-end on CPU. Open items for E10 (not
blockers): (a) Vulkan backend not yet built (needs Vulkan SDK; CPU is the safe default);
(b) confirm whisper-rs 0.16 word-level token-timestamp precision once the real crate needs
TranscriptionWord; (c) decide the shipped model path + Tauri resource packaging.

## Timeline
2026-06-20 | setup - probed (cmake missing, libclang+MSVC present); installed portable CMake 4.3.3
after winget MSI wedged; added scripts/whisper-env.ps1 (CMake on PATH + Ninja generator) and wired
it into with-msvc.ps1; verified whisper-rs 0.16 builds whisper.cpp from source and transcribes the
JFK sample on CPU with ggml-small.en.
