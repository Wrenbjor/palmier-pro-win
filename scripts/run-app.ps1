# run-app.ps1 — one-command launcher for the Palmier Pro dev build.
#
# Why this exists: `tauri dev` has a beforeDevCommand path bug on this repo layout,
# and the app binary needs the FFmpeg DLLs (C:\ffmpeg\bin) on PATH at runtime. This
# script starts Vite if it isn't already up, sets the env, and launches the built
# binary. Runtime logs go to %LOCALAPPDATA%\PalmierProWin\Logs\palmier.log.<date>.
#
# Usage:
#   pwsh -File scripts/run-app.ps1            # launch (builds first if no binary)
#   pwsh -File scripts/run-app.ps1 -Build     # force a rebuild, then launch
#
# Open DevTools in the running app: press  Ctrl+Shift+I  (or right-click -> Inspect).

[CmdletBinding()]
param([switch] $Build)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

. "$PSScriptRoot\ffmpeg-env.ps1"
. "$PSScriptRoot\whisper-env.ps1"
$env:RUST_LOG = "info,mcp=info,app=info"

# 1. Ensure Vite dev server is up on 5173 (the dev build loads its UI from there).
$viteUp = $false
try { Invoke-WebRequest -Uri "http://localhost:5173" -TimeoutSec 2 -UseBasicParsing | Out-Null; $viteUp = $true } catch {}
if (-not $viteUp) {
  Write-Host "Starting Vite dev server..." -ForegroundColor Cyan
  # Launch via cmd.exe so PATHEXT resolves pnpm -> pnpm.cmd. Calling Start-Process on
  # "pnpm" directly hits the pnpm.ps1 shim, which ShellExecute can't run (Windows then
  # pops the "how do you want to open this .ps1 file?" dialog).
  Start-Process -FilePath "cmd.exe" -ArgumentList '/c','pnpm','--dir','src-ui','dev' -WorkingDirectory $repo -WindowStyle Hidden
  for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 500
    try { Invoke-WebRequest -Uri "http://localhost:5173" -TimeoutSec 2 -UseBasicParsing | Out-Null; $viteUp = $true; break } catch {}
  }
  if (-not $viteUp) { throw "Vite did not come up on http://localhost:5173" }
}
Write-Host "Vite ready on http://localhost:5173" -ForegroundColor Green

# 2. Build the binary if requested or missing.
$exe = Join-Path $repo "target\debug\palmier-tauri.exe"
if ($Build -or -not (Test-Path $exe)) {
  Write-Host "Building palmier-tauri (dev)..." -ForegroundColor Cyan
  $vcvars = @(
    "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
    "C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat"
  ) | Where-Object { Test-Path $_ } | Select-Object -First 1
  cmd /c "call `"$vcvars`" >nul 2>&1 && cd /d `"$repo`" && cargo build -p palmier-tauri --no-default-features"
  if ($LASTEXITCODE -ne 0) { throw "build failed (exit $LASTEXITCODE)" }
}

# 3. Launch (no std-handle redirection — keeps the GUI subsystem happy).
Write-Host "Launching app..." -ForegroundColor Cyan
$p = Start-Process -FilePath $exe -WorkingDirectory $repo -PassThru
Write-Host ("Palmier Pro running (pid {0})." -f $p.Id) -ForegroundColor Green
Write-Host "Logs: $env:LOCALAPPDATA\PalmierProWin\Logs\palmier.log.*" -ForegroundColor DarkGray
Write-Host "DevTools in-app: Ctrl+Shift+I  (or right-click -> Inspect)" -ForegroundColor DarkGray
