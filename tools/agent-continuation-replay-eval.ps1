param(
  [string]$BaseUrl = 'http://127.0.0.1:3018',
  [string]$ReplayInput,
  [string]$OutputPath,
  [string]$SessionId = '',
  [string[]]$Questions = @('结合当前版本攻略，解释这套循环的核心思路。', '这套循环做得好的地方和最主要的问题是什么？', '继续', '跑', '继续'),
  [long]$MaxTotalTokens = 400000
)
$ErrorActionPreference = 'Stop'
function Api($method, $path, $body = $null) {
  $params = @{ Uri = "$BaseUrl$path"; Method = $method; TimeoutSec = 25 }
  if ($null -ne $body) {
    $params.ContentType = 'application/json; charset=utf-8'
    $params.Body = [Text.Encoding]::UTF8.GetBytes(($body | ConvertTo-Json -Depth 90 -Compress))
  }
  Invoke-RestMethod @params
}
$source = Get-Content -Raw -Encoding UTF8 -LiteralPath $ReplayInput | ConvertFrom-Json
$simulation = $source.payload.scenario.simulation
if (-not $simulation) { throw 'Replay input has no frozen simulation.' }
$session = $SessionId
$total = 0L
$rows = @()
foreach ($question in $questions) {
  if ($total -ge $MaxTotalTokens) { break }
  $body = @{question=$question;provider_profile='deepseek-v4-flash';simulation=$simulation}
  if ($session) { $body.session_id = $session }
  $created = Api 'POST' '/api/agent/runs' $body
  $session = $created.session_id
  $deadline = (Get-Date).AddSeconds(660)
  do {
    Start-Sleep -Milliseconds 500
    $status = Api 'GET' $created.status_url
    if ((Get-Date) -gt $deadline) { throw 'Polling exceeded expected task deadline.' }
  } while ($status.running)
  $result = $status.result
  $total += [long]$result.accounting.total_tokens
  $row = [pscustomobject]@{
    question=$question;session_id=$session;run_id=$result.run_id;status=$result.status
    prompt=$result.prompt_version;accounting=$result.accounting
    restored=@($result.debug.tool_calls|Where-Object {$_.call_id -like 'resume-*'}).Count
    comparisons=@($result.debug.tool_calls|Where-Object {$_.tool_name -eq 'compare_scenarios'}).Count
    calls=$result.debug.tool_calls;report=$result.report;clarification=$result.clarification;error=$result.error
  }
  $rows += $row
  # Evaluation artifacts only; no provider transcripts or credentials.
  $rows | ConvertTo-Json -Depth 90 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
  [pscustomobject]@{question=$question;status=$row.status;seconds=[math]::Round($row.accounting.duration_ms/1000,1);restored=$row.restored;comparisons=$row.comparisons;tokens=$total} | ConvertTo-Json -Compress
}
