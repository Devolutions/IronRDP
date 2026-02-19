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
    [int]$PortNumber = 0,

    [Parameter()]
    [switch]$RestartTermService
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$defaultsScript = Join-Path -Path $scriptRoot -ChildPath "side-by-side-defaults.ps1"
. $defaultsScript

$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber -PersistResolvedDefault
$portSettingInfo = Get-SideBySideListenerPortSettingInfo

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
    throw "listener port $PortNumber conflicts with existing WinStation listeners: $($conflictingListeners -join ', ')"
}

if (-not (Test-Path -LiteralPath $targetListener)) {
    Copy-Item -Path $sourceListener -Destination $targetListener -Recurse -Force
}

Set-ItemProperty -Path $targetListener -Name "LoadableProtocol_Object" -Type String -Value $ProtocolManagerClsid
Set-ItemProperty -Path $targetListener -Name "PortNumber" -Type DWord -Value $PortNumber

$clsidRoot = "HKLM:\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid"
$inprocServer32 = Join-Path -Path $clsidRoot -ChildPath "InprocServer32"
$clsidRootRegPath = "HKLM\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid"
$inprocServer32RegPath = "$clsidRootRegPath\InprocServer32"

& reg.exe add $clsidRootRegPath /f | Out-Null
& reg.exe add $clsidRootRegPath /ve /t REG_SZ /d "IronRDP WTS Protocol Manager" /f | Out-Null

& reg.exe add $inprocServer32RegPath /f | Out-Null
& reg.exe add $inprocServer32RegPath /ve /t REG_SZ /d $providerDllPathResolved /f | Out-Null
Set-ItemProperty -Path $inprocServer32 -Name "ThreadingModel" -Type String -Value "Both"

Write-Host "Installed side-by-side protocol provider"
Write-Host "  listener: $ListenerName"
Write-Host "  port: $PortNumber"
Write-Host "  default-listener-port registry: $($portSettingInfo.RegistryPath)\\$($portSettingInfo.RegistryValueName)"
Write-Host "  clsid: $ProtocolManagerClsid"
Write-Host "  dll: $providerDllPathResolved"

if ($RestartTermService.IsPresent) {
    Write-Warning "Restarting TermService now"
    Restart-Service -Name "TermService" -Force
}
