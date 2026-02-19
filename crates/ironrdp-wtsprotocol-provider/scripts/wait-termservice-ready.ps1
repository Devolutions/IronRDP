[CmdletBinding()]
param(
    [Parameter()]
    [ValidateRange(5, 600)]
    [int]$TimeoutSeconds = 90,

    [Parameter()]
    [ValidateRange(1, 30)]
    [int]$PollIntervalSeconds = 2,

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

$termDdServiceKey = "HKLM:\SYSTEM\CurrentControlSet\Services\TermDD"
if (-not (Test-Path -LiteralPath $termDdServiceKey)) {
    throw "rdp tcp transport service key is missing ($termDdServiceKey); this host does not expose the standard TermDD listener path required for mstsc tcp validation"
}

$deadline = (Get-Date).AddSeconds($TimeoutSeconds)

while ((Get-Date) -lt $deadline) {
    $service = Get-Service -Name "TermService" -ErrorAction SilentlyContinue

    if ($null -ne $service -and $service.Status -eq "Running") {
        $listeners = Get-NetTCPConnection -State Listen -LocalPort $PortNumber -ErrorAction SilentlyContinue

        if ($listeners -and $listeners.Count -gt 0) {
            Write-Host "TermService ready"
            Write-Host "  status: Running"
            Write-Host "  listening port: $PortNumber"
            return
        }
    }

    Start-Sleep -Seconds $PollIntervalSeconds
}

$status = "Unknown"
$service = Get-Service -Name "TermService" -ErrorAction SilentlyContinue
if ($null -ne $service) {
    $status = [string]$service.Status
}

$qWinSta = "unavailable"
try {
    $qWinSta = (qwinsta | Out-String).Trim()
}
catch {
}

throw "timeout waiting for TermService/port readiness (status=$status, port=$PortNumber, timeout=${TimeoutSeconds}s); qwinsta: $qWinSta"
