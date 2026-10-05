param([string]$PrivatePath = (Join-Path $env:USERPROFILE '.jx3-public'))
$ErrorActionPreference = 'Stop'
$lifecycleLock = [Threading.Mutex]::new($false, 'Global\Jx3PublicLifecycle')
$lockHeld = $false
try {
try { $lockHeld = $lifecycleLock.WaitOne(30000) } catch [Threading.AbandonedMutexException] { $lockHeld = $true }
if (-not $lockHeld) { throw 'Public lifecycle is busy.' }
$stateFile = Join-Path $PrivatePath 'running.json'
$state = Get-Content -LiteralPath $stateFile -Raw | ConvertFrom-Json
$expectedExe = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'frpc.exe')).Path
if ($state.frpc_exe -ne $expectedExe) { throw 'Unrecognized tunnel executable.' }
if (Test-Path -LiteralPath (Join-Path $PrivatePath 'disabled')) { throw 'Public deployment is manually disabled.' }
if ($state.frpc_pid) {
    $existing = Get-Process -Id $state.frpc_pid -ErrorAction SilentlyContinue
    if ($existing) {
        if ($existing.Path -ne $expectedExe -or $existing.StartTime.ToUniversalTime().Ticks -ne $state.frpc_started) { throw 'Tunnel process identity changed.' }
        Stop-Process -Id $existing.Id
        $existing.WaitForExit(5000) | Out-Null
    }
}
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss-fff'
$tunnel = Start-Process -FilePath $expectedExe -ArgumentList @('-c', ('"' + (Join-Path $PrivatePath 'frpc.toml') + '"')) -WorkingDirectory $PrivatePath -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $PrivatePath "frpc-$stamp.stdout.log") -RedirectStandardError (Join-Path $PrivatePath "frpc-$stamp.stderr.log")
$state.frpc_pid = $tunnel.Id
$state.frpc_started = $tunnel.StartTime.ToUniversalTime().Ticks
$temporaryState = $stateFile + '.new'
$state | ConvertTo-Json | Set-Content -LiteralPath $temporaryState
Move-Item -LiteralPath $temporaryState -Destination $stateFile -Force
Write-Output 'Public tunnel restarted; router, workers and local 3005 preserved.'
} finally {
    if ($lockHeld) { $lifecycleLock.ReleaseMutex() }
    $lifecycleLock.Dispose()
}
