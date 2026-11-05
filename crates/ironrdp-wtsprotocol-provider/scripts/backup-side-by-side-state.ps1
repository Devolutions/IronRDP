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
    [string]$OutputDirectory = "",

    [Parameter()]
    [switch]$PassThru
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
    $crateRoot = Resolve-Path -LiteralPath (Join-Path -Path $scriptRoot -ChildPath "..")
    $workspaceRoot = Resolve-Path -LiteralPath (Join-Path -Path $crateRoot -ChildPath "..\\..")
    $artifactsRoot = Join-Path -Path $workspaceRoot -ChildPath "artifacts"

    $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $OutputDirectory = Join-Path -Path $artifactsRoot -ChildPath ("wtsprotocol-backup-" + $timestamp)
}

New-Item -Path $OutputDirectory -ItemType Directory -Force | Out-Null
$outputPath = (Resolve-Path -LiteralPath $OutputDirectory).Path

$keysToExport = @(
    @{ Key = "HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\RDP-Tcp"; Name = "winstation-rdp-tcp.reg" },
    @{ Key = "HKLM\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\$ListenerName"; Name = "winstation-$ListenerName.reg" },
    @{ Key = "HKLM\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid"; Name = "clsid-$($ProtocolManagerClsid.Trim('{}')).reg" }
)

foreach ($item in $keysToExport) {
    $targetFile = Join-Path -Path $outputPath -ChildPath $item.Name
    $exportExitCode = 0

    try {
        $null = & reg.exe export $item.Key $targetFile /y 2>$null
        $exportExitCode = $LASTEXITCODE
    }
    catch {
        $exportExitCode = 1
    }

    if ($exportExitCode -eq 0) {
        Write-Host "Exported: $($item.Key) -> $targetFile"
    } else {
        Write-Warning "Skipped (not found or inaccessible): $($item.Key)"
    }
}

$manifest = [PSCustomObject]@{
    createdAtUtc = (Get-Date).ToUniversalTime().ToString("o")
    listenerName = $ListenerName
    protocolManagerClsid = $ProtocolManagerClsid
    outputDirectory = $outputPath
    files = @(
        "winstation-rdp-tcp.reg",
        "winstation-$ListenerName.reg",
        "clsid-$($ProtocolManagerClsid.Trim('{}')).reg"
    )
}

$manifestPath = Join-Path -Path $outputPath -ChildPath "manifest.json"
$manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath $manifestPath -Encoding UTF8

Write-Host "Backup complete: $outputPath"
Write-Host "Manifest: $manifestPath"

if ($PassThru.IsPresent) {
    Write-Output $outputPath
}
