param(
  [string]$BaseUrl = 'http://127.0.0.1:3005',
  [int]$TimeoutSec = 30
)

$ErrorActionPreference = 'Stop'
$base = $BaseUrl.TrimEnd('/')
$repoRoot = Split-Path -Parent $PSScriptRoot
$casesPath = Join-Path $repoRoot 'backend\tests\agent_model_eval\cases.json'

function Assert-True {
  param([bool]$Condition, [string]$Message)
  if (-not $Condition) { throw $Message }
}

function Invoke-Json {
  param([string]$Method, [string]$Path, [object]$Body = $null)
  $params = @{
    Uri = "$base$Path"
    Method = $Method
    UseBasicParsing = $true
    TimeoutSec = $TimeoutSec
  }
  if ($null -ne $Body) {
    $params.ContentType = 'application/json; charset=utf-8'
    $params.Body = [System.Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Depth 40 -Compress))
  }
  $response = Invoke-WebRequest @params
  $response.Content | ConvertFrom-Json
}

function Invoke-ExpectedError {
  param([string]$Method, [string]$Path, [object]$Body, [int]$Status, [string]$Code)
  try {
    Invoke-Json $Method $Path $Body | Out-Null
    throw "Expected HTTP $Status $Code."
  } catch {
    $errorRecord = $_
    $response = $_.Exception.Response
    Assert-True ($null -ne $response) "Missing error response for $Code."
    Assert-True ([int]$response.StatusCode -eq $Status) "Unexpected HTTP status for $Code."
    if (-not [string]::IsNullOrWhiteSpace($errorRecord.ErrorDetails.Message)) {
      $payload = $errorRecord.ErrorDetails.Message | ConvertFrom-Json
    } elseif ($response.PSObject.Properties.Name -contains 'Content') {
      $payload = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-Json
    } else {
      $reader = New-Object System.IO.StreamReader($response.GetResponseStream())
      try { $payload = $reader.ReadToEnd() | ConvertFrom-Json } finally { $reader.Dispose() }
    }
    Assert-True ($payload.error.code -eq $Code) "Unexpected error code; wanted $Code."
  }
}

function Wait-AgentRun {
  param([object]$Created)
  for ($i = 0; $i -lt 100; $i++) {
    $status = Invoke-Json 'GET' $Created.status_url
    if (-not $status.running) { return $status }
    Start-Sleep -Milliseconds 60
  }
  throw "Run $($Created.run_id) did not terminate."
}

function Start-AgentCase {
  param([string]$Question, [object]$Simulation, [string]$SessionId = '')
  $body = @{
    question = $Question
    provider_profile = 'offline'
    simulation = $Simulation
  }
  if ($SessionId) { $body.session_id = $SessionId }
  $created = Invoke-Json 'POST' '/api/agent/runs' $body
  $status = Wait-AgentRun $created
  Assert-True ($status.status -eq 'completed') "Run $($created.run_id) did not complete."
  Assert-True (-not $status.persistence_error) "Run $($created.run_id) was not persisted."
  $toolNames = @($status.result.trace | Where-Object { $_.kind -eq 'tool_started' } | Select-Object -ExpandProperty tool_name)
  $scenarioCalls = @($toolNames | Where-Object { $_ -eq 'get_current_scenario' }).Count
  $simulationCalls = @($toolNames | Where-Object { $_ -eq 'simulate_scenario' }).Count
  $timelineCalls = @($toolNames | Where-Object { $_ -eq 'analyze_timeline' }).Count
  $baselineCalls = $simulationCalls + $timelineCalls
  $knowledgeCalls = @($toolNames | Where-Object { $_ -eq 'search_knowledge_base' }).Count
  $unexpectedCalls = @($toolNames | Where-Object { $_ -notin @('get_current_scenario', 'simulate_scenario', 'analyze_timeline', 'search_knowledge_base') })
  Assert-True ($status.result.accounting.tool_calls -eq $toolNames.Count) "Run $($created.run_id) tool accounting does not match its trace."
  Assert-True ($scenarioCalls -eq 1) "Run $($created.run_id) did not capture exactly one scenario."
  Assert-True ($baselineCalls -eq 1) "Run $($created.run_id) did not execute exactly one deterministic baseline or timeline diagnosis."
  Assert-True ($knowledgeCalls -le 1) "Run $($created.run_id) repeated its optional knowledge prefetch."
  Assert-True ($unexpectedCalls.Count -eq 0) "Run $($created.run_id) crossed the offline read-only tool boundary."
  Assert-True ($status.result.accounting.simulations -eq 1) "Run $($created.run_id) used an unexpected simulation count."
  Assert-True ($status.result.report.evidence_ids.Count -ge 1) "Run $($created.run_id) has no grounded evidence."
  Assert-True ($status.result.report.evidence_ids -contains $status.result.report.content.findings[0].metrics[0].evidence_id) "Metric evidence is disconnected."
  return @{ created = $created; status = $status }
}

