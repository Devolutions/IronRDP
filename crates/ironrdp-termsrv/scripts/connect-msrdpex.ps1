[CmdletBinding()]
param(
    [Parameter()]
    [string]$Hostname = 'IT-HELP-TEST',

    [Parameter()]
    [int]$Port = 4489,

    [Parameter()]
    [string]$Username = 'IT-HELP\Administrator',

    [Parameter()]
    [string]$PasswordEnvVar = 'IRONRDP_TESTVM_RDP_PASSWORD',

    [Parameter()]
    [string]$MsRdpExPath = 'C:\Program Files\Devolutions\MsRdpEx\mstscex.exe',

    [Parameter()]
    [string]$LogPath = (Join-Path $env:TEMP 'MsRdpEx.log')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $MsRdpExPath -PathType Leaf)) {
    throw "MsRdpEx not found at: $MsRdpExPath"
}

$pwd = [Environment]::GetEnvironmentVariable($PasswordEnvVar)
if ([string]::IsNullOrWhiteSpace($pwd)) {
    $pwd = [Environment]::GetEnvironmentVariable('IRONRDP_TESTVM_PASSWORD')
    if ([string]::IsNullOrWhiteSpace($pwd)) {
        throw "Missing password: set env:$PasswordEnvVar (or env:IRONRDP_TESTVM_PASSWORD)"
    }
}

# NOTE: Prefer down-level username (DOMAIN\\user) when testing NLA against ironrdp-termsrv.
# UPNs (user@domain) can make mstsc choose Kerberos inside SPNEGO, which may fail if the server is NTLM-only.
$rdpContent = @"
full address:s:${Hostname}:$Port
username:s:$Username
ClearTextPassword:s:$pwd
authentication level:i:0
enablecredsspsupport:i:1
"@

$rdpPath = Join-Path $env:TEMP 'ironrdp-msrdpex.rdp'
Set-Content -LiteralPath $rdpPath -Value $rdpContent -Encoding Ascii

$Env:MSRDPEX_LOG_ENABLED = '1'
$Env:MSRDPEX_LOG_LEVEL = 'TRACE'
$Env:MSRDPEX_LOG_FILE_PATH = $LogPath

& $MsRdpExPath $rdpPath
