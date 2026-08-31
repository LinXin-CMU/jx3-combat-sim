param(
  [string]$BaseUrl = 'http://127.0.0.1:3005',
  [int]$TimeoutSec = 20
)

$ErrorActionPreference = 'Stop'
$base = $BaseUrl.TrimEnd('/')

function Assert-True {
  param([bool]$Condition, [string]$Message)
  if (-not $Condition) { throw $Message }
}

function Invoke-JsonRequest {
  param([string]$Method, [string]$Path, [object]$Body = $null)
  $params = @{
    Uri = "$base$Path"
    Method = $Method
    UseBasicParsing = $true
    TimeoutSec = $TimeoutSec
  }
  if ($null -ne $Body) {
    $params.ContentType = 'application/json; charset=utf-8'
    $params.Body = [System.Text.Encoding]::UTF8.GetBytes(
      ($Body | ConvertTo-Json -Depth 30 -Compress)
    )
  }
  $response = Invoke-WebRequest @params
  $response.Content | ConvertFrom-Json
}

$shieldStrike = [string][char]0x76FE + [string][char]0x51FB
$shieldPress = [string][char]0x76FE + [string][char]0x538B
$simulation = @{
  haste_level = 42087
  sequence = @($shieldStrike, $shieldPress)
  network_delay = 0
  attributes = @{
    base_attack = 38466.0
    weapon_damage = 10986.0
    crit_level = 54841.0
    crit_effect_level = 0.0
    overcome_level = 29480.0
    strain_level = 66031.0
    haste_level = 42087.0
  }
  target = @{
    level = 134
    defense_bonus = 0.0
    damage_cof = 0.0
  }
  initial_rage = 50
  tiegu_mode = 2
}

Write-Host '[1/6] Create offline Agent run'
$created = Invoke-JsonRequest 'POST' '/api/agent/runs' @{
  question = 'Analyze the deterministic baseline and explain the evidence boundary.'
  provider_profile = 'offline'
  simulation = $simulation
}
Assert-True ($created.schema_version -eq 'agent-run-created/v1') 'Unexpected create schema.'
Assert-True ($created.run_id -match '^run-[a-f0-9]+-[a-f0-9]+$') 'Unsafe run id.'
Assert-True ($created.session_id -match '^session-[a-f0-9]+-[a-f0-9]+$') 'Unsafe session id.'
Assert-True ($created.scenario_hash.Length -eq 64) 'Scenario hash is invalid.'

Write-Host '[2/6] Poll terminal status'
$status = $null
for ($i = 0; $i -lt 40; $i++) {
  $status = Invoke-JsonRequest 'GET' $created.status_url
  if (-not $status.running) { break }
  Start-Sleep -Milliseconds 100
}
Assert-True ($null -ne $status) 'Run status was not returned.'
Assert-True (-not $status.running) 'Offline Agent run did not finish.'
Assert-True ($status.status -eq 'completed') 'Offline Agent run did not complete.'
Assert-True ($status.result.report.evidence_ids.Count -ge 1) 'Grounded report evidence is missing.'
Assert-True ($status.result.accounting.tool_calls -eq 2) 'Unexpected tool-call count.'
Assert-True (@($status.result.trace | Where-Object {
  $_.kind -eq 'tool_started' -and $_.tool_name -eq 'analyze_timeline'
}).Count -eq 1) 'Rotation run did not execute exactly one baseline timeline diagnosis.'
Assert-True ($status.session_id -eq $created.session_id) 'Run status lost its session id.'
Assert-True (-not $status.persistence_error) 'Run reported a persistence failure.'

Write-Host '[3/6] Replay SSE after completion'
$stream = Invoke-WebRequest `
  -Uri "$base$($created.stream_url)" `
  -Method Get `
  -UseBasicParsing `
  -TimeoutSec $TimeoutSec
Assert-True ($stream.Headers.'Content-Type' -match '^text/event-stream') 'SSE content type is missing.'
Assert-True ($stream.Content -match 'event: planning') 'Planning event was not replayed.'
Assert-True ($stream.Content -match 'event: tool_started') 'Tool event was not replayed.'
Assert-True ($stream.Content -match 'event: run_result') 'Terminal result event was not replayed.'
Assert-True (-not ($stream.Content -match 'Authorization|api_key|hidden_reasoning')) 'Sensitive field leaked into SSE.'

Write-Host '[4/6] Restore persisted session'
$detailRaw = Invoke-WebRequest `
  -Uri "$base$($created.session_url)" `
  -Method Get `
  -UseBasicParsing `
  -TimeoutSec $TimeoutSec
$detail = $detailRaw.Content | ConvertFrom-Json
Assert-True ($detail.schema_version -eq 'agent-session-detail/v1') 'Unexpected session detail schema.'
Assert-True ($detail.summary.status -eq 'completed') 'Persisted session status is incorrect.'
Assert-True (@($detail.events | Where-Object { $_.kind -eq 'user_message' }).Count -eq 1) 'User message was not persisted once.'
Assert-True (@($detail.events | Where-Object { $_.kind -eq 'run_result' }).Count -eq 1) 'Run result was not persisted once.'
Assert-True ((($detail.events | Select-Object -ExpandProperty sequence) -join ',') -eq ((1..$detail.events.Count) -join ',')) 'Session event sequence is not continuous.'
Assert-True (-not ($detailRaw.Content -match 'Authorization|api_key|hidden_reasoning|provider_request|provider_response')) 'Sensitive field leaked into session.'
$sessions = Invoke-JsonRequest 'GET' '/api/agent/sessions'
Assert-True (@($sessions.sessions | Where-Object { $_.session_id -eq $created.session_id }).Count -eq 1) 'Session list is missing the new session.'

Write-Host '[5/6] Cancel is safe after terminal state'
$cancelled = Invoke-JsonRequest 'POST' $created.cancel_url @{}
Assert-True (-not $cancelled.accepted) 'Terminal run unexpectedly accepted cancellation.'
Assert-True ($cancelled.already_terminal) 'Terminal cancellation state is incorrect.'

Write-Host '[6/6] Reject unavailable run id'
$notFound = $false
try {
  Invoke-WebRequest `
    -Uri "$base/api/agent/runs/run-does-not-exist" `
    -Method Get `
    -UseBasicParsing `
    -TimeoutSec $TimeoutSec | Out-Null
} catch {
  $notFound = $_.Exception.Response.StatusCode -eq 404
}
Assert-True $notFound 'Unknown run id did not return 404.'

Write-Host '[OK] Agent Run/session API and SSE smoke test passed.'
Write-Host "     run=$($created.run_id) session=$($created.session_id) scenario=$($created.scenario_hash)"
