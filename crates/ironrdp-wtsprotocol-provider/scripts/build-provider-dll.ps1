[CmdletBinding()]
param(
    [Parameter()]
    [ValidateSet("release", "debug")]
    [string]$Profile = "release",

    [Parameter()]
    [bool]$Locked = $true
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$crateRoot = Resolve-Path -LiteralPath (Join-Path -Path $scriptRoot -ChildPath "..")
$workspaceRoot = Resolve-Path -LiteralPath (Join-Path -Path $crateRoot -ChildPath "..\\..")

$targetSubdir = if ($Profile -eq "release") { "release" } else { "debug" }
$providerDllPath = Join-Path -Path $workspaceRoot -ChildPath ("target\\" + $targetSubdir + "\\ironrdp_wtsprotocol_provider.dll")

$buildArgs = @("build", "-p", "ironrdp-wtsprotocol-provider")
if ($Profile -eq "release") {
    $buildArgs += "--release"
}
if ($Locked) {
    $buildArgs += "--locked"
}

Write-Host "Building provider DLL"
Write-Host "  profile: $Profile"
Write-Host "  locked: $Locked"

Push-Location $workspaceRoot
try {
    & cargo @buildArgs
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

if (-not (Test-Path -LiteralPath $providerDllPath -PathType Leaf)) {
    throw "provider dll not found after build: $providerDllPath"
}

$resolvedProviderDllPath = (Resolve-Path -LiteralPath $providerDllPath).Path
Write-Host "Provider DLL built: $resolvedProviderDllPath"

$resolvedProviderDllPath
