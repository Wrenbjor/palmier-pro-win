# mcp-smoke.ps1 — backend "oracle": automated, no-human-clicks validation of the
# running app via its live MCP JSON-RPC server.
#
# WHY: the app's strategic surface is agent-controlled timeline editing over a local
# MCP server (127.0.0.1:19789/mcp), sharing ONE EditorState with the in-app agent.
# This script proves that surface end to end with zero UI interaction: it launches the
# app, waits for the MCP listener, then drives a deterministic edit sequence and
# asserts editor state after each step. Any failure -> nonzero exit. The app is always
# torn down at the end (even on failure), so it is safe to run in CI / a loop.
#
# Sequence (each step asserts):
#   1. get_timeline   -> empty (no clips yet)
#   2. import_media    -> a test clip; returns an asset id
#   3. get_media       -> exactly 1 asset
#   4. add_clips       -> place the asset on the timeline
#   5. get_timeline    -> totalFrames > 0 and the clip is present
#   6. undo            -> add_clips reverted; get_timeline empty again
#
# Usage:
#   pwsh -File scripts/mcp-smoke.ps1                 # generate/launch/test/teardown
#   pwsh -File scripts/mcp-smoke.ps1 -KeepAlive      # leave the app running after
#   pwsh -File scripts/mcp-smoke.ps1 -NoLaunch       # assume app already running
#   pwsh -File scripts/mcp-smoke.ps1 -Clip <path>    # use a specific media file
#
# Exit code: 0 = all steps PASS; 1 = any assertion or step failed.

