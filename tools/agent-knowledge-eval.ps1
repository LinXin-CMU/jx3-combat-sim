param(
  [string]$VaultRoot = '',
  [ValidateSet('bm25', 'embedded')]
  [string]$Retrieval = 'bm25',
  [string]$CachePath = '',
  [string]$EmbeddingEndpoint = 'https://huggingface.co'
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($VaultRoot)) {
  $documentsRoot = Join-Path $env:USERPROFILE 'Documents'
  $manifests = @(Get-ChildItem -LiteralPath $documentsRoot -Filter '_migration-manifest.json' -Recurse -File -ErrorAction SilentlyContinue)
  if ($manifests.Count -ne 1) {
    throw "Expected exactly one knowledge manifest below $documentsRoot; found $($manifests.Count). Use -VaultRoot."
  }
  $VaultRoot = $manifests[0].DirectoryName
}
$manifest = Join-Path $VaultRoot '_migration-manifest.json'
if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) {
  throw "Knowledge manifest not found: $manifest"
}

$previousRoot = $env:JX3_KNOWLEDGE_ROOT
$previousRetrieval = $env:JX3_KNOWLEDGE_RETRIEVAL
$previousCache = $env:JX3_KNOWLEDGE_CACHE_DIR
$previousHfEndpoint = $env:HF_ENDPOINT
try {
  $env:JX3_KNOWLEDGE_ROOT = [IO.Path]::GetFullPath($VaultRoot)
  $env:JX3_KNOWLEDGE_RETRIEVAL = $Retrieval
  if ($Retrieval -eq 'embedded') {
    if ([string]::IsNullOrWhiteSpace($CachePath)) {
      $CachePath = Join-Path $repoRoot 'backend\userdata\knowledge_index\v1'
    }
    New-Item -ItemType Directory -Path $CachePath -Force | Out-Null
    $env:JX3_KNOWLEDGE_CACHE_DIR = [IO.Path]::GetFullPath($CachePath)
    $env:HF_ENDPOINT = $EmbeddingEndpoint
  }
  Push-Location $repoRoot
  try {
    & cargo test --manifest-path backend/Cargo.toml configured_vault_fixed_retrieval_eval -- --nocapture
    if ($LASTEXITCODE -ne 0) {
      throw "Knowledge evaluation failed with exit code $LASTEXITCODE"
    }
  } finally {
    Pop-Location
  }
} finally {
  $env:JX3_KNOWLEDGE_ROOT = $previousRoot
  $env:JX3_KNOWLEDGE_RETRIEVAL = $previousRetrieval
  $env:JX3_KNOWLEDGE_CACHE_DIR = $previousCache
  $env:HF_ENDPOINT = $previousHfEndpoint
}
