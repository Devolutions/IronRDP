[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$TargetHost,

    [Parameter()]
    [string]$ProviderDllPath = "",

    [Parameter()]
    [switch]$BuildProvider,

    [Parameter()]
    [ValidateSet("release", "debug")]
    [string]$BuildProfile = "release",

    [Parameter()]
    [bool]$LockedBuild = $true,

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
    [switch]$RestartTermService,

    [Parameter()]
    [bool]$CreateZip = $true,

    [Parameter()]
    [string]$OutputDirectory = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
. (Join-Path $scriptRoot "side-by-side-defaults.ps1")
$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber
$workspaceRoot = (Resolve-Path -LiteralPath (Join-Path $scriptRoot "..\..\..\..\..")).Path

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $workspaceRoot ("artifacts\first-run-package-" + (Get-Date -Format "yyyyMMdd-HHmmss"))
}
$OutputDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)

if (Test-Path -LiteralPath $OutputDirectory) {
    if (-not (Test-Path -LiteralPath $OutputDirectory -PathType Container)) {
        throw "OutputDirectory is not a directory: $OutputDirectory"
    }
    if (@(Get-ChildItem -LiteralPath $OutputDirectory -Force).Count -gt 0) {
        throw "OutputDirectory must be empty so obsolete launchers cannot remain: $OutputDirectory"
    }
}

if ([string]::IsNullOrWhiteSpace($ProviderDllPath)) {
    if ($BuildProvider.IsPresent) {
        $ProviderDllPath = & (Join-Path $scriptRoot "build-provider-dll.ps1") -Profile $BuildProfile -Locked $LockedBuild
    } else {
        $ProviderDllPath = Join-Path $workspaceRoot "target\$BuildProfile\ironrdp_wtsprotocol_provider.dll"
    }
}

# PowerShell 5.1 can interpret brackets in the current directory even with a relative LiteralPath.
$ProviderDllPath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($ProviderDllPath)
if (-not (Test-Path -LiteralPath $ProviderDllPath -PathType Leaf)) {
    throw "Provider DLL does not exist: $ProviderDllPath. Pass -BuildProvider or -ProviderDllPath."
}
$resolvedProviderDllPath = (Resolve-Path -LiteralPath $ProviderDllPath).Path

New-Item -Path $OutputDirectory -ItemType Directory -Force | Out-Null
$resolvedOutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path
$rdpFilePath = Join-Path $resolvedOutputDirectory "irdp-side-by-side.rdp"
& (Join-Path $scriptRoot "new-side-by-side-mstsc-file.ps1") -TargetHost $TargetHost -PortNumber $PortNumber -OutputPath $rdpFilePath

$configuration = [ordered]@{
    createdAtUtc = (Get-Date).ToUniversalTime().ToString("o")
    workspaceRoot = $workspaceRoot
    targetHost = $TargetHost
    listenerName = $ListenerName
    protocolManagerClsid = $ProtocolManagerClsid
    portNumber = $PortNumber
    providerDllPath = $resolvedProviderDllPath
    rdpFilePath = $rdpFilePath
    restartTermService = $RestartTermService.IsPresent
}
$configuration | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $resolvedOutputDirectory "package.json") -Encoding UTF8

$startHere = @'
# First-run provider configuration

Review `package.json`, then run the commands you need from this directory in an elevated PowerShell on the server being configured.
These commands act locally; `targetHost` is the client connection endpoint, not a deployment destination.
The matching companion and signed IDD must already be deployed on Windows Server with the RDS-RD-Server role.
See `SIDEBYSIDE_SETUP.md` in the provider checkout for prerequisites and connection instructions.

This folder and its optional ZIP are not standalone deployment bundles.
They contain no DLL, companion, driver, or helper scripts.
The absolute checkout and DLL paths in `package.json` must exist on the server where commands run; update them if the files move.
The checkout must include `server\termsrv\scripts\TestVm.psm1`, which the installer imports.
The `.rdp` file records the endpoint at generation time; regenerate it if the host or port changes.

## Load the configuration

```powershell
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$config = Get-Content -LiteralPath (Join-Path $PWD.Path 'package.json') -Raw | ConvertFrom-Json
$scripts = Join-Path $config.workspaceRoot 'server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts'
$provider = @{
    ProviderDllPath = $config.providerDllPath
    ListenerName = $config.listenerName
    ProtocolManagerClsid = $config.protocolManagerClsid
    PortNumber = $config.portNumber
}
$registration = @{
    ListenerName = $config.listenerName
    ProtocolManagerClsid = $config.protocolManagerClsid
}
```

## Preflight

```powershell
& (Join-Path $scripts 'preflight-side-by-side.ps1') @provider
```

## Back up before installing

Keep the returned directory for recovery, including after closing this shell.

```powershell
$backupDirectory = & (Join-Path $scripts 'backup-side-by-side-state.ps1') @registration -PassThru
$backupDirectory
```

## Install

`restartTermService` defaults to false.
Setting it to true restarts TermService and disconnects RDP sessions.
Otherwise, restart through your normal server procedure before running the runtime checks.

```powershell
& (Join-Path $scripts 'install-side-by-side.ps1') @provider -RestartTermService:$config.restartTermService
& (Join-Path $scripts 'configure-side-by-side-firewall.ps1') -Mode Add -PortNumber $config.portNumber
```

## Verify registration without connecting

```powershell
& (Join-Path $scripts 'verify-side-by-side.ps1') @provider
```

## Check runtime readiness

Run these only after the companion and TermService have started.
These checks do not prove that an RDP logon or desktop session works.
Use `ironrdp-agent`, not mstsc, to connect to IT-HELP-TERMSRV.

```powershell
& (Join-Path $scripts 'wait-termservice-ready.ps1') -PortNumber $config.portNumber
& (Join-Path $scripts 'smoke-test-side-by-side.ps1') @provider -CheckLocalPortListener
```

## Collect diagnostics

```powershell
& (Join-Path $scripts 'collect-side-by-side-diagnostics.ps1') @provider
```

## Uninstall

This removes the selected provider registration and firewall rule without restoring a backup.

```powershell
& (Join-Path $scripts 'uninstall-side-by-side.ps1') @registration
& (Join-Path $scripts 'configure-side-by-side-firewall.ps1') -Mode Remove -PortNumber $config.portNumber
```

## Restore a saved backup

If this is a new shell, set `$backupDirectory` to the saved path first.
Restart TermService through your normal server procedure after recovery.

```powershell
& (Join-Path $scripts 'restore-side-by-side-state.ps1') -BackupDirectory $backupDirectory
```
'@
$startHerePath = Join-Path $resolvedOutputDirectory "START-HERE.md"
Set-Content -LiteralPath $startHerePath -Value $startHere -Encoding UTF8

if ($CreateZip) {
    $zipPath = Join-Path (Split-Path $resolvedOutputDirectory -Parent) ((Split-Path $resolvedOutputDirectory -Leaf) + ".zip")
    Compress-Archive -LiteralPath $resolvedOutputDirectory -DestinationPath $zipPath -Force
    $zipHash = Get-FileHash -LiteralPath $zipPath -Algorithm SHA256
    Set-Content -LiteralPath ($zipPath + ".sha256.txt") -Value $zipHash.Hash -Encoding ASCII
    Write-Host "Configuration archive (not a deployment bundle): $zipPath"
}

Write-Host "First-run configuration prepared: $resolvedOutputDirectory"
Write-Host "  provider dll: $resolvedProviderDllPath"
Write-Host "  guide: $startHerePath"
