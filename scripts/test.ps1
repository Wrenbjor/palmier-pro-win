# test.ps1 — modular, selectable test runner for palmier-pro-win.
#
# WHY: the workspace has ~1,300 tests across 19 crates. Running everything on every
# change is wasteful. This runner lets you test only the section(s) you touched,
# pick a tier (fast unit tests vs. live/gated tests), and get a per-section
# PASS/FAIL summary. It self-sources the MSVC + FFmpeg + whisper build env so cargo
# links correctly (you do NOT need to wrap it in with-msvc.ps1).
#
# Sections (feature areas) and the crates they cover:
#   model       palmier-model, palmier-project, palmier-history   (M1 data model + I/O)
#   edit        palmier-edit, palmier-engine                      (M1 timeline edit engine)
#   media       palmier-media, palmier-text                       (M1 import/thumbnails/captions text)
#   export      palmier-export                                    (M1 XMEML + video export)
#   shell       palmier-tauri, palmier-update, palmier-telemetry, palmier-auth  (M1/M5 app shell)
#   mcp         palmier-mcp, palmier-tools, palmier-prompt        (M2 MCP server + 30 tools)
#   agent       palmier-agent                                     (M2 agentic run loop)
#   gen         palmier-gen                                       (M3 generation lifecycle)
#   transcribe  palmier-transcribe                                (M3 whisper transcription)
#   search      palmier-search                                    (M4 visual + transcript search)
#
# Milestone groups:  m1 = model+edit+media+export+shell  ·  m2 = mcp+agent
#                    m3 = gen+transcribe  ·  m4 = search
#
# Tiers:
#   unit  (default)  fast tests, no external deps  (cargo test, ignored tests skipped)
#   live             ONLY the #[ignore]'d tests (real model/endpoint/whisper/ort) — needs env set up
#   all              unit + live  (cargo test -- --include-ignored)
#
# Usage:
#   pwsh -File scripts/test.ps1 -List                 # show sections and exit
#   pwsh -File scripts/test.ps1                        # all sections, unit tier
#   pwsh -File scripts/test.ps1 -Section edit,export   # just those sections
#   pwsh -File scripts/test.ps1 -Milestone m1          # everything in M1
#   pwsh -File scripts/test.ps1 -Section agent -Tier live   # live agent tests only
#   pwsh -File scripts/test.ps1 -Tier all              # everything, including gated
#   pwsh -File scripts/test.ps1 -Section search -Features ort   # pass extra cargo features

[CmdletBinding()]
param(
  [string[]] $Section,
  [string]   $Milestone,
  [ValidateSet('unit','live','all')] [string] $Tier = 'unit',
  [string]   $Features,
  [switch]   $List,
  [switch]   $NoCapture
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

# --- section / milestone maps ------------------------------------------------
$Sections = [ordered]@{
  model      = @('palmier-model','palmier-project','palmier-history')
  edit       = @('palmier-edit','palmier-engine')
  media      = @('palmier-media','palmier-text')
  export     = @('palmier-export')
  shell      = @('palmier-tauri','palmier-update','palmier-telemetry','palmier-auth')
  mcp        = @('palmier-mcp','palmier-tools','palmier-prompt')
  agent      = @('palmier-agent')
  gen        = @('palmier-gen')
  transcribe = @('palmier-transcribe')
  search     = @('palmier-search')
}
$Milestones = @{
  m1 = @('model','edit','media','export','shell')
  m2 = @('mcp','agent')
  m3 = @('gen','transcribe')
  m4 = @('search')
}

# Harness sections — automated whole-app self-validation (no cargo, no human clicks).
# These run a script rather than `cargo test`, but feed the same PASS/FAIL summary.
#   mcp-smoke  backend oracle: launch app, drive the live MCP server, assert editor state
#   ui-smoke   Playwright UI smoke: render Home/Project/Settings with a mocked Tauri bridge
# They are NOT part of the default "all sections" run (they boot the app / a browser);
# select them explicitly:  -Section mcp-smoke   /   -Section ui-smoke
$HarnessSections = [ordered]@{
  'mcp-smoke' = 'mcp-smoke.ps1'
  'ui-smoke'  = 'ui-smoke'   # handled specially below (pnpm + playwright in e2e/)
}

if ($List) {
  Write-Host "`nSections:" -ForegroundColor Cyan
  foreach ($k in $Sections.Keys) { "{0,-12} {1}" -f $k, ($Sections[$k] -join ', ') }
  Write-Host "`nMilestones:" -ForegroundColor Cyan
  foreach ($k in $Milestones.Keys | Sort-Object) { "{0,-12} {1}" -f $k, ($Milestones[$k] -join ', ') }
  Write-Host "`nHarness (automated whole-app validation; select explicitly):" -ForegroundColor Cyan
  "{0,-12} {1}" -f 'mcp-smoke', 'backend oracle: launch app + drive live MCP server, assert editor state'
  "{0,-12} {1}" -f 'ui-smoke',  'Playwright UI smoke: render Home/Project/Settings (mocked Tauri bridge)'
  Write-Host "`nTiers: unit (default) | live | all`n" -ForegroundColor Cyan
  exit 0
}

# normalize comma-joined args (pwsh -File passes "a,b" as one token)
if ($Section) { $Section = $Section | ForEach-Object { $_ -split ',' } | Where-Object { $_ } }

# --- resolve which sections to run -------------------------------------------
if ($Milestone) {
  if (-not $Milestones.ContainsKey($Milestone)) { throw "Unknown milestone '$Milestone'. Known: $($Milestones.Keys -join ', ')" }
  $run = $Milestones[$Milestone]
} elseif ($Section) {
  foreach ($s in $Section) {
    if (-not ($Sections.Contains($s) -or $HarnessSections.Contains($s))) {
      throw "Unknown section '$s'. Run -List to see options."
    }
  }
  $run = $Section
} else {
  $run = @($Sections.Keys)   # default: everything
}

# --- source the build env (MSVC + FFmpeg + whisper) --------------------------
$vcCandidates = @(
  "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat",
  "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat",
  "C:\Program Files\Microsoft Visual Studio\2022\Professional\VC\Auxiliary\Build\vcvars64.bat",
  "C:\Program Files\Microsoft Visual Studio\2022\Enterprise\VC\Auxiliary\Build\vcvars64.bat"
)
$vcvars = $vcCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $vcvars) { throw "vcvars64.bat not found. Install the VS 2022 C++ build tools." }
foreach ($envScript in @('ffmpeg-env.ps1','whisper-env.ps1')) {
  $p = Join-Path $PSScriptRoot $envScript
  if (Test-Path $p) { . $p }
}

