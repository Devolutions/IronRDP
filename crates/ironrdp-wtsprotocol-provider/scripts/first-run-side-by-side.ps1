[CmdletBinding()]
param(
    [Parameter()]
    [ValidateSet("Preview", "Install", "Rollback")]
    [string]$Mode = "Preview",

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
    [string]$BackupDirectory = "",

    [Parameter()]
    [string]$TargetHost = "localhost",

    [Parameter()]
    [switch]$RestartTermService,

    [Parameter()]
    [switch]$WaitForServiceReadyAfterRestart,

    [Parameter()]
    [ValidateRange(5, 600)]
    [int]$ServiceReadyTimeoutSeconds = 90,

    [Parameter()]
    [switch]$SkipFirewall,

    [Parameter()]
    [switch]$RestoreBackupOnRollback,

    [Parameter()]
    [switch]$SkipSmokeTest,

    [Parameter()]
    [switch]$GenerateRdpFile,

    [Parameter()]
    [string]$RdpOutputPath = ".\\artifacts\\irdp-side-by-side.rdp"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($WaitForServiceReadyAfterRestart.IsPresent -and -not $RestartTermService.IsPresent) {
    throw "WaitForServiceReadyAfterRestart requires RestartTermService"
}

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$defaultsScript = Join-Path -Path $scriptRoot -ChildPath "side-by-side-defaults.ps1"
. $defaultsScript

$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber

$crateRoot = Resolve-Path -LiteralPath (Join-Path -Path $scriptRoot -ChildPath "..")
$workspaceRoot = Resolve-Path -LiteralPath (Join-Path -Path $crateRoot -ChildPath "..\\..")
$preflightScript = Join-Path -Path $scriptRoot -ChildPath "preflight-side-by-side.ps1"
$buildProviderScript = Join-Path -Path $scriptRoot -ChildPath "build-provider-dll.ps1"
$backupScript = Join-Path -Path $scriptRoot -ChildPath "backup-side-by-side-state.ps1"
$installScript = Join-Path -Path $scriptRoot -ChildPath "install-side-by-side.ps1"
$verifyScript = Join-Path -Path $scriptRoot -ChildPath "verify-side-by-side.ps1"
$smokeScript = Join-Path -Path $scriptRoot -ChildPath "smoke-test-side-by-side.ps1"
$waitServiceScript = Join-Path -Path $scriptRoot -ChildPath "wait-termservice-ready.ps1"
$firewallScript = Join-Path -Path $scriptRoot -ChildPath "configure-side-by-side-firewall.ps1"
$uninstallScript = Join-Path -Path $scriptRoot -ChildPath "uninstall-side-by-side.ps1"
$restoreScript = Join-Path -Path $scriptRoot -ChildPath "restore-side-by-side-state.ps1"
$newRdpFileScript = Join-Path -Path $scriptRoot -ChildPath "new-side-by-side-mstsc-file.ps1"

function Assert-ProviderPathRequired {
    if ([string]::IsNullOrWhiteSpace($ProviderDllPath)) {
        throw "ProviderDllPath is required for Mode=$Mode"
    }
}

function Resolve-DefaultProviderDllPath {
    $targetSubdir = if ($BuildProfile -eq "release") { "release" } else { "debug" }
    return Join-Path -Path $workspaceRoot -ChildPath ("target\\" + $targetSubdir + "\\ironrdp_wtsprotocol_provider.dll")
}

function Ensure-ProviderDllPath {
    if (-not [string]::IsNullOrWhiteSpace($ProviderDllPath)) {
        return
    }

    if ($BuildProvider.IsPresent) {
        $script:ProviderDllPath = (& $buildProviderScript -Profile $BuildProfile -Locked $LockedBuild)
        return
    }

    $defaultProviderDllPath = Resolve-DefaultProviderDllPath
    if (Test-Path -LiteralPath $defaultProviderDllPath -PathType Leaf) {
        $script:ProviderDllPath = (Resolve-Path -LiteralPath $defaultProviderDllPath).Path
        Write-Host "Using detected provider DLL: $script:ProviderDllPath"
        return
    }

    throw "ProviderDllPath is required or use -BuildProvider; looked for default path: $defaultProviderDllPath"
}

function Show-Plan {
    Write-Host "Mode: $Mode"

    if ($Mode -eq "Install") {
        Write-Host "Planned actions:"
        Write-Host "  1) Backup registry state"
        Write-Host "  2) Preflight checks"
        Write-Host "  3) Install side-by-side listener"
        Write-Host "  4) Verify registration"
        if (-not $SkipFirewall.IsPresent) {
            Write-Host "  5) Add and verify firewall rule for port $PortNumber"
        }

        if (-not $SkipSmokeTest.IsPresent) {
            Write-Host "  6) Run smoke test checks"
        }

        if ($GenerateRdpFile.IsPresent) {
            Write-Host "  7) Generate mstsc .rdp file"
        }

        Write-Host "  8) Optional TermService restart"
        if ($RestartTermService.IsPresent -and $WaitForServiceReadyAfterRestart.IsPresent) {
            Write-Host "  9) Wait for TermService and port readiness"
            Write-Host "  10) Connect: mstsc /v:$TargetHost`:$PortNumber"
        } else {
            Write-Host "  9) Connect: mstsc /v:$TargetHost`:$PortNumber"
        }
        if ($BuildProvider.IsPresent) {
            Write-Host "  build provider: enabled (profile=$BuildProfile locked=$LockedBuild)"
        }
        return
    }

    if ($Mode -eq "Rollback") {
        Write-Host "Planned actions:"
        Write-Host "  1) Uninstall side-by-side listener registration"
        if (-not $SkipFirewall.IsPresent) {
            Write-Host "  2) Remove firewall rule"
        }
        if ($RestoreBackupOnRollback.IsPresent) {
            Write-Host "  3) Restore registry backup from: $BackupDirectory"
        }
        Write-Host "  4) Optional TermService restart"
        return
    }

    Write-Host "Planned install command sequence preview:"
    Write-Host "  .\backup-side-by-side-state.ps1 -ListenerName $ListenerName"
    if ($BuildProvider.IsPresent) {
        Write-Host "  .\build-provider-dll.ps1 -Profile $BuildProfile -Locked:$LockedBuild"
    } else {
        Write-Host "  ProviderDllPath: explicit value or auto-detect from target\\$BuildProfile\\ironrdp_wtsprotocol_provider.dll"
    }
    Write-Host "  .\preflight-side-by-side.ps1 -ProviderDllPath <resolved-path> -ListenerName $ListenerName -PortNumber $PortNumber"
    Write-Host "  .\install-side-by-side.ps1 -ProviderDllPath <resolved-path> -ListenerName $ListenerName -PortNumber $PortNumber"
    Write-Host "  .\verify-side-by-side.ps1 -ProviderDllPath <resolved-path> -ListenerName $ListenerName -PortNumber $PortNumber"
    if (-not $SkipFirewall.IsPresent) {
        Write-Host "  .\configure-side-by-side-firewall.ps1 -Mode Add -PortNumber $PortNumber"
    }
    if (-not $SkipSmokeTest.IsPresent) {
        Write-Host "  .\smoke-test-side-by-side.ps1 -ProviderDllPath <path> -ListenerName $ListenerName -PortNumber $PortNumber"
    }
    if ($GenerateRdpFile.IsPresent) {
        Write-Host "  .\new-side-by-side-mstsc-file.ps1 -TargetHost $TargetHost -PortNumber $PortNumber -OutputPath $RdpOutputPath"
    }
    if ($RestartTermService.IsPresent -and $WaitForServiceReadyAfterRestart.IsPresent) {
        Write-Host "  .\wait-termservice-ready.ps1 -PortNumber $PortNumber -TimeoutSeconds $ServiceReadyTimeoutSeconds"
    }
    Write-Host "  mstsc /v:$TargetHost`:$PortNumber"
}

function Resolve-LatestBackupDirectory {
    $backupRoot = Join-Path -Path $workspaceRoot -ChildPath "artifacts"
    if (-not (Test-Path -LiteralPath $backupRoot -PathType Container)) {
        return ""
    }

    $latest = Get-ChildItem -LiteralPath $backupRoot -Directory -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "wtsprotocol-backup-*" } |
        Sort-Object -Property LastWriteTimeUtc -Descending |
        Select-Object -First 1

    if ($null -eq $latest) {
        return ""
    }

    return $latest.FullName
}

