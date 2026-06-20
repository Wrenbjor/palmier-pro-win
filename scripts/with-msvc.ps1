# with-msvc.ps1 — run a command inside the MSVC x64 build environment.
#
# WHY: this box's VS 2022 install isn't registered with vswhere, so rustc/cc cannot
# auto-detect MSVC and `cargo build` fails at link with "ensure the Visual C++ option
# was installed" — even though the toolchain (MSVC 14.29 + Windows SDK 10.0.22621) is
# fully present on disk. Calling vcvars64.bat explicitly sets INCLUDE/LIB/PATH so the
# build works. ALL Rust/Tauri builds in this repo MUST go through this wrapper (or a
# shell already initialized by vcvars64).
#
# Usage:
#   pwsh -File scripts/with-msvc.ps1 cargo build
#   pwsh -File scripts/with-msvc.ps1 cargo test
#   pwsh -File scripts/with-msvc.ps1 pnpm tauri build
param([Parameter(ValueFromRemainingArguments = $true)] [string[]] $Command)

$candidates = @(
  "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
  "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
  "C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
  "C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat"
)
$vcvars = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $vcvars) { Write-Error "vcvars64.bat not found in any known VS 2022 location. Install the C++ build tools workload."; exit 1 }
if (-not $Command -or $Command.Count -eq 0) { Write-Error "No command given. Example: pwsh -File scripts/with-msvc.ps1 cargo build"; exit 1 }

# Dot-source the FFmpeg/libclang env if present, so ffmpeg-next (ffmpeg-sys-next + bindgen)
# can build & link. Sets FFMPEG_DIR, LIBCLANG_PATH, and PATH in THIS PowerShell process;
# the cmd child below inherits the process environment. Optional: absent file = no-op.
$ffmpegEnv = Join-Path $PSScriptRoot 'ffmpeg-env.ps1'
if (Test-Path $ffmpegEnv) { . $ffmpegEnv }

# Dot-source the whisper env if present, so whisper-rs (which builds whisper.cpp from
# source via CMake + bindgen) can find cmake. Sets CMAKE_BIN and prepends it to PATH.
# libclang/LIBCLANG_PATH is already provided by ffmpeg-env.ps1 above. Optional: absent = no-op.
$whisperEnv = Join-Path $PSScriptRoot 'whisper-env.ps1'
if (Test-Path $whisperEnv) { . $whisperEnv }

$cmdline = ($Command -join ' ')
cmd /c "call `"$vcvars`" >nul 2>&1 && $cmdline"
exit $LASTEXITCODE
