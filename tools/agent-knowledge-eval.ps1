param(
  [string]$VaultRoot = ''
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
try {
  $env:JX3_KNOWLEDGE_ROOT = [IO.Path]::GetFullPath($VaultRoot)
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
}
