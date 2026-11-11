<#
.SYNOPSIS
    End-to-end test: build, deploy ironrdp-termsrv to test VM, run screenshot client, collect results.

.DESCRIPTION
    Orchestrates a full deploy-and-test cycle:
      1. Build ironrdp-termsrv (and optionally the provider DLL) locally.
      2. Deploy to the test VM via PSRemoting (reuses deploy-testvm-psremoting.ps1).
      3. Observe frames through an isolated ironrdp-agent endpoint.
      4. Collect remote logs and validate output.

.PARAMETER Mode
    "Standalone" deploys only the companion service with AUTO_LISTEN=1 (no WTS provider).
    "Provider" deploys both the provider DLL and companion service with side-by-side registration.

.PARAMETER StrictSessionProof
    Opt-in strict proof mode. Treats fallback/session-fidelity diagnostics as hard pass/fail gates.
    Alias: -Strict

.PARAMETER EnableFreshSessionCleanup
    Opt in to logging off one explicitly selected target-user session before the screenshot.

.PARAMETER AggressiveFreshSessionCleanup
    If logoff fails, reset only the selected session; requires -EnableFreshSessionCleanup.

.PARAMETER FreshSessionCleanupSessionId
    The exact target-user session ID to log off; required with -EnableFreshSessionCleanup.

.PARAMETER AllowHyperVPowerCycle
    Allow one forced Hyper-V power cycle after a recoverable deployment interruption.
    Requires an explicit -HyperVVmName.

.EXAMPLE
    .\e2e-test-screenshot.ps1 -Mode Standalone
    .\e2e-test-screenshot.ps1 -Mode Provider
    .\e2e-test-screenshot.ps1 -Mode Provider -StrictSessionProof
    .\e2e-test-screenshot.ps1 -Mode Provider -AllowHyperVPowerCycle -HyperVVmName MyTestVm
