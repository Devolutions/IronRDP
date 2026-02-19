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
$defaultsScript = Join-Path -Path $scriptRoot -ChildPath "side-by-side-defaults.ps1"
. $defaultsScript

$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber

$crateRoot = Resolve-Path -LiteralPath (Join-Path -Path $scriptRoot -ChildPath "..")
$workspaceRoot = Resolve-Path -LiteralPath (Join-Path -Path $crateRoot -ChildPath "..\\..")

$buildProviderScript = Join-Path -Path $scriptRoot -ChildPath "build-provider-dll.ps1"
$newRdpFileScript = Join-Path -Path $scriptRoot -ChildPath "new-side-by-side-mstsc-file.ps1"
$collectDiagnosticsScriptRel = ".\\crates\\ironrdp-wtsprotocol-provider\\scripts\\collect-side-by-side-diagnostics.ps1"
$orchestratorScriptRel = ".\\crates\\ironrdp-wtsprotocol-provider\\scripts\\first-run-side-by-side.ps1"
$preflightScriptRel = ".\\crates\\ironrdp-wtsprotocol-provider\\scripts\\preflight-side-by-side.ps1"
$installScriptRel = ".\\crates\\ironrdp-wtsprotocol-provider\\scripts\\install-side-by-side.ps1"
$verifyScriptRel = ".\\crates\\ironrdp-wtsprotocol-provider\\scripts\\verify-side-by-side.ps1"
$smokeScriptRel = ".\\crates\\ironrdp-wtsprotocol-provider\\scripts\\smoke-test-side-by-side.ps1"

function Quote-ForCommand {
    param([Parameter(Mandatory = $true)][string]$Value)

    return ('"' + $Value.Replace('"', '""') + '"')
}

