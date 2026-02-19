[CmdletBinding()]
param(
    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ListenerName = "IRDP-Tcp",

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ProtocolManagerClsid = "{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}",

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$OutputDirectory = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $OutputDirectory = Join-Path -Path (Get-Location) -ChildPath ("artifacts\\wtsprotocol-backup-" + $timestamp)
}

New-Item -Path $OutputDirectory -ItemType Directory -Force | Out-Null
$outputPath = (Resolve-Path -LiteralPath $OutputDirectory).Path

$keysToExport = @(
    @{ Key = "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Terminal Server\\WinStations\\RDP-Tcp"; Name = "winstation-rdp-tcp.reg" },
    @{ Key = "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Terminal Server\\WinStations\\$ListenerName"; Name = "winstation-$ListenerName.reg" },
    @{ Key = "HKLM\\SOFTWARE\\Classes\\CLSID\\$ProtocolManagerClsid"; Name = "clsid-$($ProtocolManagerClsid.Trim('{}')).reg" }
)

foreach ($item in $keysToExport) {
    $targetFile = Join-Path -Path $outputPath -ChildPath $item.Name
    $null = & reg.exe export $item.Key $targetFile /y 2>$null

    if ($LASTEXITCODE -eq 0) {
        Write-Host "Exported: $($item.Key) -> $targetFile"
    } else {
        Write-Warning "Skipped (not found or inaccessible): $($item.Key)"
    }
}

Write-Host "Backup complete: $outputPath"
