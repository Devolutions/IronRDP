[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$ProviderDllPath,

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ListenerName = "IRDP-Tcp",

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ProtocolManagerClsid = "{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}",

    [Parameter()]
    [ValidateRange(0, 65535)]
    [int]$PortNumber = 0
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$defaultsScript = Join-Path -Path $scriptRoot -ChildPath "side-by-side-defaults.ps1"
. $defaultsScript

$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber

if (-not (Test-Path -LiteralPath $ProviderDllPath -PathType Leaf)) {
    throw "provider dll path does not exist: $ProviderDllPath"
}

$providerDllPathResolved = (Resolve-Path -LiteralPath $ProviderDllPath).Path

$winStationsRoot = "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations"
$listenerKey = Join-Path -Path $winStationsRoot -ChildPath $ListenerName
if (-not (Test-Path -LiteralPath $listenerKey)) {
    throw "listener key not found: $listenerKey"
}

$listenerProps = Get-ItemProperty -LiteralPath $listenerKey
if ($listenerProps.LoadableProtocol_Object -ne $ProtocolManagerClsid) {
    throw "listener LoadableProtocol_Object mismatch: expected $ProtocolManagerClsid got $($listenerProps.LoadableProtocol_Object)"
}

if ($listenerProps.PortNumber -ne $PortNumber) {
    throw "listener PortNumber mismatch: expected $PortNumber got $($listenerProps.PortNumber)"
}

$clsidRoot = "HKLM:\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid"
$inprocServer32 = Join-Path -Path $clsidRoot -ChildPath "InprocServer32"
if (-not (Test-Path -LiteralPath $inprocServer32)) {
    throw "inproc registration key not found: $inprocServer32"
}

$inprocProps = Get-ItemProperty -LiteralPath $inprocServer32
$registeredDll = $inprocProps.'(default)'
$threadingModel = $inprocProps.ThreadingModel

if (-not $registeredDll) {
    throw "inproc default value is empty"
}

$registeredDllResolved = (Resolve-Path -LiteralPath $registeredDll).Path
if ($registeredDllResolved -ne $providerDllPathResolved) {
    throw "registered dll mismatch: expected $providerDllPathResolved got $registeredDllResolved"
}

if ($threadingModel -ne "Both") {
    throw "threading model mismatch: expected Both got $threadingModel"
}

Write-Host "Verification passed"
Write-Host "  listener: $ListenerName"
Write-Host "  listener port: $($listenerProps.PortNumber)"
Write-Host "  loadable protocol clsid: $($listenerProps.LoadableProtocol_Object)"
Write-Host "  registered dll: $registeredDllResolved"
Write-Host "  threading model: $threadingModel"
Write-Host "  mstsc target: <host>:$($listenerProps.PortNumber)"