function New-WrapperScript {
    param(
        [Parameter(Mandatory = $true)][string]$OutputPath,
        [Parameter(Mandatory = $true)][string]$WorkspaceRoot,
        [Parameter(Mandatory = $true)][string]$CommandLine
    )

    $wrapperContent = @"
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
`$ErrorActionPreference = "Stop"

Push-Location $(Quote-ForCommand -Value $WorkspaceRoot)
try {
    $CommandLine
}
finally {
    Pop-Location
}
"@

    Set-Content -LiteralPath $OutputPath -Value $wrapperContent -Encoding UTF8
}

function New-ElevatingProxyScript {
    param(
        [Parameter(Mandatory = $true)][string]$OutputPath,
        [Parameter(Mandatory = $true)][string]$TargetScriptName
    )

    $proxyContent = @"
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
`$ErrorActionPreference = "Stop"

function Test-IsAdministrator {
    `$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    `$principal = New-Object Security.Principal.WindowsPrincipal(`$identity)
    return `$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

`$scriptDirectory = Split-Path -Parent `$PSCommandPath
`$targetScript = Join-Path -Path `$scriptDirectory -ChildPath $(Quote-ForCommand -Value $TargetScriptName)

if (-not (Test-IsAdministrator)) {
    `$arguments = @(
        '-NoProfile',
        '-ExecutionPolicy', 'Bypass',
        '-File', `$targetScript
    )

    Start-Process -FilePath 'powershell.exe' -Verb RunAs -ArgumentList `$arguments | Out-Null
    return
}

& `$targetScript
"@

    Set-Content -LiteralPath $OutputPath -Value $proxyContent -Encoding UTF8
}

function New-CmdLauncher {
    param(
        [Parameter(Mandatory = $true)][string]$OutputPath,
        [Parameter(Mandatory = $true)][string]$TargetScriptName
    )

    $cmdContent = @(
        "@echo off",
        "setlocal",
        "set SCRIPT_DIR=%~dp0",
        ('powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT_DIR%' + $TargetScriptName + '" %*'),
        "set EXIT_CODE=%ERRORLEVEL%",
        "endlocal & exit /b %EXIT_CODE%"
    )

    Set-Content -LiteralPath $OutputPath -Value $cmdContent -Encoding ASCII
}

function Resolve-DefaultProviderDllPath {
    $targetSubdir = if ($BuildProfile -eq "release") { "release" } else { "debug" }
    return Join-Path -Path $workspaceRoot -ChildPath ("target\\" + $targetSubdir + "\\ironrdp_wtsprotocol_provider.dll")
}

if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $OutputDirectory = Join-Path -Path (Join-Path -Path $workspaceRoot -ChildPath "artifacts") -ChildPath ("first-run-package-" + $timestamp)
}

New-Item -Path $OutputDirectory -ItemType Directory -Force | Out-Null
$resolvedOutputDirectory = (Resolve-Path -LiteralPath $OutputDirectory).Path

if ([string]::IsNullOrWhiteSpace($ProviderDllPath)) {
    if ($BuildProvider.IsPresent) {
        $ProviderDllPath = (& $buildProviderScript -Profile $BuildProfile -Locked $LockedBuild)
    } else {
        $defaultPath = Resolve-DefaultProviderDllPath
        if (Test-Path -LiteralPath $defaultPath -PathType Leaf) {
            $ProviderDllPath = (Resolve-Path -LiteralPath $defaultPath).Path
        } else {
            throw "Provider DLL was not provided and default path does not exist: $defaultPath. Pass -BuildProvider or -ProviderDllPath."
        }
    }
}

if (-not (Test-Path -LiteralPath $ProviderDllPath -PathType Leaf)) {
    throw "provider dll path does not exist: $ProviderDllPath"
}

$resolvedProviderDllPath = (Resolve-Path -LiteralPath $ProviderDllPath).Path

$rdpFilePath = Join-Path -Path $resolvedOutputDirectory -ChildPath "irdp-side-by-side.rdp"
& $newRdpFileScript -TargetHost $TargetHost -PortNumber $PortNumber -OutputPath $rdpFilePath
$resolvedRdpFilePath = (Resolve-Path -LiteralPath $rdpFilePath).Path

$baseInstall = @(
    $orchestratorScriptRel,
    "-Mode Install",
    "-ProviderDllPath " + (Quote-ForCommand -Value $resolvedProviderDllPath),
    "-TargetHost " + (Quote-ForCommand -Value $TargetHost),
    "-ListenerName " + (Quote-ForCommand -Value $ListenerName),
    "-ProtocolManagerClsid " + (Quote-ForCommand -Value $ProtocolManagerClsid),
    "-PortNumber $PortNumber",
    "-GenerateRdpFile",
    "-RdpOutputPath " + (Quote-ForCommand -Value $resolvedRdpFilePath)
)

if ($RestartTermService.IsPresent) {
    $baseInstall += "-RestartTermService"
    $baseInstall += "-WaitForServiceReadyAfterRestart"
}

$installCommand = $baseInstall -join " "

$installWithRestartCommand = @(
    $baseInstall + @(
        "-RestartTermService",
        "-WaitForServiceReadyAfterRestart"
    )
) -join " "

$rollbackCommand = @(
    $orchestratorScriptRel,
    "-Mode Rollback",
    "-ListenerName " + (Quote-ForCommand -Value $ListenerName),
    "-ProtocolManagerClsid " + (Quote-ForCommand -Value $ProtocolManagerClsid),
    "-PortNumber $PortNumber",
    "-RestoreBackupOnRollback"
) -join " "

$connectCommand = "mstsc /v:$TargetHost`:$PortNumber"

$diagnosticsCommand = @(
    $collectDiagnosticsScriptRel,
    "-ListenerName " + (Quote-ForCommand -Value $ListenerName),
    "-ProtocolManagerClsid " + (Quote-ForCommand -Value $ProtocolManagerClsid),
    "-PortNumber $PortNumber",
    "-ProviderDllPath " + (Quote-ForCommand -Value $resolvedProviderDllPath)
) -join " "

$previewCommand = @(
    $orchestratorScriptRel,
    "-Mode Preview"
) -join " "

$preflightCommand = @(
    $preflightScriptRel,
    "-ProviderDllPath " + (Quote-ForCommand -Value $resolvedProviderDllPath),
    "-ListenerName " + (Quote-ForCommand -Value $ListenerName),
    "-PortNumber $PortNumber"
) -join " "

$verifyCommand = @(
    $verifyScriptRel,
    "-ProviderDllPath " + (Quote-ForCommand -Value $resolvedProviderDllPath),
    "-ListenerName " + (Quote-ForCommand -Value $ListenerName),
    "-PortNumber $PortNumber"
) -join " "

$smokeCommand = @(
    $smokeScriptRel,
    "-ProviderDllPath " + (Quote-ForCommand -Value $resolvedProviderDllPath),
    "-ListenerName " + (Quote-ForCommand -Value $ListenerName),
    "-PortNumber $PortNumber"
) -join " "

$manualInstallCommand = @(
    $installScriptRel,
    "-ProviderDllPath " + (Quote-ForCommand -Value $resolvedProviderDllPath),
    "-ListenerName " + (Quote-ForCommand -Value $ListenerName),
    "-ProtocolManagerClsid " + (Quote-ForCommand -Value $ProtocolManagerClsid),
    "-PortNumber $PortNumber"
) -join " "

$manualSteps = @(
    $previewCommand,
    $preflightCommand,
    $manualInstallCommand,
    $verifyCommand,
    $smokeCommand,
    $connectCommand
)

$firstRunCommand = @(
    $previewCommand,
    $preflightCommand,
    $installWithRestartCommand,
    $verifyCommand,
    $smokeCommand,
    $connectCommand
) -join "; "

Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "install-now.ps1.txt") -Value $installCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "install-with-restart-now.ps1.txt") -Value $installWithRestartCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "rollback-now.ps1.txt") -Value $rollbackCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "connect-now.txt") -Value $connectCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "collect-diagnostics-now.ps1.txt") -Value $diagnosticsCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "preview-now.ps1.txt") -Value $previewCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "preflight-now.ps1.txt") -Value $preflightCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "first-run-now.ps1.txt") -Value $firstRunCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "verify-now.ps1.txt") -Value $verifyCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "smoke-now.ps1.txt") -Value $smokeCommand -Encoding UTF8
Set-Content -LiteralPath (Join-Path -Path $resolvedOutputDirectory -ChildPath "manual-steps.ps1.txt") -Value $manualSteps -Encoding UTF8

$runFirstRunScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-first-run.ps1"
$runFirstRunElevatedScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-first-run-elevated.ps1"
$runPreviewScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-preview.ps1"
$runPreflightScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-preflight.ps1"
$runInstallScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-install.ps1"
$runInstallRestartScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-install-restart.ps1"
$runVerifyScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-verify.ps1"
$runSmokeScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-smoke.ps1"
$runRollbackScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-rollback.ps1"
$runDiagnosticsScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-diagnostics.ps1"
$runConnectScriptPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-connect.ps1"

$runFirstRunCmdPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-first-run.cmd"
$runFirstRunElevatedCmdPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-first-run-elevated.cmd"
$runDiagnosticsCmdPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-diagnostics.cmd"
$runRollbackCmdPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-rollback.cmd"
$runConnectCmdPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "run-connect.cmd"

New-WrapperScript -OutputPath $runFirstRunScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $firstRunCommand
New-ElevatingProxyScript -OutputPath $runFirstRunElevatedScriptPath -TargetScriptName "run-first-run.ps1"
New-WrapperScript -OutputPath $runPreviewScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $previewCommand
New-WrapperScript -OutputPath $runPreflightScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $preflightCommand
New-WrapperScript -OutputPath $runInstallScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $installCommand
New-WrapperScript -OutputPath $runInstallRestartScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $installWithRestartCommand
New-WrapperScript -OutputPath $runVerifyScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $verifyCommand
New-WrapperScript -OutputPath $runSmokeScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $smokeCommand
New-WrapperScript -OutputPath $runRollbackScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $rollbackCommand
New-WrapperScript -OutputPath $runDiagnosticsScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $diagnosticsCommand
New-WrapperScript -OutputPath $runConnectScriptPath -WorkspaceRoot $workspaceRoot -CommandLine $connectCommand

New-CmdLauncher -OutputPath $runFirstRunCmdPath -TargetScriptName "run-first-run.ps1"
New-CmdLauncher -OutputPath $runFirstRunElevatedCmdPath -TargetScriptName "run-first-run-elevated.ps1"
New-CmdLauncher -OutputPath $runDiagnosticsCmdPath -TargetScriptName "run-diagnostics.ps1"
New-CmdLauncher -OutputPath $runRollbackCmdPath -TargetScriptName "run-rollback.ps1"
New-CmdLauncher -OutputPath $runConnectCmdPath -TargetScriptName "run-connect.ps1"

$startHerePath = Join-Path -Path $resolvedOutputDirectory -ChildPath "START-HERE.md"
$startHere = @(
    "# IronRDP first-run package",
    "",
    "1. Open an elevated PowerShell.",
    "2. Quick path: run ./run-first-run-elevated.ps1 (auto-prompts UAC if needed).",
    "   Alternative: run ./run-first-run.ps1 from an already elevated shell.",
    "   You can also run run-first-run-elevated.cmd or run-first-run.cmd.",
    "3. Step-by-step path:",
    "   - ./run-preview.ps1",
    "   - ./run-preflight.ps1",
    "   - ./run-install-restart.ps1 (or ./run-install.ps1 + manual TermService restart)",
    "   - ./run-verify.ps1 and ./run-smoke.ps1",
    "   - ./run-connect.ps1 or open irdp-side-by-side.rdp",
    "4. If connection fails, run ./run-diagnostics.ps1.",
    "5. Roll back using ./run-rollback.ps1.",
    "",
    "Workspace root:",
    "- $workspaceRoot",
    "",
    "Target endpoint:",
    "- ${TargetHost}:$PortNumber"
)
Set-Content -LiteralPath $startHerePath -Value $startHere -Encoding UTF8

