[CmdletBinding()]
param(
    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ListenerName = "IRDP-Tcp",

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ProtocolManagerClsid = "{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}",

    [Parameter()]
    [switch]$RestartTermService
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (Test-IsAdministrator)) {
    throw "this script must be run from an elevated PowerShell session"
}

$winStationsRoot = "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations"
$targetListener = Join-Path -Path $winStationsRoot -ChildPath $ListenerName
if (Test-Path -LiteralPath $targetListener) {
    Remove-Item -LiteralPath $targetListener -Recurse -Force
}

$clsidRoot = "HKLM:\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid"
if (Test-Path -LiteralPath $clsidRoot) {
    Remove-Item -LiteralPath $clsidRoot -Recurse -Force
}

Write-Host "Removed side-by-side protocol provider"
Write-Host "  listener: $ListenerName"
Write-Host "  clsid: $ProtocolManagerClsid"

if ($RestartTermService.IsPresent) {
    Write-Warning "Restarting TermService now"
    Restart-Service -Name "TermService" -Force
}
