param([switch]$External, [switch]$RealModel)
$ErrorActionPreference='Stop'
$private=Join-Path $env:USERPROFILE '.jx3-public'
$base='http://127.0.0.1:3006'
if ($External) {
  $address=((Get-Content -LiteralPath (Join-Path $env:USERPROFILE '.ssh/known_hosts_jx3_deploy') -TotalCount 1) -split '\s+')[0]
  $base="https://${address}:2014"
}
function Request([string]$path, [string]$method='GET', $payload=$null, [string]$cookie='') {
  $args=@{Uri=($base+$path); Method=$method; TimeoutSec=180; SkipHttpErrorCheck=$true; NoProxy=$true}
  if ($cookie) {$args.Headers=@{Cookie=$cookie}}
  if ($null -ne $payload) {$args.ContentType='application/json'; $args.Body=($payload | ConvertTo-Json -Depth 20 -Compress)}
  # Retry only read requests; never duplicate a login or model run after an uncertain POST.
  for ($retry=0; $retry -lt 3; $retry++) {
    try {return (Invoke-WebRequest @args)} catch {
      if ($method -ne 'GET' -or $retry -eq 2) {throw 'Deployment HTTP connection failed; address omitted.'}
      Start-Sleep -Milliseconds 500
    }
  }
}
function Assert([bool]$ok,[string]$message) {if (-not $ok) {throw $message}}
Assert ((Request '/api/agent/providers').StatusCode -eq 401) 'Anonymous provider access was not rejected.'
Assert ((Request '/api/auth/reload' 'POST').StatusCode -eq 401) 'Anonymous authentication reload was not rejected.'
$oldAuth=Get-Content -LiteralPath (Join-Path (Split-Path -Parent $PSScriptRoot) 'backend/userdata/whitelist.json') -Raw | ConvertFrom-Json
$legacy=@($oldAuth.members | Where-Object {-not $_.password})[0]
$legacyLogin=Request '/api/auth/login' 'POST' @{username=$legacy.username;password=''}
Assert ($legacyLogin.StatusCode -eq 200) 'Legacy passwordless login failed.'
$legacyCookie=(([string]($legacyLogin.Headers['Set-Cookie'] -join ';')) -split ';')[0]
[void](Request '/api/auth/logout' 'POST' $null $legacyCookie)
$credentialPath=Join-Path $private 'smoke-account.xml'
if (Test-Path -LiteralPath $credentialPath) {
  $credential=Import-Clixml -LiteralPath $credentialPath
  $password=[Net.NetworkCredential]::new('',$credential.password).Password
  $payload=@{username=$credential.username;password=$password}
} else {
  $username='deployment-check-'+(Get-Date -Format 'MMddHHmmss')
  $password=[Guid]::NewGuid().ToString('N')+[Guid]::NewGuid().ToString('N')
  $credential=[pscustomobject]@{username=$username;password=(ConvertTo-SecureString $password -AsPlainText -Force)}
  $bundle=Import-Clixml -LiteralPath (Join-Path $private 'credentials.xml')
  $invite=[Net.NetworkCredential]::new('',$bundle.invite).Password
  $payload=@{username=$username;password=$invite;set_password=$password}
}
$login=Request '/api/auth/login' 'POST' $payload
Assert ($login.StatusCode -eq 200) 'Authenticated login failed.'
$credential | Export-Clixml -LiteralPath $credentialPath
$setCookie=[string]($login.Headers['Set-Cookie'] -join ';')
Assert ($setCookie.Contains('Secure') -and $setCookie.Contains('HttpOnly') -and $setCookie.Contains('SameSite=Lax')) 'Cookie protections missing.'
$cookie=($setCookie -split ';')[0]
$response=Request '/api/agent/providers' 'GET' $null $cookie
Assert ($response.StatusCode -eq 200) 'Authenticated provider request failed.'
$providers=$response.Content | ConvertFrom-Json
Assert ($providers.profiles.Count -eq 1 -and $providers.profiles[0].id -eq 'deepseek-v4-flash' -and $providers.profiles[0].available) 'Public provider catalog is not exclusively available Flash.'
$simulation=@{haste_level=42087; sequence=@('盾击','盾压'); network_delay=0; attributes=@{base_attack=38466;weapon_damage=10986;crit_level=54841;crit_effect_level=0;overcome_level=29480;strain_level=66031;haste_level=42087};target=@{level=134;defense_bonus=0;damage_cof=0}}
$rejected=Request '/api/agent/runs' 'POST' @{question='部署权限检查';provider_profile='deepseek-v4-pro';simulation=$simulation} $cookie
Assert ($rejected.StatusCode -ge 400 -and $rejected.Content.Contains('provider_not_found')) 'Pro was not rejected server-side.'
$page=Request '/' 'GET' $null $cookie
Assert ($page.StatusCode -eq 200 -and $page.Content.Contains('<html')) 'Authenticated frontend is unavailable.'
if ($RealModel) {
  $created=Request '/api/agent/runs' 'POST' @{question='你好，请用一句话介绍你能帮我做什么。';provider_profile='deepseek-v4-flash';simulation=$simulation} $cookie
  Assert ($created.StatusCode -lt 300) 'Flash run could not start.'
  $run=$created.Content | ConvertFrom-Json
  $finished=$false
  for ($attempt=0; $attempt -lt 120; $attempt++) {
    Start-Sleep -Seconds 2
    $status=(Request $run.status_url 'GET' $null $cookie).Content | ConvertFrom-Json
    if (-not $status.running) {$finished=$true;break}
  }
  Assert $finished 'Real model check exceeded its monitoring deadline; inspect the test session.'
  Assert ($status.status -in @('completed','partially_verified','awaiting_user')) 'Real Flash run failed; inspect the test session.'
  Write-Output ('PASS: real Flash request; status='+$status.status)
}
[void](Request '/api/auth/logout' 'POST' $null $cookie)
Assert ((Request '/api/agent/providers' 'GET' $null $cookie).StatusCode -eq 401) 'Logout did not revoke session.'
Write-Output ('PASS: anonymous isolation, legacy passwordless login, secure cookie, Flash-only catalog, server-side Pro rejection, frontend, logout. HTTPS='+$External.IsPresent)
