[CmdletBinding()]
param(
    [Parameter()]
    [string]$ServiceExePath = "..\\..\\..\\..\\target\\release\\ironrdp-termsrv.exe",

    [Parameter()]
    [string]$ProviderDllPath = "..\\..\\..\\..\\target\\release\\ironrdp_wtsprotocol_provider.dll",

    [Parameter()]
    [ValidateSet("x64", "x86", "arm64")]
    [string]$Platform = "x64",

    [Parameter()]
    [string]$Version = "",

    [Parameter()]
    [string]$ConfigDirectory = "",

    [Parameter()]
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Release"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$packageRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $packageRoot
try {
    $resolvedServicePath = Resolve-Path -LiteralPath $ServiceExePath -ErrorAction Stop
    $env:IRDP_TERMSRV_SERVICE_EXECUTABLE = $resolvedServicePath.Path
    $env:IRDP_CEVICHE_SERVICE_EXECUTABLE = $resolvedServicePath.Path

    if (-not [string]::IsNullOrWhiteSpace($ProviderDllPath)) {
        if (Test-Path -LiteralPath $ProviderDllPath -PathType Leaf) {
            $resolvedProviderDllPath = Resolve-Path -LiteralPath $ProviderDllPath
            $env:IRDP_PROVIDER_DLL = $resolvedProviderDllPath.Path
        } else {
            Remove-Item Env:IRDP_PROVIDER_DLL -ErrorAction SilentlyContinue
        }
    }

    if (-not [string]::IsNullOrWhiteSpace($ConfigDirectory)) {
        $resolvedConfigDirectory = Resolve-Path -LiteralPath $ConfigDirectory -ErrorAction Stop
        $env:IRDP_TERMSRV_CONFIG_DIR = $resolvedConfigDirectory.Path
        $env:IRDP_CEVICHE_CONFIG_DIR = $resolvedConfigDirectory.Path
    } else {
        Remove-Item Env:IRDP_TERMSRV_CONFIG_DIR -ErrorAction SilentlyContinue
        Remove-Item Env:IRDP_CEVICHE_CONFIG_DIR -ErrorAction SilentlyContinue
    }

    $env:IRDP_TERMSRV_MSI_PLATFORM = $Platform
    $env:IRDP_CEVICHE_MSI_PLATFORM = $Platform

    if (-not [string]::IsNullOrWhiteSpace($Version)) {
        $env:IRDP_TERMSRV_MSI_VERSION = $Version
        $env:IRDP_CEVICHE_MSI_VERSION = $Version
    } else {
        Remove-Item Env:IRDP_TERMSRV_MSI_VERSION -ErrorAction SilentlyContinue
        Remove-Item Env:IRDP_CEVICHE_MSI_VERSION -ErrorAction SilentlyContinue
    }

    $msiPath = Join-Path -Path $packageRoot -ChildPath (Join-Path -Path $Configuration -ChildPath "IronRdpTermSrv.msi")
    & dotnet build .\IronRdpTermSrv.Installer.csproj -c $Configuration
    if ($LASTEXITCODE -ne 0) {
        throw "dotnet build failed with exit code $LASTEXITCODE"
    }

    if (Test-Path -LiteralPath $msiPath -PathType Leaf) {
        Remove-Item -LiteralPath $msiPath -Force
    }

    & dotnet run --project .\IronRdpTermSrv.Installer.csproj -c $Configuration
    if ($LASTEXITCODE -ne 0) {
        throw "dotnet run failed with exit code $LASTEXITCODE"
    }

    if (Test-Path -LiteralPath $msiPath -PathType Leaf) {
        Write-Host "MSI ready: $msiPath"
    } else {
        Write-Warning "MSI build finished but expected output was not found at: $msiPath"
    }
}
finally {
    Pop-Location
}
