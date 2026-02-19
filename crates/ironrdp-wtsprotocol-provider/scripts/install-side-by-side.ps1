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
    [ValidateRange(1, 65535)]
    [int]$PortNumber = 3390,

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

if (-not (Test-Path -LiteralPath $ProviderDllPath -PathType Leaf)) {
    throw "provider dll path does not exist: $ProviderDllPath"
}

$providerDllPathResolved = (Resolve-Path -LiteralPath $ProviderDllPath).Path

$winStationsRoot = "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations"
$sourceListener = Join-Path -Path $winStationsRoot -ChildPath "RDP-Tcp"
$targetListener = Join-Path -Path $winStationsRoot -ChildPath $ListenerName

if (-not (Test-Path -LiteralPath $sourceListener)) {
    throw "source listener key not found: $sourceListener"
}

if ($ListenerName -ne "RDP-Tcp" -and $PortNumber -eq 3389) {
    throw "side-by-side listener cannot use port 3389; use a dedicated port such as 3390"
}

$conflictingListeners = @(Get-ChildItem -LiteralPath $winStationsRoot -ErrorAction Stop |
        Where-Object { $_.PSChildName -ne $ListenerName } |
        ForEach-Object {
            $name = $_.PSChildName
            $props = Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue

            if ($null -ne $props -and $null -ne $props.PortNumber -and [int]$props.PortNumber -eq $PortNumber) {
                $name
            }
        })

if ($conflictingListeners.Count -gt 0) {
    throw "listener port $PortNumber conflicts with existing WinStation listeners: $($conflictingListeners -join ', ')"
}

if (-not (Test-Path -LiteralPath $targetListener)) {
    Copy-Item -Path $sourceListener -Destination $targetListener -Recurse -Force
}

Set-ItemProperty -Path $targetListener -Name "LoadableProtocol_Object" -Type String -Value $ProtocolManagerClsid
Set-ItemProperty -Path $targetListener -Name "PortNumber" -Type DWord -Value $PortNumber

$clsidRoot = "HKLM:\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid"
$inprocServer32 = Join-Path -Path $clsidRoot -ChildPath "InprocServer32"

New-Item -Path $clsidRoot -Force | Out-Null
Set-ItemProperty -Path $clsidRoot -Name "(default)" -Type String -Value "IronRDP WTS Protocol Manager"

New-Item -Path $inprocServer32 -Force | Out-Null
Set-ItemProperty -Path $inprocServer32 -Name "(default)" -Type String -Value $providerDllPathResolved
Set-ItemProperty -Path $inprocServer32 -Name "ThreadingModel" -Type String -Value "Both"

Write-Host "Installed side-by-side protocol provider"
Write-Host "  listener: $ListenerName"
Write-Host "  port: $PortNumber"
Write-Host "  clsid: $ProtocolManagerClsid"
Write-Host "  dll: $providerDllPathResolved"

if ($RestartTermService.IsPresent) {
    Write-Warning "Restarting TermService now"
    Restart-Service -Name "TermService" -Force
}
