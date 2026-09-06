# Fixed VPS/frp deployment; private credentials are provisioned separately.
# Never terminates or changes the local 3005 application.
param([string]$PrivatePath = (Join-Path $env:USERPROFILE '.jx3-public'))
$ErrorActionPreference = 'Stop'
& (Join-Path $PSScriptRoot 'tools/start-public.ps1') -PrivatePath $PrivatePath
