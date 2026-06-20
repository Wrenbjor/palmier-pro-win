# whisper-env.ps1 — set the env that whisper-rs / whisper-rs-sys need to build & link.
#
# WHY: `whisper-rs` builds whisper.cpp FROM SOURCE in its build script via:
#   (a) CMake — to configure & build the bundled whisper.cpp/ggml C/C++ sources, and
#   (b) bindgen (libclang) — to generate the Rust FFI from whisper.cpp headers, and
#   (c) a C/C++ compiler — MSVC, provided by with-msvc.ps1 (vcvars64).
# libclang is already pointed at by ffmpeg-env.ps1 (LIBCLANG_PATH) — whisper reuses it.
# The ONE thing ffmpeg-env.ps1 does NOT provide is CMake: this box has no system CMake
# (winget's MSI install wedged the SYSTEM msiexec service — see spikes/whisper-setup/
# FINDINGS.md), so CMake was installed PORTABLY (no admin/MSI) to
# %LOCALAPPDATA%\Programs\cmake and must be put on PATH here.
#
# This script is dot-sourced by scripts/with-msvc.ps1 (right after ffmpeg-env.ps1) so
# EVERY Rust build through the wrapper can find cmake. Idempotent; PATH is de-duplicated.
#
# Override by setting CMAKE_BIN before invoking the build.

if (-not $env:CMAKE_BIN) {
  $cmakeCandidates = @(
    "$env:LOCALAPPDATA\Programs\cmake\bin",  # per-user portable ZIP — primary on this box
    'C:\Program Files\CMake\bin',            # system-wide MSI install, if ever present
    "$env:ProgramFiles\CMake\bin"
  )
  foreach ($cand in $cmakeCandidates) {
    if (Test-Path "$cand\cmake.exe") { $env:CMAKE_BIN = (Resolve-Path $cand).Path; break }
  }
}
if ($env:CMAKE_BIN -and (($env:PATH -split ';') -notcontains $env:CMAKE_BIN)) {
  $env:PATH = "$env:CMAKE_BIN;$env:PATH"
}

# --- CMake generator -------------------------------------------------------
# CRITICAL: the `cmake` Rust crate defaults to the "Visual Studio 17 2022" generator,
# which runs its OWN Visual Studio discovery (vswhere) and FAILS on this box with
# "could not find any instance of Visual Studio" — the same vswhere blind-spot that
# forces with-msvc.ps1 to exist (see docs/windows-harness-notes.md). vcvars64 already
# put the MSVC cl.exe + Ninja on PATH, so tell CMake to use Ninja, which builds with
# whatever compiler is on PATH instead of re-discovering VS. Ninja ships inside VS's
# CMake component (Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja) and vcvars64
# adds it to PATH. NMake Makefiles also works as a fallback if Ninja is ever absent.
if (-not $env:CMAKE_GENERATOR) {
  $env:CMAKE_GENERATOR = 'Ninja'
}
# Ninja is a single-config generator that REJECTS a platform spec ("Ninja does not
# support platform specification, but platform x64 was specified"). CMake picks up a
# platform from the CMAKE_GENERATOR_PLATFORM env var if it is *defined* (even empty),
# so ensure it is fully UNSET (not ""), along with the toolset var. The x64 target
# still comes from vcvars64's cl.exe being on PATH. Use Remove-Item, since `$env:X=''`
# leaves it defined-but-empty, which still trips the check.
Remove-Item Env:\CMAKE_GENERATOR_PLATFORM -ErrorAction SilentlyContinue
Remove-Item Env:\CMAKE_GENERATOR_TOOLSET -ErrorAction SilentlyContinue