#>
[CmdletBinding()]
param(
    [Parameter()]
    [ValidateSet('Standalone', 'Provider')]
    [string]$Mode = 'Standalone',

    [Parameter()]
    [Alias('ComputerName')]
    [string]$Hostname = $env:IRONRDP_TEST_VM_HOSTNAME,

    [Parameter()]
    [string]$RemoteRoot = $(if ($env:IRONRDP_DEPLOY_ROOT) { $env:IRONRDP_DEPLOY_ROOT } else { 'C:\IronRDPDeploy' }),

    [Parameter()]
    [string]$IddRemotePath = $(if ($env:IRONRDP_IDD_REMOTE_PATH) { $env:IRONRDP_IDD_REMOTE_PATH } else { 'C:\Program Files\IronRDP\idd' }),

    [Parameter()]
    [string]$TestCertificatesDirectory = '',

    [Parameter()]
    [Alias('ServerPort')]
    [int]$Port = 4489,

    [Parameter()]
    [pscredential]$Credential,

    [Parameter()]
    [string]$AdminUsername = $env:RDP_USERNAME,

    [Parameter()]
    [string]$AdminPassword = '',

    [Parameter()]
    [string]$AdminPasswordEnvVar = 'RDP_PASSWORD',

    [Parameter()]
    [string]$RdpUsername = $env:RDP_USERNAME,

    [Parameter()]
    [string]$RdpPassword = '',

    [Parameter()]
    [string]$RdpPasswordEnvVar = 'RDP_PASSWORD',

    [Parameter()]
    [string]$RdpDomain = '',

    [Parameter()]
    [int]$RdpPort = 3389,

    [Parameter()]
    [switch]$AutoLogon,

    [Parameter()]
    [string]$OutputPng = '',

    [Parameter()]
    [switch]$SkipBuild,

    [Parameter()]
    [switch]$SkipDeploy,

    [Parameter()]
    [switch]$InstallIdd,

    [Parameter()]
    [switch]$SkipScreenshot,

    [Parameter()]
    [switch]$EnableFreshSessionCleanup,

    [Parameter()]
    [ValidateRange(0, 65535)]
    [int]$FreshSessionCleanupSessionId = 0,

    [Parameter()]
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',

    [Parameter()]
    [int]$ScreenshotTimeoutSeconds = 30,

    [Parameter()]
    [int]$NoGraphicsTimeoutSeconds = 20,

    [Parameter()]
    [int]$AfterFirstGraphicsSeconds = 20,

    [Parameter()]
    [switch]$DisableFreshSessionCleanup,

    [Parameter()]
    [switch]$AggressiveFreshSessionCleanup,

    [Parameter()]
    [switch]$AllowHyperVPowerCycle,

    [Parameter()]
    [string]$HyperVVmName = '',

    [Parameter()]
    [Alias('Strict')]
    [switch]$StrictSessionProof
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = $PSScriptRoot
$screenshotHelpers = Join-Path $scriptRoot 'e2e-test-screenshot.helpers.ps1'
. $screenshotHelpers
. (Join-Path $scriptRoot 'e2e-agent.helpers.ps1')
Import-Module (Join-Path $scriptRoot '..\..\..\scripts\TestVm.psm1') -Force
if ($InstallIdd.IsPresent -and $Mode -ne 'Provider') {
    throw '-InstallIdd requires -Mode Provider'
}
Assert-E2ESafetyOptions `
    -EnableFreshSessionCleanup $EnableFreshSessionCleanup.IsPresent `
    -AggressiveFreshSessionCleanup $AggressiveFreshSessionCleanup.IsPresent `
    -FreshSessionCleanupSessionId $FreshSessionCleanupSessionId `
    -AllowHyperVPowerCycle $AllowHyperVPowerCycle.IsPresent `
    -HyperVVmName $HyperVVmName

function Resolve-RdpIdentityParts {
    param(
        [Parameter()]
        [string]$Username,

        [Parameter()]
        [string]$Domain
    )

    $resolvedUsername = [string]$Username
    $resolvedDomain = [string]$Domain

    if ($null -eq $resolvedUsername) { $resolvedUsername = '' }
    if ($null -eq $resolvedDomain) { $resolvedDomain = '' }

    $resolvedUsername = $resolvedUsername.Trim()
    $resolvedDomain = $resolvedDomain.Trim()

    $downLevelMatch = [regex]::Match($resolvedUsername, '^(?<domain>[^\\]+)\\(?<user>.+)$')
    if ($downLevelMatch.Success) {
        return @{
            Username = $downLevelMatch.Groups['user'].Value.Trim()
            Domain   = $downLevelMatch.Groups['domain'].Value.Trim()
        }
    }

    if ($resolvedUsername.Contains('@')) {
        return @{
            Username = $resolvedUsername
            Domain   = ''
        }
    }

    return @{
        Username = $resolvedUsername
        Domain   = $resolvedDomain
    }
}

function Test-DeployRestartRecoverableError {
    param(
        [Parameter()]
        [string]$Message
    )

    if ([string]::IsNullOrWhiteSpace($Message)) {
        return $false
    }

    $recoverablePatterns = @(
        'TermService did not stop within',
        'The I/O operation has been aborted because of either a thread exit or an application request',
        'A remote shell operation was attempted on a shell that has already exited',
        'failed because the shell was not found on the server',
        'The WSMan provider host process did not return a proper response',
        'The client cannot connect to the destination specified in the request',
        'PSSession state is not opened'
    )

    foreach ($pattern in $recoverablePatterns) {
        if ($Message -match [regex]::Escape($pattern)) {
            return $true
        }
    }

    return $false
}

function Restart-TestVmViaHyperV {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Hostname,

        [Parameter(Mandatory = $true)]
        [string]$VmName,

        [Parameter(Mandatory = $true)]
        [pscredential]$Credential,

        [Parameter()]
        [ValidateRange(1, 120)]
        [int]$InitialBootWaitSeconds = 15,

        [Parameter()]
        [ValidateRange(30, 900)]
        [int]$RemotingReadyTimeoutSeconds = 300
    )

    if (-not (Get-Command -Name Stop-VM -ErrorAction SilentlyContinue)) {
        throw "Hyper-V cmdlets are unavailable (Stop-VM not found); install Hyper-V management tools or restart VM manually"
    }

    Write-Warning "TermService stop timeout detected; force power-cycling Hyper-V VM '$vmName'"

    try {
        Stop-VM -Name $vmName -TurnOff -Force -ErrorAction Stop | Out-Null
    }
    catch {
        Write-Warning "Stop-VM reported: $($_.Exception.Message)"
    }

    Start-Sleep -Seconds 2
    Start-VM -Name $vmName -ErrorAction Stop | Out-Null

    Write-Host "Waiting ${InitialBootWaitSeconds}s for VM boot..." -ForegroundColor Yellow
    Start-Sleep -Seconds $InitialBootWaitSeconds

    $deadline = (Get-Date).AddSeconds($RemotingReadyTimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $probe = $null
        try {
            $probe = New-TestVmSession -Hostname $Hostname -Credential $Credential
            if ($null -ne $probe) {
                Write-Host "VM is reachable over WinRM after Hyper-V restart" -ForegroundColor Green
                return
            }
        }
        catch {
            Start-Sleep -Seconds 5
        }
        finally {
            if ($null -ne $probe) {
                Remove-PSSession -Session $probe -ErrorAction SilentlyContinue
            }
        }
    }

    throw "VM '$vmName' did not become reachable over WinRM within ${RemotingReadyTimeoutSeconds}s after Hyper-V restart"
}

function Invoke-RemoteCommandWithTimeout {
    param(
        [Parameter(Mandatory = $true)]
        [System.Management.Automation.Runspaces.PSSession]$Session,

        [Parameter(Mandatory = $true)]
        [scriptblock]$ScriptBlock,

        [Parameter()]
        [object[]]$ArgumentList = @(),

        [Parameter()]
        [int]$TimeoutSeconds = 90,

        [Parameter()]
        [string]$OperationName = 'remote command'
    )

    $job = $null
    try {
        $job = Invoke-Command -Session $Session -AsJob -ScriptBlock $ScriptBlock -ArgumentList $ArgumentList
        $completed = Wait-Job -Job $job -Timeout $TimeoutSeconds
        if ($null -eq $completed) {
            Stop-Job -Job $job -ErrorAction SilentlyContinue | Out-Null
            throw "$OperationName timed out after ${TimeoutSeconds}s"
        }

        return Receive-Job -Job $job -ErrorAction Stop
    }
    finally {
        if ($null -ne $job) {
            Remove-Job -Job $job -Force -ErrorAction SilentlyContinue | Out-Null
        }
    }
}

function Wait-TestVmRdsReadyAfterBoot {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Hostname,

        [Parameter(Mandatory = $true)]
        [pscredential]$Credential,

        [Parameter()]
        [ValidateRange(30, 900)]
        [int]$MinimumUptimeSeconds = 180,

        [Parameter()]
        [ValidateRange(30, 900)]
        [int]$TimeoutSeconds = 420,

        [Parameter()]
        [ValidateRange(5, 60)]
        [int]$PollSeconds = 10
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        $session = $null
        try {
            $session = New-TestVmSession -Hostname $Hostname -Credential $Credential
            $status = Invoke-RemoteCommandWithTimeout -Session $session -TimeoutSeconds 60 -OperationName 'post-boot RDS readiness probe' -ScriptBlock {
                $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
                $bootTime = [datetime]$os.LastBootUpTime
                $now = [datetime]$os.LocalDateTime
                $uptimeSeconds = [int][Math]::Max(0, ($now - $bootTime).TotalSeconds)

                $serviceNames = @('TermService', 'SessionEnv', 'UmRdpService', 'gpsvc', 'ProfSvc', 'Netlogon')
                $services = @(Get-Service -Name $serviceNames -ErrorAction SilentlyContinue)
                $notRunning = @($services | Where-Object { $_.Status -ne 'Running' } | ForEach-Object { $_.Name })

                $recentStartFailures = @(Get-WinEvent -FilterHashtable @{
                        LogName   = 'Microsoft-Windows-TerminalServices-LocalSessionManager/Operational'
                        Id        = 17
                        StartTime = $now.AddSeconds(-60)
                    } -ErrorAction SilentlyContinue)
                $recentGpWarnings = @(Get-WinEvent -LogName 'Application' -ErrorAction SilentlyContinue |
                    Where-Object {
                        $_.TimeCreated -ge $now.AddSeconds(-60) -and
                        $_.ProviderName -eq 'Microsoft-Windows-Winlogon' -and
                        $_.Id -in 6005, 6006
                    })

                [pscustomobject]@{
                    BootTimeUtc            = $bootTime.ToUniversalTime().ToString('o')
                    NowUtc                 = $now.ToUniversalTime().ToString('o')
                    UptimeSeconds          = $uptimeSeconds
                    MissingOrStopped       = [string]::Join(',', @($notRunning))
                    RecentLsm17Count       = @($recentStartFailures).Count
                    RecentWinlogonGpCount  = @($recentGpWarnings).Count
                    Ready                  = ($uptimeSeconds -ge $using:MinimumUptimeSeconds) -and (@($notRunning).Count -eq 0) -and (@($recentStartFailures).Count -eq 0) -and (@($recentGpWarnings).Count -eq 0)
                }
            }

            $missingOrStopped = [string]$status.MissingOrStopped
            if ([string]::IsNullOrWhiteSpace($missingOrStopped)) {
                $missingOrStopped = 'none'
            }

            if ([bool]$status.Ready) {
                Write-Host ("RDS host is settled after boot (uptime={0}s, missing_or_stopped={1}, recent_lsm17={2}, recent_winlogon_gp={3})" -f `
                        [int]$status.UptimeSeconds, $missingOrStopped, [int]$status.RecentLsm17Count, [int]$status.RecentWinlogonGpCount) -ForegroundColor Green
                return
            }

            Write-Host ("Waiting for RDS host to settle after boot (uptime={0}s/{1}s, missing_or_stopped={2}, recent_lsm17={3}, recent_winlogon_gp={4})" -f `
                    [int]$status.UptimeSeconds, $MinimumUptimeSeconds, $missingOrStopped, [int]$status.RecentLsm17Count, [int]$status.RecentWinlogonGpCount) -ForegroundColor Yellow
        }
        catch {
            Write-Warning "RDS readiness probe failed: $($_.Exception.Message)"
        }
        finally {
            if ($null -ne $session) {
                Remove-PSSession -Session $session -ErrorAction SilentlyContinue
            }
        }

        Start-Sleep -Seconds $PollSeconds
    }

    throw "VM '$Hostname' did not reach a settled RDS-ready state within ${TimeoutSeconds}s after boot"
}

$adminCred = Resolve-TestVmCredential -Credential $Credential -Username $AdminUsername `
    -Password $AdminPassword -PasswordEnvVar $AdminPasswordEnvVar -Hostname $Hostname

$adminUsernameEffective = $adminCred.UserName

$rdpPasswordEffective = $null
if ($PSBoundParameters.ContainsKey('RdpPassword') -and (-not [string]::IsNullOrWhiteSpace($RdpPassword))) {
    $rdpPasswordEffective = $RdpPassword
} else {
    $fromEnv = [Environment]::GetEnvironmentVariable($RdpPasswordEnvVar)
    if (-not [string]::IsNullOrWhiteSpace($fromEnv)) {
        $rdpPasswordEffective = $fromEnv
    } else {
        $rdpPasswordEffective = $RdpPassword
    }
}

$resolvedRdpIdentity = Resolve-RdpIdentityParts -Username $RdpUsername -Domain $RdpDomain
$RdpUsername = [string]$resolvedRdpIdentity.Username
$RdpDomain = [string]$resolvedRdpIdentity.Domain

$workspaceRoot = (Resolve-Path (Join-Path $scriptRoot '..\..\..\..\..')).Path
$cargoTargetDirectory = Get-CargoBuildOutputDirectory -WorkspaceRoot $workspaceRoot
$artifactsDir = Join-Path $workspaceRoot 'artifacts'
New-Item -ItemType Directory -Path $artifactsDir -Force | Out-Null

$timestamp = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N')
$testStartTime = Get-Date
$evidenceBaseline = $null
$testedClientEndpoint = ''
$evidenceCorrelated = $false
$session = $null
$evidenceCorrelationReason = 'No connection evidence was collected'
$evidenceCollector = [scriptblock]::Create((Get-Content -LiteralPath (Join-Path $scriptRoot 'e2e-server-evidence.ps1') -Raw))
$auditIdentity = if ($RdpDomain) { "$RdpDomain\$RdpUsername" } else { $RdpUsername }
if ([string]::IsNullOrWhiteSpace($OutputPng)) {
    $OutputPng = Join-Path $artifactsDir "screenshot-$timestamp.png"
}

$RemoteRoot = if ([string]::IsNullOrWhiteSpace($RemoteRoot)) { 'C:\IronRDPDeploy' } else { $RemoteRoot }
$IddRemotePath = if ([string]::IsNullOrWhiteSpace($IddRemotePath)) { 'C:\Program Files\IronRDP\idd' } else { $IddRemotePath }
$dumpRemoteDir = Join-Path $RemoteRoot "bitmap-dumps-$timestamp"
$dumpLocalDir = Join-Path $artifactsDir "bitmap-dumps-$timestamp"
$iddRuntimeRoot = 'C:\ProgramData\IronRDP\Idd'
$iddRuntimeStateFile = Join-Path $iddRuntimeRoot 'runtime-config.txt'
$iddDebugTraceFile = Join-Path $iddRuntimeRoot 'ironrdp-idd-debug.log'
$iddOpenProbeScriptFile = Join-Path $iddRuntimeRoot 'probe-open-paths.ps1'
$iddOpenProbeLogFile = Join-Path $iddRuntimeRoot 'probe-admin.log'
$iddOpenProbeInterfaceGuid = '{1ea642e3-6a78-4f4b-9a19-2eb4f0f33b82}'
$iddDumpRemoteDir = Join-Path $iddRuntimeRoot "bitmap-dumps-$timestamp"
$iddDumpLocalDir = Join-Path $artifactsDir "idd-bitmap-dumps-$timestamp"
$iddOpenProbeScript = Get-Content -LiteralPath (Join-Path $scriptRoot 'e2e-idd-open-probe.ps1') -Raw

$profileDir = if ($Configuration -eq 'Release') { 'release' } else { 'debug' }

$remoteLogCollectionSucceeded = $false
$screenshotSucceeded = $false
$remoteServiceRunning = $false
$remotePortListening = $false
$iddOpenProbeSession = $null
$preConnectSnapshotSession = $null
$preConnectSessionSnapshotArtifact = $null
$iddOpenProbeJob = $null
$securityLogonType10Count = $null
$termsrvFallbackMarkerCount = $null
$termsrvFallbackMarkers = ''
$providerSessionProofMarkerCount = $null
$providerSessionProofMarkers = ''
$termsrvSessionProofMarkerCount = $null
$termsrvSessionProofMarkers = ''
$notifyCommandProcessCreatedCount = $null
$iddDriverLoadedNotified = $false
$iddWddmEnabledSignalCount = $null
$providerVideoHandleMarkerCount = $null
$providerVideoHandleMarkers = ''
$termsrvIddMarkerCount = $null
$termsrvIddMarkers = ''
$providerLogonPolicyMarkerCount = $null
$providerLogonPolicyMarkers = ''
$remoteConnectionSignalCount = $null
$remoteGraphicsSignalCount = $null
$remoteConnectionSignalsLog = ''
$activationLicenseStatus = $null
$activationLicenseStatusReasonHex = ''
$activationNotificationMode = $false
$guiTargetSessionId = $null
$guiTargetSessionSource = ''
$guiTargetSessionResolved = $false
$guiTargetSessionProcessProof = $false
$guiTargetSessionExplorerCount = $null
$guiTargetSessionGuiProcessCount = $null
$guiTargetSessionWinlogonCount = $null
$guiTargetSessionLogonUiCount = $null
$guiTargetSessionProcesses = ''
$guiTargetSessionExplorerBootstrapPid = $null
$guiTargetSessionExplorerBootstrapTimeUtc = ''
$guiTargetSessionExplorerBootstrapPidAlive = $false
$guiTargetSessionExplorerEarliestStartUtc = ''
$guiTargetSessionExplorerAfterCreationKnown = $false
$guiTargetSessionExplorerAfterCreation = $false
$guiTargetSessionCreationSignalUtc = ''
$guiTargetSessionCreationSignals = ''
$guiTargetSessionExplorerDetails = ''
$explorerCrashEventCount = $null
$explorerHangEventCount = $null
$explorerWerEventCount = $null
$explorerLifecycleEvents = ''
$bitmapDumpCount = 0
$bitmapObservedSessionIds = @()
$bitmapTargetSessionMatchCount = 0
$bitmapTargetSessionHasGraphics = $false
$type10GraphicsSessionConfirmed = $false
$freshSessionCleanupAttempted = $false
$freshSessionCleanupSucceeded = $false
$freshSessionCleanupTargetUser = ''
$freshSessionCleanupDiscoveredSessionIds = ''
$freshSessionCleanupLoggedOffSessionIds = ''
$freshSessionCleanupResetSessionIds = ''
$freshSessionCleanupExplorerMatches = ''
$freshSessionCleanupQuserMatches = ''
$freshSessionCleanupWinstaMatches = ''
$freshSessionCleanupSuspiciousSessionMatches = ''
$freshSessionCleanupLogoffErrors = ''
$freshSessionCleanupResetErrors = ''
$freshSessionCleanupError = ''
$freshSessionCleanupAggressiveMode = $false
$iddRuntimeConfigText = ''
$iddDebugTraceText = ''
$iddDeviceInventory = ''
$iddDriverInventory = ''
$iddDumpCount = 0
$iddObservedSessionIds = @()
$iddTargetSessionMatchCount = 0
$iddTargetSessionHasGraphics = $false
$type10IddGraphicsSessionConfirmed = $false
$providerCustomVideoSourceSelected = $false
$providerManualVideoSourceSelected = $false
$providerFallbackVideoSourceSelected = $false
$providerHardwareIdCustom = $false
$providerGetHardwareIdCount = 0
$providerOnDriverLoadCount = 0
$providerAssumePresentCount = 0
$providerVideoHandleDriverLoadCount = 0
$iddMonitorArrivalMarkerCount = 0
$iddDisplayConfigUpdateRequestCount = 0
$iddDisplayConfigUpdateSuccessCount = 0
$iddCommitModesMarkerCount = 0
$iddCommitModesActivePathCount = 0
$iddSwapchainAssignedCount = 0
$iddSessionReadyMarkerCount = 0
$iddFirstFrameMarkerCount = 0
$preflightSucceeded = $false
$preflightBlockedConnectionAttempt = $false
$preflightIssues = New-Object System.Collections.Generic.List[string]
$hardPreflightIssues = New-Object System.Collections.Generic.List[string]

# ── Step 1: Build ───────────────────────────────────────────────────────────
if (-not $SkipBuild.IsPresent) {
    Write-Host "`n=== Step 1: Building ironrdp-termsrv ($Configuration) ===" -ForegroundColor Cyan
    Push-Location $workspaceRoot
    try {
        $buildArgs = @('build', '-p', 'ironrdp-agent')
        if ($Configuration -eq 'Release') { $buildArgs += '--release' }
        & cargo @buildArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo build ironrdp-agent failed (exit $LASTEXITCODE)" }
    }
    finally {
        Pop-Location
    }
    Write-Host "Build succeeded" -ForegroundColor Green
} else {
    Write-Host "`n=== Step 1: Build skipped ===" -ForegroundColor Yellow
}

$screenshotExe = Join-Path $cargoTargetDirectory "$profileDir\ironrdp-agent.exe"
if (-not $SkipScreenshot.IsPresent -and -not (Test-Path -LiteralPath $screenshotExe)) {
    throw "ironrdp-agent.exe not found: $screenshotExe"
}

# ── Step 2: Deploy ──────────────────────────────────────────────────────────
if (-not $SkipDeploy.IsPresent) {
    Write-Host "`n=== Step 2: Deploying to $Hostname (mode=$Mode) ===" -ForegroundColor Cyan

    $deployScript = Join-Path $scriptRoot 'deploy-testvm-psremoting.ps1'

    $deployArgs = @{
        Hostname         = $Hostname
        Username         = $adminUsernameEffective
        Password         = $adminCred.Password
        RdpUsername      = $RdpUsername
        RdpDomain        = $RdpDomain
        RdpPasswordEnvVar = $RdpPasswordEnvVar
        Configuration    = $Configuration
        RemoteRoot       = $RemoteRoot
        IddRemotePath    = $IddRemotePath
        TestCertificatesDirectory = $TestCertificatesDirectory
        SkipBuild        = $SkipBuild.IsPresent
        InstallIdd       = $InstallIdd.IsPresent
        ListenerAddr     = "0.0.0.0:$Port"
        CaptureIpc       = 'tcp'
        DumpBitmapUpdatesDir = $dumpRemoteDir
        # In Provider mode the companion must NOT self-bind port 4489.  The WTS provider DLL
        # connects to the companion's named-pipe control server, sends WaitForIncoming (which
        # auto-starts the TCP listener), and notifies TermService about each incoming connection.
        # TermService then calls NotifySessionId / IsUserAllowedToLogon on the DLL, which in turn
        # calls SetCaptureSessionId / GetConnectionCredentials on the companion.
        # AutoListen=true puts the companion in standalone mode with NO named-pipe server at all,
        # so the provider DLL has nothing to connect to and session management never happens.
        AutoListen           = ($Mode -ne 'Provider')
        WtsProvider          = ($Mode -eq 'Provider')
        AutoSendSas          = ($Mode -eq 'Provider')
    }

    # Prefer env var on the deploy script side to avoid passing plaintext passwords.
    $canUseEnvRdp = (-not $PSBoundParameters.ContainsKey('RdpPassword')) -and (-not [string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($RdpPasswordEnvVar)))
    if (-not $canUseEnvRdp) {
        $deployArgs.RdpPassword = $rdpPasswordEffective
    }

    $deployMaxAttempts = 2
    $deployAttempt = 0
    while ($true) {
        $deployAttempt++
        try {
            & $deployScript @deployArgs

            if ($Mode -eq 'Provider') {
                Write-Host "Waiting for post-deploy RDS stabilization..." -ForegroundColor Cyan
                Wait-TestVmRdsReadyAfterBoot -Hostname $Hostname -Credential $adminCred
            }

            Write-Host "Deploy succeeded" -ForegroundColor Green
            break
        }
        catch {
            $errorMessage = $_.Exception.Message
            if ($Mode -eq 'Provider') {
                $diagnosticSession = $null
                try {
                    $diagnosticSession = New-TestVmSession -Hostname $Hostname -Credential $adminCred
                    Invoke-Command -Session $diagnosticSession -FilePath (Join-Path $scriptRoot 'e2e-provider-diagnostics.ps1') -ArgumentList $Port, $RemoteRoot
                } catch {
                    Write-Warning "Deployment diagnostics failed: $($_.Exception.Message)"
                } finally {
                    if ($null -ne $diagnosticSession) { Remove-PSSession -Session $diagnosticSession }
                }
            }
            $canRetry = ($deployAttempt -lt $deployMaxAttempts) -and (Test-DeployRestartRecoverableError -Message $errorMessage)
            if (-not $canRetry) {
                throw
            }

            if (-not $AllowHyperVPowerCycle.IsPresent) {
                throw "Deploy attempt $deployAttempt failed with a recoverable TermService/WinRM interruption; retry after a reboot, or explicitly authorize one with -AllowHyperVPowerCycle -HyperVVmName <name>. Original error: $errorMessage"
            }

            Write-Warning "Deploy attempt $deployAttempt failed with a recoverable TermService/WinRM interruption; power-cycling explicitly selected Hyper-V VM '$HyperVVmName'"
            Restart-TestVmViaHyperV -Hostname $Hostname -VmName $HyperVVmName -Credential $adminCred
            Write-Host "Retrying deploy after Hyper-V reboot..." -ForegroundColor Yellow
        }
    }
} else {
    Write-Host "`n=== Step 2: Deploy skipped ===" -ForegroundColor Yellow
}

if ($Mode -eq 'Provider' -and -not $SkipScreenshot.IsPresent) {
    Write-Host "Checking custom IDD prerequisites on $Hostname..." -ForegroundColor Cyan

    $session = New-TestVmSession -Hostname $Hostname -Credential $adminCred
    try {
        $preflight = Invoke-Command -Session $session -ScriptBlock ([scriptblock]::Create((Get-Content -LiteralPath (Join-Path $scriptRoot 'e2e-idd-preflight.ps1') -Raw))) -ArgumentList $iddRuntimeRoot, $iddRuntimeStateFile, $iddDebugTraceFile, $iddDumpRemoteDir, (Join-Path $IddRemotePath 'IronRdpIdd.inf')

        $preflightIssues.Clear()
        $hardPreflightIssues.Clear()
        foreach ($issue in @($preflight['issues'])) {
            if (-not [string]::IsNullOrWhiteSpace([string]$issue)) {
                $preflightIssues.Add([string]$issue)
                if ([string]$issue -notlike 'Windows activation is in notification mode*') {
                    $hardPreflightIssues.Add([string]$issue)
                }
            }
        }
        $preflightSucceeded = ($preflightIssues.Count -eq 0)
        $iddRuntimeConfigText = [string]$preflight['runtime_state']

        if ($preflightSucceeded) {
            Write-Host "Custom IDD preflight passed" -ForegroundColor Green
        } else {
            Write-Warning ("Custom IDD preflight failed: " + ($preflightIssues -join '; '))
            $activationGateHit = $false
            foreach ($issue in $preflightIssues) {
                if ([string]$issue -like 'Windows activation is in notification mode*') {
                    $activationGateHit = $true
                    break
                }
            }

            if ($activationGateHit) {
                Write-Warning 'Windows activation is in notification mode; continuing because it is treated as a diagnostic signal, not a hard preflight blocker'
            }

            if ($StrictSessionProof.IsPresent -and ($hardPreflightIssues.Count -gt 0)) {
                $preflightBlockedConnectionAttempt = $true
            }
        }
    }
    finally {
        if ($null -ne $session) {
            Remove-PSSession -Session $session -ErrorAction SilentlyContinue
        }
    }
}

# ── Step 2.5: Fresh-session cleanup (Provider mode) ─────────────────────────
if (($Mode -eq 'Provider') -and $EnableFreshSessionCleanup.IsPresent -and (-not $DisableFreshSessionCleanup.IsPresent) -and (-not $SkipScreenshot.IsPresent)) {
    Write-Host "`n=== Step 2.5: Fresh-session cleanup for target user ===" -ForegroundColor Cyan
    $freshSessionCleanupAttempted = $true
    $freshSessionCleanupAggressiveMode = $AggressiveFreshSessionCleanup.IsPresent

    try {
        $session = New-TestVmSession -Hostname $Hostname -Credential $adminCred
        try {
            $cleanup = Invoke-Command -Session $session -ScriptBlock {
                param($RdpUsernameRaw, $RdpDomainRaw, $AggressiveModeRaw, $CleanupSessionIdRaw)

                $result = @{}
                $result['target_user'] = ''
                $result['aggressive_mode'] = $false
                $result['discovered_session_ids'] = ''
                $result['logged_off_session_ids'] = ''
                $result['reset_session_ids'] = ''
                $result['explorer_matches'] = ''
                $result['quser_matches'] = ''
                $result['winsta_matches'] = ''
                $result['suspicious_session_matches'] = ''
                $result['logoff_errors'] = ''
                $result['reset_errors'] = ''

                $aggressiveMode = [bool]$AggressiveModeRaw
                $cleanupSessionId = [int]$CleanupSessionIdRaw
                $result['aggressive_mode'] = $aggressiveMode

                $targetUser = [string]$RdpUsernameRaw
                if ($targetUser.Contains('\')) {
                    $targetUser = $targetUser.Split('\\')[-1]
                }
                if ($targetUser.Contains('@')) {
                    $targetUser = $targetUser.Split('@')[0]
                }
                $targetUser = $targetUser.Trim()
                $result['target_user'] = $targetUser

                $domainHints = New-Object System.Collections.Generic.List[string]
                $rdpDomain = [string]$RdpDomainRaw
                if ([string]::IsNullOrWhiteSpace($rdpDomain)) {
                    $rdpDomain = if ($RdpUsernameRaw.Contains('@')) { $RdpUsernameRaw.Split('@')[-1] } else { $env:COMPUTERNAME }
                }
                if (-not [string]::IsNullOrWhiteSpace($rdpDomain)) {
                    $rdpDomain = $rdpDomain.Trim()
                    $domainHints.Add($rdpDomain)
                    $domainShort = ($rdpDomain -split '\\.')[0]
                    if (-not [string]::IsNullOrWhiteSpace($domainShort)) {
                        $domainHints.Add($domainShort)
                    }
                }

                $sessionIds = New-Object System.Collections.Generic.List[int]
                $resetSessionIds = New-Object System.Collections.Generic.List[int]
                $explorerRows = New-Object System.Collections.Generic.List[object]

                $explorerProcs = @(Get-Process -Name 'explorer' -IncludeUserName -ErrorAction SilentlyContinue)
                foreach ($proc in $explorerProcs) {
                    $rawUserName = [string]$proc.UserName
                    if ([string]::IsNullOrWhiteSpace($rawUserName)) {
                        continue
                    }

                    $userPart = $rawUserName
                    $domainPart = ''

                    if ($rawUserName.Contains('\')) {
                        $split = $rawUserName.Split([char]'\', 2)
                        $domainPart = [string]$split[0]
                        $userPart = [string]$split[1]
                    } elseif ($rawUserName.Contains('@')) {
                        $split = $rawUserName.Split('@', 2)
                        $userPart = [string]$split[0]
                        $domainPart = [string]$split[1]
                    }

                    if (-not ($userPart -ieq $targetUser)) {
                        continue
                    }

                    if ($domainHints.Count -gt 0) {
                        if ([string]::IsNullOrWhiteSpace($domainPart)) { continue }
                        $matchesDomain = $false
                        foreach ($hint in $domainHints) {
                            if ($domainPart -ieq $hint) {
                                $matchesDomain = $true
                                break
                            }

                            $domainPartShort = ($domainPart -split '\\.')[0]
                            if (-not [string]::IsNullOrWhiteSpace($domainPartShort) -and ($domainPartShort -ieq $hint)) {
                                $matchesDomain = $true
                                break
                            }
                        }

                        if (-not $matchesDomain) {
                            continue
                        }
                    }

                    $sessionId = [int]$proc.SessionId
                    if (($sessionId -gt 0) -and (-not $sessionIds.Contains($sessionId))) {
                        $sessionIds.Add($sessionId)
                    }

                    $startUtc = ''
                    try {
                        $startUtc = $proc.StartTime.ToUniversalTime().ToString('o')
                    } catch {
                        $startUtc = ''
                    }

                    $explorerRows.Add([pscustomobject]@{
                        SessionId    = $sessionId
                        Id           = $proc.Id
                        UserName     = $rawUserName
                        StartTimeUtc = $startUtc
                    })
                }

                if ($explorerRows.Count -gt 0) {
                    $result['explorer_matches'] = ($explorerRows | Sort-Object SessionId, Id | Format-Table -AutoSize | Out-String)
                }

                $suspiciousSessionRows = New-Object System.Collections.Generic.List[object]
                $sessionProcessState = @{}
                $sessionProcesses = @(Get-Process -Name 'winlogon', 'LogonUI', 'explorer' -ErrorAction SilentlyContinue)
                foreach ($proc in $sessionProcesses) {
                    $sid = [int]$proc.SessionId
                    if (-not $sessionProcessState.ContainsKey($sid)) {
                        $sessionProcessState[$sid] = @{
                            Winlogon = $false
                            LogonUI  = $false
                            Explorer = $false
                        }
                    }

                    switch -Regex ([string]$proc.ProcessName) {
                        '^winlogon$' { $sessionProcessState[$sid]['Winlogon'] = $true; break }
                        '^LogonUI$' { $sessionProcessState[$sid]['LogonUI'] = $true; break }
                        '^explorer$' { $sessionProcessState[$sid]['Explorer'] = $true; break }
                    }
                }

                foreach ($sid in @($sessionProcessState.Keys | Sort-Object)) {
                    $sessionId = [int]$sid
                    if (($sessionId -le 1) -or ($sessionId -ge 65535)) {
                        continue
                    }

                    $state = $sessionProcessState[$sid]
                    $reason = ''
                    if ($state['Winlogon'] -and (-not $state['Explorer']) -and $state['LogonUI']) {
                        # Hidden pre-logon sessions can linger without appearing in qwinsta and then
                        # absorb later provider connections without ever producing a user shell.
                        $reason = 'winlogon_with_logonui_without_shell'
                    } elseif ($state['Winlogon'] -and (-not $state['Explorer']) -and (-not $state['LogonUI'])) {
                        $reason = 'winlogon_without_logonui_or_shell'
                    }

                    if (-not [string]::IsNullOrWhiteSpace($reason)) {
                        $suspiciousSessionRows.Add([pscustomobject]@{
                            SessionId = $sessionId
                            Winlogon  = $state['Winlogon']
                            LogonUI   = $state['LogonUI']
                            Explorer  = $state['Explorer']
                            Reason    = $reason
                        })

                        if (-not $resetSessionIds.Contains($sessionId)) {
                            $resetSessionIds.Add($sessionId)
                        }
                    }
                }

                if ($suspiciousSessionRows.Count -gt 0) {
                    $result['suspicious_session_matches'] = ($suspiciousSessionRows | Sort-Object SessionId | Format-Table -AutoSize | Out-String)
                }

                if ($aggressiveMode -or ($cleanupSessionId -gt 0)) {
                    $quserRows = New-Object System.Collections.Generic.List[object]
                    $quserLines = @(& quser 2>$null)
                    if ($quserLines.Count -gt 1) {
                        foreach ($line in ($quserLines | Select-Object -Skip 1)) {
                            if ([string]::IsNullOrWhiteSpace([string]$line)) {
                                continue
                            }

                            $m = [regex]::Match([string]$line, '^\s*>?\s*(?<user>\S+)\s+(?:(?<session>\S+)\s+)?(?<id>\d+)\s+(?<state>\S+)\s+')
                            if (-not $m.Success) {
                                continue
                            }

                            $quserUser = [string]$m.Groups['user'].Value
                            $quserSessionName = [string]$m.Groups['session'].Value
                            $quserSessionId = [int]$m.Groups['id'].Value
                            $quserState = [string]$m.Groups['state'].Value

                            $quserUserPart = $quserUser
                            $quserDomainPart = ''

                            if ($quserUser.Contains('\')) {
                                $splitUser = $quserUser.Split('\\', 2)
                                $quserDomainPart = [string]$splitUser[0]
                                $quserUserPart = [string]$splitUser[1]
                            } elseif ($quserUser.Contains('@')) {
                                $splitUser = $quserUser.Split('@', 2)
                                $quserUserPart = [string]$splitUser[0]
                                $quserDomainPart = [string]$splitUser[1]
                            }

                            if (-not ($quserUserPart -ieq $targetUser)) {
                                continue
                            }

                            if (($domainHints.Count -gt 0) -and (-not [string]::IsNullOrWhiteSpace($quserDomainPart))) {
                                $matchesDomain = $false
                                foreach ($hint in $domainHints) {
                                    if ($quserDomainPart -ieq $hint) {
                                        $matchesDomain = $true
                                        break
                                    }

                                    $quserDomainPartShort = ($quserDomainPart -split '\\.')[0]
                                    if (-not [string]::IsNullOrWhiteSpace($quserDomainPartShort) -and ($quserDomainPartShort -ieq $hint)) {
                                        $matchesDomain = $true
                                        break
                                    }
                                }

                                if (-not $matchesDomain) {
                                    continue
                                }
                            }

                            $quserRows.Add([pscustomobject]@{
                                UserName    = $quserUser
                                SessionName = $quserSessionName
                                SessionId   = $quserSessionId
                                State       = $quserState
                                Raw         = [string]$line
                            })

                            # quser can omit the domain; it is diagnostic, not sufficient proof of ownership.
                        }
                    }

                    if ($quserRows.Count -gt 0) {
                        $result['quser_matches'] = ($quserRows | Sort-Object SessionId | Format-Table -AutoSize | Out-String)
                    }

                    $winstaRows = New-Object System.Collections.Generic.List[object]

                    $winstaLines = @(& qwinsta 2>$null)
                    if ($winstaLines.Count -gt 1) {
                        foreach ($line in ($winstaLines | Select-Object -Skip 1)) {
                            if ([string]::IsNullOrWhiteSpace([string]$line)) {
                                continue
                            }

                            $m = [regex]::Match([string]$line, '^\s*>?\s*(?<session>\S+)?(?:\s+(?<user>\S+))?\s+(?<id>\d+)\s+(?<state>\S+)')
                            if (-not $m.Success) {
                                continue
                            }

                            $sessionName = [string]$m.Groups['session'].Value
                            $userName = [string]$m.Groups['user'].Value
                            $sessionId = [int]$m.Groups['id'].Value
                            $state = [string]$m.Groups['state'].Value

                            $winstaRows.Add([pscustomobject]@{
                                SessionName = $sessionName
                                UserName    = $userName
                                SessionId   = $sessionId
                                State       = $state
                                Raw         = [string]$line
                            })

                            if (($sessionId -le 1) -or ($sessionId -ge 65535)) {
                                continue
                            }

                            $isRemoteTcpSession = $sessionName -match '^(?i:(?:irdp|rdp)-tcp#)'
                            $isBrokenState = $state -match '^(?i:disc|disconnected|down|init|conn)$'
                            if (($isRemoteTcpSession -or $isBrokenState) -and (-not $resetSessionIds.Contains($sessionId))) {
                                $resetSessionIds.Add($sessionId)
                            }
                        }
                    }

                    if ($winstaRows.Count -gt 0) {
                        $result['winsta_matches'] = ($winstaRows | Sort-Object SessionId | Format-Table -AutoSize | Out-String)
                    }
                }

                $loggedOff = New-Object System.Collections.Generic.List[int]
                $sortedSessionIds = @($sessionIds | Sort-Object -Unique)
                if ($sortedSessionIds.Count -gt 0) {
                    $result['discovered_session_ids'] = ($sortedSessionIds -join ',')
                }
                if ($sortedSessionIds -notcontains $cleanupSessionId) {
                    throw "requested session_id=$cleanupSessionId was not discovered for target_user=$targetUser"
                }

                $logoffErrors = New-Object System.Collections.Generic.List[string]
                try {
                    $logoffOutput = & logoff $cleanupSessionId 2>&1
                    if ($LASTEXITCODE -ne 0) {
                        throw "logoff failed (exit $LASTEXITCODE): $($logoffOutput | Out-String)"
                    }
                    $loggedOff.Add($cleanupSessionId)
                } catch {
                    $logoffErrors.Add("session_id=$cleanupSessionId error=$($_.Exception.Message)")
                }

                if ($loggedOff.Count -gt 0) {
                    $result['logged_off_session_ids'] = (($loggedOff | Sort-Object -Unique) -join ',')
                }

                if ($logoffErrors.Count -gt 0) {
                    $result['logoff_errors'] = ($logoffErrors | Out-String)
                }

                if (($logoffErrors.Count -gt 0) -and $aggressiveMode -and $resetSessionIds.Contains($cleanupSessionId)) {
                    $resetErrors = New-Object System.Collections.Generic.List[string]
                    try {
                        $resetOutput = & rwinsta $cleanupSessionId 2>&1
                        if ($LASTEXITCODE -ne 0) {
                            throw "rwinsta failed (exit $LASTEXITCODE): $($resetOutput | Out-String)"
                        }
                        $result['reset_session_ids'] = [string]$cleanupSessionId
                    } catch {
                        $resetErrors.Add("session_id=$cleanupSessionId error=$($_.Exception.Message)")
                    }

                    if ($resetErrors.Count -gt 0) {
                        $result['reset_errors'] = ($resetErrors | Out-String)
                    }
                }

                $result
            } -ArgumentList $RdpUsername, $RdpDomain, $freshSessionCleanupAggressiveMode, $FreshSessionCleanupSessionId

            if ($cleanup.ContainsKey('target_user')) {
                $freshSessionCleanupTargetUser = [string]$cleanup['target_user']
            }
            if ($cleanup.ContainsKey('aggressive_mode')) {
                $freshSessionCleanupAggressiveMode = [bool]$cleanup['aggressive_mode']
            }
            if ($cleanup.ContainsKey('discovered_session_ids')) {
                $freshSessionCleanupDiscoveredSessionIds = [string]$cleanup['discovered_session_ids']
            }
            if ($cleanup.ContainsKey('logged_off_session_ids')) {
                $freshSessionCleanupLoggedOffSessionIds = [string]$cleanup['logged_off_session_ids']
            }
            if ($cleanup.ContainsKey('reset_session_ids')) {
                $freshSessionCleanupResetSessionIds = [string]$cleanup['reset_session_ids']
            }
            if ($cleanup.ContainsKey('explorer_matches')) {
                $freshSessionCleanupExplorerMatches = [string]$cleanup['explorer_matches']
            }
            if ($cleanup.ContainsKey('quser_matches')) {
                $freshSessionCleanupQuserMatches = [string]$cleanup['quser_matches']
            }
            if ($cleanup.ContainsKey('winsta_matches')) {
                $freshSessionCleanupWinstaMatches = [string]$cleanup['winsta_matches']
            }
            if ($cleanup.ContainsKey('suspicious_session_matches')) {
                $freshSessionCleanupSuspiciousSessionMatches = [string]$cleanup['suspicious_session_matches']
            }
            if ($cleanup.ContainsKey('logoff_errors')) {
                $freshSessionCleanupLogoffErrors = [string]$cleanup['logoff_errors']
            }
            if ($cleanup.ContainsKey('reset_errors')) {
                $freshSessionCleanupResetErrors = [string]$cleanup['reset_errors']
            }

            $cleanupRecoveredByReset = $freshSessionCleanupAggressiveMode -and
                (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupResetSessionIds)) -and
                [string]::IsNullOrWhiteSpace($freshSessionCleanupResetErrors)
            $freshSessionCleanupSucceeded = ([string]::IsNullOrWhiteSpace($freshSessionCleanupLogoffErrors) -or $cleanupRecoveredByReset) -and
                [string]::IsNullOrWhiteSpace($freshSessionCleanupResetErrors)

            Write-Host "Fresh-session cleanup target user: $freshSessionCleanupTargetUser"
            Write-Host "Fresh-session cleanup target session ID: $FreshSessionCleanupSessionId"
            Write-Host "Fresh-session cleanup aggressive mode: $freshSessionCleanupAggressiveMode"
            Write-Host "Fresh-session cleanup discovered session IDs: $(if ([string]::IsNullOrWhiteSpace($freshSessionCleanupDiscoveredSessionIds)) { 'none' } else { $freshSessionCleanupDiscoveredSessionIds })"
            Write-Host "Fresh-session cleanup logged off session IDs: $(if ([string]::IsNullOrWhiteSpace($freshSessionCleanupLoggedOffSessionIds)) { 'none' } else { $freshSessionCleanupLoggedOffSessionIds })"

            if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupExplorerMatches)) {
                Write-Host "`n---- Fresh-session cleanup explorer matches ----" -ForegroundColor Yellow
                Write-Host $freshSessionCleanupExplorerMatches
            }
            if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupQuserMatches)) {
                Write-Host "`n---- Fresh-session cleanup quser matches ----" -ForegroundColor Yellow
                Write-Host $freshSessionCleanupQuserMatches
            }
            if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupWinstaMatches)) {
                Write-Host "`n---- Fresh-session cleanup qwinsta matches ----" -ForegroundColor Yellow
                Write-Host $freshSessionCleanupWinstaMatches
            }
            if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupSuspiciousSessionMatches)) {
                Write-Host "`n---- Fresh-session cleanup suspicious sessions ----" -ForegroundColor Yellow
                Write-Host $freshSessionCleanupSuspiciousSessionMatches
            }
            if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupLogoffErrors)) {
                Write-Warning "Fresh-session cleanup logoff errors:`n$freshSessionCleanupLogoffErrors"
            }
            if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupResetErrors)) {
                Write-Warning "Fresh-session cleanup reset errors:`n$freshSessionCleanupResetErrors"
            }
        }
        finally {
            if ($null -ne $session) {
                Remove-PSSession -Session $session -ErrorAction SilentlyContinue
            }
        }
    } catch {
        $freshSessionCleanupError = [string]$_
        Write-Warning "Fresh-session cleanup failed: $_"
    }
} elseif (($Mode -eq 'Provider') -and $DisableFreshSessionCleanup.IsPresent) {
    Write-Host "`n=== Step 2.5: Fresh-session cleanup disabled ===" -ForegroundColor Yellow
} elseif (($Mode -eq 'Provider') -and $EnableFreshSessionCleanup.IsPresent -and $SkipScreenshot.IsPresent) {
    Write-Host "`n=== Step 2.5: Fresh-session cleanup skipped because screenshot was skipped ===" -ForegroundColor Yellow
} elseif (($Mode -eq 'Provider') -and (-not $SkipScreenshot.IsPresent)) {
    Write-Host "`n=== Step 2.5: Fresh-session cleanup disabled (pass -EnableFreshSessionCleanup with -FreshSessionCleanupSessionId to log off one session) ===" -ForegroundColor Yellow
}

