param([string]$PrivatePath = (Join-Path $env:USERPROFILE '.jx3-public'))
$ErrorActionPreference='Stop'
$stateFile = Join-Path $PrivatePath 'running.json'
if (-not (Test-Path -LiteralPath $stateFile)) { Write-Output 'No tracked public deployment.'; exit }
$state=Get-Content -LiteralPath $stateFile -Raw | ConvertFrom-Json
foreach ($kind in @('frpc','router')) {
  $processId=$state."${kind}_pid"
  if (-not $processId) {continue}
  $process=Get-Process -Id $processId -ErrorAction SilentlyContinue
  if (-not $process) {continue}
  if ($process.Path -ne $state."${kind}_exe" -or $process.StartTime.ToUniversalTime().Ticks -ne $state."${kind}_started") {throw 'Process identity changed; refusing to stop it.'}
  if ($kind -eq 'router') {
    $children=Get-CimInstance Win32_Process -Filter "ParentProcessId=$processId"
    foreach ($child in $children) {
      if ($child.ExecutablePath -eq $state.router_exe) { Stop-Process -Id $child.ProcessId }
    }
  }
  Stop-Process -Id $processId
}
Move-Item -LiteralPath $stateFile -Destination (Join-Path $PrivatePath ('stopped-'+(Get-Date -Format 'yyyyMMdd-HHmmss')+'.json'))
Write-Output 'Only tracked public processes stopped. Local service and all user data preserved.'
