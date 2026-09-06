param(
  [int]$Port = 3005,
  [string]$ExePath = '',
  [string]$ConfigPath = '',
  [string]$UserdataPath = '',
  [string]$KnowledgeRoot = '',
  [string]$KnowledgeCachePath = '',
  [ValidateSet('embedded', 'bm25')]
  [string]$KnowledgeRetrieval = 'embedded',
  [string]$EmbeddingEndpoint = 'https://huggingface.co',
  [string]$ApiKeyEnv = 'JX3_DEEPSEEK_API_KEY',
  [ValidateRange(5, 600)]
  [int]$StartupTimeoutSec = 240
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$backendRoot = Join-Path $repoRoot 'backend'
$exe = if ($ExePath) { $ExePath } else { Join-Path $backendRoot 'target\release\jx3-combat-sim.exe' }
if (-not $ConfigPath) { $ConfigPath = Join-Path $repoRoot 'agent.providers.toml' }
if (-not $UserdataPath) { $UserdataPath = Join-Path $backendRoot 'userdata' }
if (-not $KnowledgeRoot) {
  $documentsRoot = Join-Path $env:USERPROFILE 'Documents'
  $manifests = @(Get-ChildItem -LiteralPath $documentsRoot -Filter '_migration-manifest.json' -Recurse -File -ErrorAction SilentlyContinue)
  if ($manifests.Count -eq 1) {
    $KnowledgeRoot = $manifests[0].DirectoryName
  } elseif ($manifests.Count -gt 1) {
    throw "Found multiple knowledge manifests below $documentsRoot. Use -KnowledgeRoot."
  }
}

if (Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue) {
  throw "Port $Port is already in use."
}
$resolvedExe = (Resolve-Path -LiteralPath $exe).Path
$resolvedConfig = (Resolve-Path -LiteralPath $ConfigPath).Path
$resolvedUserdata = (Resolve-Path -LiteralPath $UserdataPath).Path
$resolvedKnowledge = if ($KnowledgeRoot) { (Resolve-Path -LiteralPath $KnowledgeRoot).Path } else { $null }
$knowledgeCache = if ($KnowledgeCachePath) {
  [IO.Path]::GetFullPath($KnowledgeCachePath)
} else {
  Join-Path $resolvedUserdata 'knowledge_index\v1'
}
New-Item -ItemType Directory -Path $knowledgeCache -Force | Out-Null
$credential = [Environment]::GetEnvironmentVariable($ApiKeyEnv, 'User')
if ([string]::IsNullOrWhiteSpace($credential)) {
  throw "Credential environment variable $ApiKeyEnv is unavailable at User scope."
}

$stdout = Join-Path (Split-Path -Parent $resolvedExe) 'agent-secure.stdout.log'
$stderr = Join-Path (Split-Path -Parent $resolvedExe) 'agent-secure.stderr.log'
$names = @(
  'JX3_BIND',
  'JX3_PORT',
  'JX3_NO_BROWSER',
  'JX3_USERDATA_DIR',
  'JX3_AGENT_CONFIG',
  'JX3_KNOWLEDGE_ROOT',
  'JX3_KNOWLEDGE_RETRIEVAL',
  'JX3_KNOWLEDGE_CACHE_DIR',
  'HF_ENDPOINT',
  $ApiKeyEnv
)
$previous = @{}
foreach ($name in $names) {
  $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}

try {
  [Environment]::SetEnvironmentVariable('JX3_BIND', '127.0.0.1', 'Process')
  [Environment]::SetEnvironmentVariable('JX3_PORT', [string]$Port, 'Process')
  [Environment]::SetEnvironmentVariable('JX3_NO_BROWSER', '1', 'Process')
  [Environment]::SetEnvironmentVariable('JX3_USERDATA_DIR', $resolvedUserdata, 'Process')
  [Environment]::SetEnvironmentVariable('JX3_AGENT_CONFIG', $resolvedConfig, 'Process')
  [Environment]::SetEnvironmentVariable('JX3_KNOWLEDGE_ROOT', $resolvedKnowledge, 'Process')
  [Environment]::SetEnvironmentVariable('JX3_KNOWLEDGE_RETRIEVAL', $KnowledgeRetrieval, 'Process')
  [Environment]::SetEnvironmentVariable('JX3_KNOWLEDGE_CACHE_DIR', $knowledgeCache, 'Process')
  [Environment]::SetEnvironmentVariable('HF_ENDPOINT', $EmbeddingEndpoint, 'Process')
  [Environment]::SetEnvironmentVariable($ApiKeyEnv, $credential, 'Process')
  $server = Start-Process -FilePath $resolvedExe -WorkingDirectory $backendRoot `
    -WindowStyle Hidden -RedirectStandardOutput $stdout -RedirectStandardError $stderr -PassThru
} finally {
  foreach ($name in $names) {
    [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
  }
  Remove-Variable credential -ErrorAction SilentlyContinue
}

$healthy = $false
for ($attempt = 0; $attempt -lt ($StartupTimeoutSec * 10); $attempt++) {
  if ($server.HasExited) {
    throw "Agent backend exited during startup with code $($server.ExitCode)."
  }
  try {
    if (Invoke-RestMethod -Uri "http://127.0.0.1:$Port/health" -TimeoutSec 1) {
      $healthy = $true
      break
    }
  } catch {}
  Start-Sleep -Milliseconds 100
}
if (-not $healthy) {
  throw 'Agent backend did not become healthy.'
}

[pscustomobject]@{
  pid = $server.Id
  port = $Port
  config = Split-Path -Leaf $resolvedConfig
  knowledge = if ($resolvedKnowledge) { Split-Path -Leaf $resolvedKnowledge } else { 'disabled' }
  retrieval = $KnowledgeRetrieval
  credential_source = "User environment: $ApiKeyEnv"
} | ConvertTo-Json -Compress
