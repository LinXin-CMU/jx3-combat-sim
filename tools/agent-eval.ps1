param(
  [string]$BaseUrl = 'http://127.0.0.1:3005'
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$runner = Join-Path $repoRoot 'backend\tests\run_agent_eval.py'

if (-not (Test-Path -LiteralPath $runner -PathType Leaf)) {
  throw "Agent evaluation runner not found: $runner"
}

Push-Location (Join-Path $repoRoot 'backend')
try {
  & python 'tests/run_agent_eval.py' '--backend' $BaseUrl
  if ($LASTEXITCODE -ne 0) {
    throw "Agent evaluation failed with exit code $LASTEXITCODE."
  }
} finally {
  Pop-Location
}
