param(
  [string]$BaseUrl = 'http://127.0.0.1:3005',
  [string]$ProviderProfile = 'deepseek-v4-flash',
  [int]$MaxCases = 12,
  [string[]]$CaseIds = @(),
  [long]$MaxTotalTokens = 1500000,
  [string]$OutputPath = ''
)

$ErrorActionPreference = 'Stop'
$base = $BaseUrl.TrimEnd('/')
$repoRoot = Split-Path -Parent $PSScriptRoot
$casesPath = Join-Path $repoRoot 'backend\tests\agent_diagnostic_eval\cases.json'
$scenarioPath = Join-Path $repoRoot 'backend\tests\agent_diagnostic_eval\scenario.json'
if (-not $OutputPath) {
  $OutputPath = Join-Path $repoRoot 'backend\runs\agent-diagnostic-real-eval-latest.json'
}

function Invoke-Json {
  param([string]$Method, [string]$Path, [object]$Body = $null)
  $params = @{
    Uri = "$base$Path"
    Method = $Method
    UseBasicParsing = $true
    TimeoutSec = 20
  }
  if ($null -ne $Body) {
    $params.ContentType = 'application/json; charset=utf-8'
    $params.Body = [Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Depth 80 -Compress))
  }
  $response = Invoke-WebRequest @params
  $response.Content | ConvertFrom-Json
}

function Wait-AgentRun {
  param([object]$Created)
  for ($attempt = 0; $attempt -lt 450; $attempt++) {
    $status = Invoke-Json 'GET' $Created.status_url
    if (-not $status.running) { return $status }
    Start-Sleep -Milliseconds 500
  }
  throw "Run $($Created.run_id) exceeded the polling window."
}

function Contains-Any {
  param([string]$Text, [object[]]$Signals)
  foreach ($signal in @($Signals)) {
    if ($Text.Contains([string]$signal)) { return $true }
  }
  return $false
}

$suite = Get-Content -LiteralPath $casesPath -Raw -Encoding UTF8 | ConvertFrom-Json
$fixture = Get-Content -LiteralPath $scenarioPath -Raw -Encoding UTF8 | ConvertFrom-Json
$simulation = $fixture.simulation
$simulation.sequence = @('__macro__') * [int]$fixture.macro_slots
$allowedTools = @(
  'get_current_scenario', 'ask_user_question', 'search_knowledge_base',
  'simulate_scenario', 'analyze_timeline', 'inspect_timeline_events',
  'inspect_rotation_input', 'compare_scenarios', 'list_saved_artifacts',
  'read_saved_artifact', 'compare_saved_macros', 'compare_saved_scenarios',
  'inspect_equipment_workspace', 'compare_focused_equipment',
  'search_equipment_catalog', 'compare_equipment_strategies'
)