# --- tier -> cargo test arg --------------------------------------------------
$tierArgs = switch ($Tier) {
  'unit' { '' }
  'live' { '-- --ignored' }
  'all'  { '-- --include-ignored' }
}
$featArgs = if ($Features) { "--features $Features" } else { '' }
$capArg   = if ($NoCapture) { if ($tierArgs) { '--nocapture' } else { '-- --nocapture' } } else { '' }

# --- run each section, capture result ----------------------------------------
$results = @()
foreach ($sec in $run) {
  $sw = [System.Diagnostics.Stopwatch]::StartNew()

  if ($HarnessSections.Contains($sec)) {
    # --- harness section: run a script, not cargo ----------------------------
    if ($sec -eq 'mcp-smoke') {
      $script = Join-Path $PSScriptRoot 'mcp-smoke.ps1'
      Write-Host "`n=== [$sec] pwsh -File scripts/mcp-smoke.ps1 ===" -ForegroundColor Yellow
      & pwsh -NoProfile -File $script
      $code = $LASTEXITCODE
    }
    elseif ($sec -eq 'ui-smoke') {
      # Playwright (mock-Tauri) UI smoke. Vite auto-starts via the e2e webServer
      # config. `cmd /c` so PATHEXT resolves pnpm/npx (.cmd) without the .ps1 dialog.
      $e2e = Join-Path $repo 'e2e'
      Write-Host "`n=== [$sec] (cd e2e) pnpm install + npx playwright test ui-smoke ===" -ForegroundColor Yellow
      cmd /c "cd /d `"$e2e`" && (if not exist node_modules pnpm install) && npx playwright test ui-smoke --reporter=list"
      $code = $LASTEXITCODE
    }
    $sw.Stop()
    $results += [pscustomobject]@{
      Section = $sec
      Status  = if ($code -eq 0) { 'PASS' } else { "FAIL($code)" }
      Seconds = [math]::Round($sw.Elapsed.TotalSeconds,1)
    }
    continue
  }

  # --- normal cargo section --------------------------------------------------
  $pkgs = ($Sections[$sec] | ForEach-Object { "-p $_" }) -join ' '
  $cmd  = "cargo test $pkgs $featArgs $tierArgs $capArg".Trim() -replace '\s+',' '
  Write-Host "`n=== [$sec] $cmd ===" -ForegroundColor Yellow
  cmd /c "call `"$vcvars`" >nul 2>&1 && cd /d `"$repo`" && $cmd"
  $code = $LASTEXITCODE
  $sw.Stop()
  $results += [pscustomobject]@{
    Section = $sec
    Status  = if ($code -eq 0) { 'PASS' } else { "FAIL($code)" }
    Seconds = [math]::Round($sw.Elapsed.TotalSeconds,1)
  }
}

# --- summary -----------------------------------------------------------------
Write-Host "`n================ SUMMARY (tier=$Tier) ================" -ForegroundColor Cyan
$results | Format-Table -AutoSize | Out-String | Write-Host
$failed = @($results | Where-Object { $_.Status -ne 'PASS' })
if ($failed.Count -gt 0) {
  Write-Host ("FAILED sections: {0}" -f ($failed.Section -join ', ')) -ForegroundColor Red
  exit 1
}
Write-Host "All selected sections PASSED." -ForegroundColor Green
exit 0
