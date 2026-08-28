param(
  [int]$Port = 3021,
  [double]$MaxCostUsd = 0.20,
  [string]$VaultRoot = '',
  [string]$CasesPath = '',
  [string[]]$CaseId = @(),
  [switch]$KeepTemp
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$backendRoot = Join-Path $repoRoot 'backend'
$exe = (Resolve-Path -LiteralPath (Join-Path $backendRoot 'target\release\jx3-combat-sim.exe')).Path
$config = (Resolve-Path -LiteralPath (Join-Path $repoRoot 'agent.providers.toml')).Path
if ([string]::IsNullOrWhiteSpace($CasesPath)) {
  $CasesPath = Join-Path $backendRoot 'tests\agent_k5b_eval\cases.json'
}
$casesPath = (Resolve-Path -LiteralPath $CasesPath).Path
$base = "http://127.0.0.1:$Port"
$tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testRoot = [IO.Path]::GetFullPath((Join-Path $tempBase ("jx3-agent-k5b-{0}-{1}" -f $PID, [Guid]::NewGuid().ToString('N'))))
$server = $null

if ($MaxCostUsd -le 0 -or $MaxCostUsd -gt 0.70) {
  throw 'MaxCostUsd must be within (0, 0.70].'
}
if (Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue) {
  throw "Port $Port is already in use."
}
if (-not $testRoot.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Temporary path escaped the system temp directory.'
}
if ([string]::IsNullOrWhiteSpace($VaultRoot)) {
  $documentsRoot = Join-Path $env:USERPROFILE 'Documents'
  $manifests = @(Get-ChildItem -LiteralPath $documentsRoot -Filter '_migration-manifest.json' -Recurse -File -ErrorAction SilentlyContinue)
  if ($manifests.Count -ne 1) {
    throw "Expected exactly one knowledge manifest below $documentsRoot; found $($manifests.Count). Use -VaultRoot."
  }
  $VaultRoot = $manifests[0].DirectoryName
}
$knowledgeRoot = [IO.Path]::GetFullPath($VaultRoot)
if (-not (Test-Path -LiteralPath (Join-Path $knowledgeRoot '_migration-manifest.json') -PathType Leaf)) {
  throw 'Knowledge manifest is unavailable.'
}
$credential = [Environment]::GetEnvironmentVariable('JX3_DEEPSEEK_API_KEY', 'User')
if ([string]::IsNullOrWhiteSpace($credential)) {
  throw 'JX3_DEEPSEEK_API_KEY is unavailable at User scope.'
}

$profiles = @(
  [pscustomobject]@{ id = 'deepseek-v4-pro'; input_price = 0.435; output_price = 0.87 },
  [pscustomobject]@{ id = 'deepseek-v4-flash'; input_price = 0.14; output_price = 0.28 }
)
$allowedTools = @('get_current_scenario', 'search_knowledge_base', 'simulate_scenario', 'compare_scenarios', 'analyze_timeline')
$machineTerms = @('simulate_scenario', 'compare_scenarios', 'search_knowledge_base', 'json_pointer', 'evidence_id', 'fact_eligible', 'source_url', 'scenario_hash', 'engine_string')

function Assert-True {
  param([bool]$Condition, [string]$Message)
  if (-not $Condition) { throw $Message }
}

function Invoke-Json {
  param([string]$Method, [string]$Path, [object]$Body = $null)
  $params = @{ Uri = "$base$Path"; Method = $Method; UseBasicParsing = $true; TimeoutSec = 15 }
  if ($null -ne $Body) {
    $params.ContentType = 'application/json; charset=utf-8'
    $params.Body = [Text.Encoding]::UTF8.GetBytes(($Body | ConvertTo-Json -Depth 40 -Compress))
  }
  $response = Invoke-WebRequest @params
  $response.Content | ConvertFrom-Json
}

function Wait-AgentRun {
  param([object]$Created)
  for ($attempt = 0; $attempt -lt 150; $attempt++) {
    $status = Invoke-Json 'GET' $Created.status_url
    if (-not $status.running) { return $status }
    Start-Sleep -Milliseconds 500
  }
  throw "Run $($Created.run_id) exceeded the polling window."
}

function Get-ReportText {
  param([object]$Report)
  if ($null -eq $Report -or $null -eq $Report.content) { return '' }
  $parts = New-Object System.Collections.Generic.List[string]
  if ($Report.content.summary) { $parts.Add([string]$Report.content.summary) }
  foreach ($finding in @($Report.content.findings)) {
    if ($finding.title) { $parts.Add([string]$finding.title) }
    if ($finding.explanation) { $parts.Add([string]$finding.explanation) }
  }
  foreach ($item in @($Report.content.limitations)) { if ($item) { $parts.Add([string]$item) } }
  $parts -join "`n"
}

function Get-ExpressionProxy {
  param([object]$Result)
  $report = $Result.report
  $content = if ($null -ne $report) { $report.content } else { $null }
  $summary = if ($null -ne $content) { [string]$content.summary } else { '' }
  $findings = if ($null -ne $content) { @($content.findings) } else { @() }
  $limitations = if ($null -ne $content) { @($content.limitations | Where-Object { $_ }) } else { @() }
  $text = Get-ReportText $report
  $leaks = @($machineTerms | Where-Object { $text -match [regex]::Escape($_) })
  $checks = [ordered]@{
    conclusion_first = $summary.Trim().Length -ge 12 -and $summary.Trim().Length -le 180
    information_hierarchy = $findings.Count -ge 1 -and $findings.Count -le 4
    boundary_articulated = $limitations.Count -ge 1 -or $Result.status -in @('refused', 'evidence_insufficient')
    no_machine_leak = $leaks.Count -eq 0
    concise = $text.Length -ge 30 -and $text.Length -le 1200
  }
  $passed = @($checks.GetEnumerator() | Where-Object Value).Count
  [pscustomobject]@{
    score = $passed * 20
    checks = $checks
    leaked_terms = $leaks
    text_characters = $text.Length
    summary = $summary
    finding_titles = @($findings | ForEach-Object { $_.title })
    limitations = $limitations
  }
}

function Get-LayerScores {
  param([object]$Case, [object]$Result, [string[]]$Tools)
  $report = $Result.report
  $sources = if ($null -ne $report) { @($report.sources) } else { @() }
  $metrics = if ($null -ne $report) { @($report.content.findings | ForEach-Object { $_.metrics } | Where-Object { $null -ne $_ }) } else { @() }
  $evidence = if ($null -ne $report) { @($report.evidence_ids | Where-Object { $_ }) } else { @() }
  $domainTools = @($Tools | Where-Object { $_ -in @('simulate_scenario', 'compare_scenarios', 'analyze_timeline') })
  $blockedUnregisteredAttempt = $null -ne $Result.error -and $Result.error.code -eq 'unregistered_provider_tool'
  $toolBoundary = @($Tools | Where-Object { $_ -notin $allowedTools }).Count -eq 0 -and -not $blockedUnregisteredAttempt
  $planning = 0
  switch ($Case.mode) {
    'knowledge_only' { if ('search_knowledge_base' -in $Tools) { $planning += 70 }; if ($domainTools.Count -eq 0) { $planning += 20 }; if ($Tools.Count -le 3) { $planning += 10 } }
    'baseline' { if ($domainTools.Count -gt 0) { $planning += 70 }; if ('search_knowledge_base' -notin $Tools) { $planning += 10 }; if ($Tools.Count -le 3) { $planning += 20 } }
    'dual_evidence' {
      $knowledgeIndex = [array]::IndexOf($Tools, 'search_knowledge_base')
      $compareIndex = [array]::IndexOf($Tools, 'compare_scenarios')
      if ($knowledgeIndex -ge 0) { $planning += 40 }
      if ($compareIndex -ge 0) { $planning += 40 }
      if ($knowledgeIndex -ge 0 -and $compareIndex -gt $knowledgeIndex) { $planning += 10 }
      if ($Tools.Count -le 4) { $planning += 10 }
    }
    'historical' { if ('search_knowledge_base' -in $Tools) { $planning += 60 }; if ($domainTools.Count -eq 0) { $planning += 20 }; if ($Tools.Count -le 3) { $planning += 20 } }
    'no_answer' { if ('search_knowledge_base' -in $Tools) { $planning += 70 }; if ($domainTools.Count -eq 0) { $planning += 20 }; if ($Tools.Count -le 3) { $planning += 10 } }
    'refusal' { if ($Result.status -eq 'refused') { $planning += 60 }; if (@($Tools | Where-Object { $_ -ne 'get_current_scenario' }).Count -eq 0) { $planning += 40 } }
  }
  if (-not $toolBoundary) { $planning = 0 }

  $versionApplicable = $Case.mode -in @('knowledge_only', 'dual_evidence', 'historical')
  $version = $null
  if ($versionApplicable) {
    $version = 0
    if ($sources.Count -gt 0) { $version += 30 }
    if ($sources.Count -gt 0 -and @($sources | Where-Object { $_.season -ne $Case.expected_season }).Count -eq 0) { $version += 35 }
    if ($sources.Count -gt 0 -and @($sources | Where-Object { $_.version_match -ne $Case.expected_version_match }).Count -eq 0) { $version += 35 }
  }

  $evidenceScore = 0
  switch ($Case.mode) {
    'knowledge_only' { if ($sources.Count -gt 0) { $evidenceScore += 50 }; if ($metrics.Count -eq 0) { $evidenceScore += 30 }; if ($evidence.Count -gt 0) { $evidenceScore += 20 } }
    'baseline' { if ($metrics.Count -gt 0) { $evidenceScore += 50 }; if ($evidence.Count -gt 0) { $evidenceScore += 30 }; if ($sources.Count -eq 0) { $evidenceScore += 20 } }
    'dual_evidence' { if ($sources.Count -gt 0) { $evidenceScore += 30 }; if ($metrics.Count -gt 0) { $evidenceScore += 30 }; if ($evidence.Count -ge 2) { $evidenceScore += 30 }; if ('compare_scenarios' -in $Tools) { $evidenceScore += 10 } }
    'historical' { if ($sources.Count -gt 0) { $evidenceScore += 50 }; if ($metrics.Count -eq 0) { $evidenceScore += 30 }; if ($evidence.Count -gt 0) { $evidenceScore += 20 } }
    'no_answer' { if ($metrics.Count -eq 0) { $evidenceScore += 40 }; if ($sources.Count -eq 0) { $evidenceScore += 30 }; if ($Result.status -in @('refused', 'evidence_insufficient', 'completed', 'partially_verified')) { $evidenceScore += 30 } }
    'refusal' { if ($metrics.Count -eq 0) { $evidenceScore += 30 }; if ($sources.Count -eq 0) { $evidenceScore += 30 }; if ($Result.status -eq 'refused') { $evidenceScore += 40 } }
  }
  [pscustomobject]@{
    planning = $planning
    version = $version
    evidence = $evidenceScore
    tool_boundary = $toolBoundary
    source_count = $sources.Count
    metric_count = $metrics.Count
    evidence_count = $evidence.Count
  }
}

function Get-DomainLayerScores {
  param([object]$Case, [object]$Result, [string[]]$Tools)
  $report = $Result.report
  $sources = if ($null -ne $report) { @($report.sources) } else { @() }
  $evidence = if ($null -ne $report) { @($report.evidence_ids | Where-Object { $_ }) } else { @() }
  $text = Get-ReportText $report
  $playbook = @($Result.trace | Where-Object { $_.playbook_id } | Select-Object -First 1 -ExpandProperty playbook_id)
  $expectedAny = @($Case.expected_tools_any | Where-Object { $_ })
  $forbidden = @($Case.forbidden_tools | Where-Object { $_ })
  $requiredTermsAny = @($Case.required_terms_any | Where-Object { $_ })
  $requiresKnowledge = @($Case.required_dimensions) -contains 'versioned_knowledge'
  $routeMatch = $playbook.Count -eq 1 -and $playbook[0] -eq $Case.expected_playbook
  $expectedToolMatch = $expectedAny.Count -eq 0 -or @($Tools | Where-Object { $_ -in $expectedAny }).Count -gt 0
  $forbiddenToolMatch = @($Tools | Where-Object { $_ -in $forbidden }).Count -eq 0
  $termMatch = $requiredTermsAny.Count -eq 0 -or @($requiredTermsAny | Where-Object { $text.Contains([string]$_) }).Count -gt 0

  $planning = 0
  if ($routeMatch) { $planning += 60 }
  if ($expectedToolMatch) { $planning += 25 }
  if ($forbiddenToolMatch) { $planning += 15 }

  $version = $null
  if ($requiresKnowledge) {
    $version = 0
    if ($sources.Count -gt 0) { $version += 40 }
    if ($sources.Count -gt 0 -and @($sources | Where-Object { $_.season -ne '暗影千机（2026）' }).Count -eq 0) { $version += 30 }
    if ($sources.Count -gt 0 -and @($sources | Where-Object { $_.version_match -ne 'current_exact' }).Count -eq 0) { $version += 30 }
  }

  $evidenceScore = 0
  if ($evidence.Count -gt 0) { $evidenceScore += 35 }
  if (-not $requiresKnowledge -or $sources.Count -gt 0) { $evidenceScore += 25 }
  if ($Result.status -in @('completed', 'partially_verified')) { $evidenceScore += 20 }
  if ($termMatch) { $evidenceScore += 20 }

  [pscustomobject]@{
    planning = $planning
    version = $version
    evidence = $evidenceScore
    tool_boundary = $forbiddenToolMatch
    source_count = $sources.Count
    metric_count = if ($null -ne $report) { @($report.content.findings | ForEach-Object { $_.metrics } | Where-Object { $null -ne $_ }).Count } else { 0 }
    evidence_count = $evidence.Count
    playbook = if ($playbook.Count) { $playbook[0] } else { '' }
    route_match = $routeMatch
    expected_tool_match = $expectedToolMatch
    forbidden_tool_match = $forbiddenToolMatch
    required_term_match = $termMatch
  }
}

$fixture = Get-Content -LiteralPath $casesPath -Raw -Encoding UTF8 | ConvertFrom-Json
if ($CaseId.Count -gt 0) {
  $fixture.cases = @($fixture.cases | Where-Object { $_.id -in $CaseId })
  if ($fixture.cases.Count -eq 0) { throw 'No requested case id exists in the fixture.' }
}
$shieldStrike = [string][char]0x76FE + [string][char]0x51FB
$shieldPress = [string][char]0x76FE + [string][char]0x538B
$simulation = @{
  haste_level = 42087
  sequence = @($shieldStrike, $shieldPress)
  network_delay = 0
  attributes = @{ base_attack = 38466.0; weapon_damage = 10986.0; crit_level = 54841.0; crit_effect_level = 0.0; overcome_level = 29480.0; strain_level = 66031.0; haste_level = 42087.0 }
  target = @{ level = 134; defense_bonus = 0.0; damage_cof = 0.0 }
  initial_rage = 50
  tiegu_mode = 2
}

$names = @('JX3_BIND', 'JX3_PORT', 'JX3_NO_BROWSER', 'JX3_USERDATA_DIR', 'JX3_AGENT_CONFIG', 'JX3_KNOWLEDGE_ROOT', 'JX3_DEEPSEEK_API_KEY')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
New-Item -ItemType Directory -Path $testRoot -ErrorAction Stop | Out-Null
$stdout = Join-Path $testRoot 'backend.stdout.log'
$stderr = Join-Path $testRoot 'backend.stderr.log'
$results = New-Object System.Collections.Generic.List[object]
$knownCost = 0.0
$startedAt = Get-Date

try {
  try {
    [Environment]::SetEnvironmentVariable('JX3_BIND', '127.0.0.1', 'Process')
    [Environment]::SetEnvironmentVariable('JX3_PORT', [string]$Port, 'Process')
    [Environment]::SetEnvironmentVariable('JX3_NO_BROWSER', '1', 'Process')
    [Environment]::SetEnvironmentVariable('JX3_USERDATA_DIR', $testRoot, 'Process')
    [Environment]::SetEnvironmentVariable('JX3_AGENT_CONFIG', $config, 'Process')
    [Environment]::SetEnvironmentVariable('JX3_KNOWLEDGE_ROOT', $knowledgeRoot, 'Process')
    [Environment]::SetEnvironmentVariable('JX3_DEEPSEEK_API_KEY', $credential, 'Process')
    $server = Start-Process -FilePath $exe -WorkingDirectory $backendRoot -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
  } finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
    Remove-Variable credential -ErrorAction SilentlyContinue
  }
  $healthy = $false
  for ($attempt = 0; $attempt -lt 150; $attempt++) {
    if ($server.HasExited) { throw "Isolated backend exited with code $($server.ExitCode)." }
    try { if (Invoke-RestMethod -Uri "$base/health" -TimeoutSec 1) { $healthy = $true; break } } catch {}
    Start-Sleep -Milliseconds 100
  }
  Assert-True $healthy 'Isolated backend did not become healthy.'

  $total = $profiles.Count * $fixture.cases.Count
  foreach ($profile in $profiles) {
    foreach ($case in $fixture.cases) {
      if ($knownCost -ge $MaxCostUsd) { Write-Host "[STOP] Cost guard reached before $($profile.id)/$($case.id)."; break }
      Write-Host "[$($results.Count + 1)/$total] $($profile.id) / $($case.id)"
      $created = Invoke-Json 'POST' '/api/agent/runs' @{ question = $case.question; provider_profile = $profile.id; simulation = $simulation }
      $status = Wait-AgentRun $created
      $result = $status.result
      $accounting = $result.accounting
      $cost = (([double]$accounting.input_tokens * $profile.input_price) + ([double]$accounting.output_tokens * $profile.output_price)) / 1000000
      $knownCost += $cost
      $tools = @($result.trace | Where-Object { $_.kind -eq 'tool_finished' -and $_.tool_name } | Select-Object -ExpandProperty tool_name)
      $isDomainCase = $case.PSObject.Properties.Name -contains 'expected_playbook'
      $layers = if ($isDomainCase) { Get-DomainLayerScores $case $result $tools } else { Get-LayerScores $case $result $tools }
      $expression = Get-ExpressionProxy $result
      $applicableScores = @($layers.planning, $layers.evidence, $expression.score)
      if ($null -ne $layers.version) { $applicableScores += $layers.version }
      $overall = [Math]::Round((($applicableScores | Measure-Object -Average).Average), 1)
      $results.Add([pscustomobject]@{
        profile = $profile.id
        id = $case.id
        mode = $case.mode
        expected_playbook = $case.expected_playbook
        playbook = $layers.playbook
        route_match = $layers.route_match
        expected_tool_match = $layers.expected_tool_match
        forbidden_tool_match = $layers.forbidden_tool_match
        required_term_match = $layers.required_term_match
        run_id = $created.run_id
        prompt_version = $result.prompt_version
        status = $status.status
        error_code = $result.error.code
        tools = $tools
        planning_score = $layers.planning
        version_score = $layers.version
        evidence_score = $layers.evidence
        expression_proxy_score = $expression.score
        overall_score = $overall
        tool_boundary = $layers.tool_boundary
        source_count = $layers.source_count
        metric_count = $layers.metric_count
        evidence_count = $layers.evidence_count
        expression = $expression
        input_tokens = $accounting.input_tokens
        output_tokens = $accounting.output_tokens
        total_tokens = $accounting.total_tokens
        duration_ms = $accounting.duration_ms
        conservative_cost_usd = [Math]::Round($cost, 6)
        usage_unavailable = $accounting.model_turns -gt 0 -and $accounting.total_tokens -eq 0
        repair_requested = @($result.trace | Where-Object { $_.kind -eq 'report_repair_requested' }).Count -gt 0
        claims_sanitized = @($result.trace | Where-Object { $_.kind -eq 'report_claims_sanitized' }).Count -gt 0
      })
    }
  }

  $profileSummaries = @($profiles | ForEach-Object {
    $profileId = $_.id
    $items = @($results | Where-Object { $_.profile -eq $profileId })
    [ordered]@{
      profile = $profileId
      executed = $items.Count
      planning_average = [Math]::Round((($items | Measure-Object planning_score -Average).Average), 1)
      version_average = [Math]::Round((($items | Where-Object { $null -ne $_.version_score } | Measure-Object version_score -Average).Average), 1)
      evidence_average = [Math]::Round((($items | Measure-Object evidence_score -Average).Average), 1)
      expression_proxy_average = [Math]::Round((($items | Measure-Object expression_proxy_score -Average).Average), 1)
      overall_average = [Math]::Round((($items | Measure-Object overall_score -Average).Average), 1)
      tool_boundary_violations = @($items | Where-Object { -not $_.tool_boundary }).Count
      total_tokens = [long](($items | Measure-Object total_tokens -Sum).Sum)
      conservative_cost_usd = [Math]::Round((($items | Measure-Object conservative_cost_usd -Sum).Sum), 6)
      p50_ms = @($items.duration_ms | Sort-Object)[[Math]::Floor([Math]::Max(0, $items.Count - 1) * 0.50)]
      p95_ms = @($items.duration_ms | Sort-Object)[[Math]::Floor([Math]::Max(0, $items.Count - 1) * 0.95)]
    }
  })
  $report = [ordered]@{
    schema_version = 'agent-k5b-real-eval/v1'
    generated_at = (Get-Date).ToUniversalTime().ToString('o')
    prompt_versions = @($results | Select-Object -ExpandProperty prompt_version -Unique)
    corpus_hash = '8af06e3bf05c0eb90cf9934655f12700499b86ff221d6e5c238d225c727a579e'
    no_retry = $true
    max_cost_usd = $MaxCostUsd
    known_cost_usd = [Math]::Round($knownCost, 6)
    wall_ms = [Math]::Round(((Get-Date) - $startedAt).TotalMilliseconds)
    profiles = $profileSummaries
    results = $results
  }
  $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  $output = Join-Path $backendRoot "runs\agent-k5b-real-eval-$stamp.json"
  New-Item -ItemType Directory -Path (Split-Path -Parent $output) -Force | Out-Null
  $report | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $output -Encoding UTF8
  Write-Host '[DONE] K5B real-model layered evaluation finished.'
  foreach ($summary in $profileSummaries) {
    Write-Host "       $($summary.profile): planning=$($summary.planning_average) version=$($summary.version_average) evidence=$($summary.evidence_average) expression=$($summary.expression_proxy_average) overall=$($summary.overall_average) cost=$($summary.conservative_cost_usd)"
  }
  Write-Host "       combined_cost_usd=$($report.known_cost_usd) report=$output"
} finally {
  if ($server -and -not $server.HasExited) {
    $listener = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue | Where-Object { $_.OwningProcess -eq $server.Id }
    if ($listener -and ([IO.Path]::GetFullPath($server.Path) -eq [IO.Path]::GetFullPath($exe))) { Stop-Process -Id $server.Id -Force }
  }
  if (-not $KeepTemp -and (Test-Path -LiteralPath $testRoot)) {
    $resolved = [IO.Path]::GetFullPath($testRoot)
    if ($resolved.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -and (Split-Path -Leaf $resolved).StartsWith('jx3-agent-k5b-')) {
      Remove-Item -LiteralPath $resolved -Recurse -Force
    }
  } elseif ($KeepTemp) {
    Write-Host "       isolated_userdata=$testRoot"
  }
}
