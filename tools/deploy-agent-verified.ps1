param(
  [Parameter(Mandatory=$true)][string]$CandidateExe,
  [Parameter(Mandatory=$true)][string]$ExpectedSha256,
  [string]$KnowledgeRoot = '',
  [int]$Port = 3005
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$destination = Join-Path $root 'backend/target/release/jx3-combat-sim.exe'
$candidate = (Resolve-Path -LiteralPath $CandidateExe).Path
if ((Get-FileHash -LiteralPath $candidate).Hash -ne $ExpectedSha256) { throw 'Candidate hash mismatch.' }
$listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue)
if ($listeners.Count -ne 1 -or $listeners[0].LocalAddress -ne '127.0.0.1') { throw 'Unexpected listener.' }
$owner = Get-CimInstance Win32_Process -Filter "ProcessId = $($listeners[0].OwningProcess)"
if ($owner.ExecutablePath -ne (Resolve-Path -LiteralPath $destination).Path) { throw 'Unexpected running executable.' }
$userdata = Join-Path $root 'backend/userdata'
$before = @(Get-ChildItem -LiteralPath $userdata -File -Recurse |
    Where-Object FullName -notlike '*\knowledge_index\*' |
    ForEach-Object { [pscustomobject]@{Path=$_.FullName;Hash=(Get-FileHash -LiteralPath $_.FullName).Hash} })
$backupDir = Join-Path $root ('backend/target/deployment-backups/v42-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $backupDir | Out-Null
$backup = Join-Path $backupDir 'jx3-combat-sim.exe'
Copy-Item -LiteralPath $destination -Destination $backup
$oldHash = (Get-FileHash -LiteralPath $backup).Hash
# Recheck identity after the inventory operation, before stopping only this listener.
$current = Get-CimInstance Win32_Process -Filter "ProcessId = $($owner.ProcessId)"
if ($current.ExecutablePath -ne $owner.ExecutablePath -or $current.CreationDate -ne $owner.CreationDate) { throw 'Process identity changed.' }
Stop-Process -Id $owner.ProcessId
Wait-Process -Id $owner.ProcessId -Timeout 15 -ErrorAction SilentlyContinue
try {
  $copied = $false
  for ($attempt = 0; $attempt -lt 30; $attempt++) {
    try {
      Copy-Item -LiteralPath $candidate -Destination $destination -Force
      $copied = $true
      break
    } catch [System.IO.IOException] {
      if ($attempt -eq 29) { throw }
      Start-Sleep -Milliseconds 100
    }
  }
  if (-not $copied) { throw 'Executable replacement did not complete.' }
  $started = & "$PSScriptRoot/start-agent-secure.ps1" -Port $Port -KnowledgeRoot $KnowledgeRoot |
    ConvertFrom-Json
} catch {
  Write-Warning ("Deployment step failed: " + $_.Exception.GetType().Name + "; " + $_.ScriptStackTrace)
  # Only roll back a listener that is the executable just installed.
  $failedListener = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
  if ($failedListener) {
    $failedOwner = Get-CimInstance Win32_Process -Filter "ProcessId = $($failedListener.OwningProcess)"
    if ($failedOwner.ExecutablePath -ne $destination) { throw 'Unexpected process during rollback; manual intervention required.' }
    Stop-Process -Id $failedOwner.ProcessId
    Wait-Process -Id $failedOwner.ProcessId -Timeout 15 -ErrorAction SilentlyContinue
  }
  Copy-Item -LiteralPath $backup -Destination $destination -Force
  & "$PSScriptRoot/start-agent-secure.ps1" -Port $Port -KnowledgeRoot $KnowledgeRoot | Out-Null
  throw 'Deployment failed; previous binary restored.'
}
$changed = @($before | Where-Object {
    -not (Test-Path -LiteralPath $_.Path) -or (Get-FileHash -LiteralPath $_.Path).Hash -ne $_.Hash
})
[pscustomobject]@{
  pid=$started.pid;port=$Port;sha256=(Get-FileHash -LiteralPath $destination).Hash
  backup=$backup;previous_sha256=$oldHash;existing_files=$before.Count;changed_existing_files=$changed.Count
} | ConvertTo-Json -Compress
if ($changed.Count) { throw 'Existing userdata changed during deployment; investigate before further actions.' }
