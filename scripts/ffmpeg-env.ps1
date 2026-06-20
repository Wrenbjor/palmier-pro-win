# ffmpeg-env.ps1 — set the env that ffmpeg-sys-next / ffmpeg-next need to build & link.
#
# WHY: the `ffmpeg-next` (FFmpeg 7.x) bindings build via `ffmpeg-sys-next`, which needs
#   (a) FFmpeg SHARED dev libraries — include headers + import .libs + runtime .dlls, and
#   (b) LLVM/libclang for bindgen to parse the FFmpeg headers.
# Neither is auto-discoverable on this box, so we point the build at them explicitly.
# This script is dot-sourced by scripts/with-msvc.ps1 so EVERY Rust build inherits it.
#
# Idempotent: safe to dot-source repeatedly; PATH entries are de-duplicated.
#
# Install locations (provisioned by infra/ffmpeg-toolchain):
#   FFmpeg 7.1 win64 LGPL shared (BtbN)  ->  C:\ffmpeg   (include\ lib\ bin\)
#   libclang.dll (PyPI `libclang` wheel) ->  %LOCALAPPDATA%\Programs\libclang\bin
#     (per-user, no elevation; the LLVM NSIS installer requires admin/UAC and so
#      cannot run non-interactively on this box — see spikes/ffmpeg-setup/FINDINGS.md).
#     A system-wide LLVM at C:\Program Files\LLVM\bin is detected too, if present.
#
# Override either by setting FFMPEG_DIR / LIBCLANG_PATH before invoking the build.

# --- FFmpeg ---------------------------------------------------------------
if (-not $env:FFMPEG_DIR) {
  if (Test-Path 'C:\ffmpeg\include') { $env:FFMPEG_DIR = 'C:\ffmpeg' }
}
if ($env:FFMPEG_DIR -and (Test-Path "$env:FFMPEG_DIR\bin")) {
  $ffbin = (Resolve-Path "$env:FFMPEG_DIR\bin").Path
  # Prepend the FFmpeg bin dir so the freshly-built test exe finds avcodec-61.dll etc. at runtime.
  if (($env:PATH -split ';') -notcontains $ffbin) {
    $env:PATH = "$ffbin;$env:PATH"
  }
}

# --- LLVM / libclang (for bindgen) ----------------------------------------
if (-not $env:LIBCLANG_PATH) {
  $clangCandidates = @(
    "$env:LOCALAPPDATA\Programs\libclang\bin",  # per-user (PyPI libclang wheel) — primary on this box
    "$env:LOCALAPPDATA\Programs\LLVM\bin",       # per-user LLVM, if ever installed there
    'C:\Program Files\LLVM\bin',                 # system-wide LLVM (winget/installer, needs admin)
    "$env:ProgramFiles\LLVM\bin"
  )
  foreach ($cand in $clangCandidates) {
    if (Test-Path "$cand\libclang.dll") { $env:LIBCLANG_PATH = (Resolve-Path $cand).Path; break }
  }
}
if ($env:LIBCLANG_PATH -and (($env:PATH -split ';') -notcontains $env:LIBCLANG_PATH)) {
  $env:PATH = "$env:LIBCLANG_PATH;$env:PATH"
}