if ($Mode -eq "Preview") {
    Show-Plan
    return
}

if ($Mode -eq "Install") {
    Ensure-ProviderDllPath
    Assert-ProviderPathRequired

    $backupArguments = @{ ListenerName = $ListenerName }
    if (-not [string]::IsNullOrWhiteSpace($BackupDirectory)) {
        $backupArguments.OutputDirectory = $BackupDirectory
    }

    $backupPathOutput = & $backupScript @backupArguments -PassThru
    $resolvedBackupPath = "$backupPathOutput".Trim()
    if ([string]::IsNullOrWhiteSpace($resolvedBackupPath)) {
        $resolvedBackupPath = Resolve-LatestBackupDirectory
    }

    if (-not [string]::IsNullOrWhiteSpace($resolvedBackupPath)) {
        Write-Host "Backup directory: $resolvedBackupPath"
        Write-Host "Rollback restore command:"
        Write-Host "  .\crates\ironrdp-wtsprotocol-provider\scripts\first-run-side-by-side.ps1 -Mode Rollback -PortNumber $PortNumber -RestoreBackupOnRollback -BackupDirectory `"$resolvedBackupPath`""
    }

    & $preflightScript `
        -ProviderDllPath $ProviderDllPath `
        -ListenerName $ListenerName `
        -ProtocolManagerClsid $ProtocolManagerClsid `
        -PortNumber $PortNumber

    $installArguments = @{
        ProviderDllPath = $ProviderDllPath
        ListenerName = $ListenerName
        ProtocolManagerClsid = $ProtocolManagerClsid
        PortNumber = $PortNumber
    }
    if ($RestartTermService.IsPresent) {
        $installArguments.RestartTermService = $true
    }

    & $installScript @installArguments

    & $verifyScript `
        -ProviderDllPath $ProviderDllPath `
        -ListenerName $ListenerName `
        -ProtocolManagerClsid $ProtocolManagerClsid `
        -PortNumber $PortNumber

    if (-not $SkipFirewall.IsPresent) {
        & $firewallScript -Mode Add -PortNumber $PortNumber
        & $firewallScript -Mode Verify -PortNumber $PortNumber
    }

    if (-not $SkipSmokeTest.IsPresent) {
        $smokeArguments = @{
            ProviderDllPath = $ProviderDllPath
            ListenerName = $ListenerName
            ProtocolManagerClsid = $ProtocolManagerClsid
            PortNumber = $PortNumber
        }

        if ($RestartTermService.IsPresent) {
            $smokeArguments.CheckLocalPortListener = $true
        }

        & $smokeScript @smokeArguments
    }

    if ($GenerateRdpFile.IsPresent) {
        & $newRdpFileScript -TargetHost $TargetHost -PortNumber $PortNumber -OutputPath $RdpOutputPath
    }

    if ($RestartTermService.IsPresent -and $WaitForServiceReadyAfterRestart.IsPresent) {
        & $waitServiceScript -PortNumber $PortNumber -TimeoutSeconds $ServiceReadyTimeoutSeconds
    } elseif (-not $RestartTermService.IsPresent) {
        Write-Warning "TermService was not restarted; if this is a first registration, restart service before mstsc testing"
    }

    Write-Host "Install flow completed"
    Write-Host "Connect with mstsc to: $TargetHost`:$PortNumber"
    return
}

$uninstallArguments = @{
    ListenerName = $ListenerName
    ProtocolManagerClsid = $ProtocolManagerClsid
}
if ($RestartTermService.IsPresent) {
    $uninstallArguments.RestartTermService = $true
}

& $uninstallScript @uninstallArguments

if (-not $SkipFirewall.IsPresent) {
    & $firewallScript -Mode Remove -PortNumber $PortNumber
}

if ($RestoreBackupOnRollback.IsPresent) {
    if ([string]::IsNullOrWhiteSpace($BackupDirectory)) {
        $BackupDirectory = Resolve-LatestBackupDirectory
    }

    if ([string]::IsNullOrWhiteSpace($BackupDirectory)) {
        throw "BackupDirectory is required when RestoreBackupOnRollback is set and no backup directory could be auto-detected"
    }

    & $restoreScript -BackupDirectory $BackupDirectory
}

Write-Host "Rollback flow completed"