[CmdletBinding()]
param(
  [string] $Clip,
  [switch] $NoLaunch,
  [switch] $KeepAlive,
  [int]    $TimeoutSec = 180
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

$McpUrl = 'http://127.0.0.1:19789/mcp'
$script:rpcId = 0
$script:failures = 0
$script:appProc = $null

# --- pretty per-step reporting ----------------------------------------------
function Step-Pass([string]$msg) { Write-Host ("  PASS  " + $msg) -ForegroundColor Green }
function Step-Fail([string]$msg) { Write-Host ("  FAIL  " + $msg) -ForegroundColor Red; $script:failures++ }
function Step-Info([string]$msg) { Write-Host ("  ....  " + $msg) -ForegroundColor DarkGray }
function Section([string]$msg)   { Write-Host ("`n>> " + $msg) -ForegroundColor Cyan }

# --- MCP JSON-RPC client -----------------------------------------------------
# Calls tools/call and returns the PARSED object from result.content[0].text when it
# is JSON, else the raw text string. Throws on transport / JSON-RPC error.
function Invoke-Mcp {
  param([string]$Name, [hashtable]$Arguments = @{})
  $script:rpcId++
  $payload = @{
    jsonrpc = '2.0'
    id      = $script:rpcId
    method  = 'tools/call'
    params  = @{ name = $Name; arguments = $Arguments }
  } | ConvertTo-Json -Depth 12 -Compress

  $resp = Invoke-RestMethod -Uri $McpUrl -Method Post -Body $payload `
    -ContentType 'application/json' `
    -Headers @{ 'Accept' = 'application/json, text/event-stream' } `
    -TimeoutSec 60

  if ($null -ne $resp.error) {
    throw "MCP error on '$Name': $($resp.error.message)"
  }
  $text = $resp.result.content[0].text
  if ($null -eq $text) { return $resp.result }
  # Try to parse the inner text as JSON; many tools return prose instead.
  try { return ($text | ConvertFrom-Json) } catch { return $text }
}

# Raw text result (for prose-returning tools like import_media / add_clips / undo).
function Invoke-McpText {
  param([string]$Name, [hashtable]$Arguments = @{})
  $script:rpcId++
  $payload = @{
    jsonrpc = '2.0'; id = $script:rpcId; method = 'tools/call'
    params  = @{ name = $Name; arguments = $Arguments }
  } | ConvertTo-Json -Depth 12 -Compress
  $resp = Invoke-RestMethod -Uri $McpUrl -Method Post -Body $payload `
    -ContentType 'application/json' `
    -Headers @{ 'Accept' = 'application/json, text/event-stream' } -TimeoutSec 60
  if ($null -ne $resp.error) { throw "MCP error on '$Name': $($resp.error.message)" }
  return [string]$resp.result.content[0].text
}

# Count clips across all tracks in a parsed get_timeline result.
function Get-ClipCount($timeline) {
  $n = 0
  if ($timeline.tracks) {
    foreach ($t in $timeline.tracks) {
      if ($t.clips) { $n += @($t.clips).Count }
    }
  }
  return $n
}

# --- teardown (always runs) --------------------------------------------------
function Stop-App {
  if ($KeepAlive) { Step-Info 'KeepAlive set; leaving app running.'; return }
  Get-Process palmier-tauri -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

# ============================================================================
try {
  # --- resolve a test clip ---------------------------------------------------
  if (-not $Clip) {
    $Clip = Join-Path $repo 'test-assets\oracle-clip-5s.mp4'
    if (-not (Test-Path $Clip)) {
      Section "Generating test clip (none found at $Clip)"
      . "$PSScriptRoot\ffmpeg-env.ps1"
      New-Item -ItemType Directory -Force (Split-Path $Clip) | Out-Null
      ffmpeg -y -f lavfi -i "testsrc2=size=1280x720:rate=30:duration=5" `
        -f lavfi -i "sine=frequency=440:duration=5" `
        -c:v libopenh264 -b:v 3M -pix_fmt yuv420p -c:a aac -shortest $Clip 2>&1 | Out-Null
      if (-not (Test-Path $Clip)) { throw "ffmpeg failed to produce $Clip" }
    }
  }
  if (-not (Test-Path $Clip)) { throw "Test clip not found: $Clip" }
  $Clip = (Resolve-Path $Clip).Path
  Step-Info "Test clip: $Clip"

  # --- launch the app (unless told it is already up) -------------------------
  if (-not $NoLaunch) {
    Section 'Launching app (scripts/run-app.ps1)'
    # run-app.ps1 builds if needed, starts Vite, then launches the binary.
    & pwsh -NoProfile -File "$PSScriptRoot\run-app.ps1"
    if ($LASTEXITCODE -ne 0) { throw "run-app.ps1 failed (exit $LASTEXITCODE)" }
  }

  # --- wait for the MCP listener --------------------------------------------
  Section "Waiting for MCP server at $McpUrl (timeout ${TimeoutSec}s)"
  $deadline = (Get-Date).AddSeconds($TimeoutSec)
  $ready = $false
  while ((Get-Date) -lt $deadline) {
    try {
      $ping = @{ jsonrpc='2.0'; id=0; method='ping' } | ConvertTo-Json -Compress
      $r = Invoke-RestMethod -Uri $McpUrl -Method Post -Body $ping `
        -ContentType 'application/json' `
        -Headers @{ 'Accept' = 'application/json, text/event-stream' } -TimeoutSec 5
      if ($r) { $ready = $true; break }
    } catch { Start-Sleep -Milliseconds 700 }
  }
  if (-not $ready) { throw "MCP server never answered at $McpUrl within ${TimeoutSec}s" }
  Step-Pass 'MCP server is up (ping answered).'

  # --- STEP 1: get_timeline -> empty ----------------------------------------
  Section 'Step 1 — get_timeline (expect empty)'
  $tl0 = Invoke-Mcp 'get_timeline'
  $count0 = Get-ClipCount $tl0
  if ($count0 -eq 0) { Step-Pass "timeline empty (0 clips)" }
  else { Step-Fail "expected 0 clips, found $count0" }

  # --- STEP 2: import_media -> asset id --------------------------------------
  Section 'Step 2 — import_media (local path)'
  $importText = Invoke-McpText 'import_media' @{ source = @{ path = $Clip } }
  Step-Info "import_media said: $importText"
  $assetId = $null
  $m = [regex]::Match($importText, 'id:\s*([0-9a-fA-F\-]{8,})')
  if ($m.Success) { $assetId = $m.Groups[1].Value }
  if ($assetId) { Step-Pass "imported asset id=$assetId" }
  else { Step-Fail "could not parse an asset id from import_media result" }

  # --- STEP 3: get_media -> 1 asset ------------------------------------------
  Section 'Step 3 — get_media (expect 1 asset)'
  $media = Invoke-Mcp 'get_media'
  $assets = @($media.assets)
  if ($assets.Count -eq 1) { Step-Pass "get_media has exactly 1 asset" }
  else { Step-Fail "expected 1 asset, found $($assets.Count)" }
  # Fall back to the media list for the asset id if the import text didn't yield one.
  if (-not $assetId -and $assets.Count -ge 1) {
    $assetId = $assets[0].id
    Step-Info "using asset id from get_media: $assetId"
  }
  $durSec = if ($assets.Count -ge 1) { [double]$assets[0].duration } else { 0 }
  Step-Info "asset duration ~= $durSec s"

  # --- STEP 4: add_clips -> place on timeline --------------------------------
  Section 'Step 4 — add_clips (place asset @ frame 0)'
  if (-not $assetId) { throw "no asset id available to add_clips" }
  # 30fps test clip ~5s; place ~120 frames to keep within the source duration.
  $durFrames = 120
  $addText = Invoke-McpText 'add_clips' @{
    entries = @(@{ mediaRef = $assetId; startFrame = 0; durationFrames = $durFrames })
  }
  Step-Info "add_clips said: $addText"
  if ($addText -match 'Added\s+\d+\s+clip') { Step-Pass "add_clips reported clip(s) added" }
  else { Step-Fail "add_clips did not confirm a clip was added" }

  # --- STEP 5: get_timeline -> totalFrames>0 and clip present ----------------
  Section 'Step 5 — get_timeline (expect totalFrames>0 and a clip)'
  $tl1 = Invoke-Mcp 'get_timeline'
  $count1 = Get-ClipCount $tl1
  $total1 = [int]($tl1.totalFrames)
  if ($total1 -gt 0) { Step-Pass "totalFrames=$total1 (> 0)" }
  else { Step-Fail "expected totalFrames>0, got $total1" }
  if ($count1 -ge 1) { Step-Pass "clip present on timeline ($count1 clip(s))" }
  else { Step-Fail "expected >=1 clip on timeline, found $count1" }

  # --- STEP 6: undo -> reverted ----------------------------------------------
  Section 'Step 6 — undo (expect add_clips reverted)'
  $undoText = Invoke-McpText 'undo'
  Step-Info "undo said: $undoText"
  if ($undoText -match '(?i)^Undid') { Step-Pass "undo confirmed: reverted an agent edit" }
  else { Step-Fail "undo did not confirm a revert (got: $undoText)" }
  $tl2 = Invoke-Mcp 'get_timeline'
  $count2 = Get-ClipCount $tl2
  if ($count2 -eq 0) { Step-Pass "timeline reverted to empty (0 clips)" }
  else { Step-Fail "expected 0 clips after undo, found $count2" }
}
catch {
  Step-Fail "EXCEPTION: $($_.Exception.Message)"
}
finally {
  Section 'Teardown'
  Stop-App
}

# --- summary -----------------------------------------------------------------
Write-Host "`n================ MCP SMOKE SUMMARY ================" -ForegroundColor Cyan
if ($script:failures -eq 0) {
  Write-Host "ALL STEPS PASSED" -ForegroundColor Green
  exit 0
} else {
  Write-Host ("FAILED: {0} assertion(s)/step(s) failed" -f $script:failures) -ForegroundColor Red
  exit 1
}