$packageMetadata = [PSCustomObject]@{
    createdAtUtc = (Get-Date).ToUniversalTime().ToString("o")
    workspaceRoot = [string]$workspaceRoot
    targetHost = $TargetHost
    listenerName = $ListenerName
    protocolManagerClsid = $ProtocolManagerClsid
    portNumber = $PortNumber
    providerDllPath = $resolvedProviderDllPath
    rdpFilePath = $resolvedRdpFilePath
    installCommandFile = "install-now.ps1.txt"
    installWithRestartCommandFile = "install-with-restart-now.ps1.txt"
    rollbackCommandFile = "rollback-now.ps1.txt"
    connectCommandFile = "connect-now.txt"
    diagnosticsCommandFile = "collect-diagnostics-now.ps1.txt"
    firstRunCommandFile = "first-run-now.ps1.txt"
    previewCommandFile = "preview-now.ps1.txt"
    preflightCommandFile = "preflight-now.ps1.txt"
    verifyCommandFile = "verify-now.ps1.txt"
    smokeCommandFile = "smoke-now.ps1.txt"
    manualStepsFile = "manual-steps.ps1.txt"
    runFirstRunElevatedScript = "run-first-run-elevated.ps1"
    runFirstRunScript = "run-first-run.ps1"
    runPreviewScript = "run-preview.ps1"
    runPreflightScript = "run-preflight.ps1"
    runInstallScript = "run-install.ps1"
    runInstallRestartScript = "run-install-restart.ps1"
    runVerifyScript = "run-verify.ps1"
    runSmokeScript = "run-smoke.ps1"
    runRollbackScript = "run-rollback.ps1"
    runDiagnosticsScript = "run-diagnostics.ps1"
    runConnectScript = "run-connect.ps1"
    runFirstRunCmd = "run-first-run.cmd"
    runFirstRunElevatedCmd = "run-first-run-elevated.cmd"
    runDiagnosticsCmd = "run-diagnostics.cmd"
    runRollbackCmd = "run-rollback.cmd"
    runConnectCmd = "run-connect.cmd"
    startHereFile = "START-HERE.md"
}

$metadataPath = Join-Path -Path $resolvedOutputDirectory -ChildPath "package.json"
$packageMetadata | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $metadataPath -Encoding UTF8

$zipPath = ""
$zipSha256Path = ""

if ($CreateZip) {
    $packageRoot = Split-Path -Path $resolvedOutputDirectory -Parent
    $packageName = Split-Path -Path $resolvedOutputDirectory -Leaf
    $zipPath = Join-Path -Path $packageRoot -ChildPath ($packageName + ".zip")

    Compress-Archive -LiteralPath $resolvedOutputDirectory -DestinationPath $zipPath -Force

    $zipHash = Get-FileHash -LiteralPath $zipPath -Algorithm SHA256
    $zipSha256Path = $zipPath + ".sha256.txt"
    Set-Content -LiteralPath $zipSha256Path -Value $zipHash.Hash -Encoding ASCII

    Write-Host "  package zip: $zipPath"
    Write-Host "  package zip sha256: $zipSha256Path"
}

Write-Host "First-run package prepared: $resolvedOutputDirectory"
Write-Host "  provider dll: $resolvedProviderDllPath"
Write-Host "  rdp file: $resolvedRdpFilePath"
Write-Host "  install command file: $(Join-Path -Path $resolvedOutputDirectory -ChildPath 'install-now.ps1.txt')"
Write-Host "  install+restart command file: $(Join-Path -Path $resolvedOutputDirectory -ChildPath 'install-with-restart-now.ps1.txt')"
Write-Host "  first-run command file: $(Join-Path -Path $resolvedOutputDirectory -ChildPath 'first-run-now.ps1.txt')"
Write-Host "  first-run elevated launcher: $runFirstRunElevatedScriptPath"
Write-Host "  first-run cmd launcher: $runFirstRunCmdPath"
Write-Host "  rollback command file: $(Join-Path -Path $resolvedOutputDirectory -ChildPath 'rollback-now.ps1.txt')"
Write-Host "  diagnostics command file: $(Join-Path -Path $resolvedOutputDirectory -ChildPath 'collect-diagnostics-now.ps1.txt')"
Write-Host "  preflight command file: $(Join-Path -Path $resolvedOutputDirectory -ChildPath 'preflight-now.ps1.txt')"
Write-Host "  run script: $runFirstRunScriptPath"
Write-Host "  start guide: $startHerePath"
