param(
  [string]$BaseUrl = 'http://127.0.0.1:3005',
  [int]$TimeoutSec = 20
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

function Invoke-AgentTool {
  param(
    [string]$Path,
    [hashtable]$Payload
  )
  $json = $Payload | ConvertTo-Json -Depth 30 -Compress
  $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
  $response = Invoke-WebRequest `
    -Uri "$base$Path" `
    -Method Post `
    -ContentType 'application/json; charset=utf-8' `
    -Body $bytes `
    -UseBasicParsing `
    -TimeoutSec $TimeoutSec
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
}

Write-Host "[1/5] Capture scenario"
$captured = Invoke-AgentTool '/api/agent/tools/scenario' @{
  trace_id = 'agent-scenario-smoke'
  simulation = $simulation
}
Assert-True ($captured.evidence.tool_name -eq 'get_current_scenario') 'Unexpected scenario tool name.'
Assert-True ($captured.scenario.scenario_hash.Length -eq 64) 'Scenario hash is not SHA-256.'
Assert-True ($captured.evidence.data_hash.Length -eq 64) 'Runtime data hash is unavailable.'

Write-Host "[2/5] Simulate immutable scenario"
$simulated = Invoke-AgentTool '/api/agent/tools/simulate' @{
  trace_id = 'agent-simulate-smoke'
  scenario = $captured.scenario
}
Assert-True ($simulated.evidence.tool_name -eq 'simulate_scenario') 'Unexpected simulation tool name.'
Assert-True ($simulated.evidence.result.skill_count -gt 0) 'Agent simulation executed no skills.'
Assert-True ($simulated.evidence.scenario_hash -eq $captured.scenario.scenario_hash) 'Scenario hash changed during simulation.'

Write-Host "[3/5] Compare typed candidate"
$compared = Invoke-AgentTool '/api/agent/tools/compare' @{
  trace_id = 'agent-compare-smoke'
  baseline = $captured.scenario
  candidates = @(
    @{
      label = 'network-25ms'
      patch = @{ network_delay = 25 }
    }
  )
}
Assert-True ($compared.evidence.tool_name -eq 'compare_scenarios') 'Unexpected comparison tool name.'
Assert-True ($compared.evidence.result.candidates.Count -eq 1) 'Comparison did not return one candidate.'
Assert-True ($compared.evidence.result.candidates[0].changes[0].field -eq 'simulation.network_delay') 'Comparison diff is not explicit.'

Write-Host "[4/5] Analyze grounded timeline"
$analyzed = Invoke-AgentTool '/api/agent/tools/timeline' @{
  trace_id = 'agent-timeline-smoke'
  scenario = $captured.scenario
}
Assert-True ($analyzed.timeline.tool_name -eq 'analyze_timeline') 'Unexpected timeline tool name.'
Assert-True ($analyzed.timeline.result.active_event_count -gt 0) 'Timeline has no active events.'
Assert-True ($analyzed.simulation.evidence_id -eq $simulated.evidence.evidence_id) 'Repeated simulation evidence is not deterministic.'
Assert-True ($analyzed.timeline.args.source_evidence_id -eq $analyzed.simulation.evidence_id) 'Timeline evidence chain is broken.'

Write-Host "[5/5] Reject excessive endpoint budget"
$invalidBudget = @{
  trace_id = 'agent-budget-smoke'
  scenario = $captured.scenario
  max_simulations = 2
} | ConvertTo-Json -Depth 30 -Compress
try {
  Invoke-WebRequest `
    -Uri "$base/api/agent/tools/simulate" `
    -Method Post `
    -ContentType 'application/json; charset=utf-8' `
    -Body ([System.Text.Encoding]::UTF8.GetBytes($invalidBudget)) `
    -TimeoutSec $TimeoutSec | Out-Null
  throw 'Excessive budget was accepted.'
} catch {
  $response = $_.Exception.Response
  Assert-True ($null -ne $response) 'Budget rejection did not return an HTTP response.'
  Assert-True ([int]$response.StatusCode -eq 400) 'Budget rejection did not return HTTP 400.'
  $errorText = $_.ErrorDetails.Message
  if ([string]::IsNullOrWhiteSpace($errorText) -and
      $response.PSObject.Methods.Name -contains 'GetResponseStream') {
    $stream = $response.GetResponseStream()
    $reader = New-Object System.IO.StreamReader($stream, [System.Text.Encoding]::UTF8)
    try {
      $errorText = $reader.ReadToEnd()
    } finally {
      $reader.Dispose()
    }
  }
  Assert-True (-not [string]::IsNullOrWhiteSpace($errorText)) 'Budget rejection returned no JSON body.'
  $body = $errorText | ConvertFrom-Json
  Assert-True ($body.error.code -eq 'invalid_budget_limit') 'Budget rejection returned the wrong stable error code.'
}

Write-Host '[OK] Agent HTTP smoke test passed.' -ForegroundColor Green
Write-Host "     scenario=$($captured.scenario.scenario_hash) fingerprint=$($simulated.evidence.result.fingerprint_hex)"
