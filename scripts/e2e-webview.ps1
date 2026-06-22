# e2e-webview.ps1 — Layer B real-webview smoke (tauri-driver + WebdriverIO).
#
# Drives the REAL WebView2 window the app ships and the REAL Rust invoke bridge:
# launches target/debug/palmier-tauri.exe under WebDriver via tauri-driver, then asserts
# Home renders, the live MCP backend is up, and a real Settings-button click opens a new
# native window (the invoke bridge side effect a mock can't produce).
#
# Prereqs (provisioned once on this box; verified at runtime by wdio.conf.ts):
#   - tauri-driver   (~/.cargo/bin/tauri-driver.exe)   cargo install tauri-driver --locked
#   - msedgedriver   (e2e/real-webview/.driver/)       MUST match installed Edge/WebView2
#   - app binary     target/debug/palmier-tauri.exe     cargo build -p palmier-tauri --no-default-features
#   - wdio deps       cmd /c "cd e2e/real-webview && pnpm install"
#
# Usage:  pwsh -File scripts/e2e-webview.ps1
# Exit:   0 = all specs passed; nonzero = a spec failed or setup is missing.

[CmdletBinding()]
param(
  [string] $AppBin
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

# The frozen binary needs the FFmpeg DLLs (C:\ffmpeg\bin) on PATH at runtime, or boot
# fails partway and MCP never binds. tauri-driver spawns the app as a CHILD of this
# process, so it inherits this PATH — source the env here.
$ffmpegEnv = Join-Path $PSScriptRoot 'ffmpeg-env.ps1'
if (Test-Path $ffmpegEnv) { . $ffmpegEnv }
$whisperEnv = Join-Path $PSScriptRoot 'whisper-env.ps1'
if (Test-Path $whisperEnv) { . $whisperEnv }

# Resolve the testable binary. MUST be a `tauri build --debug` (custom-protocol) binary
# with the frontend FROZEN in — a bare `cargo build` binary points at the Vite devUrl
# and shows a blank webview under WebDriver. Prefer this worktree's target/.
if (-not $AppBin) {
  $candidates = @(
    (Join-Path $repo 'target\debug\palmier-tauri.exe'),
    'E:\projects\palmier-pro-win\target\debug\palmier-tauri.exe'
  )
  $AppBin = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $AppBin -or -not (Test-Path $AppBin)) {
  throw "app binary not found. Build it: pwsh -File scripts/with-msvc.ps1 cargo build --package palmier-tauri --no-default-features"
}
$AppBin = (Resolve-Path $AppBin).Path
Write-Host "[e2e-webview] app binary: $AppBin" -ForegroundColor Cyan

# tauri-driver runs ONE app session at a time — make sure no stray instance is up.
Get-Process palmier-tauri -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process tauri-driver -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

$env:PALMIER_APP_BIN = $AppBin

$rw = Join-Path $repo 'e2e\real-webview'
Write-Host "[e2e-webview] running WebdriverIO suite in $rw" -ForegroundColor Cyan
cmd /c "cd /d `"$rw`" && (if not exist node_modules pnpm install) && npx wdio run ./wdio.conf.ts"
$code = $LASTEXITCODE

# Teardown: kill anything tauri-driver may have left running.
Get-Process palmier-tauri -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process tauri-driver -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process msedgedriver -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

if ($code -eq 0) {
  Write-Host "`n[e2e-webview] PASS — real-webview smoke green." -ForegroundColor Green
} else {
  Write-Host "`n[e2e-webview] FAIL (exit $code)." -ForegroundColor Red
}
exit $code