# ── Step 3: Run screenshot client ───────────────────────────────────────────
try {
if ((-not $SkipScreenshot.IsPresent) -and (-not $preflightBlockedConnectionAttempt)) {
    Write-Host "`n=== Step 3: Running screenshot client against ${Hostname}:${Port} ===" -ForegroundColor Cyan

    $effectiveScreenshotTimeoutSeconds = [Math]::Max(
        $ScreenshotTimeoutSeconds,
        ($NoGraphicsTimeoutSeconds + $AfterFirstGraphicsSeconds + 10)
    )

    if ($Mode -eq 'Provider') {
        try {
            $preConnectSnapshotSession = New-TestVmSession -Hostname $Hostname -Credential $adminCred
            $preConnectSnapshot = Invoke-RemoteCommandWithTimeout -Session $preConnectSnapshotSession -TimeoutSeconds 60 -OperationName 'pre-connect session snapshot' -ScriptBlock {
                $nowUtc = (Get-Date).ToUniversalTime().ToString('o')
                $result = [ordered]@{
                    TimestampUtc = $nowUtc
                    Qwinsta = ''
                    SessionProcessState = ''
                    ProcessGroups = ''
                }

                $qwinstaLines = @(& qwinsta 2>&1)
                if ($qwinstaLines.Count -gt 0) {
                    $result.Qwinsta = ($qwinstaLines -join [Environment]::NewLine)
                }

                $interestingNames = @('winlogon', 'LogonUI', 'explorer', 'dwm', 'csrss', 'fontdrvhost', 'ctfmon')
                $interestingProcesses = @(Get-Process -Name $interestingNames -ErrorAction SilentlyContinue)
                if ($interestingProcesses.Count -gt 0) {
                    $rows = foreach ($proc in ($interestingProcesses | Sort-Object SessionId, ProcessName, Id)) {
                        [pscustomobject]@{
                            SessionId = [int]$proc.SessionId
                            Process   = [string]$proc.ProcessName
                            Id        = [int]$proc.Id
                        }
                    }

                    $result.ProcessGroups = ($rows | Format-Table -AutoSize | Out-String)

                    $sessionRows = New-Object System.Collections.Generic.List[object]
                    foreach ($group in ($rows | Group-Object SessionId | Sort-Object Name)) {
                        $names = @($group.Group | ForEach-Object { $_.Process })
                        $sessionRows.Add([pscustomobject]@{
                            SessionId   = [int]$group.Name
                            Winlogon    = ($names -contains 'winlogon')
                            LogonUI     = ($names -contains 'LogonUI')
                            Explorer    = ($names -contains 'explorer')
                            Dwm         = ($names -contains 'dwm')
                            Csrss       = ($names -contains 'csrss')
                            FontDrvHost = ($names -contains 'fontdrvhost')
                            Ctfmon      = ($names -contains 'ctfmon')
                        })
                    }

                    $result.SessionProcessState = ($sessionRows | Format-Table -AutoSize | Out-String)
                }

                @(
                    "TimestampUtc: $($result.TimestampUtc)"
                    ''
                    'qwinsta:'
                    $result.Qwinsta
                    ''
                    'session_process_state:'
                    $result.SessionProcessState
                    ''
                    'process_groups:'
                    $result.ProcessGroups
                ) -join [Environment]::NewLine
            }

            $preConnectSessionSnapshotArtifact = Join-Path $artifactsDir "preconnect-session-snapshot-$timestamp.log"
            $preConnectSnapshot | Set-Content -Path $preConnectSessionSnapshotArtifact
            Write-Host "Pre-connect session snapshot saved: $preConnectSessionSnapshotArtifact"
        } catch {
            Write-Warning "Pre-connect session snapshot failed: $_"
        } finally {
            if ($null -ne $preConnectSnapshotSession) {
                Remove-PSSession -Session $preConnectSnapshotSession -ErrorAction SilentlyContinue
            }
        }

        try {
            $iddOpenProbeSession = New-TestVmSession -Hostname $Hostname -Credential $adminCred
            $iddOpenProbeJob = Invoke-Command -Session $iddOpenProbeSession -AsJob -ScriptBlock {
                param($ProbeScriptPath, $ProbeLogPath, $ProbeScriptText, $ProbeRunSeconds, $ProbeInterfaceGuid)

                New-Item -ItemType Directory -Path (Split-Path -Parent $ProbeScriptPath) -Force | Out-Null
                Set-Content -LiteralPath $ProbeScriptPath -Value $ProbeScriptText -Encoding UTF8
                Remove-Item -LiteralPath $ProbeLogPath -Force -ErrorAction SilentlyContinue
                & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ProbeScriptPath -LogPath $ProbeLogPath -RunSeconds $ProbeRunSeconds -InterfaceGuid $ProbeInterfaceGuid
                if (Test-Path -LiteralPath $ProbeLogPath) {
                    Get-Content -LiteralPath $ProbeLogPath -Raw -ErrorAction SilentlyContinue
                }
            } -ArgumentList $iddOpenProbeScriptFile, $iddOpenProbeLogFile, $iddOpenProbeScript, ([Math]::Max(90, [Math]::Max($ScreenshotTimeoutSeconds, ($NoGraphicsTimeoutSeconds + $AfterFirstGraphicsSeconds + 25)))), $iddOpenProbeInterfaceGuid
            Write-Host "IDD open-path probe job started: $iddOpenProbeLogFile"
        } catch {
            Write-Warning "IDD open-path probe failed: $_"
        }
    }

    $resolvedOutputPng = [System.IO.Path]::GetFullPath($OutputPng)
    $outputDirectory = [System.IO.Path]::GetDirectoryName($resolvedOutputPng)
    $outputBaseName = [System.IO.Path]::GetFileNameWithoutExtension($resolvedOutputPng)
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    $screenshotTempPng = Join-Path $outputDirectory "$outputBaseName-$([guid]::NewGuid().ToString('N')).tmp.png"

    $baselineSession = New-TestVmSession -Hostname $Hostname -Credential $adminCred
    $screenshotLog = Join-Path $artifactsDir "screenshot-$timestamp.log"
    $agentKnownCertificates = Join-Path $artifactsDir "agent-known-certificates-$timestamp"
    try {
        $serverFingerprint = Invoke-Command -Session $baselineSession -ScriptBlock {
            $certificates = @(Get-ChildItem Cert:\LocalMachine\My -ErrorAction Stop |
                Where-Object { $_.Subject -eq 'CN=IronRDP TermSrv' -and $_.HasPrivateKey })
            if ($certificates.Count -ne 1) {
                throw "Expected one private-key certificate for CN=IronRDP TermSrv; found $($certificates.Count)"
            }
            $hasher = [System.Security.Cryptography.SHA256]::Create()
            try {
                [BitConverter]::ToString($hasher.ComputeHash($certificates[0].RawData)).Replace('-', '').ToLowerInvariant()
            } finally {
                $hasher.Dispose()
            }
        }
        $evidenceBaseline = Invoke-RemoteCommandWithTimeout -Session $baselineSession -TimeoutSeconds 30 `
            -OperationName 'pre-connect evidence baseline' -ScriptBlock $evidenceCollector `
            -ArgumentList (@($null) * 5 + @($iddRuntimeStateFile, $iddDebugTraceFile, $iddOpenProbeLogFile, $null, $null, $true, $null, $RemoteRoot))
    }
    finally { Remove-PSSession -Session $baselineSession -ErrorAction SilentlyContinue }

    $observer = {
        param($Endpoint)
        Get-E2EAgentClientEndpoint -Endpoint $Endpoint -AgentPath $screenshotExe -Hostname $Hostname -Port $Port
    }
    try {
        $trustResult = Invoke-E2EAgentCertificateTrust -AgentPath $screenshotExe -Endpoint "${Hostname}:$Port" `
            -Fingerprint $serverFingerprint -KnownCertificatesPath $agentKnownCertificates
        Add-Content -LiteralPath $screenshotLog -Value $trustResult.Output
        if ($trustResult.ExitCode -ne 0) {
            throw "Could not pin the TermSrv certificate for ${Hostname}:$Port`: $($trustResult.Output)"
        }
        $clientResult = Invoke-E2EAgentScreenshot -AgentPath $screenshotExe -Server "${Hostname}:$Port" `
            -Username $RdpUsername -Password $rdpPasswordEffective -Domain $RdpDomain `
            -OutputPath $screenshotTempPng -LogPath $screenshotLog -TimeoutSeconds $effectiveScreenshotTimeoutSeconds `
            -NoGraphicsTimeoutSeconds $NoGraphicsTimeoutSeconds -AfterFirstGraphicsSeconds $AfterFirstGraphicsSeconds `
            -KnownCertificatesPath $agentKnownCertificates -ConnectionObserver $observer
    }
    finally {
        Remove-Item -LiteralPath $agentKnownCertificates -Force -ErrorAction SilentlyContinue
    }
    $exitCode = $clientResult.ExitCode
    $testedClientEndpoint = $clientResult.ClientEndpoint
    Write-Host "Screenshot client exited with code: $exitCode"

    if (Test-Path $screenshotLog) {
        $logContent = Get-Content $screenshotLog -Raw -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($logContent)) {
            Write-Host "`n---- screenshot client log ----" -ForegroundColor Yellow
            Write-Host $logContent
        }
    }

    $screenshotSucceeded = Publish-ScreenshotOutput -ExitCode $exitCode -TemporaryPath $screenshotTempPng -OutputPath $resolvedOutputPng
    if ($screenshotSucceeded) {
        $fileInfo = Get-Item -LiteralPath $OutputPng
        Write-Host "`nScreenshot saved: $OutputPng ($($fileInfo.Length) bytes)" -ForegroundColor Green
    } else {
        Write-Warning "Screenshot failed (exit code $exitCode); no current-run PNG was published to: $OutputPng"
    }
} else {
    Write-Host "`n=== Step 3: Screenshot skipped ===" -ForegroundColor Yellow
}

