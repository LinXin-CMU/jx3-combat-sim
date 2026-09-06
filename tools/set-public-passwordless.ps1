param([Parameter(Mandatory=$true)][string]$Username)
# Explicit operator action. Private authentication material is never printed.
$ErrorActionPreference='Stop'
if (-not $Username.Trim() -or [Text.Encoding]::UTF8.GetByteCount($Username) -gt 40) {throw 'Invalid username.'}
$Username=$Username.Trim()
$private=Join-Path $env:USERPROFILE '.jx3-public'
$credential=Import-Clixml -LiteralPath (Join-Path $private 'smoke-account.xml')
$body=@{username=$credential.username;password=[Net.NetworkCredential]::new('',$credential.password).Password}|ConvertTo-Json -Compress
$base='http://127.0.0.1:3006'
$login=Invoke-WebRequest -Uri "$base/api/auth/login" -Method Post -ContentType 'application/json' -Body $body -NoProxy
$cookie=(([string]($login.Headers['Set-Cookie'] -join ';')) -split ';')[0]
try {
  $path=Join-Path $private 'whitelist.json'
  $hash=(Get-FileHash -LiteralPath $path).Hash
  $auth=Get-Content -LiteralPath $path -Raw|ConvertFrom-Json
  $matches=@($auth.members|Where-Object username -CEQ $Username)
  if ($matches.Count -gt 1) {throw 'Duplicate account; refusing to modify.'}
  $created=$matches.Count -eq 0
  if ($created) {
    $auth.members=@($auth.members)+@([pscustomobject]@{username=$Username;password=$null;tokens=@();created=[DateTimeOffset]::UtcNow.ToUnixTimeSeconds()})
  } else {$matches[0].password=$null}
  $backup=Join-Path $private ('whitelist-before-passwordless-'+[DateTime]::UtcNow.Ticks+'.json')
  Copy-Item -LiteralPath $path -Destination $backup
  if ((Get-FileHash -LiteralPath $path).Hash -ne $hash) {throw 'Authentication changed concurrently; retry after inspection.'}
  $auth|ConvertTo-Json -Depth 12|Set-Content -LiteralPath $path -Encoding utf8NoBOM
  $reload=Invoke-RestMethod -Uri "$base/api/auth/reload" -Method Post -Headers @{Cookie=$cookie} -NoProxy
  if (-not $reload.ok) {throw 'Authentication reload failed.'}
  $probe=Invoke-WebRequest -Uri "$base/api/auth/login" -Method Post -ContentType 'application/json' -Body (@{username=$Username;password=''}|ConvertTo-Json -Compress) -NoProxy
  $probeCookie=(([string]($probe.Headers['Set-Cookie'] -join ';')) -split ';')[0]
  Invoke-RestMethod -Uri "$base/api/auth/logout" -Method Post -Headers @{Cookie=$probeCookie} -NoProxy|Out-Null
  Write-Output ('PASS: passwordless login; created='+$created+'. Other accounts and user data preserved; private backup retained.')
} finally {
  Invoke-RestMethod -Uri "$base/api/auth/logout" -Method Post -Headers @{Cookie=$cookie} -NoProxy|Out-Null
}
