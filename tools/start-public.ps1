param([string]$PrivatePath = (Join-Path $env:USERPROFILE '.jx3-public'))
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$backend = Join-Path $repo 'backend'
$exe = (Resolve-Path -LiteralPath (Join-Path $backend 'target/public-runtime/jx3-combat-sim.exe')).Path
$frpc = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot 'frpc.exe')).Path
$private = (Resolve-Path -LiteralPath $PrivatePath).Path
if (Get-NetTCPConnection -LocalPort 3006 -State Listen -ErrorAction SilentlyContinue) { throw 'Port 3006 is occupied; no process was stopped.' }
if (Test-Path -LiteralPath (Join-Path $private 'running.json')) { throw 'Deployment state exists; run stop-public.ps1 before restarting.' }
$key = [Environment]::GetEnvironmentVariable('JX3_DEEPSEEK_API_KEY','User')
if (-not $key) { throw 'User-scoped provider credential is missing.' }
$manifests = @(Get-ChildItem -LiteralPath (Join-Path $env:USERPROFILE 'Documents') -Filter '_migration-manifest.json' -Recurse -File -ErrorAction SilentlyContinue)
if ($manifests.Count -ne 1) { throw 'Expected exactly one knowledge vault.' }
$secure = Import-Clixml -LiteralPath (Join-Path $private 'credentials.xml')
$invite = [Net.NetworkCredential]::new('', $secure.invite).Password
$values = @{
  JX3_ROUTER='1'; JX3_PORT='3006'; JX3_BIND='127.0.0.1'; JX3_NO_BROWSER='1';
  JX3_PUBLIC_DEPLOYMENT='1'; JX3_MAX_WORKERS='100'; JX3_AUTH_PASSWORD=$invite;
  JX3_AUTH_FILE=(Join-Path $private 'whitelist.json'); JX3_USERDATA_DIR=(Join-Path $backend 'userdata');
  JX3_AGENT_CONFIG=(Join-Path $repo 'config/agent.public.toml'); JX3_DEEPSEEK_API_KEY=$key;
  JX3_KNOWLEDGE_ROOT=$manifests[0].DirectoryName; JX3_KNOWLEDGE_RETRIEVAL='embedded';
  JX3_KNOWLEDGE_CACHE_DIR=(Join-Path $backend 'userdata/knowledge_index/v1'); HF_ENDPOINT='https://huggingface.co'
}
$prior = @{}
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
try {
  foreach ($name in $values.Keys) { $prior[$name]=[Environment]::GetEnvironmentVariable($name,'Process'); [Environment]::SetEnvironmentVariable($name,$values[$name],'Process') }
  $router = Start-Process -FilePath $exe -WorkingDirectory $backend -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $private "router-$stamp.stdout.log") -RedirectStandardError (Join-Path $private "router-$stamp.stderr.log")
} finally {
  foreach ($name in $prior.Keys) { [Environment]::SetEnvironmentVariable($name,$prior[$name],'Process') }
  $key=$null; $invite=$null; $values=$null
}
$state = [ordered]@{ router_pid=$router.Id; router_exe=$exe; router_started=$router.StartTime.ToUniversalTime().Ticks; frpc_pid=0; frpc_exe=$frpc; frpc_started=0 }
$state | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $private 'running.json')
$healthy=$false
for ($i=0; $i -lt 100; $i++) {
  if ($router.HasExited) { throw 'Public router exited; inspect private logs.' }
  try { if ((Invoke-RestMethod 'http://127.0.0.1:3006/health' -TimeoutSec 1) -eq 'OK') {$healthy=$true; break} } catch {}
  Start-Sleep -Milliseconds 200
}
if (-not $healthy) { throw 'Public router did not become healthy; use stop-public.ps1.' }
$tunnel = Start-Process -FilePath $frpc -ArgumentList @('-c',('"'+(Join-Path $private 'frpc.toml')+'"')) -WorkingDirectory $private -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $private "frpc-$stamp.stdout.log") -RedirectStandardError (Join-Path $private "frpc-$stamp.stderr.log")
$state.frpc_pid=$tunnel.Id; $state.frpc_started=$tunnel.StartTime.ToUniversalTime().Ticks
$state | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $private 'running.json')
Write-Output 'Public router started on loopback 3006; Flash only. Local 3005 was not modified.'