# ── Step 4: Collect remote logs ─────────────────────────────────────────────
Write-Host "`n=== Step 4: Collecting remote logs ===" -ForegroundColor Cyan

    $remoteLogs = $null
    $logCollectionAttempts = 5

    for ($logCollectionAttempt = 1; $logCollectionAttempt -le $logCollectionAttempts; $logCollectionAttempt++) {
        $session = $null
        try {
            $session = New-TestVmSession -Hostname $Hostname -Credential $adminCred
            $remoteLogs = Invoke-RemoteCommandWithTimeout -Session $session -TimeoutSeconds 90 -OperationName 'remote log collection' `
                -ScriptBlock $evidenceCollector -ArgumentList @($Port, $testStartTime, $Mode, $dumpRemoteDir, $auditIdentity, $iddRuntimeStateFile, $iddDebugTraceFile, $iddOpenProbeLogFile, $evidenceBaseline, $testedClientEndpoint, $false, $null, $RemoteRoot)
            break
        }
        catch {
            if ($logCollectionAttempt -ge $logCollectionAttempts) {
                throw
            }

            Write-Warning "Remote log collection attempt $logCollectionAttempt/$logCollectionAttempts failed: $($_.Exception.Message)"
            Start-Sleep -Seconds 3
        }
        finally {
            # UNTESTED: Keep successful collection sessions alive through IDD and bitmap downloads; re-check every FromSession transfer when changing this loop.
            if (($null -ne $session) -and ($null -eq $remoteLogs)) {
                Remove-PSSession -Session $session -ErrorAction SilentlyContinue
                $session = $null
            }
        }
    }

    if ($null -eq $remoteLogs) {
        throw "failed to collect remote logs after $logCollectionAttempts attempts"
    }

    $remoteLogCollectionSucceeded = $true
    $evidenceCorrelated = $remoteLogs.evidence_correlated -eq $true
    $evidenceCorrelationReason = [string]$remoteLogs.evidence_correlation_reason
    Write-Host "Connection evidence: correlated=$evidenceCorrelated; $evidenceCorrelationReason"

    $isRunning = $remoteLogs['running']
    $isListening = $remoteLogs['listening']
    $remoteServiceRunning = [bool]$isRunning
    $remotePortListening = [bool]$isListening

        if ($remoteLogs.ContainsKey('security_4624_type10_user_count')) {
            $securityLogonType10Count = [int]$remoteLogs['security_4624_type10_user_count']
        }
        if ($remoteLogs.ContainsKey('termsrv_fallback_marker_count')) {
            $termsrvFallbackMarkerCount = [int]$remoteLogs['termsrv_fallback_marker_count']
        }
        if ($remoteLogs.ContainsKey('termsrv_fallback_markers')) {
            $termsrvFallbackMarkers = [string]$remoteLogs['termsrv_fallback_markers']
        }
        if ($remoteLogs.ContainsKey('provider_session_proof_marker_count')) {
            $providerSessionProofMarkerCount = [int]$remoteLogs['provider_session_proof_marker_count']
        }
        if ($remoteLogs.ContainsKey('provider_session_proof_markers')) {
            $providerSessionProofMarkers = [string]$remoteLogs['provider_session_proof_markers']
        }
        if ($remoteLogs.ContainsKey('termsrv_session_proof_marker_count')) {
            $termsrvSessionProofMarkerCount = [int]$remoteLogs['termsrv_session_proof_marker_count']
        }
        if ($remoteLogs.ContainsKey('termsrv_session_proof_markers')) {
            $termsrvSessionProofMarkers = [string]$remoteLogs['termsrv_session_proof_markers']
        }
        if ($remoteLogs.ContainsKey('idd_driver_loaded_notified')) {
            $iddDriverLoadedNotified = [bool]$remoteLogs['idd_driver_loaded_notified']
        }
        if ($remoteLogs.ContainsKey('idd_wddm_enabled_signal_count')) {
            $iddWddmEnabledSignalCount = [int]$remoteLogs['idd_wddm_enabled_signal_count']
        }
        if ($remoteLogs.ContainsKey('provider_video_handle_marker_count')) {
            $providerVideoHandleMarkerCount = [int]$remoteLogs['provider_video_handle_marker_count']
        }
        if ($remoteLogs.ContainsKey('provider_video_handle_markers')) {
            $providerVideoHandleMarkers = [string]$remoteLogs['provider_video_handle_markers']
        }
        if ($remoteLogs.ContainsKey('termsrv_idd_marker_count')) {
            $termsrvIddMarkerCount = [int]$remoteLogs['termsrv_idd_marker_count']
        }
        if ($remoteLogs.ContainsKey('termsrv_idd_markers')) {
            $termsrvIddMarkers = [string]$remoteLogs['termsrv_idd_markers']
        }
        if ($remoteLogs.ContainsKey('provider_logon_policy_marker_count')) {
            $providerLogonPolicyMarkerCount = [int]$remoteLogs['provider_logon_policy_marker_count']
        }
        if ($remoteLogs.ContainsKey('provider_logon_policy_markers')) {
            $providerLogonPolicyMarkers = [string]$remoteLogs['provider_logon_policy_markers']
        }
        if ($remoteLogs.ContainsKey('provider_get_hardware_id_count')) {
            $providerGetHardwareIdCount = [int]$remoteLogs['provider_get_hardware_id_count']
        }
        if ($remoteLogs.ContainsKey('provider_on_driver_load_count')) {
            $providerOnDriverLoadCount = [int]$remoteLogs['provider_on_driver_load_count']
        }
        if ($remoteLogs.ContainsKey('provider_assume_present_count')) {
            $providerAssumePresentCount = [int]$remoteLogs['provider_assume_present_count']
        }
        if ($remoteLogs.ContainsKey('provider_video_handle_driver_load_count')) {
            $providerVideoHandleDriverLoadCount = [int]$remoteLogs['provider_video_handle_driver_load_count']
        }
        if ($remoteLogs.ContainsKey('notify_command_process_created_count')) {
            $notifyCommandProcessCreatedCount = [int]$remoteLogs['notify_command_process_created_count']
        }
        if ($remoteLogs.ContainsKey('remote_connection_signal_count')) {
            $remoteConnectionSignalCount = [int]$remoteLogs['remote_connection_signal_count']
        }
        if ($remoteLogs.ContainsKey('remote_graphics_signal_count')) {
            $remoteGraphicsSignalCount = [int]$remoteLogs['remote_graphics_signal_count']
        }
        if ($remoteLogs.ContainsKey('remote_connection_signals')) {
            $remoteConnectionSignalsLog = [string]$remoteLogs['remote_connection_signals']
        }
        if ($remoteLogs.ContainsKey('activation_license_status')) {
            $activationLicenseStatus = [int]$remoteLogs['activation_license_status']
        }
        if ($remoteLogs.ContainsKey('activation_license_status_reason_hex')) {
            $activationLicenseStatusReasonHex = [string]$remoteLogs['activation_license_status_reason_hex']
        }
        if ($remoteLogs.ContainsKey('activation_notification_mode')) {
            $activationNotificationMode = [bool]$remoteLogs['activation_notification_mode']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_id')) {
            $rawGuiTargetSessionId = [int]$remoteLogs['gui_target_session_id']
            if ($rawGuiTargetSessionId -ge 0) {
                $guiTargetSessionId = $rawGuiTargetSessionId
            }
        }
        if ($remoteLogs.ContainsKey('gui_target_session_source')) {
            $guiTargetSessionSource = [string]$remoteLogs['gui_target_session_source']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_resolved')) {
            $guiTargetSessionResolved = [bool]$remoteLogs['gui_target_session_resolved']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_process_proof')) {
            $guiTargetSessionProcessProof = [bool]$remoteLogs['gui_target_session_process_proof']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_explorer_count')) {
            $guiTargetSessionExplorerCount = [int]$remoteLogs['gui_target_session_explorer_count']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_gui_process_count')) {
            $guiTargetSessionGuiProcessCount = [int]$remoteLogs['gui_target_session_gui_process_count']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_winlogon_count')) {
            $guiTargetSessionWinlogonCount = [int]$remoteLogs['gui_target_session_winlogon_count']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_logonui_count')) {
            $guiTargetSessionLogonUiCount = [int]$remoteLogs['gui_target_session_logonui_count']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_processes')) {
            $guiTargetSessionProcesses = [string]$remoteLogs['gui_target_session_processes']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_explorer_bootstrap_pid')) {
            $rawBootstrapPid = [int]$remoteLogs['gui_target_session_explorer_bootstrap_pid']
            if ($rawBootstrapPid -gt 0) {
                $guiTargetSessionExplorerBootstrapPid = $rawBootstrapPid
            }
        }
        if ($remoteLogs.ContainsKey('gui_target_session_explorer_bootstrap_time_utc')) {
            $guiTargetSessionExplorerBootstrapTimeUtc = [string]$remoteLogs['gui_target_session_explorer_bootstrap_time_utc']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_explorer_bootstrap_pid_alive')) {
            $guiTargetSessionExplorerBootstrapPidAlive = [bool]$remoteLogs['gui_target_session_explorer_bootstrap_pid_alive']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_explorer_earliest_start_utc')) {
            $guiTargetSessionExplorerEarliestStartUtc = [string]$remoteLogs['gui_target_session_explorer_earliest_start_utc']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_explorer_after_creation_known')) {
            $guiTargetSessionExplorerAfterCreationKnown = [bool]$remoteLogs['gui_target_session_explorer_after_creation_known']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_explorer_after_creation')) {
            $guiTargetSessionExplorerAfterCreation = [bool]$remoteLogs['gui_target_session_explorer_after_creation']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_creation_signal_utc')) {
            $guiTargetSessionCreationSignalUtc = [string]$remoteLogs['gui_target_session_creation_signal_utc']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_creation_signals')) {
            $guiTargetSessionCreationSignals = [string]$remoteLogs['gui_target_session_creation_signals']
        }
        if ($remoteLogs.ContainsKey('gui_target_session_explorer_details')) {
            $guiTargetSessionExplorerDetails = [string]$remoteLogs['gui_target_session_explorer_details']
        }
        if ($remoteLogs.ContainsKey('explorer_crash_event_count')) {
            $explorerCrashEventCount = [int]$remoteLogs['explorer_crash_event_count']
        }
        if ($remoteLogs.ContainsKey('explorer_hang_event_count')) {
            $explorerHangEventCount = [int]$remoteLogs['explorer_hang_event_count']
        }
        if ($remoteLogs.ContainsKey('explorer_wer_event_count')) {
            $explorerWerEventCount = [int]$remoteLogs['explorer_wer_event_count']
        }
        if ($remoteLogs.ContainsKey('explorer_lifecycle_events')) {
            $explorerLifecycleEvents = [string]$remoteLogs['explorer_lifecycle_events']
        }
        if ($remoteLogs.ContainsKey('idd_open_probe')) {
            $iddOpenProbeText = [string]$remoteLogs['idd_open_probe']
        }

        Write-Host "Service running: $isRunning$(if ($isRunning) { " (PID $($remoteLogs['pid']))" })"
        Write-Host "Port $Port listening: $isListening"

        $remoteLogDir = Join-Path $artifactsDir "remote-logs-$timestamp"
        New-Item -ItemType Directory -Path $remoteLogDir -Force | Out-Null
        if (($null -ne $preConnectSessionSnapshotArtifact) -and (Test-Path -LiteralPath $preConnectSessionSnapshotArtifact)) {
            Copy-Item -LiteralPath $preConnectSessionSnapshotArtifact -Destination (Join-Path $remoteLogDir 'preconnect-session-snapshot.log') -Force
        }

        if ($Mode -eq 'Provider') {
            $helperLogSession = $null
            try {
                $helperLogSession = New-TestVmSession -Hostname $Hostname -Credential $adminCred
                foreach ($pattern in @(
                    (Join-Path $RemoteRoot 'logs\capture-helper-*.log'),
                    (Join-Path $RemoteRoot 'logs\capture-helper-early-*.txt')
                )) {
                    Copy-Item -FromSession $helperLogSession -Path $pattern -Destination $remoteLogDir -Force -ErrorAction SilentlyContinue
                }
            }
            catch {
                Write-Warning "Failed to collect capture-helper logs: $_"
            }
            finally {
                if ($null -ne $helperLogSession) {
                    Remove-PSSession -Session $helperLogSession -ErrorAction SilentlyContinue
                }
            }
        }

        if ($remoteLogs['stdout']) {
            $termsrvLogArtifact = if ($remoteLogs['stdout_raw']) { $remoteLogs['stdout_raw'] } else { $remoteLogs['stdout'] }
            $termsrvLogArtifact | Set-Content (Join-Path $remoteLogDir 'ironrdp-termsrv.log')
            Write-Host "`n---- ironrdp-termsrv.log (tail) ----" -ForegroundColor Yellow
            Write-Host $remoteLogs['stdout']
        }
        if ($remoteLogs['stderr']) {
            $termsrvErrLogArtifact = if ($remoteLogs['stderr_raw']) { $remoteLogs['stderr_raw'] } else { $remoteLogs['stderr'] }
            $termsrvErrLogArtifact | Set-Content (Join-Path $remoteLogDir 'ironrdp-termsrv.err.log')
            Write-Host "`n---- ironrdp-termsrv.err.log (tail) ----" -ForegroundColor Yellow
            Write-Host $remoteLogs['stderr']
        }
        if ($remoteLogs['dll_debug']) {
            $providerLogArtifact = if ($remoteLogs['dll_debug_raw']) { $remoteLogs['dll_debug_raw'] } else { $remoteLogs['dll_debug'] }
            $providerLogArtifact | Set-Content (Join-Path $remoteLogDir 'wts-provider-debug.log')
            Write-Host "`n---- wts-provider-debug.log (tail) ----" -ForegroundColor Magenta
            Write-Host $remoteLogs['dll_debug']

            if ($remoteLogs['dll_debug_key']) {
                $remoteLogs['dll_debug_key'] | Set-Content (Join-Path $remoteLogDir 'wts-provider-debug.key.log')
                Write-Host "`n---- wts-provider-debug.key.log (filtered) ----" -ForegroundColor Magenta
                Write-Host $remoteLogs['dll_debug_key']
            }
        } elseif ($Mode -eq 'Provider') {
            Write-Host "`n---- wts-provider-debug.log: not present (set IRONRDP_WTS_PROVIDER_DEBUG_LOG for TermService to enable) ----" -ForegroundColor DarkGray
        }

        if ($remoteLogs['security_4624_type10_user']) {
            $remoteLogs['security_4624_type10_user'] | Set-Content (Join-Path $remoteLogDir 'security-4624-type10-user.log')
            Write-Host "`n---- Security 4624 (LogonType=10 for configured RDP user) ----" -ForegroundColor Yellow
            Write-Host $remoteLogs['security_4624_type10_user']
        }
        if ($remoteLogs['termservice_events']) {
            Write-Host "`n---- TermService event log (recent) ----" -ForegroundColor Yellow
            Write-Host $remoteLogs['termservice_events']
        }
        if ($remoteLogs['remote_connection_signals']) {
            $remoteLogs['remote_connection_signals'] | Set-Content (Join-Path $remoteLogDir 'remote-connection-signals.log')
            Write-Host "`n---- RemoteConnectionManager operational signals ----" -ForegroundColor Yellow
            Write-Host $remoteLogs['remote_connection_signals']
        }
        if (($Mode -eq 'Provider') -and ($providerSessionProofMarkerCount -gt 0)) {
            Write-Host "`n---- provider session proof markers ----" -ForegroundColor Cyan
            Write-Host $providerSessionProofMarkers
        }
        if (($Mode -eq 'Provider') -and ($termsrvSessionProofMarkerCount -gt 0)) {
            Write-Host "`n---- termsrv session proof markers ----" -ForegroundColor Cyan
            Write-Host $termsrvSessionProofMarkers
        }
        if (($Mode -eq 'Provider') -and ($providerVideoHandleMarkerCount -gt 0)) {
            Write-Host "`n---- provider video-handle markers ----" -ForegroundColor Cyan
            Write-Host $providerVideoHandleMarkers
        }
        if (($Mode -eq 'Provider') -and ($termsrvIddMarkerCount -gt 0)) {
            Write-Host "`n---- termsrv IDD markers ----" -ForegroundColor Cyan
            Write-Host $termsrvIddMarkers
        }
        if (($Mode -eq 'Provider') -and ($providerLogonPolicyMarkerCount -gt 0)) {
            Write-Host "`n---- provider logon-policy markers ----" -ForegroundColor Cyan
            Write-Host $providerLogonPolicyMarkers
        }
        if (($Mode -eq 'Provider') -and ($termsrvFallbackMarkerCount -gt 0)) {
            Write-Host "`n---- termsrv fallback markers (strict-relevant) ----" -ForegroundColor Yellow
            Write-Host $termsrvFallbackMarkers

            if ($StrictSessionProof.IsPresent) {
                throw "strict session proof failed: termsrv fallback markers detected (count=$termsrvFallbackMarkerCount)"
            }
        }

        if ($Mode -eq 'Provider') {
            Write-Host "Interactive proof signals: Security4624Type10=$securityLogonType10Count RemoteConnection261=$remoteConnectionSignalCount RemoteGraphics263=$remoteGraphicsSignalCount"
            Write-Host "IDD diagnostics signals: OnDriverLoad=$providerOnDriverLoadCount GetHardwareId=$providerGetHardwareIdCount ProviderEnableWddmIdd=$iddWddmEnabledSignalCount DriverLoadVideoHandle=$providerVideoHandleDriverLoadCount AssumePresent=$providerAssumePresentCount ProviderVideoHandleMarkers=$providerVideoHandleMarkerCount TermsrvIddMarkers=$termsrvIddMarkerCount ProviderLogonPolicyMarkers=$providerLogonPolicyMarkerCount"
            Write-Host "Shell transition signals: NotifyCommandProcessCreated=$notifyCommandProcessCreatedCount"
            Write-Host "Environment signals: ActivationStatus=$activationLicenseStatus NotificationMode=$activationNotificationMode Reason=$activationLicenseStatusReasonHex"
            Write-Host "GUI session proof: TargetSessionId=$guiTargetSessionId Source=$guiTargetSessionSource Explorer=$guiTargetSessionExplorerCount GuiProcesses=$guiTargetSessionGuiProcessCount Winlogon=$guiTargetSessionWinlogonCount LogonUI=$guiTargetSessionLogonUiCount"
            Write-Host "Explorer lifecycle signals: BootstrapPid=$guiTargetSessionExplorerBootstrapPid BootstrapPidAlive=$guiTargetSessionExplorerBootstrapPidAlive BootstrapTimeUtc=$guiTargetSessionExplorerBootstrapTimeUtc EarliestStartUtc=$guiTargetSessionExplorerEarliestStartUtc SessionCreateUtc=$guiTargetSessionCreationSignalUtc AfterCreateKnown=$guiTargetSessionExplorerAfterCreationKnown AfterCreate=$guiTargetSessionExplorerAfterCreation"
            Write-Host "Explorer error signals: Crash1000=$explorerCrashEventCount Wer1001=$explorerWerEventCount Hang1002=$explorerHangEventCount"

            if (-not [string]::IsNullOrWhiteSpace($guiTargetSessionProcesses)) {
                $guiTargetSessionProcesses | Set-Content (Join-Path $remoteLogDir 'gui-target-session-processes.log')
                Write-Host "`n---- GUI target-session process snapshot ----" -ForegroundColor Yellow
                Write-Host $guiTargetSessionProcesses
            }
            if (-not [string]::IsNullOrWhiteSpace($guiTargetSessionExplorerDetails)) {
                $guiTargetSessionExplorerDetails | Set-Content (Join-Path $remoteLogDir 'gui-target-session-explorer-details.log')
                Write-Host "`n---- GUI target-session explorer details ----" -ForegroundColor Yellow
                Write-Host $guiTargetSessionExplorerDetails
            }
            if (-not [string]::IsNullOrWhiteSpace($guiTargetSessionCreationSignals)) {
                $guiTargetSessionCreationSignals | Set-Content (Join-Path $remoteLogDir 'gui-target-session-creation-signals.log')
                Write-Host "`n---- GUI target-session creation signals ----" -ForegroundColor Yellow
                Write-Host $guiTargetSessionCreationSignals
            }
            if (-not [string]::IsNullOrWhiteSpace($explorerLifecycleEvents)) {
                $explorerLifecycleEvents | Set-Content (Join-Path $remoteLogDir 'explorer-lifecycle-events.log')
                Write-Host "`n---- explorer lifecycle events (Application log) ----" -ForegroundColor Yellow
                Write-Host $explorerLifecycleEvents
            }
            if (-not [string]::IsNullOrWhiteSpace($iddOpenProbeText)) {
                $iddOpenProbeText | Set-Content (Join-Path $remoteLogDir 'idd-open-probe.log')
                Write-Host "`n---- IDD open-path probe ----" -ForegroundColor Cyan
                Write-Host $iddOpenProbeText
            }
        }

        if ($Mode -eq 'Provider') {
            $providerCustomVideoSourceSelected = (-not [string]::IsNullOrWhiteSpace($providerVideoHandleMarkers)) -and ($providerVideoHandleMarkers -match 'source=driver_load_callback')
            $providerManualVideoSourceSelected = (-not [string]::IsNullOrWhiteSpace($providerVideoHandleMarkers)) -and ($providerVideoHandleMarkers -match 'source=ironrdp_idd')
            $providerFallbackVideoSourceSelected = (-not [string]::IsNullOrWhiteSpace($providerVideoHandleMarkers)) -and ($providerVideoHandleMarkers -match 'source=rdp_video_miniport')

            try {
                $iddDiagnostics = Invoke-RemoteCommandWithTimeout -Session $session -TimeoutSeconds 45 -OperationName 'IDD diagnostics collection' -ScriptBlock {
                    param($RuntimeStateFile, $DebugTraceFile)

                    $result = @{}
                    $result['runtime_config'] = if (Test-Path -LiteralPath $RuntimeStateFile) { Get-Content -LiteralPath $RuntimeStateFile -Raw } else { '' }
                    $result['debug_trace'] = if (Test-Path -LiteralPath $DebugTraceFile) { Get-Content -LiteralPath $DebugTraceFile -Raw -ErrorAction SilentlyContinue } else { '' }

                    try {
                        $iddDevices = @(Get-PnpDevice -Class Display -ErrorAction SilentlyContinue |
                            Where-Object { $_.FriendlyName -like 'IronRDP Indirect Display*' -or $_.InstanceId -like '*IronRdpIdd*' })
                        $result['device_inventory'] = if ($iddDevices.Count -gt 0) {
                            $iddDevices | Select-Object Status, Class, FriendlyName, InstanceId, Present | Format-Table -AutoSize | Out-String
                        } else {
                            ''
                        }
                    } catch {
                        $result['device_inventory'] = "Could not collect IDD device inventory: $_"
                    }

                    try {
                        $driverEnumLines = @(& pnputil /enum-drivers 2>$null)
                        $result['driver_inventory'] = if ($driverEnumLines.Count -gt 0) {
                            ($driverEnumLines | Select-String -Pattern 'IronRdpIdd|IronRDP Project|Published Name:|Original Name:|Provider Name:|Class Name:' -Context 0,2 | ForEach-Object {
                                if ($_.Context.PostContext.Count -gt 0) {
                                    @($_.Line) + $_.Context.PostContext
                                } else {
                                    $_.Line
                                }
                            }) | Out-String
                        } else {
                            ''
                        }
                    } catch {
                        $result['driver_inventory'] = "Could not collect IDD driver inventory: $_"
                    }

                    $result
                } -ArgumentList @($iddRuntimeStateFile, $iddDebugTraceFile)

                $iddRuntimeConfigText = [string]$iddDiagnostics['runtime_config']
                $iddDebugTraceText = [string]$iddDiagnostics['debug_trace']
                $iddDeviceInventory = [string]$iddDiagnostics['device_inventory']
                $iddDriverInventory = [string]$iddDiagnostics['driver_inventory']
                $iddCorrelatedDebugTraceText = [string]$remoteLogs.idd_debug_trace_lines
                $providerHardwareIdCustom = (-not [string]::IsNullOrWhiteSpace($iddRuntimeConfigText)) -and ($iddRuntimeConfigText -match '(?m)^hardware_id=IronRdpIdd$')
                $providerHardwareIdCustom = $providerHardwareIdCustom -and ($providerGetHardwareIdCount -ge 1)
                $iddMonitorArrivalMarkerCount = [int]$remoteLogs.idd_monitor_arrival_marker_count
                $iddDisplayConfigUpdateRequestCount = [int]$remoteLogs.idd_display_config_update_request_count
                $iddDisplayConfigUpdateSuccessCount = [int]$remoteLogs.idd_display_config_update_success_count
                $iddCommitModesMarkerCount = [int]$remoteLogs.idd_commit_modes_marker_count
                $iddCommitModesActivePathCount = [int]$remoteLogs.idd_commit_modes_active_path_count
                $iddSwapchainAssignedCount = [int]$remoteLogs.idd_swapchain_assigned_count
                $iddSessionReadyMarkerCount = [int]$remoteLogs.idd_session_ready_marker_count
                $iddFirstFrameMarkerCount = [int]$remoteLogs.idd_first_frame_marker_count

                if (-not [string]::IsNullOrWhiteSpace($iddRuntimeConfigText)) {
                    $iddRuntimeConfigText | Set-Content (Join-Path $remoteLogDir 'idd-runtime-config.txt')
                }
                if (-not [string]::IsNullOrWhiteSpace($iddDebugTraceText)) {
                    $iddDebugTraceText | Set-Content (Join-Path $remoteLogDir 'idd-debug-trace.log')
                    Write-Host "`n---- IDD debug trace ----" -ForegroundColor Cyan
                    Write-Host $iddDebugTraceText
                }
                if (-not [string]::IsNullOrWhiteSpace($iddCorrelatedDebugTraceText)) {
                    $iddCorrelatedDebugTraceText | Set-Content (Join-Path $remoteLogDir 'idd-debug-trace-current-session.log')
                }
                if (-not [string]::IsNullOrWhiteSpace($iddDeviceInventory)) {
                    $iddDeviceInventory | Set-Content (Join-Path $remoteLogDir 'idd-device-inventory.log')
                }
                if (-not [string]::IsNullOrWhiteSpace($iddDriverInventory)) {
                    $iddDriverInventory | Set-Content (Join-Path $remoteLogDir 'idd-driver-inventory.log')
                }

                Write-Host "IDD strict proof signals: HardwareIdCustom=$providerHardwareIdCustom GetHardwareId=$providerGetHardwareIdCount OnDriverLoad=$providerOnDriverLoadCount DriverLoadVideoHandle=$providerVideoHandleDriverLoadCount AssumePresent=$providerAssumePresentCount VideoSourceDriverLoad=$providerCustomVideoSourceSelected VideoSourceManual=$providerManualVideoSourceSelected VideoSourceFallback=$providerFallbackVideoSourceSelected MonitorArrived=$iddMonitorArrivalMarkerCount DisplayConfigRequest=$iddDisplayConfigUpdateRequestCount DisplayConfigSuccess=$iddDisplayConfigUpdateSuccessCount CommitModes=$iddCommitModesMarkerCount CommitModesActive=$iddCommitModesActivePathCount SwapchainAssigned=$iddSwapchainAssignedCount SessionReady=$iddSessionReadyMarkerCount FirstFrame=$iddFirstFrameMarkerCount"
            } catch {
                Write-Warning "Failed to collect IDD diagnostics: $_"
            }
        }

        # Pull bitmap dumps (best-effort) so we can prove capture was producing real frames.
        try {
            New-Item -ItemType Directory -Path $dumpLocalDir -Force | Out-Null

            $remoteHasDump = Invoke-Command -Session $session -ScriptBlock {
                param($DumpDir)
                Test-Path -LiteralPath $DumpDir
            } -ArgumentList $dumpRemoteDir

            if ($remoteHasDump) {
                Copy-Item -FromSession $session -Path (Join-Path $dumpRemoteDir '*') -Destination $dumpLocalDir -Recurse -Force -ErrorAction SilentlyContinue
                $bmps = @(Get-ChildItem -LiteralPath $dumpLocalDir -Filter '*.bmp' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending)
                $bmpCount = ($bmps | Measure-Object).Count
                if ($bmpCount -gt 0) {
                    $bitmapDumpCount = $bmpCount

                    $bitmapSessionIds = @()
                    foreach ($bmp in $bmps) {
                        if ($bmp.Name -match 'bitmap-update-s(\d+)-p\d+-\d+\.bmp') {
                            $bitmapSessionIds += [int]$Matches[1]
                        }
                    }

                    if ($bitmapSessionIds.Count -gt 0) {
                        $bitmapObservedSessionIds = @($bitmapSessionIds | Sort-Object -Unique)
                        if ($null -ne $guiTargetSessionId) {
                            $bitmapTargetSessionMatchCount = @($bitmapSessionIds | Where-Object { $_ -eq $guiTargetSessionId }).Count
                            $bitmapTargetSessionHasGraphics = ($bitmapTargetSessionMatchCount -ge 1)
                        }
                    }

                    if (($null -ne $securityLogonType10Count) -and ($securityLogonType10Count -ge 1) -and $guiTargetSessionResolved -and $bitmapTargetSessionHasGraphics) {
                        $type10GraphicsSessionConfirmed = $true
                    }

                    $latest = $bmps | Select-Object -First 1
                    Copy-Item -LiteralPath $latest.FullName -Destination (Join-Path $artifactsDir 'latest-bitmap-dump.bmp') -Force
                    Write-Host "`nBitmap dumps: $bmpCount file(s) downloaded to $dumpLocalDir" -ForegroundColor Green
                    Write-Host "Latest dump: $($latest.Name) ($([math]::Round($latest.Length / 1MB, 2)) MB)" -ForegroundColor Green

                    if ($bitmapObservedSessionIds.Count -gt 0) {
                        Write-Host "Bitmap session IDs observed: $($bitmapObservedSessionIds -join ',')"
                    }

                    if ($null -ne $guiTargetSessionId) {
                        Write-Host "Session-linked graphics proof: target_session=$guiTargetSessionId bitmap_matches=$bitmapTargetSessionMatchCount confirmed=$bitmapTargetSessionHasGraphics"
                    }

                    Write-Host "Type10+graphics-in-target-session proof: $type10GraphicsSessionConfirmed"
                } else {
                    Write-Host "`nBitmap dumps: directory exists but no .bmp files were found at $dumpLocalDir" -ForegroundColor Yellow
                }
            } else {
                Write-Host "`nBitmap dumps: remote dump directory not present: $dumpRemoteDir" -ForegroundColor DarkGray
            }
        } catch {
            Write-Warning "Failed to download bitmap dumps: $_"
        }

        if ($Mode -eq 'Provider') {
            try {
                New-Item -ItemType Directory -Path $iddDumpLocalDir -Force | Out-Null

                $remoteHasIddDump = Invoke-Command -Session $session -ScriptBlock {
                    param($DumpDir)
                    Test-Path -LiteralPath $DumpDir
                } -ArgumentList $iddDumpRemoteDir

                if ($remoteHasIddDump) {
                    Copy-Item -FromSession $session -Path (Join-Path $iddDumpRemoteDir '*') -Destination $iddDumpLocalDir -Recurse -Force -ErrorAction SilentlyContinue
                    $iddBmps = @(Get-ChildItem -LiteralPath $iddDumpLocalDir -Filter '*.bmp' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending)
                    $iddDumpCount = ($iddBmps | Measure-Object).Count
                    if ($iddDumpCount -gt 0) {
                        $iddSessionIds = @()
                        foreach ($bmp in $iddBmps) {
                            if ($bmp.Name -match 'idd-swapchain-s(\d+)-f\d+-\d+\.bmp') {
                                $iddSessionIds += [int]$Matches[1]
                            }
                        }

                        if ($iddSessionIds.Count -gt 0) {
                            $iddObservedSessionIds = @($iddSessionIds | Sort-Object -Unique)
                            if ($null -ne $guiTargetSessionId) {
                                $iddTargetSessionMatchCount = @($iddSessionIds | Where-Object { $_ -eq $guiTargetSessionId }).Count
                                $iddTargetSessionHasGraphics = ($iddTargetSessionMatchCount -ge 1)
                            }
                        }

                        if (($null -ne $securityLogonType10Count) -and ($securityLogonType10Count -ge 1) -and $guiTargetSessionResolved -and $iddTargetSessionHasGraphics) {
                            $type10IddGraphicsSessionConfirmed = $true
                            $type10GraphicsSessionConfirmed = $true
                        }

                        $bitmapDumpCount = $iddDumpCount
                        $bitmapObservedSessionIds = $iddObservedSessionIds
                        $bitmapTargetSessionMatchCount = $iddTargetSessionMatchCount
                        $bitmapTargetSessionHasGraphics = $iddTargetSessionHasGraphics

                        $latestIddDump = $iddBmps | Select-Object -First 1
                        Copy-Item -LiteralPath $latestIddDump.FullName -Destination (Join-Path $artifactsDir 'latest-idd-bitmap-dump.bmp') -Force
                        Write-Host "`nIDD bitmap dumps: $iddDumpCount file(s) downloaded to $iddDumpLocalDir" -ForegroundColor Green
                        Write-Host "Latest IDD dump: $($latestIddDump.Name) ($([math]::Round($latestIddDump.Length / 1MB, 2)) MB)" -ForegroundColor Green
                    } else {
                        Write-Host "`nIDD bitmap dumps: directory exists but no .bmp files were found at $iddDumpLocalDir" -ForegroundColor Yellow
                    }
                } else {
                    Write-Host "`nIDD bitmap dumps: remote dump directory not present: $iddDumpRemoteDir" -ForegroundColor DarkGray
                }
            } catch {
                Write-Warning "Failed to download IDD bitmap dumps: $_"
            }
        }
} catch {
    Write-Warning "Client observation or remote evidence collection failed: $_"
    if ($StrictSessionProof.IsPresent) {
        throw
    }
} finally {
    if ($null -ne $session) {
        Remove-PSSession -Session $session -ErrorAction SilentlyContinue
        $session = $null
    }

    if ($null -ne $iddOpenProbeJob) {
        Wait-Job -Job $iddOpenProbeJob -Timeout 5 | Out-Null
        Receive-Job -Job $iddOpenProbeJob -Keep -ErrorAction SilentlyContinue | Out-Null
        Remove-Job -Job $iddOpenProbeJob -Force -ErrorAction SilentlyContinue
        $iddOpenProbeJob = $null
    }

    if ($null -ne $iddOpenProbeSession) {
        Remove-PSSession -Session $iddOpenProbeSession -ErrorAction SilentlyContinue
        $iddOpenProbeSession = $null
    }
}

