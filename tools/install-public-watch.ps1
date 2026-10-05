# Runs as the deployment user so existing DPAPI credentials stay usable.
$ErrorActionPreference = 'Stop'
$pwsh = (Get-Command pwsh).Source
$script = Join-Path $PSScriptRoot 'watch-public.ps1'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$action = New-ScheduledTaskAction -Execute $pwsh -Argument ('-NoProfile -NonInteractive -WindowStyle Hidden -File "' + $script + '"') -WorkingDirectory (Split-Path -Parent $PSScriptRoot)
$logon = New-ScheduledTaskTrigger -AtLogOn -User $identity
$periodic = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) -RepetitionInterval (New-TimeSpan -Minutes 1)
$principal = New-ScheduledTaskPrincipal -UserId $identity -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
try {
    Register-ScheduledTask -TaskName 'JX3-Public-Watch' -Action $action -Trigger @($logon, $periodic) -Principal $principal -Settings $settings -Description 'Checks public route and recovers only tracked public services. Requires deployment user login for DPAPI.' -Force -ErrorAction Stop | Out-Null
    Start-ScheduledTask -TaskName 'JX3-Public-Watch' -ErrorAction Stop
    Write-Output 'Public watchdog installed as scheduled task.'
} catch {
    $hostScript = Join-Path $PSScriptRoot 'host-public-watch.ps1'
    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    if (-not (Test-Path $runKey)) { New-Item -Path $runKey -Force | Out-Null }
    $command = '"' + $pwsh + '" -NoProfile -NonInteractive -WindowStyle Hidden -File "' + $hostScript + '"'
    New-ItemProperty -Path $runKey -Name 'JX3PublicWatch' -PropertyType String -Value $command -Force | Out-Null
    Start-Process -FilePath $pwsh -ArgumentList @('-NoProfile','-NonInteractive','-WindowStyle','Hidden','-File',('"'+$hostScript+'"')) -WindowStyle Hidden | Out-Null
    Write-Output 'Task Scheduler unavailable; installed user-login startup with a persistent watchdog host.'
}
Write-Output 'Automatic startup requires deployment-user login. Lock screen supported; sleep suspends hosting.'
