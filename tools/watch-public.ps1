# Read-only probes; recovery never replays user requests or calls model APIs.
param([string]$PrivatePath = (Join-Path $env:USERPROFILE '.jx3-public'), [switch]$Once)
$ErrorActionPreference = 'Stop'
$singleton = [Threading.Mutex]::new($false, 'Global\Jx3PublicWatch')
$ownsSingleton = $false
try {
    try { $ownsSingleton = $singleton.WaitOne(0) } catch [Threading.AbandonedMutexException] { $ownsSingleton = $true }
    if (-not $ownsSingleton) { return }
    $knownHosts = Join-Path $env:USERPROFILE '.ssh/known_hosts_jx3_deploy'
    $address = ((Get-Content -LiteralPath $knownHosts -TotalCount 1) -split '\s+')[0]
    if ($address -notmatch '^\d{1,3}(\.\d{1,3}){3}$') { throw 'Expected pinned deployment host.' }
    $localFailures = 0
    $publicFailures = 0
    $recoveryCount = 0
    $nextRecovery = [DateTime]::MinValue
    function Test-Health([string]$Uri) {
        try {
            $response = Invoke-WebRequest -Uri $Uri -NoProxy -TimeoutSec 6 -MaximumRedirection 0
            return ($response.StatusCode -eq 200 -and $response.Content.Trim() -eq 'OK')
        } catch { return $false }
    }
    function Write-GuardEvent([string]$Message) {
        $log = Join-Path $PrivatePath 'watch.log'
        if ((Test-Path -LiteralPath $log) -and (Get-Item -LiteralPath $log).Length -gt 1MB) {
            Move-Item -LiteralPath $log -Destination (Join-Path $PrivatePath 'watch.previous.log') -Force
        }
        Add-Content -LiteralPath $log -Value ((Get-Date -Format o) + ' ' + $Message)
    }
    function Test-RemoteRelay {
        # Distinguish a broken tunnel from an external HTTPS/browser route problem.
        $reply = & ssh -i (Join-Path $env:USERPROFILE '.ssh/jx3_deploy_20260906') -o ConnectTimeout=5 -o ConnectionAttempts=1 -o ServerAliveInterval=5 -o ServerAliveCountMax=1 -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o "UserKnownHostsFile=$knownHosts" ('ubuntu@' + $address) 'curl --noproxy "*" -s -o /dev/null --max-time 6 -w "%{http_code}" http://127.0.0.1:2015/health' 2>$null
        if ($LASTEXITCODE -eq 255) { return 'unreachable' }
        if (($reply -join '').Trim() -eq '200') { return 'healthy' }
        return 'failed'
    }
    Write-GuardEvent 'guard_started'
    do {
        $lifecycle = [Threading.Mutex]::new($false, 'Global\Jx3PublicLifecycle')
        $held = $false
        $status = 'checking'
        try {
            try { $held = $lifecycle.WaitOne(1000) } catch [Threading.AbandonedMutexException] { $held = $true }
            if (-not $held) { $status = 'lifecycle_busy' }
            elseif (Test-Path -LiteralPath (Join-Path $PrivatePath 'disabled')) { $status = 'manually_disabled' }
            else {
                $localHealthy = Test-Health 'http://127.0.0.1:3006/health'
                if ($localHealthy) { $localFailures = 0 } else { $localFailures++ }
                $status = 'local_unhealthy'
                if ($localHealthy) {
                    if (Test-Health ('https://' + $address + ':2014/health')) {
                        $publicFailures = 0
                        $recoveryCount = 0
                        $status = 'healthy'
                    } else { $publicFailures++; $status = 'public_unhealthy' }
                }
                if ((Get-Date) -ge $nextRecovery) {
                    if ($localFailures -ge 3) {
                        Write-GuardEvent 'recover_router'
                        $nextRecovery = (Get-Date).AddSeconds(120)
                        & (Join-Path $PSScriptRoot 'stop-public.ps1') -PrivatePath $PrivatePath -ForRecovery | Out-Null
                        & (Join-Path $PSScriptRoot 'start-public.ps1') -PrivatePath $PrivatePath -ForRecovery | Out-Null
                        $localFailures = 0; $publicFailures = 0
                        $status = 'router_restarted'
                    } elseif ($localHealthy -and $publicFailures -ge 3) {
                        $relay = Test-RemoteRelay
                        $nextRecovery = (Get-Date).AddSeconds([Math]::Min(600, 60 * [Math]::Pow(2, [Math]::Min($recoveryCount, 4))))
                        if ($relay -eq 'failed') {
                            Write-GuardEvent 'recover_tunnel'
                            & (Join-Path $PSScriptRoot 'repair-public-tunnel.ps1') -PrivatePath $PrivatePath | Out-Null
                            $recoveryCount++
                            $publicFailures = 0
                            $status = 'tunnel_restarted'
                        } else { $status = 'external_route_' + $relay; Write-GuardEvent $status }
                    }
                }
            }
        } catch {
            # Exception messages can contain addresses/credentials; keep only the type.
            $status = 'guard_error'
            Write-GuardEvent ('guard_error ' + $_.Exception.GetType().Name)
            $nextRecovery = (Get-Date).AddSeconds(60)
        } finally {
            if ($held) { $lifecycle.ReleaseMutex() }
            $lifecycle.Dispose()
        }
        $healthState = @{updated_at=(Get-Date -Format o); status=$status; local_failures=$localFailures; public_failures=$publicFailures; guard_pid=$PID; next_recovery_at=$nextRecovery.ToString('o')}
        $healthState | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $PrivatePath 'watch-status.new')
        Move-Item -LiteralPath (Join-Path $PrivatePath 'watch-status.new') -Destination (Join-Path $PrivatePath 'watch-status.json') -Force
        if (-not $Once) { Start-Sleep -Seconds 15 }
    } while (-not $Once)
} finally {
    if ($ownsSingleton) { $singleton.ReleaseMutex() }
    $singleton.Dispose()
}
