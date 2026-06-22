# fetch-msedgedriver.ps1 — download the msedgedriver that matches THIS box's Edge/WebView2.
#
# tauri-driver needs an msedgedriver whose version matches the installed Edge/WebView2
# (WebView2 is Chromium/Edge under the hood on Windows). This vendors an exact-match copy
# into .driver/ so the suite is hermetic. Re-run after an Edge update.
#
# Usage:  pwsh -File e2e/real-webview/fetch-msedgedriver.ps1

$ErrorActionPreference = 'Stop'
$dest = Join-Path $PSScriptRoot '.driver'
New-Item -ItemType Directory -Force $dest | Out-Null

# Detect installed Edge version.
$edgeExe = 'C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe'
if (-not (Test-Path $edgeExe)) { throw "Edge not found at $edgeExe; cannot determine the matching driver version." }
$ver = (Get-Item $edgeExe).VersionInfo.ProductVersion
Write-Host "Installed Edge/WebView2: $ver" -ForegroundColor Cyan

$zip = Join-Path $dest 'edgedriver_win64.zip'
$urls = @(
  "https://msedgedriver.microsoft.com/$ver/edgedriver_win64.zip",
  "https://msedgedriver.azureedge.net/$ver/edgedriver_win64.zip"
)
$ok = $false
foreach ($u in $urls) {
  try { Write-Host "Downloading $u"; Invoke-WebRequest -Uri $u -OutFile $zip -UseBasicParsing -ErrorAction Stop; $ok = $true; break }
  catch { Write-Host "  failed: $($_.Exception.Message)" -ForegroundColor Yellow }
}
if (-not $ok) { throw "Could not download msedgedriver $ver. Check https://developer.microsoft.com/microsoft-edge/tools/webdriver/" }

Expand-Archive -Path $zip -DestinationPath $dest -Force
$drv = Join-Path $dest 'msedgedriver.exe'
if (-not (Test-Path $drv)) { throw "msedgedriver.exe not found after extracting $zip" }
Write-Host ("Vendored: " + (& $drv --version)) -ForegroundColor Green
