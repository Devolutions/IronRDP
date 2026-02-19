[CmdletBinding()]
param(
    [Parameter()]
    [string]$ServiceExePath = "..\\..\\target\\release\\ceviche-service.exe",

    [Parameter()]
    [string]$ProviderDllPath = "..\\..\\target\\release\\ironrdp_wtsprotocol_provider.dll",

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
        $env:IRDP_CEVICHE_CONFIG_DIR = $resolvedConfigDirectory.Path
    } else {
        Remove-Item Env:IRDP_CEVICHE_CONFIG_DIR -ErrorAction SilentlyContinue
    }

    $env:IRDP_CEVICHE_MSI_PLATFORM = $Platform

    if (-not [string]::IsNullOrWhiteSpace($Version)) {
        $env:IRDP_CEVICHE_MSI_VERSION = $Version
    } else {
        Remove-Item Env:IRDP_CEVICHE_MSI_VERSION -ErrorAction SilentlyContinue
    }

    dotnet build .\CevicheServiceWindowsManaged.csproj -c $Configuration
    dotnet run --project .\CevicheServiceWindowsManaged.csproj -c $Configuration

    $msiPath = Join-Path -Path $packageRoot -ChildPath (Join-Path -Path $Configuration -ChildPath "IronRdpCevicheService.msi")
    if (Test-Path -LiteralPath $msiPath -PathType Leaf) {
        Write-Host "MSI ready: $msiPath"
    } else {
        Write-Warning "MSI build finished but expected output was not found at: $msiPath"
    }
}
finally {
    Pop-Location
}
