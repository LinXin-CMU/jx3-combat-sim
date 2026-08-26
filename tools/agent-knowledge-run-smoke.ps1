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
    $params.Body = [Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Depth 30 -Compress))
  }
  $response = Invoke-WebRequest @params
  $response.Content | ConvertFrom-Json
}

$shieldStrike = [string][char]0x76FE + [string][char]0x51FB
$shieldPress = [string][char]0x76FE + [string][char]0x538B
$question = -join @(
  [char]0x7ED3, [char]0x5408, [char]0x5F53, [char]0x524D, [char]0x7248,
  [char]0x672C, [char]0x653B, [char]0x7565, [char]0x8BF4, [char]0x660E,
  [char]0x5FAA, [char]0x73AF, [char]0x601D, [char]0x8DEF, [char]0x3002
)
$currentSeason = -join @(
  [char]0x6697, [char]0x5F71, [char]0x5343, [char]0x673A, [char]0xFF08,
  [char]0x0032, [char]0x0030, [char]0x0032, [char]0x0036, [char]0xFF09
)
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

Write-Host '[1/3] Create offline version-knowledge run'
$created = Invoke-JsonRequest 'POST' '/api/agent/runs' @{
  question = $question
  provider_profile = 'offline'
  simulation = $simulation
}

Write-Host '[2/3] Wait for grounded source report'
$status = $null
for ($i = 0; $i -lt 100; $i++) {
  $status = Invoke-JsonRequest 'GET' $created.status_url
  if (-not $status.running) { break }
  Start-Sleep -Milliseconds 60
}
Assert-True ($null -ne $status -and -not $status.running) 'Knowledge run did not terminate.'
Assert-True ($status.status -eq 'completed') 'Knowledge run did not complete.'
$result = $status.result
Assert-True ($result.prompt_version -eq 'agent-system/v5') 'Knowledge run used an unexpected prompt.'
Assert-True ($result.accounting.knowledge_searches -eq 1) 'Knowledge search count is incorrect.'
Assert-True ($result.accounting.simulations -eq 0) 'Knowledge-only run unexpectedly simulated.'
$sources = @($result.report.sources)
Assert-True ($sources.Count -ge 1) 'Structured knowledge sources are missing.'
Assert-True (@($sources | Where-Object { $_.season -ne $currentSeason }).Count -eq 0) 'A historical source crossed into current scope.'
Assert-True (@($sources | Where-Object { $_.version_match -ne 'current_exact' }).Count -eq 0) 'Source version labels are incorrect.'
Assert-True (@($sources | Where-Object { -not $_.fact_eligible }).Count -eq 0) 'Non-factual source entered the demo report.'
Assert-True (@($sources | Where-Object { $_.source_url -notmatch '^https?://' }).Count -eq 0) 'Unsafe source URL entered the report.'

Write-Host '[3/3] Restore persisted source cards'
$session = Invoke-JsonRequest 'GET' $created.session_url
$runResults = @($session.events | Where-Object { $_.kind -eq 'run_result' })
Assert-True ($runResults.Count -eq 1) 'Knowledge run result was not persisted exactly once.'
$persistedSources = @($runResults[0].result.report.sources)
Assert-True ($persistedSources.Count -eq $sources.Count) 'Persisted source cards differ from the run result.'

Write-Host '[OK] Offline Agent knowledge/source-card smoke test passed.'
Write-Host "     run=$($created.run_id) sources=$($sources.Count) corpus=$($sources[0].document_hash.Substring(0, 12))"