# ── Summary ─────────────────────────────────────────────────────────────────
Write-Host "`n=== Summary ===" -ForegroundColor Cyan
Write-Host "Mode:       $Mode"
Write-Host "VM:         $Hostname"
Write-Host "Port:       $Port"
Write-Host "Strict:     $($StrictSessionProof.IsPresent)"
Write-Host "Screenshot: $(if ($screenshotSucceeded -and (Test-Path -LiteralPath $OutputPng)) { "$OutputPng ($($(Get-Item -LiteralPath $OutputPng).Length) bytes)" } else { 'NOT PRODUCED BY THIS RUN' })"

if ($Mode -eq 'Provider') {
    $cleanupStatus = if ($DisableFreshSessionCleanup.IsPresent) {
        'disabled'
    } elseif (-not $freshSessionCleanupAttempted) {
        'not-attempted'
    } elseif ($freshSessionCleanupSucceeded) {
        'ok'
    } else {
        'failed'
    }

    Write-Host "Fresh cleanup: status=$cleanupStatus aggressive=$freshSessionCleanupAggressiveMode target_user=$(if ([string]::IsNullOrWhiteSpace($freshSessionCleanupTargetUser)) { 'n/a' } else { $freshSessionCleanupTargetUser }) target_session=$FreshSessionCleanupSessionId discovered=$(if ([string]::IsNullOrWhiteSpace($freshSessionCleanupDiscoveredSessionIds)) { 'none' } else { $freshSessionCleanupDiscoveredSessionIds }) logged_off=$(if ([string]::IsNullOrWhiteSpace($freshSessionCleanupLoggedOffSessionIds)) { 'none' } else { $freshSessionCleanupLoggedOffSessionIds }) reset=$(if ([string]::IsNullOrWhiteSpace($freshSessionCleanupResetSessionIds)) { 'none' } else { $freshSessionCleanupResetSessionIds })"

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupExplorerMatches)) {
        $freshSessionCleanupExplorerMatches | Set-Content (Join-Path $artifactsDir 'fresh-session-cleanup-explorer-matches.log')
    }

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupQuserMatches)) {
        $freshSessionCleanupQuserMatches | Set-Content (Join-Path $artifactsDir 'fresh-session-cleanup-quser-matches.log')
    }

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupWinstaMatches)) {
        $freshSessionCleanupWinstaMatches | Set-Content (Join-Path $artifactsDir 'fresh-session-cleanup-qwinsta-matches.log')
    }

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupSuspiciousSessionMatches)) {
        $freshSessionCleanupSuspiciousSessionMatches | Set-Content (Join-Path $artifactsDir 'fresh-session-cleanup-suspicious-sessions.log')
    }

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupLogoffErrors)) {
        $freshSessionCleanupLogoffErrors | Set-Content (Join-Path $artifactsDir 'fresh-session-cleanup-logoff-errors.log')
    }

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupResetErrors)) {
        $freshSessionCleanupResetErrors | Set-Content (Join-Path $artifactsDir 'fresh-session-cleanup-reset-errors.log')
    }

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupError)) {
        Write-Warning "Fresh-session cleanup error: $freshSessionCleanupError"
    }
}

