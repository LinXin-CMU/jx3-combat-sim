# One-time user-authorized correction. Run with the public router stopped.
$ErrorActionPreference='Stop'
$repo=Split-Path -Parent $PSScriptRoot
$private=Join-Path $env:USERPROFILE '.jx3-public'
if (Get-NetTCPConnection -LocalPort 3006 -State Listen -ErrorAction SilentlyContinue) {throw 'Stop public router first.'}
$originalPath=Join-Path $repo 'backend/userdata/whitelist.json'
$originalHash=(Get-FileHash -LiteralPath $originalPath).Hash
$original=Get-Content -LiteralPath $originalPath -Raw | ConvertFrom-Json
$authPath=Join-Path $private 'whitelist.json'
$backup=Join-Path $private 'whitelist.before-passwordless-restore.json'
if (Test-Path -LiteralPath $backup) {throw 'Correction already attempted; inspect before repeating.'}
$auth=Get-Content -LiteralPath $authPath -Raw | ConvertFrom-Json
$names=@($original.members | Where-Object {-not $_.password} | ForEach-Object {$_.username})
$targets=@($auth.members | Where-Object {$_.username -cin $names})
if ($targets.Count -ne $names.Count -or $names.Count -eq 0) {throw 'Legacy account mapping mismatch.'}
Copy-Item -LiteralPath $authPath -Destination $backup
foreach ($member in $targets) {$member.password=$null}
$auth | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $authPath -Encoding utf8NoBOM
$handoff=Join-Path $private '访问与账号.md'
Copy-Item -LiteralPath $handoff -Destination (Join-Path $private '访问与账号.before-passwordless-restore.md')
$lines=@(Get-Content -LiteralPath $handoff | Where-Object {
  $line=$_
  -not ($line.StartsWith('- ') -and @($names | Where-Object {$line.StartsWith("- $_ ： ")}).Count -gt 0)
})
$lines=$lines | ForEach-Object {if ($_.StartsWith('已有账号：')) {'已有账号：原个人密码继续有效；原免密账号只填用户名即可登录。之前生成的临时密码已撤销。'} else {$_}}
$lines | Set-Content -LiteralPath $handoff -Encoding utf8NoBOM
if ((Get-FileHash -LiteralPath $originalPath).Hash -ne $originalHash) {throw 'Original auth file changed.'}
Write-Output ('Restored passwordless accounts: '+$targets.Count+'. Other passwords, tokens and original authentication file preserved. Private rollback copies retained.')
