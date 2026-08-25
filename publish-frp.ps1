# ============================================================
# Cangyun Qiling - publish through a self-hosted frp relay
#
# Prerequisites:
#   1. Set a fresh JX3_AUTH_PASSWORD in the current shell.
#   2. Copy tools/frpc.example.toml to tools/frpc.toml and replace every
#      documentation value with the matching private deployment value.
#   3. Put an HTTPS reverse proxy in front of the VPS origin port.
#
# Existing members, personal passwords, session tokens, and user data are
# preserved when the deployment master password changes.
# ============================================================

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot

if ([string]::IsNullOrWhiteSpace($env:JX3_AUTH_PASSWORD)) {
  Write-Host '[X] JX3_AUTH_PASSWORD is required.' -ForegroundColor Red
  Write-Host '    Set a new deployment password in the current shell, then retry.' -ForegroundColor Yellow
  exit 1
}
if ($env:JX3_AUTH_PASSWORD.Length -lt 12) {
  Write-Host '[X] JX3_AUTH_PASSWORD must contain at least 12 characters.' -ForegroundColor Red
  exit 1
}

$env:JX3_ROUTER = '1'
$env:JX3_PORT = '3006'

$exe = Join-Path $root 'backend\target\release\jx3-combat-sim.exe'
$frpc = Join-Path $root 'tools\frpc.exe'
$config = Join-Path $root 'tools\frpc.toml'

if (-not (Test-Path -LiteralPath $exe)) {
  Write-Host "[X] Backend executable not found: $exe" -ForegroundColor Red
  Write-Host '    Build it with: cargo build --release --manifest-path backend/Cargo.toml' -ForegroundColor Yellow
  exit 1
}
if (-not (Test-Path -LiteralPath $frpc)) {
  Write-Host "[X] frpc.exe not found: $frpc" -ForegroundColor Red
  exit 1
}
if (-not (Test-Path -LiteralPath $config)) {
  Write-Host "[X] Private frp config not found: $config" -ForegroundColor Red
  Write-Host '    Copy tools/frpc.example.toml and fill it outside version control.' -ForegroundColor Yellow
  exit 1
}

$configText = Get-Content -LiteralPath $config -Raw
if ($configText -match '203\.0\.113\.' -or $configText -match 'replace-with-') {
  Write-Host '[X] frpc.toml still contains documentation placeholders.' -ForegroundColor Red
  exit 1
}

Write-Host '[*] Cleaning leftover simulator/frpc processes ...' -ForegroundColor DarkCyan
Get-Process jx3-combat-sim, frpc -ErrorAction SilentlyContinue |
  Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400

Write-Host '[*] Starting process-isolation router on 127.0.0.1:3006 ...' -ForegroundColor Cyan
Start-Process -FilePath $exe -WorkingDirectory (Join-Path $root 'backend') -WindowStyle Normal

$healthUrl = 'http://127.0.0.1:3006/health'
$ready = $false
for ($attempt = 0; $attempt -lt 60; $attempt++) {
  try {
    $response = Invoke-WebRequest -Uri $healthUrl -UseBasicParsing -TimeoutSec 2
    if ($response.StatusCode -ge 200) {
      $ready = $true
      break
    }
  } catch {}
  Start-Sleep -Milliseconds 500
}

if (-not $ready) {
  Write-Host '[X] Router did not become healthy within 30 seconds; frpc was not started.' -ForegroundColor Red
  exit 1
}

Write-Host '[OK] Router ready; starting frpc.' -ForegroundColor Green
if (-not [string]::IsNullOrWhiteSpace($env:JX3_PUBLIC_URL)) {
  Write-Host "  Public HTTPS URL: $env:JX3_PUBLIC_URL" -ForegroundColor Green
} else {
  Write-Host '  Public URL: use the HTTPS endpoint configured on your reverse proxy.' -ForegroundColor Green
}
Write-Host '  The deployment password and frp token are never printed.' -ForegroundColor Green
Write-Host '  Stop with Ctrl+C, then close the Router window.' -ForegroundColor Green

& $frpc -c $config

