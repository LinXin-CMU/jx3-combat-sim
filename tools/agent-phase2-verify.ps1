param(
  [int]$Port = 3017,
  [string]$VaultRoot = '',
  [switch]$KeepTemp
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$backendRoot = Join-Path $repoRoot 'backend'
$realUserdata = Join-Path $backendRoot 'userdata'
$baseUrl = "http://127.0.0.1:$Port"
$tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$testRoot = Join-Path $tempBase ("jx3-agent-phase2-{0}-{1}" -f $PID, [Guid]::NewGuid().ToString('N'))
$testRootFull = [System.IO.Path]::GetFullPath($testRoot)
$isolatedTarget = Join-Path $testRootFull 'target'
$exe = Join-Path $isolatedTarget 'release\jx3-combat-sim.exe'
$server = $null

if ([string]::IsNullOrWhiteSpace($VaultRoot)) {
  $documentsRoot = Join-Path $env:USERPROFILE 'Documents'
  $manifests = @(Get-ChildItem -LiteralPath $documentsRoot -Filter '_migration-manifest.json' -Recurse -File -ErrorAction SilentlyContinue)
  if ($manifests.Count -ne 1) {
    throw "Expected exactly one knowledge manifest below $documentsRoot; found $($manifests.Count). Use -VaultRoot."
  }
  $VaultRoot = $manifests[0].DirectoryName
}
$knowledgeRootFull = [IO.Path]::GetFullPath($VaultRoot)
if (-not (Test-Path -LiteralPath (Join-Path $knowledgeRootFull '_migration-manifest.json') -PathType Leaf)) {
  throw "Knowledge manifest not found below $knowledgeRootFull."
}

function Get-TreeFingerprint {
  param([string]$Root)
  $resolved = (Resolve-Path -LiteralPath $Root).Path
  $rows = Get-ChildItem -LiteralPath $resolved -File -Recurse -Force | Sort-Object FullName | ForEach-Object {
    $relative = $_.FullName.Substring($resolved.Length).TrimStart('\').Replace('\', '/')
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant()
    "$relative`t$($_.Length)`t$hash"
  }
  $sha = [System.Security.Cryptography.SHA256]::Create()
  try {
    $bytes = [System.Text.Encoding]::UTF8.GetBytes(($rows -join "`n"))
    return ([System.BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
  } finally {
    $sha.Dispose()
  }
}

function Invoke-Step {
  param([string]$Name, [scriptblock]$Action)
  Write-Host $Name
  & $Action
  if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit code $LASTEXITCODE." }
}

if (-not $testRootFull.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw 'Temporary path escaped the system temp directory.'
}
if (Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue) {
  throw "Port $Port is already in use."
}

$beforeFingerprint = Get-TreeFingerprint $realUserdata
New-Item -ItemType Directory -Path $testRootFull -ErrorAction Stop | Out-Null
$stdout = Join-Path $testRootFull 'backend.stdout.log'
$stderr = Join-Path $testRootFull 'backend.stderr.log'

try {
  Invoke-Step '[1/10] Rust tests' {
    Push-Location $backendRoot
    try { & cargo test } finally { Pop-Location }
  }
  Invoke-Step '[2/10] Fixed knowledge retrieval evaluation' {
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'agent-knowledge-eval.ps1') -VaultRoot $knowledgeRootFull
  }
  Invoke-Step '[3/10] Frontend syntax' {
    & node --check (Join-Path $repoRoot 'frontend\app.js')
    if ($LASTEXITCODE -eq 0) { & node --check (Join-Path $repoRoot 'frontend\agent.js') }
  }
  Invoke-Step '[4/10] Release build' {
    Push-Location $backendRoot
    $previousTarget = [Environment]::GetEnvironmentVariable('CARGO_TARGET_DIR', 'Process')
    [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $isolatedTarget, 'Process')
    try {
      & cargo build --release
    } finally {
      [Environment]::SetEnvironmentVariable('CARGO_TARGET_DIR', $previousTarget, 'Process')
      Pop-Location
    }
  }

  $previousBind = [Environment]::GetEnvironmentVariable('JX3_BIND', 'Process')
  $previousPort = [Environment]::GetEnvironmentVariable('JX3_PORT', 'Process')
  $previousUserdata = [Environment]::GetEnvironmentVariable('JX3_USERDATA_DIR', 'Process')
  $previousNoBrowser = [Environment]::GetEnvironmentVariable('JX3_NO_BROWSER', 'Process')
  $previousKnowledgeRoot = [Environment]::GetEnvironmentVariable('JX3_KNOWLEDGE_ROOT', 'Process')
  [Environment]::SetEnvironmentVariable('JX3_BIND', '127.0.0.1', 'Process')
  [Environment]::SetEnvironmentVariable('JX3_PORT', [string]$Port, 'Process')
  [Environment]::SetEnvironmentVariable('JX3_USERDATA_DIR', $testRootFull, 'Process')
  [Environment]::SetEnvironmentVariable('JX3_NO_BROWSER', '1', 'Process')
  [Environment]::SetEnvironmentVariable('JX3_KNOWLEDGE_ROOT', $knowledgeRootFull, 'Process')
  try {
    $server = Start-Process -FilePath $exe -WorkingDirectory $backendRoot -WindowStyle Hidden `
      -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
  } finally {
    [Environment]::SetEnvironmentVariable('JX3_BIND', $previousBind, 'Process')
    [Environment]::SetEnvironmentVariable('JX3_PORT', $previousPort, 'Process')
    [Environment]::SetEnvironmentVariable('JX3_USERDATA_DIR', $previousUserdata, 'Process')
    [Environment]::SetEnvironmentVariable('JX3_NO_BROWSER', $previousNoBrowser, 'Process')
    [Environment]::SetEnvironmentVariable('JX3_KNOWLEDGE_ROOT', $previousKnowledgeRoot, 'Process')
  }
  for ($i = 0; $i -lt 150; $i++) {
    if ($server.HasExited) { throw "Isolated backend exited early. See $stderr" }
    try {
      $health = Invoke-RestMethod -Uri "$baseUrl/health" -TimeoutSec 1
      if ($health) { break }
    } catch {}
    Start-Sleep -Milliseconds 100
  }
  if (-not $health) { throw 'Isolated backend did not become healthy.' }

  Invoke-Step '[5/10] Phase 1 deterministic 20-case evaluation' {
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'agent-eval.ps1') -BaseUrl $baseUrl
  }
  Invoke-Step '[6/10] Run/session/SSE smoke' {
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'agent-run-smoke.ps1') -BaseUrl $baseUrl
  }
  Invoke-Step '[7/10] Phase 2 offline model-layer evaluation' {
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'agent-model-eval.ps1') -BaseUrl $baseUrl
  }
  Invoke-Step '[8/10] Knowledge/source-card Agent smoke' {
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'agent-knowledge-run-smoke.ps1') -BaseUrl $baseUrl
  }
  Invoke-Step '[9/10] Diff and credential hygiene' {
    Push-Location $repoRoot
    try {
      & git diff --check
      if ($LASTEXITCODE -ne 0) { return }
      $candidateFiles = git ls-files | Where-Object { $_ -match '\.(rs|js|html|css|md|ps1|py|toml|json)$' }
      $pattern = '(?i)(ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|sk-[A-Za-z0-9_-]{20,}|Bearer\s+[A-Za-z0-9._-]{20,})'
      $hits = Select-String -LiteralPath $candidateFiles -Pattern $pattern -ErrorAction SilentlyContinue
      if ($hits) { throw "High-confidence credential pattern found in tracked files ($($hits.Count) hit(s))." }
    } finally { Pop-Location }
  }
  Write-Host '[10/10] Real userdata write protection'
  $afterFingerprint = Get-TreeFingerprint $realUserdata
  if ($afterFingerprint -ne $beforeFingerprint) { throw 'Real userdata changed during isolated verification.' }

  Write-Host '[OK] Agent offline + knowledge verification passed.'
  Write-Host '     rust=passed retrieval=20/20 phase1=20/20 phase2=12/12 knowledge_smoke=passed real_userdata_unchanged=true'
} finally {
  if ($server -and -not $server.HasExited) {
    $listener = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue |
      Where-Object { $_.OwningProcess -eq $server.Id }
    if ($listener -and $server.Path -eq $exe) { Stop-Process -Id $server.Id -Force }
  }
  if (-not $KeepTemp -and (Test-Path -LiteralPath $testRootFull)) {
    $resolvedTestRoot = [System.IO.Path]::GetFullPath($testRootFull)
    if ($resolvedTestRoot.StartsWith($tempBase, [System.StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $resolvedTestRoot).StartsWith('jx3-agent-phase2-')) {
      Remove-Item -LiteralPath $resolvedTestRoot -Recurse -Force
    }
  } elseif ($KeepTemp) {
    Write-Host "     isolated_userdata=$testRootFull"
  }
}
