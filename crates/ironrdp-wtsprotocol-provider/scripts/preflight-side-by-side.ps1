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

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (Test-IsAdministrator)) {
    throw "this script must be run from an elevated PowerShell session"
}

if (-not (Test-Path -LiteralPath $ProviderDllPath -PathType Leaf)) {
    throw "provider dll path does not exist: $ProviderDllPath"
}

$providerDllPathResolved = (Resolve-Path -LiteralPath $ProviderDllPath).Path

$termService = Get-Service -Name "TermService" -ErrorAction Stop
$terminalServerKey = "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server"
$terminalServerProps = Get-ItemProperty -LiteralPath $terminalServerKey -ErrorAction Stop
$denyTsConnections = [int]$terminalServerProps.fDenyTSConnections

if ($denyTsConnections -ne 0) {
    throw "remote desktop connections are disabled (fDenyTSConnections=$denyTsConnections); enable Remote Desktop before mstsc testing"
}

$winStationsRoot = "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations"
$sourceListener = Join-Path -Path $winStationsRoot -ChildPath "RDP-Tcp"
$targetListener = Join-Path -Path $winStationsRoot -ChildPath $ListenerName

if (-not (Test-Path -LiteralPath $sourceListener)) {
    throw "source listener key not found: $sourceListener"
}

if ($ListenerName -ne "RDP-Tcp" -and $PortNumber -eq 3389) {
    throw "side-by-side listener cannot use port 3389; use a dedicated port such as 4489"
}

$conflictingListeners = @(Get-ChildItem -LiteralPath $winStationsRoot -ErrorAction Stop |
        Where-Object { $_.PSChildName -ne $ListenerName } |
        ForEach-Object {
            $name = $_.PSChildName
            $props = Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue

            if ($null -ne $props) {
                $portProperty = $props.PSObject.Properties['PortNumber']
                if ($null -ne $portProperty) {
                    try {
                        if ([int]$portProperty.Value -eq $PortNumber) {
                            $name
                        }
                    }
                    catch {
                    }
                }
            }
        })

if ($conflictingListeners.Count -gt 0) {
    throw "planned listener port $PortNumber conflicts with existing WinStation listeners: $($conflictingListeners -join ', ')"
}

$clsidRoot = "HKLM:\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid"
$inprocServer32 = Join-Path -Path $clsidRoot -ChildPath "InprocServer32"

Write-Host "Preflight checks passed"
Write-Host "  elevated session: yes"
Write-Host "  provider dll: $providerDllPathResolved"
Write-Host "  termservice state: $($termService.Status)"
Write-Host "  remote desktop enabled: yes"
Write-Host "  source listener key: $sourceListener"
Write-Host "  target listener key: $targetListener"
Write-Host "  planned listener port: $PortNumber"
Write-Host "  listener port conflict check: clear"
Write-Host "  clsid key: $clsidRoot"
Write-Host "  inproc key: $inprocServer32"

Write-Host "Next: run install-side-by-side.ps1 with same parameters"
