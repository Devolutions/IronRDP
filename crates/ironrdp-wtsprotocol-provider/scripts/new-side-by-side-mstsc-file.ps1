[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$TargetHost,

    [Parameter()]
    [ValidateRange(1, 65535)]
    [int]$PortNumber = 3390,

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$OutputPath = ".\\artifacts\\irdp-side-by-side.rdp"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$outputDirectory = Split-Path -Path $OutputPath -Parent
if (-not [string]::IsNullOrWhiteSpace($outputDirectory)) {
    New-Item -Path $outputDirectory -ItemType Directory -Force | Out-Null
}

$fullAddress = "$TargetHost`:$PortNumber"
$rdpContent = @(
    "screen mode id:i:2",
    "use multimon:i:0",
    "desktopwidth:i:1600",
    "desktopheight:i:900",
    "session bpp:i:32",
    "compression:i:1",
    "keyboardhook:i:2",
    "audiocapturemode:i:0",
    "videoplaybackmode:i:1",
    "connection type:i:7",
    "networkautodetect:i:1",
    "bandwidthautodetect:i:1",
    "displayconnectionbar:i:1",
    "enableworkspacereconnect:i:0",
    "disable wallpaper:i:0",
    "allow font smoothing:i:1",
    "allow desktop composition:i:1",
    "redirectclipboard:i:1",
    "redirectprinters:i:0",
    "redirectcomports:i:0",
    "redirectsmartcards:i:0",
    "redirectdrives:i:0",
    "redirectposdevices:i:0",
    "autoreconnection enabled:i:1",
    "authentication level:i:2",
    "prompt for credentials:i:1",
    "negotiate security layer:i:1",
    "enablecredsspsupport:i:1",
    "full address:s:$fullAddress",
    "alternate full address:s:$fullAddress"
)

Set-Content -LiteralPath $OutputPath -Value $rdpContent -Encoding Unicode

$resolved = (Resolve-Path -LiteralPath $OutputPath).Path
Write-Host "Created mstsc file: $resolved"
Write-Host "Target: $fullAddress"