$screenshotExists = $screenshotSucceeded -and (Test-Path -LiteralPath $OutputPng)
$screenshotFileInfo = $null
$hasMeaningfulContent = $false
$analysisSucceeded = $false
$analysisMessage = ''

if ($preflightBlockedConnectionAttempt) {
    $blockedReason = if ($preflightIssues.Count -gt 0) {
        if ($hardPreflightIssues.Count -gt 0) {
            $hardPreflightIssues -join '; '
        } else {
            $preflightIssues -join '; '
        }
    } elseif ($activationNotificationMode) {
        "Windows activation notification mode is enabled (LicenseStatus=$activationLicenseStatus, Reason=$activationLicenseStatusReasonHex)"
    } else {
        'connection attempt skipped by preflight gate'
    }
    Write-Host "RESULT: BLOCKED ($blockedReason)" -ForegroundColor Yellow
} elseif ($screenshotExists) {
    $screenshotFileInfo = Get-Item $OutputPng

    try {
        Add-Type -AssemblyName System.Drawing -ErrorAction Stop

        $bitmap = [System.Drawing.Bitmap]::new($OutputPng)
        try {
            $pixelAnalysis = Get-ScreenshotPixelAnalysis -Bitmap $bitmap
            $hasMeaningfulContent = $pixelAnalysis.HasMeaningfulContent
            $analysisSucceeded = $true
            $analysisMessage = "sampled_pixels=$($pixelAnalysis.SampleCount) uniform_rgb=$($pixelAnalysis.IsUniform) non_black_rgb=$($pixelAnalysis.HasNonBlackPixel)"
        }
        finally {
            $bitmap.Dispose()
        }
    }
    catch {
        $analysisMessage = "image analysis failed: $_"
    }

    Write-Host "Screenshot analysis: $analysisMessage"

    if ($screenshotFileInfo.Length -le 1000) {
        Write-Host "RESULT: WARN (screenshot produced but suspiciously small: $($screenshotFileInfo.Length) bytes)" -ForegroundColor Yellow
    } elseif (-not $analysisSucceeded) {
        Write-Host "RESULT: WARN (screenshot produced but content analysis failed)" -ForegroundColor Yellow
    } elseif ($hasMeaningfulContent) {
        Write-Host "RESULT: PASS (screenshot produced with meaningful content)" -ForegroundColor Green
    } else {
        Write-Host "RESULT: FAIL (screenshot produced but frame is blank/uniform)" -ForegroundColor Red
    }
} elseif ($SkipScreenshot) {
    Write-Host "RESULT: DIAGNOSTICS ONLY (screenshot intentionally skipped)" -ForegroundColor Yellow
} else {
    Write-Host "RESULT: FAIL (no screenshot produced)" -ForegroundColor Red
}