$fixture = Get-Content -LiteralPath $casesPath -Raw -Encoding UTF8 | ConvertFrom-Json
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
  target = @{ level = 134; defense_bonus = 0.0; damage_cof = 0.0 }
  initial_rage = 50
  tiegu_mode = 2
}

$startedAt = Get-Date
$durations = New-Object System.Collections.Generic.List[double]
$categoryCounts = @{}
$firstSession = ''
$lastRun = ''

Write-Host "[1/4] Run $($fixture.cases.Count) grounded language/security cases"
foreach ($case in $fixture.cases) {
  $result = Start-AgentCase $case.question $simulation
  $durations.Add([double]$result.status.result.accounting.duration_ms)
  $categoryCounts[$case.category] = 1 + [int]($categoryCounts[$case.category])
  if (-not $firstSession) { $firstSession = $result.created.session_id }
  $lastRun = $result.created.run_id
}

Write-Host '[2/4] Continue one persisted session for three turns'
$continued = Start-AgentCase 'Continue this session and verify the same deterministic evidence.' $simulation $firstSession
$continuedAgain = Start-AgentCase 'Third turn: summarize only grounded evidence.' $simulation $firstSession
$durations.Add([double]$continued.status.result.accounting.duration_ms)
$durations.Add([double]$continuedAgain.status.result.accounting.duration_ms)
$detail = Invoke-Json 'GET' "/api/agent/sessions/$firstSession"
$userEvents = @($detail.events | Where-Object { $_.kind -eq 'user_message' })
$resultEvents = @($detail.events | Where-Object { $_.kind -eq 'run_result' })
$startedEvents = @($detail.events | Where-Object { $_.kind -eq 'run_started' })
Assert-True ($userEvents.Count -eq 3) 'Long session did not retain three user turns.'
Assert-True ($resultEvents.Count -eq 3) 'Long session did not retain three run results.'
Assert-True ($startedEvents[1].parent_run_id -eq $startedEvents[0].run_id) 'Second run parent link is invalid.'
Assert-True ($startedEvents[2].parent_run_id -eq $startedEvents[1].run_id) 'Third run parent link is invalid.'
Assert-True ((($detail.events | Select-Object -ExpandProperty sequence) -join ',') -eq ((1..$detail.events.Count) -join ',')) 'Session event sequence is not continuous.'

Write-Host '[3/4] Reject credential, unknown provider and unknown session'
Invoke-ExpectedError 'POST' '/api/agent/runs' @{
  question = 'api_key=abcdefghijklmnop'
  provider_profile = 'offline'
  simulation = $simulation
} 400 'sensitive_input_rejected'
Invoke-ExpectedError 'POST' '/api/agent/runs' @{
  question = 'Analyze the current scenario.'
  provider_profile = 'not-configured'
  simulation = $simulation
} 400 'provider_not_found'
Invoke-ExpectedError 'POST' '/api/agent/runs' @{
  question = 'Continue a missing session.'
  provider_profile = 'offline'
  session_id = 'session-does-not-exist'
  simulation = $simulation
} 404 'agent_session_not_found'
Invoke-ExpectedError 'POST' '/api/agent/runs' @{
  question = 'Reject an unsafe session identifier.'
  provider_profile = 'offline'
  session_id = '..'
  simulation = $simulation
} 404 'agent_session_not_found'

Write-Host '[4/4] Summarize reproducible offline metrics'
$sorted = @($durations | Sort-Object)
$p50 = $sorted[[Math]::Floor(($sorted.Count - 1) * 0.50)]
$p95 = $sorted[[Math]::Floor(($sorted.Count - 1) * 0.95)]
$sessions = Invoke-Json 'GET' '/api/agent/sessions'
Assert-True ($sessions.sessions.Count -ge $fixture.cases.Count) 'Persisted session list is incomplete.'
$elapsed = ((Get-Date) - $startedAt).TotalMilliseconds

Write-Host '[OK] Agent offline model-layer evaluation passed.'
Write-Host "     cases=$($fixture.cases.Count + 2)/$($fixture.cases.Count + 2) evidence_citation=100% tool_boundary=100%"
Write-Host "     p50_ms=$p50 p95_ms=$p95 wall_ms=$([Math]::Round($elapsed)) tokens=0 cost=0"
Write-Host "     sessions=$($sessions.sessions.Count) long_session=$firstSession last_run=$lastRun"
