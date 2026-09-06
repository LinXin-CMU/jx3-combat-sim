# One-time private deployment material. Never emits credentials or server address.
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$private = Join-Path $env:USERPROFILE '.jx3-public'
if (Test-Path -LiteralPath $private) { throw 'Private deployment directory already exists; refusing to replace it.' }
New-Item -ItemType Directory -Path $private | Out-Null
$acl = Get-Acl -LiteralPath $private
$acl.SetAccessRuleProtection($true, $false)
$who = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($who,'FullControl','ContainerInherit,ObjectInherit','None','Allow'))
Set-Acl -LiteralPath $private -AclObject $acl
function New-RandomHex([int]$count = 32) {
  $bytes = [byte[]]::new($count)
  [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
  [Convert]::ToHexString($bytes).ToLowerInvariant()
}
$address = ((Get-Content -LiteralPath (Join-Path $env:USERPROFILE '.ssh/known_hosts_jx3_deploy') -TotalCount 1) -split '\s+')[0]
if ($address -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'Expected pinned IPv4 host.' }
$invite = New-RandomHex 24
$frpToken = New-RandomHex
$bundle = [pscustomobject]@{ invite = ConvertTo-SecureString $invite -AsPlainText -Force }
$bundle | Export-Clixml -LiteralPath (Join-Path $private 'credentials.xml')
$oldAuth = Join-Path $repo 'backend/userdata/whitelist.json'
$auth = Get-Content -LiteralPath $oldAuth -Raw | ConvertFrom-Json
$originalHash = (Get-FileHash -LiteralPath $oldAuth).Hash
$handoff = @('# 公网访问交接（私密，请勿分享整份文件）', '', "访问地址：https://${address}:2014", '', "新账号邀请码：$invite", '', '已有个人密码继续有效；原免密账号只填用户名即可登录。账号与存档目录保持原样。', '')
foreach ($member in $auth.members) {
  $member.tokens = @()
}
$auth | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $private 'whitelist.json') -Encoding utf8NoBOM
$handoff += @('', '仅开放 Flash，不设每日/累计分析次数配额。电脑需保持运行且不休眠。', '启动：在项目中运行 tools/start-public.ps1；停止：运行 tools/stop-public.ps1。', '旧认证文件保持不变；公网使用独立认证文件，并继续访问原账号存档。')
$handoff | Set-Content -LiteralPath (Join-Path $private '访问与账号.md') -Encoding utf8NoBOM
@('[InternetShortcut]',"URL=https://${address}:2014") | Set-Content -LiteralPath (Join-Path $private '打开公网.url') -Encoding ascii
@"
bindPort = 7000
proxyBindAddr = "127.0.0.1"
allowPorts = [{ single = 2015 }]
maxPortsPerClient = 1
auth.method = "token"
auth.token = "$frpToken"
transport.tcpMux = false
transport.tls.force = true
transport.tls.certFile = "/opt/frp/tls/jx3.crt"
transport.tls.keyFile = "/opt/frp/tls/jx3.key"
"@ | Set-Content -LiteralPath (Join-Path $private 'frps.toml') -Encoding utf8NoBOM
$trust = (Join-Path $private 'frp-trust.crt').Replace('\','/')
@"
serverAddr = "$address"
serverPort = 7000
auth.method = "token"
auth.token = "$frpToken"
transport.tcpMux = false
transport.tls.enable = true
transport.tls.trustedCaFile = "$trust"
transport.tls.serverName = "jx3-frp.internal"
loginFailExit = false
[[proxies]]
name = "jx3-public"
type = "tcp"
localIP = "127.0.0.1"
localPort = 3006
remotePort = 2015
"@ | Set-Content -LiteralPath (Join-Path $private 'frpc.toml') -Encoding utf8NoBOM
if ((Get-FileHash -LiteralPath $oldAuth).Hash -ne $originalHash) { throw 'Original authentication file changed.' }
Write-Output 'Private material created; old authentication file unchanged; old tokens excluded from public authentication.'