$evidence = @{
        evidenceCorrelated = $evidenceCorrelated
        evidenceCorrelationReason = $evidenceCorrelationReason
        analysisSucceeded = $analysisSucceeded
        guiTargetSessionExplorerCount = $guiTargetSessionExplorerCount
        guiTargetSessionGuiProcessCount = $guiTargetSessionGuiProcessCount
        guiTargetSessionId = $guiTargetSessionId
        guiTargetSessionLogonUiCount = $guiTargetSessionLogonUiCount
        guiTargetSessionProcessProof = $guiTargetSessionProcessProof
        guiTargetSessionResolved = $guiTargetSessionResolved
        guiTargetSessionSource = $guiTargetSessionSource
        guiTargetSessionWinlogonCount = $guiTargetSessionWinlogonCount
        hardPreflightIssues = $hardPreflightIssues
        hasMeaningfulContent = $hasMeaningfulContent
        iddCommitModesActivePathCount = $iddCommitModesActivePathCount
        iddCommitModesMarkerCount = $iddCommitModesMarkerCount
        iddDisplayConfigUpdateRequestCount = $iddDisplayConfigUpdateRequestCount
        iddDisplayConfigUpdateSuccessCount = $iddDisplayConfigUpdateSuccessCount
        iddDriverLoadedNotified = $iddDriverLoadedNotified
        iddDumpCount = $iddDumpCount
        iddFirstFrameMarkerCount = $iddFirstFrameMarkerCount
        iddMonitorArrivalMarkerCount = $iddMonitorArrivalMarkerCount
        iddObservedSessionIds = $iddObservedSessionIds
        iddSessionReadyMarkerCount = $iddSessionReadyMarkerCount
        iddSwapchainAssignedCount = $iddSwapchainAssignedCount
        iddTargetSessionHasGraphics = $iddTargetSessionHasGraphics
        iddTargetSessionMatchCount = $iddTargetSessionMatchCount
        iddWddmEnabledSignalCount = $iddWddmEnabledSignalCount
        Mode = $Mode
        notifyCommandProcessCreatedCount = $notifyCommandProcessCreatedCount
        preflightBlockedConnectionAttempt = $preflightBlockedConnectionAttempt
        providerAssumePresentCount = $providerAssumePresentCount
        providerCustomVideoSourceSelected = $providerCustomVideoSourceSelected
        providerFallbackVideoSourceSelected = $providerFallbackVideoSourceSelected
        providerGetHardwareIdCount = $providerGetHardwareIdCount
        providerHardwareIdCustom = $providerHardwareIdCustom
        providerLogonPolicyMarkerCount = $providerLogonPolicyMarkerCount
        providerManualVideoSourceSelected = $providerManualVideoSourceSelected
        providerOnDriverLoadCount = $providerOnDriverLoadCount
        providerSessionProofMarkerCount = $providerSessionProofMarkerCount
        providerVideoHandleDriverLoadCount = $providerVideoHandleDriverLoadCount
        providerVideoHandleMarkerCount = $providerVideoHandleMarkerCount
        RdpUsername = $RdpUsername
        remoteLogCollectionSucceeded = $remoteLogCollectionSucceeded
        screenshotExists = $screenshotExists
        screenshotFileInfo = $screenshotFileInfo
        securityLogonType10Count = $securityLogonType10Count
        termsrvFallbackMarkerCount = $termsrvFallbackMarkerCount
        termsrvIddMarkerCount = $termsrvIddMarkerCount
        termsrvSessionProofMarkerCount = $termsrvSessionProofMarkerCount
        type10IddGraphicsSessionConfirmed = $type10IddGraphicsSessionConfirmed
}
if ($StrictSessionProof.IsPresent) {
    & (Join-Path $scriptRoot 'assert-e2e-session-proof.ps1') -Evidence $evidence
} else {
    Assert-E2EOutput -Evidence $evidence -SkipScreenshot $SkipScreenshot.IsPresent
}