$results = New-Object System.Collections.Generic.List[object]
$tokenTotal = 0L
$startedAt = Get-Date
$selectedCases = if ($CaseIds.Count -gt 0) {
  @($suite.cases | Where-Object { $_.id -in $CaseIds } | Select-Object -First $MaxCases)
} else {
  @($suite.cases | Select-Object -First $MaxCases)
}
foreach ($case in $selectedCases) {
  if ($tokenTotal -ge $MaxTotalTokens) {
    Write-Host "[STOP] Token guard reached before $($case.id)."
    break
  }
  Write-Host "[$($results.Count + 1)/$($selectedCases.Count)] $($case.id)"
  $created = Invoke-Json 'POST' '/api/agent/runs' @{
    question = $case.question
    provider_profile = $ProviderProfile
    simulation = $simulation
  }
  $status = Wait-AgentRun $created
  $result = $status.result
  $accounting = $result.accounting
  $tokenTotal += [long]$accounting.total_tokens
  $toolCalls = @($result.debug.tool_calls)
  $toolNames = @($toolCalls | Select-Object -ExpandProperty tool_name)
  $unexpectedTools = @($toolNames | Where-Object { $_ -notin $allowedTools })
  $expectedAll = @($case.expected_tools_all | Where-Object { $null -ne $_ -and ([string]$_).Length -gt 0 })
  $expectedAny = @($case.expected_tools_any | Where-Object { $null -ne $_ -and ([string]$_).Length -gt 0 })
  $hasExpectedAll = @($expectedAll | Where-Object { $_ -notin $toolNames }).Count -eq 0
  $hasExpectedAny = $expectedAny.Count -eq 0 -or @($expectedAny | Where-Object { $_ -in $toolNames }).Count -gt 0
  $answerText = if ($result.report) {
    $result.report.content | ConvertTo-Json -Depth 20 -Compress
  } elseif ($result.clarification) {
    $result.clarification | ConvertTo-Json -Depth 10 -Compress
  } else {
    ''
  }
  $answerSignals = @($case.answer_signals_any | Where-Object { $null -ne $_ -and ([string]$_).Length -gt 0 })
  $hasAnswerSignal = $answerSignals.Count -eq 0 -or (Contains-Any $answerText $answerSignals)
  $acceptableStatus = $result.status -in @('completed', 'partially_verified', 'needs_user_input')
  $processPass = $acceptableStatus -and $unexpectedTools.Count -eq 0 -and
    $hasExpectedAll -and $hasExpectedAny -and $hasAnswerSignal
  $duplicateReuses = @($toolCalls | Where-Object reused).Count
  $results.Add([pscustomobject]@{
    id = $case.id
    question = $case.question
    run_id = $created.run_id
    session_id = $created.session_id
    status = $result.status
    process_pass = $processPass
    human_review_required = $true
    tools = $toolNames
    missing_expected_tools = @($expectedAll | Where-Object { $_ -notin $toolNames })
    unexpected_tools = $unexpectedTools
    duplicate_reuses = $duplicateReuses
    model_turns = $accounting.model_turns
    tool_calls = $accounting.tool_calls
    simulations = $accounting.simulations
    knowledge_searches = $accounting.knowledge_searches
    target_model_turns = [int]$case.max_model_turns_target
    target_tool_calls = [int]$case.max_tool_calls_target
    within_turn_target = $accounting.model_turns -le [int]$case.max_model_turns_target
    within_tool_target = $accounting.tool_calls -le [int]$case.max_tool_calls_target
    duration_ms = $accounting.duration_ms
    input_tokens = $accounting.input_tokens
    output_tokens = $accounting.output_tokens
    total_tokens = $accounting.total_tokens
    prompt_version = $result.prompt_version
    error = $result.error
    diagnostic_state = $result.debug.diagnostic_state
    trace = $result.trace
    report = $result.report
    clarification = $result.clarification
  })
}

$report = [ordered]@{
  schema_version = 'agent-diagnostic-real-eval/v1'
  generated_at = (Get-Date).ToUniversalTime().ToString('o')
  provider_profile = $ProviderProfile
  scenario_fixture = 'agent_diagnostic_eval/scenario.json'
  summary = [ordered]@{
    executed = $results.Count
    process_passed = @($results | Where-Object process_pass).Count
    process_failed = @($results | Where-Object { -not $_.process_pass }).Count
    within_turn_target = @($results | Where-Object within_turn_target).Count
    within_tool_target = @($results | Where-Object within_tool_target).Count
    duplicate_reuses = [int](($results | Measure-Object duplicate_reuses -Sum).Sum)
    total_tokens = [long](($results | Measure-Object total_tokens -Sum).Sum)
    total_duration_ms = [long](($results | Measure-Object duration_ms -Sum).Sum)
    wall_ms = [long]((Get-Date) - $startedAt).TotalMilliseconds
    note = 'process_pass only checks tool/evidence plumbing; gameplay judgment still requires human review'
  }
  results = $results
}

$outputDirectory = Split-Path -Parent $OutputPath
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$report | ConvertTo-Json -Depth 80 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
Write-Host '[DONE] Dynamic diagnostic evaluation finished.'
Write-Host "       process=$($report.summary.process_passed)/$($report.summary.executed) turns=$($report.summary.within_turn_target)/$($report.summary.executed) tools=$($report.summary.within_tool_target)/$($report.summary.executed)"
Write-Host "       tokens=$($report.summary.total_tokens) wall_ms=$($report.summary.wall_ms)"
Write-Host "       report=$OutputPath"
