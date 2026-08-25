param(
  [string]$BaseUrl = 'http://127.0.0.1:3005',
  [int]$TimeoutSec = 15
)

$ErrorActionPreference = 'Stop'
$base = $BaseUrl.TrimEnd('/')

function Assert-True {
  param(
    [bool]$Condition,
    [string]$Message
  )
  if (-not $Condition) {
    throw $Message
  }
}

Write-Host "[1/3] Health: $base/health"
$health = Invoke-WebRequest -Uri "$base/health" -UseBasicParsing -TimeoutSec $TimeoutSec
Assert-True ($health.StatusCode -eq 200) 'Health endpoint did not return HTTP 200.'
Assert-True ($health.Content.Trim() -eq 'OK') 'Health endpoint returned an unexpected body.'

Write-Host "[2/3] Frontend: $base/"
$frontendResponse = Invoke-WebRequest -Uri "$base/" -UseBasicParsing -TimeoutSec $TimeoutSec
Assert-True ($frontendResponse.StatusCode -eq 200) 'Frontend root did not return HTTP 200.'
Assert-True ($frontendResponse.Content -match '(?i)<html') 'Frontend root did not return an HTML document.'

Write-Host "[3/3] Simulation: $base/api/simulate"
$shieldStrike = [string][char]0x76FE + [string][char]0x51FB
$shieldPress = [string][char]0x76FE + [string][char]0x538B
$payload = @{
  haste_level = 0
  sequence = @('__macro__')
  macro_text = "/cast $shieldStrike`n/cast $shieldPress"
  macro_duration = 5
  lite = $true
} | ConvertTo-Json -Depth 5
$payloadBytes = [System.Text.Encoding]::UTF8.GetBytes($payload)

$simulation = Invoke-RestMethod `
  -Uri "$base/api/simulate" `
  -Method Post `
  -ContentType 'application/json; charset=utf-8' `
  -Body $payloadBytes `
  -TimeoutSec $TimeoutSec

Assert-True ($null -ne $simulation.fingerprint) 'Simulation response has no fingerprint.'
Assert-True ($null -ne $simulation.skill_count) 'Simulation response has no skill_count.'
Assert-True ($simulation.skill_count -gt 0) 'Simulation completed without executing a skill.'

Write-Host '[OK] Local smoke test passed.' -ForegroundColor Green
Write-Host "     fingerprint=$($simulation.fingerprint) skills=$($simulation.skill_count)"
