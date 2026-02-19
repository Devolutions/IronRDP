[CmdletBinding()]
param(
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
    [string]$ProviderDllPath = "",

    [Parameter()]
    [string]$OutputDirectory = "",

    [Parameter()]
    [switch]$PassThru
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$defaultsScript = Join-Path -Path $scriptRoot -ChildPath "side-by-side-defaults.ps1"
. $defaultsScript

$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber

$crateRoot = Resolve-Path -LiteralPath (Join-Path -Path $scriptRoot -ChildPath "..")
$workspaceRoot = Resolve-Path -LiteralPath (Join-Path -Path $crateRoot -ChildPath "..\\..")

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $OutputDirectory = Join-Path -Path (Join-Path -Path $workspaceRoot -ChildPath "artifacts") -ChildPath ("wtsprotocol-diagnostics-" + $timestamp)
}

New-Item -Path $OutputDirectory -ItemType Directory -Force | Out-Null
$resolvedOutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path

$summary = [ordered]@{
    createdAtUtc = (Get-Date).ToUniversalTime().ToString("o")
    listenerName = $ListenerName
    protocolManagerClsid = $ProtocolManagerClsid
    portNumber = $PortNumber
    providerDllPath = $ProviderDllPath
    files = @()
    warnings = @()
}

function Write-JsonFile {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][object]$Data
    )

    $path = Join-Path -Path $resolvedOutputDirectory -ChildPath $Name
    $Data | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $path -Encoding UTF8
    $summary.files += $Name
}

function Write-TextFile {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Text
    )

    $path = Join-Path -Path $resolvedOutputDirectory -ChildPath $Name
    Set-Content -LiteralPath $path -Value $Text -Encoding UTF8
    $summary.files += $Name
}

try {
    $service = Get-Service -Name "TermService" -ErrorAction Stop
    Write-JsonFile -Name "termservice.json" -Data $service
} catch {
    $summary.warnings += "failed to query TermService: $($_.Exception.Message)"
}

try {
    $terminalServer = Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server" -ErrorAction Stop
    Write-JsonFile -Name "terminal-server.json" -Data $terminalServer
} catch {
    $summary.warnings += "failed to read Terminal Server root key: $($_.Exception.Message)"
}

$listenerKey = "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\$ListenerName"
try {
    $listener = Get-ItemProperty -LiteralPath $listenerKey -ErrorAction Stop
    Write-JsonFile -Name "listener.json" -Data $listener
} catch {
    $summary.warnings += "failed to read listener key ${listenerKey}: $($_.Exception.Message)"
}

$clsidInproc = "HKLM:\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid\InprocServer32"
try {
    $inproc = Get-ItemProperty -LiteralPath $clsidInproc -ErrorAction Stop
    Write-JsonFile -Name "clsid-inproc.json" -Data $inproc
} catch {
    $summary.warnings += "failed to read COM registration key ${clsidInproc}: $($_.Exception.Message)"
}

try {
    $listeners = Get-NetTCPConnection -State Listen -LocalPort $PortNumber -ErrorAction SilentlyContinue
    if ($null -eq $listeners) {
        $listeners = @()
    }

    Write-JsonFile -Name "tcp-listeners.json" -Data $listeners
} catch {
    $summary.warnings += "failed to query TCP listeners on port ${PortNumber}: $($_.Exception.Message)"
}

try {
    $matchingRules = Get-NetFirewallRule -Direction Inbound -ErrorAction Stop |
        Where-Object {
            $rule = $_
            $portFilters = $rule | Get-NetFirewallPortFilter
            foreach ($filter in $portFilters) {
                if ($filter.Protocol -eq "TCP" -and ($filter.LocalPort -contains "$PortNumber" -or $filter.LocalPort -eq "Any")) {
                    return $true
                }
            }
            return $false
        }

    Write-JsonFile -Name "firewall-rules.json" -Data $matchingRules
} catch {
    $summary.warnings += "failed to query firewall rules: $($_.Exception.Message)"
}

if (-not [string]::IsNullOrWhiteSpace($ProviderDllPath)) {
    try {
        if (Test-Path -LiteralPath $ProviderDllPath -PathType Leaf) {
            $resolvedProvider = (Resolve-Path -LiteralPath $ProviderDllPath).Path
            $fileInfo = Get-Item -LiteralPath $resolvedProvider
            $hash = Get-FileHash -LiteralPath $resolvedProvider -Algorithm SHA256

            Write-JsonFile -Name "provider-dll.json" -Data ([ordered]@{
                    path = $resolvedProvider
                    length = $fileInfo.Length
                    lastWriteTimeUtc = $fileInfo.LastWriteTimeUtc
                    sha256 = $hash.Hash
                })
        } else {
            $summary.warnings += "provider dll path does not exist: $ProviderDllPath"
        }
    } catch {
        $summary.warnings += "failed to inspect provider dll: $($_.Exception.Message)"
    }
}

try {
    $events = Get-WinEvent -FilterHashtable @{
            LogName = "Microsoft-Windows-TerminalServices-LocalSessionManager/Operational"
            StartTime = (Get-Date).AddHours(-2)
        } -ErrorAction Stop -MaxEvents 200 |
        Select-Object TimeCreated, Id, LevelDisplayName, Message

    Write-JsonFile -Name "termservice-lsm-events.json" -Data $events
} catch {
    $summary.warnings += "failed to query LSM operational event log: $($_.Exception.Message)"
}

$summaryPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "summary.json"
$summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $summaryPath -Encoding UTF8

Write-Host "Diagnostics collected: $resolvedOutputDirectory"
Write-Host "Summary: $summaryPath"

if ($summary.warnings.Count -gt 0) {
    Write-Warning "Some diagnostics could not be collected. Check summary.json for details."
}

if ($PassThru.IsPresent) {
    Write-Output $resolvedOutputDirectory
}
