# User-level fallback when Task Scheduler registration is denied.
$ErrorActionPreference = 'Stop'
$mutex = [Threading.Mutex]::new($false, 'Global\Jx3PublicWatchHost')
$owned = $false
try {
    try { $owned = $mutex.WaitOne(0) } catch [Threading.AbandonedMutexException] { $owned = $true }
    if (-not $owned) { return }
    $pwsh = (Get-Command pwsh).Source
    $private = Join-Path $env:USERPROFILE '.jx3-public'
    while ($true) {
        $child = Start-Process -FilePath $pwsh -ArgumentList @('-NoProfile', '-NonInteractive', '-WindowStyle', 'Hidden', '-File', ('"' + (Join-Path $PSScriptRoot 'watch-public.ps1') + '"')) -WindowStyle Hidden -PassThru
        @{host_pid=$PID; child_pid=$child.Id; updated_at=(Get-Date -Format o)} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $private 'watch-host.json')
        $child.WaitForExit()
        Start-Sleep -Seconds 15
    }
} finally {
    if ($owned) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
