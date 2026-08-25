# ============================================================
# Cangyun Qiling - publish with a Cloudflare Quick Tunnel
#
# Before running, set a fresh master password without echoing or storing it in
# shell history:
#   $secure = Read-Host 'Deployment master password' -AsSecureString
#   $env:JX3_AUTH_PASSWORD = [Net.NetworkCredential]::new('', $secure).Password
#
# The password gates creation of new usernames. Existing members, personal
# passwords, session tokens, and user data are preserved.
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
$exe = Join-Path $root 'backend\target\release\jx3-combat-sim.exe'
$cloudflared = Join-Path $root 'tools\cloudflared.exe'

if (-not (Test-Path -LiteralPath $exe)) {
  Write-Host "[X] Backend executable not found: $exe" -ForegroundColor Red
  Write-Host '    Build it with: cargo build --release --manifest-path backend/Cargo.toml' -ForegroundColor Yellow
  exit 1
}
if (-not (Test-Path -LiteralPath $cloudflared)) {
  Write-Host "[X] cloudflared not found: $cloudflared" -ForegroundColor Red
  Write-Host '    Download it from the official Cloudflare release page into tools/.' -ForegroundColor Yellow
  exit 1
}

Write-Host '[*] Cleaning leftover simulator processes ...' -ForegroundColor DarkCyan
Get-Process jx3-combat-sim -ErrorAction SilentlyContinue |
  Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400

Write-Host '[*] Starting process-isolation router ...' -ForegroundColor Cyan
Start-Process -FilePath $exe -WorkingDirectory (Join-Path $root 'backend') -WindowStyle Normal

$healthUrl = 'http://127.0.0.1:3005/health'
Write-Host "[*] Waiting for Router at $healthUrl ..." -ForegroundColor Cyan
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
  Write-Host '[X] Router did not become healthy within 30 seconds; tunnel was not started.' -ForegroundColor Red
  exit 1
}

Write-Host '[OK] Router ready.' -ForegroundColor Green
Write-Host '============================================================' -ForegroundColor DarkGray
Write-Host '  cloudflared will print a temporary HTTPS URL below.' -ForegroundColor Green
Write-Host '  Login uses any new username plus the password supplied via' -ForegroundColor Green
Write-Host '  JX3_AUTH_PASSWORD. The password itself is never printed.' -ForegroundColor Green
Write-Host '  Stop with Ctrl+C, then close the Router window.' -ForegroundColor Green
Write-Host '============================================================' -ForegroundColor DarkGray

& $cloudflared tunnel --url http://127.0.0.1:3005 --no-autoupdate
