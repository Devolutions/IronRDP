<#
.SYNOPSIS
    End-to-end test: build, deploy ironrdp-termsrv to test VM, run screenshot client, collect results.

.DESCRIPTION
    Orchestrates a full deploy-and-test cycle:
      1. Build ironrdp-termsrv (and optionally the provider DLL) locally.
      2. Deploy to the test VM via PSRemoting (reuses deploy-testvm-psremoting.ps1).
      3. Run the IronRDP screenshot example against the VM.
      4. Collect remote logs and validate output.

.PARAMETER Mode
    "Standalone" deploys only the companion service with AUTO_LISTEN=1 (no WTS provider).
    "Provider" deploys both the provider DLL and companion service with side-by-side registration.

.PARAMETER StrictSessionProof
    Opt-in strict proof mode. Treats fallback/session-fidelity diagnostics as hard pass/fail gates.
    Alias: -Strict

.EXAMPLE
    .\e2e-test-screenshot.ps1 -Mode Standalone
    .\e2e-test-screenshot.ps1 -Mode Provider
    .\e2e-test-screenshot.ps1 -Mode Provider -StrictSessionProof
#>
[CmdletBinding()]
param(
    [Parameter()]
    [ValidateSet('Standalone', 'Provider')]
    [string]$Mode = 'Standalone',

    [Parameter()]
    [Alias('ComputerName')]
    [string]$Hostname = 'IT-HELP-TEST',

    [Parameter()]
    [Alias('ServerPort')]
    [int]$Port = 4489,

    [Parameter()]
    [pscredential]$Credential,

    [Parameter()]
    [string]$AdminUsername = 'IT-HELP\Administrator',

    [Parameter()]
    [string]$AdminPassword = 'DevoLabs123!',

    [Parameter()]
    [string]$AdminPasswordEnvVar = 'IRONRDP_TESTVM_PASSWORD',

    [Parameter()]
    [string]$RdpUsername = 'Administrator',

    [Parameter()]
    [string]$RdpPassword = 'DevoLabs123!',

    [Parameter()]
    [string]$RdpPasswordEnvVar = 'IRONRDP_TESTVM_RDP_PASSWORD',

    [Parameter()]
    [string]$RdpDomain = 'ad.it-help.ninja',

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
    [switch]$SkipScreenshot,

    [Parameter()]
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',

    [Parameter()]
    [int]$ScreenshotTimeoutSeconds = 30,

    [Parameter()]
    [int]$AfterFirstGraphicsSeconds = 20,

    [Parameter()]
    [switch]$DisableFreshSessionCleanup,

    [Parameter()]
    [switch]$AggressiveFreshSessionCleanup,

    [Parameter()]
    [Alias('Strict')]
    [switch]$StrictSessionProof
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function New-TestVmSession {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Hostname,

        [Parameter(Mandatory = $true)]
        [pscredential]$Credential
    )

    try {
        return New-PSSession -ComputerName $Hostname -Credential $Credential -ErrorAction Stop
    }
    catch {
        Write-Warning "WinRM over HTTP failed for $Hostname; trying WinRM over HTTPS (5986)"
        $sessOpts = New-PSSessionOption -SkipCACheck -SkipCNCheck -SkipRevocationCheck
        return New-PSSession -ComputerName $Hostname -Credential $Credential -UseSSL -Port 5986 -SessionOption $sessOpts -ErrorAction Stop
    }
}

function Get-HyperVVmNameFromHostname {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Hostname
    )

    $trimmed = $Hostname.Trim()
    if ([string]::IsNullOrWhiteSpace($trimmed)) {
        return $trimmed
    }

    return ($trimmed -split '\.')[0]
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
        [pscredential]$Credential,

        [Parameter()]
        [ValidateRange(1, 120)]
        [int]$InitialBootWaitSeconds = 15,

        [Parameter()]
        [ValidateRange(30, 900)]
        [int]$RemotingReadyTimeoutSeconds = 300
    )

    $vmName = Get-HyperVVmNameFromHostname -Hostname $Hostname
    if ([string]::IsNullOrWhiteSpace($vmName)) {
        throw "cannot derive Hyper-V VM name from hostname '$Hostname'"
    }

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

$adminCred = $null

if ($PSBoundParameters.ContainsKey('Credential') -and ($null -ne $Credential)) {
    $adminCred = $Credential
} else {
    $adminPasswordEffective = $null
    if ($PSBoundParameters.ContainsKey('AdminPassword') -and (-not [string]::IsNullOrWhiteSpace($AdminPassword))) {
        $adminPasswordEffective = $AdminPassword
    } else {
        $fromEnv = [Environment]::GetEnvironmentVariable($AdminPasswordEnvVar)
        if (-not [string]::IsNullOrWhiteSpace($fromEnv)) {
            $adminPasswordEffective = $fromEnv
        } else {
            $adminPasswordEffective = $AdminPassword
        }
    }

    $securePwd = ConvertTo-SecureString -String $adminPasswordEffective -AsPlainText -Force
    $adminCred = [pscredential]::new($AdminUsername, $securePwd)
}

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

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$workspaceRoot = (Resolve-Path (Join-Path $scriptRoot '..\..\..')).Path
$artifactsDir = Join-Path $workspaceRoot 'artifacts'
New-Item -ItemType Directory -Path $artifactsDir -Force | Out-Null

$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$testStartTime = Get-Date
if ([string]::IsNullOrWhiteSpace($OutputPng)) {
    $OutputPng = Join-Path $artifactsDir "screenshot-$timestamp.png"
}

$dumpRemoteDir = "C:\IronRDPDeploy\bitmap-dumps-$timestamp"
$dumpLocalDir = Join-Path $artifactsDir "bitmap-dumps-$timestamp"

$profileDir = if ($Configuration -eq 'Release') { 'release' } else { 'debug' }

$remoteLogCollectionSucceeded = $false
$remoteServiceRunning = $false
$remotePortListening = $false
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
$freshSessionCleanupExplorerMatches = ''
$freshSessionCleanupQuserMatches = ''
$freshSessionCleanupLogoffErrors = ''
$freshSessionCleanupError = ''
$freshSessionCleanupAggressiveMode = $false

# ── Step 1: Build ───────────────────────────────────────────────────────────
if (-not $SkipBuild.IsPresent) {
    Write-Host "`n=== Step 1: Building ironrdp-termsrv ($Configuration) ===" -ForegroundColor Cyan
    Push-Location $workspaceRoot
    try {
        if ($Configuration -eq 'Release') {
            cargo build -p ironrdp-termsrv --release
        } else {
            cargo build -p ironrdp-termsrv
        }
        if ($LASTEXITCODE -ne 0) { throw "cargo build ironrdp-termsrv failed (exit $LASTEXITCODE)" }

        if ($Mode -eq 'Provider') {
            Write-Host "Building ironrdp-wtsprotocol-provider ($Configuration)..." -ForegroundColor Cyan
            if ($Configuration -eq 'Release') {
                cargo build -p ironrdp-wtsprotocol-provider --release
            } else {
                cargo build -p ironrdp-wtsprotocol-provider
            }
            if ($LASTEXITCODE -ne 0) { throw "cargo build ironrdp-wtsprotocol-provider failed (exit $LASTEXITCODE)" }
        }

        Write-Host "Building screenshot example..." -ForegroundColor Cyan
        cargo build --example screenshot -p ironrdp --features "session,connector,graphics"
        if ($LASTEXITCODE -ne 0) { throw "cargo build screenshot failed (exit $LASTEXITCODE)" }
    }
    finally {
        Pop-Location
    }
    Write-Host "Build succeeded" -ForegroundColor Green
} else {
    Write-Host "`n=== Step 1: Build skipped ===" -ForegroundColor Yellow
}

$screenshotExe = Join-Path $workspaceRoot "target\debug\examples\screenshot.exe"
if (-not (Test-Path $screenshotExe)) {
    $screenshotExe = Join-Path $workspaceRoot "target\release\examples\screenshot.exe"
}
if (-not (Test-Path $screenshotExe)) {
    throw "screenshot.exe not found in target\debug\examples or target\release\examples"
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
        SkipBuild        = $true
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
        # In Provider mode, skip the TermService start in the deploy step.  TermService will be
        # started exactly once by the provider-DLL install step below.  A double TermService
        # start/stop cycle triggers StopListen IPC which aborts the companion's TCP listener task,
        # causing wait-termservice-ready.ps1 to time out after the second restart.
        NoTermServiceStart   = ($Mode -eq 'Provider')
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
                Write-Host "Installing side-by-side WTS provider on $Hostname..." -ForegroundColor Cyan

                $session = New-TestVmSession -Hostname $Hostname -Credential $adminCred
                try {
                    $providerDll = Join-Path $workspaceRoot "target\$profileDir\ironrdp_wtsprotocol_provider.dll"
                    if (-not (Test-Path $providerDll)) {
                        throw "Provider DLL not found: $providerDll"
                    }

                    $remoteProviderDir = 'C:\IronRDPDeploy\provider'
                    Invoke-Command -Session $session -ScriptBlock {
                        param($Dir)
                        New-Item -ItemType Directory -Path $Dir -Force | Out-Null
                    } -ArgumentList $remoteProviderDir

                    # Stop TermService so the provider DLL can be replaced (it's loaded by TermService)
                    Write-Host "Stopping TermService to allow provider DLL update..." -ForegroundColor Cyan
                    Invoke-Command -Session $session -ScriptBlock {
                        param($StopTimeoutSeconds)

                        & sc.exe config TermService start= disabled | Out-Null

                        $termServiceStopped = $false
                        $termServicePid = 0
                        $stopDeadline = (Get-Date).AddSeconds($StopTimeoutSeconds)
                        while ((Get-Date) -lt $stopDeadline) {
                            & sc.exe stop TermService | Out-Null

                            $waitDeadline = (Get-Date).AddSeconds(8)
                            while ((Get-Date) -lt $waitDeadline) {
                                $service = Get-Service -Name 'TermService' -ErrorAction SilentlyContinue
                                if ($null -eq $service -or $service.Status -eq 'Stopped') {
                                    $termServiceStopped = $true
                                    break
                                }

                                Start-Sleep -Milliseconds 500
                            }

                            if ($termServiceStopped) {
                                break
                            }

                            $serviceCim = Get-CimInstance Win32_Service -Filter "Name='TermService'" -ErrorAction SilentlyContinue
                            $termServicePid = 0
                            if ($null -ne $serviceCim) {
                                $termServicePid = [int]$serviceCim.ProcessId
                            }

                            if ($termServicePid -gt 0) {
                                Write-Warning "TermService still running; force-stopping host process PID $termServicePid"
                                Stop-Process -Id $termServicePid -Force -ErrorAction SilentlyContinue
                                & taskkill.exe /PID $termServicePid /F /T 2>$null | Out-Null
                            }

                            Start-Sleep -Seconds 1
                        }

                        if (-not $termServiceStopped) {
                            $service = Get-Service -Name 'TermService' -ErrorAction SilentlyContinue
                            $statusText = if ($null -eq $service) { 'missing' } else { $service.Status }
                            throw "TermService did not stop within ${StopTimeoutSeconds}s during provider DLL update (status=$statusText, last_pid=$termServicePid)"
                        }

                        & sc.exe config TermService start= demand | Out-Null
                    } -ArgumentList 60

                    # Wait for the remote DLL file to be released (svchost may still hold it briefly after Stop)
                    Invoke-Command -Session $session -ScriptBlock {
                        param($DllPath, $WaitSeconds)

                        $deadline = (Get-Date).AddSeconds($WaitSeconds)
                        $released = $false
                        while ((Get-Date) -lt $deadline) {
                            if (-not (Test-Path -LiteralPath $DllPath)) {
                                $released = $true
                                break
                            }

                            try {
                                $stream = [System.IO.File]::Open($DllPath,
                                    [System.IO.FileMode]::Open,
                                    [System.IO.FileAccess]::ReadWrite,
                                    [System.IO.FileShare]::None)
                                $stream.Close()
                                $released = $true
                                break
                            }
                            catch {
                                Start-Sleep -Milliseconds 500
                            }
                        }

                        if (-not $released) {
                            throw "Provider DLL '$DllPath' was not released within ${WaitSeconds}s after TermService stop"
                        }

                        Write-Host "Provider DLL released (file lock cleared)"
                    } -ArgumentList "$remoteProviderDir\ironrdp_wtsprotocol_provider.dll", 30

                    Copy-Item -ToSession $session -Path $providerDll -Destination "$remoteProviderDir\ironrdp_wtsprotocol_provider.dll" -Force

                    $providerScriptsDir = Join-Path $workspaceRoot 'crates\ironrdp-wtsprotocol-provider\scripts'
                    $scriptFiles = Get-ChildItem -LiteralPath $providerScriptsDir -Filter '*.ps1'
                    foreach ($sf in $scriptFiles) {
                        Copy-Item -ToSession $session -Path $sf.FullName -Destination "$remoteProviderDir\$($sf.Name)" -Force
                    }

                    Invoke-Command -Session $session -ScriptBlock {
                        param($ProviderDir, $Port)

                        $dllPath = Join-Path $ProviderDir 'ironrdp_wtsprotocol_provider.dll'
                        $installScript = Join-Path $ProviderDir 'install-side-by-side.ps1'
                        $defaultsScript = Join-Path $ProviderDir 'side-by-side-defaults.ps1'
                        $firewallScript = Join-Path $ProviderDir 'configure-side-by-side-firewall.ps1'
                        $waitScript = Join-Path $ProviderDir 'wait-termservice-ready.ps1'

                        . $defaultsScript

                        & $installScript `
                            -ProviderDllPath $dllPath `
                            -ListenerName 'IRDP-Tcp' `
                            -PortNumber $Port `
                            -RestartTermService `
                            -TermServiceStopTimeoutSeconds 60 `
                            -TermServiceStartTimeoutSeconds 60

                        & $firewallScript -Mode Add -PortNumber $Port

                        & $waitScript -PortNumber $Port -TimeoutSeconds 90
                    } -ArgumentList $remoteProviderDir, $Port
                }
                finally {
                    if ($null -ne $session) {
                        Remove-PSSession -Session $session -ErrorAction SilentlyContinue
                    }
                }
            }

            Write-Host "Deploy succeeded" -ForegroundColor Green
            break
        }
        catch {
            $errorMessage = $_.Exception.Message
            $canRetry = ($deployAttempt -lt $deployMaxAttempts) -and (Test-DeployRestartRecoverableError -Message $errorMessage)
            if (-not $canRetry) {
                throw
            }

            Write-Warning "Deploy attempt $deployAttempt failed with a recoverable TermService/WinRM interruption; forcing Hyper-V reboot and retrying once"
            Restart-TestVmViaHyperV -Hostname $Hostname -Credential $adminCred
            Write-Host "Retrying deploy after Hyper-V reboot..." -ForegroundColor Yellow
        }
    }
} else {
    Write-Host "`n=== Step 2: Deploy skipped ===" -ForegroundColor Yellow
}

# ── Step 2.5: Fresh-session cleanup (Provider mode) ─────────────────────────
if (($Mode -eq 'Provider') -and (-not $DisableFreshSessionCleanup.IsPresent) -and (-not $SkipScreenshot.IsPresent)) {
    Write-Host "`n=== Step 2.5: Fresh-session cleanup for target user ===" -ForegroundColor Cyan
    $freshSessionCleanupAttempted = $true
    $freshSessionCleanupAggressiveMode = $AggressiveFreshSessionCleanup.IsPresent

    try {
        $session = New-TestVmSession -Hostname $Hostname -Credential $adminCred
        try {
            $cleanup = Invoke-Command -Session $session -ScriptBlock {
                param($RdpUsernameRaw, $RdpDomainRaw, $AggressiveModeRaw)

                $result = @{}
                $result['target_user'] = ''
                $result['aggressive_mode'] = $false
                $result['discovered_session_ids'] = ''
                $result['logged_off_session_ids'] = ''
                $result['explorer_matches'] = ''
                $result['quser_matches'] = ''
                $result['logoff_errors'] = ''

                $aggressiveMode = [bool]$AggressiveModeRaw
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
                if (-not [string]::IsNullOrWhiteSpace($rdpDomain)) {
                    $rdpDomain = $rdpDomain.Trim()
                    $domainHints.Add($rdpDomain)
                    $domainShort = ($rdpDomain -split '\\.')[0]
                    if (-not [string]::IsNullOrWhiteSpace($domainShort)) {
                        $domainHints.Add($domainShort)
                    }
                }

                $sessionIds = New-Object System.Collections.Generic.List[int]
                $explorerRows = New-Object System.Collections.Generic.List[object]

                $explorerProcs = @(Get-Process -Name 'explorer' -IncludeUserName -ErrorAction SilentlyContinue)
                foreach ($proc in $explorerProcs) {
                    $rawUserName = [string]$proc.UserName
                    if ([string]::IsNullOrWhiteSpace($rawUserName)) {
                        continue
                    }

                    $userPart = $rawUserName
                    $domainPart = ''

                    if ($rawUserName.Contains('\\')) {
                        $split = $rawUserName.Split('\\', 2)
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

                    if (($domainHints.Count -gt 0) -and (-not [string]::IsNullOrWhiteSpace($domainPart))) {
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

                if ($aggressiveMode) {
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

                            if (-not ($quserUser -ieq $targetUser)) {
                                continue
                            }

                            $quserRows.Add([pscustomobject]@{
                                UserName    = $quserUser
                                SessionName = $quserSessionName
                                SessionId   = $quserSessionId
                                State       = $quserState
                                Raw         = [string]$line
                            })

                            if (($quserSessionId -gt 0) -and ($quserState -match '^(?i:disc|disconnected)$')) {
                                if (-not $sessionIds.Contains($quserSessionId)) {
                                    $sessionIds.Add($quserSessionId)
                                }
                            }
                        }
                    }

                    if ($quserRows.Count -gt 0) {
                        $result['quser_matches'] = ($quserRows | Sort-Object SessionId | Format-Table -AutoSize | Out-String)
                    }
                }

                $sortedSessionIds = @($sessionIds | Sort-Object -Unique)
                if ($sortedSessionIds.Count -gt 0) {
                    $result['discovered_session_ids'] = ($sortedSessionIds -join ',')

                    $loggedOff = New-Object System.Collections.Generic.List[int]
                    $logoffErrors = New-Object System.Collections.Generic.List[string]

                    foreach ($sid in $sortedSessionIds) {
                        try {
                            logoff $sid 2>$null | Out-Null
                            $loggedOff.Add($sid)
                        } catch {
                            $logoffErrors.Add("session_id=$sid error=$($_.Exception.Message)")
                        }
                    }

                    if ($loggedOff.Count -gt 0) {
                        $result['logged_off_session_ids'] = (($loggedOff | Sort-Object -Unique) -join ',')
                    }

                    if ($logoffErrors.Count -gt 0) {
                        $result['logoff_errors'] = ($logoffErrors | Out-String)
                    }
                }

                $result
            } -ArgumentList $RdpUsername, $RdpDomain, $AggressiveFreshSessionCleanup.IsPresent

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
            if ($cleanup.ContainsKey('explorer_matches')) {
                $freshSessionCleanupExplorerMatches = [string]$cleanup['explorer_matches']
            }
            if ($cleanup.ContainsKey('quser_matches')) {
                $freshSessionCleanupQuserMatches = [string]$cleanup['quser_matches']
            }
            if ($cleanup.ContainsKey('logoff_errors')) {
                $freshSessionCleanupLogoffErrors = [string]$cleanup['logoff_errors']
            }

            $freshSessionCleanupSucceeded = $true

            Write-Host "Fresh-session cleanup target user: $freshSessionCleanupTargetUser"
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
            if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupLogoffErrors)) {
                Write-Warning "Fresh-session cleanup logoff errors:`n$freshSessionCleanupLogoffErrors"
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
} elseif (($Mode -eq 'Provider') -and (-not $SkipScreenshot.IsPresent)) {
    $freshSessionCleanupAggressiveMode = $AggressiveFreshSessionCleanup.IsPresent
}

# ── Step 3: Run screenshot client ───────────────────────────────────────────
if (-not $SkipScreenshot.IsPresent) {
    Write-Host "`n=== Step 3: Running screenshot client against ${Hostname}:${Port} ===" -ForegroundColor Cyan

    $env:IRONRDP_LOG = 'debug'

    $screenshotLog = Join-Path $artifactsDir "screenshot-$timestamp.log"

    Write-Host "Client: $screenshotExe --host $Hostname --port $Port -u $RdpUsername -d $RdpDomain -o $OutputPng"

    $screenshotArgs = @(
        '--host', $Hostname,
        '--port', $Port,
        '-u', $RdpUsername,
        '-p', $rdpPasswordEffective,
        '--autologon', 'true',
        '-o', $OutputPng,
        '--after-first-graphics-seconds', $AfterFirstGraphicsSeconds
    )

    if ($Mode -eq 'Provider') {
        # Provider mode should go through the normal CredSSP/NLA auth path.
        $screenshotArgs += @('--tls-enabled', 'true', '--credssp-enabled', 'true')
    }
    if (-not [string]::IsNullOrWhiteSpace($RdpDomain)) {
        $screenshotArgs += @('-d', $RdpDomain)
    }

    $proc = Start-Process -FilePath $screenshotExe -ArgumentList $screenshotArgs `
        -NoNewWindow -PassThru -RedirectStandardError $screenshotLog

    $exited = $proc.WaitForExit($ScreenshotTimeoutSeconds * 1000)
    if (-not $exited) {
        Write-Warning "Screenshot client timed out after ${ScreenshotTimeoutSeconds}s -- killing"
        $proc.Kill()
        $proc.WaitForExit(5000) | Out-Null
    }

    $exitCode = $proc.ExitCode
    Write-Host "Screenshot client exited with code: $exitCode"

    if (Test-Path $screenshotLog) {
        $logContent = Get-Content $screenshotLog -Raw -ErrorAction SilentlyContinue
        if (-not [string]::IsNullOrWhiteSpace($logContent)) {
            Write-Host "`n---- screenshot client log ----" -ForegroundColor Yellow
            Write-Host $logContent
        }
    }

    if (Test-Path $OutputPng) {
        $fileInfo = Get-Item $OutputPng
        Write-Host "`nScreenshot saved: $OutputPng ($($fileInfo.Length) bytes)" -ForegroundColor Green
    } else {
        Write-Warning "No screenshot PNG was produced at: $OutputPng"
    }
} else {
    Write-Host "`n=== Step 3: Screenshot skipped ===" -ForegroundColor Yellow
}

# ── Step 4: Collect remote logs ─────────────────────────────────────────────
Write-Host "`n=== Step 4: Collecting remote logs ===" -ForegroundColor Cyan

try {
    $session = New-TestVmSession -Hostname $Hostname -Credential $adminCred
    try {
        $remoteLogs = Invoke-Command -Session $session -ScriptBlock {
            param($port, $securityStartTime, $mode, $dumpDir, $rdpUsernameForAudit)
            $result = @{}
            $logOut = 'C:\IronRDPDeploy\logs\ironrdp-termsrv.log'
            $logErr = 'C:\IronRDPDeploy\logs\ironrdp-termsrv.err.log'
            $dllDebugLog = 'C:\IronRDPDeploy\logs\wts-provider-debug.log'
            $stdoutTail = $null

            $result['security_4624_type10_user_count'] = 0
            $result['termsrv_fallback_marker_count'] = 0
            $result['termsrv_fallback_markers'] = ''
            $result['provider_session_proof_marker_count'] = 0
            $result['provider_session_proof_markers'] = ''
            $result['termsrv_session_proof_marker_count'] = 0
            $result['termsrv_session_proof_markers'] = ''
            $result['idd_driver_loaded_notified'] = $false
            $result['idd_wddm_enabled_signal_count'] = 0
            $result['provider_video_handle_marker_count'] = 0
            $result['provider_video_handle_markers'] = ''
            $result['termsrv_idd_marker_count'] = 0
            $result['termsrv_idd_markers'] = ''
            $result['provider_logon_policy_marker_count'] = 0
            $result['provider_logon_policy_markers'] = ''
            $result['remote_connection_signal_count'] = 0
            $result['remote_graphics_signal_count'] = 0
            $result['remote_connection_signals'] = ''
            $result['gui_target_session_id'] = -1
            $result['gui_target_session_source'] = ''
            $result['gui_target_session_resolved'] = $false
            $result['gui_target_session_process_proof'] = $false
            $result['gui_target_session_explorer_count'] = 0
            $result['gui_target_session_gui_process_count'] = 0
            $result['gui_target_session_winlogon_count'] = 0
            $result['gui_target_session_logonui_count'] = 0
            $result['gui_target_session_processes'] = ''
            $result['gui_target_session_explorer_bootstrap_pid'] = 0
            $result['gui_target_session_explorer_bootstrap_time_utc'] = ''
            $result['gui_target_session_explorer_bootstrap_pid_alive'] = $false
            $result['gui_target_session_explorer_earliest_start_utc'] = ''
            $result['gui_target_session_explorer_after_creation_known'] = $false
            $result['gui_target_session_explorer_after_creation'] = $false
            $result['gui_target_session_creation_signal_utc'] = ''
            $result['gui_target_session_creation_signals'] = ''
            $result['gui_target_session_explorer_details'] = ''
            $result['explorer_crash_event_count'] = 0
            $result['explorer_hang_event_count'] = 0
            $result['explorer_wer_event_count'] = 0
            $result['explorer_lifecycle_events'] = ''

            $termsrvSessionProofLines = @()
            $providerSessionProofLines = @()

            $targetUserName = $rdpUsernameForAudit
            if (-not [string]::IsNullOrWhiteSpace($targetUserName)) {
                if ($targetUserName.Contains('\')) {
                    $targetUserName = $targetUserName.Split('\')[-1]
                }
                if ($targetUserName.Contains('@')) {
                    $targetUserName = $targetUserName.Split('@')[0]
                }
            }

            if (Test-Path $logOut) {
                $stdoutTail = Get-Content $logOut -Tail 5000 -ErrorAction SilentlyContinue
                $result['stdout'] = ($stdoutTail | Select-Object -Last 150 | Out-String)
            }
            if (Test-Path $logErr) {
                $result['stderr'] = Get-Content $logErr -Tail 50 -ErrorAction SilentlyContinue | Out-String
            }
            if ($mode -eq 'Provider' -and $stdoutTail) {
                $fallbackPatterns = @(
                    'falling back to guessed session',
                    'sending synthetic test pattern',
                    'falling back to in-process GDI',
                    'prelogon token fallback disabled after initial helper start'
                )
                $fallbackHits = $stdoutTail | Select-String -SimpleMatch -Pattern $fallbackPatterns -ErrorAction SilentlyContinue
                if ($fallbackHits) {
                    $result['termsrv_fallback_marker_count'] = ($fallbackHits | Measure-Object).Count
                    $result['termsrv_fallback_markers'] = ($fallbackHits | Select-Object -Last 20 | ForEach-Object { $_.Line } | Out-String)
                }

                $termsrvSessionProofHits = $stdoutTail | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_TERMSRV_' -ErrorAction SilentlyContinue
                if ($termsrvSessionProofHits) {
                    $termsrvSessionProofLines = $termsrvSessionProofHits | ForEach-Object { $_.Line }
                    $result['termsrv_session_proof_marker_count'] = ($termsrvSessionProofHits | Measure-Object).Count
                    $result['termsrv_session_proof_markers'] = ($termsrvSessionProofLines | Select-Object -Last 40 | Out-String)
                }

                $termsrvShellBootstrapHits = $stdoutTail | Select-String -SimpleMatch -Pattern @(
                    'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_ATTEMPT',
                    'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS',
                    'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_ERROR'
                ) -ErrorAction SilentlyContinue
                if ($termsrvShellBootstrapHits) {
                    $termsrvShellBootstrapLines = $termsrvShellBootstrapHits | ForEach-Object { $_.Line }
                    $existing = [string]$result['termsrv_session_proof_markers']
                    $bootstrap = ($termsrvShellBootstrapLines | Select-Object -Last 40 | Out-String)
                    $result['termsrv_session_proof_markers'] = ($existing + $bootstrap)
                }

                $termsrvIddHits = $stdoutTail | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_TERMSRV_IDD_' -ErrorAction SilentlyContinue
                if ($termsrvIddHits) {
                    $termsrvIddLines = $termsrvIddHits | ForEach-Object { $_.Line }
                    $result['termsrv_idd_marker_count'] = ($termsrvIddHits | Measure-Object).Count
                    $result['termsrv_idd_markers'] = ($termsrvIddLines | Select-Object -Last 40 | Out-String)
                }
            }
            if ($mode -eq 'Provider' -and (Test-Path $dllDebugLog)) {
                # The provider debug log can be extremely noisy due to polling. Capture a signal-focused view.
                $dllTail = Get-Content $dllDebugLog -Tail 5000 -ErrorAction SilentlyContinue
                $patterns = @(
                    'IWRdsProtocolManager::',
                    'IWRdsProtocolListener::',
                    'IWRdsProtocolListenerCallback::',
                    'IWRdsProtocolConnection::',
                    'SESSION_PROOF_PROVIDER_',
                    'SESSION_PROOF_PROVIDER_IDD_NOTIFY_DRIVER_LOAD_ACK',
                    'IWRdsWddmIddProps::',
                    'NotifyIddDriverLoaded',
                    'GetUserCredentials ok',
                    'NotifyCommandProcessCreated',
                    'IsUserAllowedToLogon',
                    'LogonNotify',
                    'SessionArbitrationEnumeration',
                    'DisconnectNotify',
                    'Close called',
                    'PreDisconnect'
                )

                $result['dll_debug'] = ($dllTail | Select-Object -Last 200 | Out-String)
                $matchLines = $dllTail | Select-String -SimpleMatch -Pattern $patterns -ErrorAction SilentlyContinue
                if ($matchLines) {
                    $result['dll_debug_key'] = ($matchLines | Select-Object -Last 400 | ForEach-Object { $_.Line } | Out-String)
                }

                $providerSessionProofHits = $dllTail | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_PROVIDER_' -ErrorAction SilentlyContinue
                if ($providerSessionProofHits) {
                    $providerSessionProofLines = $providerSessionProofHits | ForEach-Object { $_.Line }
                    $result['provider_session_proof_marker_count'] = ($providerSessionProofHits | Measure-Object).Count
                    $result['provider_session_proof_markers'] = ($providerSessionProofLines | Select-Object -Last 40 | Out-String)
                }

                $providerVideoHandleHits = $dllTail | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_PROVIDER_VIDEO_HANDLE_' -ErrorAction SilentlyContinue
                if ($providerVideoHandleHits) {
                    $providerVideoHandleLines = $providerVideoHandleHits | ForEach-Object { $_.Line }
                    $result['provider_video_handle_marker_count'] = ($providerVideoHandleHits | Measure-Object).Count
                    $result['provider_video_handle_markers'] = ($providerVideoHandleLines | Select-Object -Last 40 | Out-String)
                }

                $providerLogonPolicyHits = $dllTail | Select-String -SimpleMatch -Pattern @(
                    'SESSION_PROOF_PROVIDER_SUPPRESS_LOGON_UI',
                    'SESSION_PROOF_PROVIDER_UNIVERSAL_APPS',
                    'SESSION_PROOF_PROVIDER_LOGON_GATE'
                ) -ErrorAction SilentlyContinue
                if ($providerLogonPolicyHits) {
                    $providerLogonPolicyLines = $providerLogonPolicyHits | ForEach-Object { $_.Line }
                    $result['provider_logon_policy_marker_count'] = ($providerLogonPolicyHits | Measure-Object).Count
                    $result['provider_logon_policy_markers'] = ($providerLogonPolicyLines | Select-Object -Last 40 | Out-String)
                }

                $iddLoadHit = $dllTail | Select-String -SimpleMatch -Pattern @(
                    'SESSION_PROOF_PROVIDER_IDD_NOTIFY_DRIVER_LOAD_ACK',
                    'NotifyIddDriverLoaded'
                ) -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($iddLoadHit) {
                    $result['idd_driver_loaded_notified'] = $true
                }

                $iddWddmEnableHits = $dllTail | Select-String -SimpleMatch -Pattern 'IWRdsWddmIddProps::EnableWddmIdd enabled=true' -ErrorAction SilentlyContinue
                if ($iddWddmEnableHits) {
                    $result['idd_wddm_enabled_signal_count'] = ($iddWddmEnableHits | Measure-Object).Count
                }

                $notifyCommandProcessCreatedHits = $dllTail | Select-String -SimpleMatch -Pattern 'IWRdsProtocolConnection::NotifyCommandProcessCreated called' -ErrorAction SilentlyContinue
                if ($notifyCommandProcessCreatedHits) {
                    $result['notify_command_process_created_count'] = ($notifyCommandProcessCreatedHits | Measure-Object).Count
                } else {
                    $result['notify_command_process_created_count'] = 0
                }
            }

            if ($mode -eq 'Provider') {
                $targetSessionId = $null
                $targetSessionSource = ''

                $termsrvBootstrapSuccessLines = $termsrvSessionProofLines | Where-Object {
                    $_ -match 'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS'
                }
                if ($termsrvBootstrapSuccessLines) {
                    $lastBootstrapLine = $termsrvBootstrapSuccessLines | Select-Object -Last 1
                    if ($lastBootstrapLine -match 'explorer_pid=(\d+)') {
                        $result['gui_target_session_explorer_bootstrap_pid'] = [int]$Matches[1]
                    }
                    if ($lastBootstrapLine -match '^(\d{4}-\d{2}-\d{2}T[^\s]+Z)') {
                        $result['gui_target_session_explorer_bootstrap_time_utc'] = [string]$Matches[1]
                    }
                }

                $termsrvProofText = [string]$result['termsrv_session_proof_markers']
                if (-not [string]::IsNullOrWhiteSpace($termsrvProofText)) {
                    $bootstrapPidMatches = [regex]::Matches($termsrvProofText, 'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS[^\r\n]*explorer_pid=(\d+)')
                    if ($bootstrapPidMatches.Count -gt 0) {
                        $lastPidMatch = $bootstrapPidMatches[$bootstrapPidMatches.Count - 1]
                        $result['gui_target_session_explorer_bootstrap_pid'] = [int]$lastPidMatch.Groups[1].Value
                    }

                    if ($result['gui_target_session_explorer_bootstrap_pid'] -le 0) {
                        $genericPidMatches = [regex]::Matches($termsrvProofText, 'explorer_pid=(\d+)')
                        if ($genericPidMatches.Count -gt 0) {
                            $lastGenericPid = $genericPidMatches[$genericPidMatches.Count - 1]
                            $result['gui_target_session_explorer_bootstrap_pid'] = [int]$lastGenericPid.Groups[1].Value
                        }
                    }

                    $bootstrapTimeMatches = [regex]::Matches($termsrvProofText, '(\d{4}-\d{2}-\d{2}T[^\s]+Z)[^\r\n]*SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS')
                    if ($bootstrapTimeMatches.Count -gt 0) {
                        $lastTimeMatch = $bootstrapTimeMatches[$bootstrapTimeMatches.Count - 1]
                        $result['gui_target_session_explorer_bootstrap_time_utc'] = [string]$lastTimeMatch.Groups[1].Value
                    }
                }

                if (($result['gui_target_session_explorer_bootstrap_pid'] -le 0) -and $stdoutTail) {
                    $rawBootstrapHits = $stdoutTail | Select-String -Pattern 'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS.*explorer_pid=(\d+)' -ErrorAction SilentlyContinue
                    if ($rawBootstrapHits) {
                        $lastRawBootstrap = $rawBootstrapHits | Select-Object -Last 1
                        if ($lastRawBootstrap.Matches.Count -gt 0) {
                            $result['gui_target_session_explorer_bootstrap_pid'] = [int]$lastRawBootstrap.Matches[0].Groups[1].Value
                        }
                        if ($lastRawBootstrap.Line -match '^(\d{4}-\d{2}-\d{2}T[^\s]+Z)') {
                            $result['gui_target_session_explorer_bootstrap_time_utc'] = [string]$Matches[1]
                        }
                    }
                }

                $providerAckLines = $providerSessionProofLines | Where-Object {
                    $_ -match 'SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK'
                }

                if ($providerAckLines) {
                    foreach ($line in $providerAckLines) {
                        if ($line -match 'source=([^\s]+)') {
                            $targetSessionSource = [string]$Matches[1]
                        }
                        if ($line -match 'session_id=(\d+)') {
                            $targetSessionId = [int]$Matches[1]
                        }
                    }
                }

                if ($null -eq $targetSessionId) {
                    $termsrvApplyLines = $termsrvSessionProofLines | Where-Object {
                        $_ -match 'SESSION_PROOF_TERMSRV_SET_CAPTURE_SESSION_ID_APPLIED'
                    }

                    if ($termsrvApplyLines) {
                        foreach ($line in $termsrvApplyLines) {
                            if ($line -match 'session_id=(\d+)') {
                                $targetSessionId = [int]$Matches[1]
                            }
                        }

                        if ($null -ne $targetSessionId) {
                            $targetSessionSource = 'termsrv_applied'
                        }
                    }
                }

                if ($null -ne $targetSessionId) {
                    $result['gui_target_session_id'] = $targetSessionId
                    $result['gui_target_session_source'] = $targetSessionSource
                    $result['gui_target_session_resolved'] = $true

                    try {
                        $sessionProcesses = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $targetSessionId }
                        $explorerProcesses = @($sessionProcesses | Where-Object { $_.ProcessName -ieq 'explorer' })

                        $explorerCount = ($explorerProcesses | Measure-Object).Count
                        $guiProcessCount = ($sessionProcesses | Where-Object {
                            $_.ProcessName -in @('explorer', 'dwm', 'ShellExperienceHost', 'sihost')
                        } | Measure-Object).Count
                        $winlogonCount = ($sessionProcesses | Where-Object { $_.ProcessName -ieq 'winlogon' } | Measure-Object).Count
                        $logonUiCount = ($sessionProcesses | Where-Object { $_.ProcessName -ieq 'LogonUI' } | Measure-Object).Count

                        $result['gui_target_session_explorer_count'] = $explorerCount
                        $result['gui_target_session_gui_process_count'] = $guiProcessCount
                        $result['gui_target_session_winlogon_count'] = $winlogonCount
                        $result['gui_target_session_logonui_count'] = $logonUiCount
                        $result['gui_target_session_process_proof'] = ($explorerCount -ge 1)
                        $result['gui_target_session_processes'] = ($sessionProcesses | Select-Object -First 40 Id, ProcessName, SessionId | Sort-Object ProcessName, Id | Format-Table -AutoSize | Out-String)

                        if ($explorerCount -ge 1) {
                            $explorerRows = New-Object System.Collections.Generic.List[object]
                            $explorerEarliestStartUtc = $null

                            foreach ($explorerProc in ($explorerProcesses | Sort-Object Id)) {
                                $startUtc = ''
                                try {
                                    $startLocal = $explorerProc.StartTime
                                    if ($null -ne $startLocal) {
                                        $startUtcDate = $startLocal.ToUniversalTime()
                                        $startUtc = $startUtcDate.ToString('o')
                                        if (($null -eq $explorerEarliestStartUtc) -or ($startUtcDate -lt $explorerEarliestStartUtc)) {
                                            $explorerEarliestStartUtc = $startUtcDate
                                        }
                                    }
                                } catch {
                                    # Best-effort only: process may exit while sampling details.
                                }

                                $cpuSeconds = 0.0
                                try {
                                    if ($null -ne $explorerProc.CPU) {
                                        $cpuSeconds = [double]$explorerProc.CPU
                                    }
                                } catch {
                                    $cpuSeconds = 0.0
                                }

                                $respondingValue = 'n/a'
                                try {
                                    if (($null -ne $explorerProc.MainWindowHandle) -and ($explorerProc.MainWindowHandle -ne 0)) {
                                        $respondingValue = [string]([bool]$explorerProc.Responding)
                                    }
                                } catch {
                                    $respondingValue = 'n/a'
                                }

                                $explorerRows.Add([pscustomobject]@{
                                    Id           = $explorerProc.Id
                                    StartTimeUtc = $startUtc
                                    CpuSec       = [math]::Round($cpuSeconds, 2)
                                    Handles      = $explorerProc.HandleCount
                                    Responding   = $respondingValue
                                })
                            }

                            if ($null -ne $explorerEarliestStartUtc) {
                                $result['gui_target_session_explorer_earliest_start_utc'] = $explorerEarliestStartUtc.ToString('o')
                            }

                            $result['gui_target_session_explorer_details'] = ($explorerRows | Sort-Object StartTimeUtc, Id | Format-Table -AutoSize | Out-String)

                            if ($result['gui_target_session_explorer_bootstrap_pid'] -le 0) {
                                $bootstrapTimeUtcRaw = [string]$result['gui_target_session_explorer_bootstrap_time_utc']
                                if (-not [string]::IsNullOrWhiteSpace($bootstrapTimeUtcRaw)) {
                                    try {
                                        $bootstrapTimeUtc = [datetime]::Parse($bootstrapTimeUtcRaw)
                                        $closestExplorer = $null
                                        $closestDelta = [timespan]::MaxValue

                                        foreach ($row in $explorerRows) {
                                            if ([string]::IsNullOrWhiteSpace([string]$row.StartTimeUtc)) {
                                                continue
                                            }

                                            $rowStartUtc = [datetime]::Parse([string]$row.StartTimeUtc)
                                            $delta = ($rowStartUtc - $bootstrapTimeUtc).Duration()
                                            if ($delta -lt $closestDelta) {
                                                $closestDelta = $delta
                                                $closestExplorer = $row
                                            }
                                        }

                                        if (($null -ne $closestExplorer) -and ($closestDelta.TotalSeconds -le 5)) {
                                            $result['gui_target_session_explorer_bootstrap_pid'] = [int]$closestExplorer.Id
                                        }
                                    } catch {
                                        # Best-effort only.
                                    }
                                }
                            }
                        }

                        $bootstrapExplorerPid = [int]$result['gui_target_session_explorer_bootstrap_pid']
                        if ($bootstrapExplorerPid -gt 0) {
                            $bootstrapExplorerProc = Get-Process -Id $bootstrapExplorerPid -ErrorAction SilentlyContinue
                            $result['gui_target_session_explorer_bootstrap_pid_alive'] = ($null -ne $bootstrapExplorerProc)
                        }

                        try {
                            $start = $securityStartTime.AddMinutes(-2)
                            $maxSessionEvents = 500
                            $targetSessionLifecycleEventIds = @(21, 22, 24, 25)
                            $sessionEvents = Get-WinEvent -FilterHashtable @{ LogName = 'Microsoft-Windows-TerminalServices-LocalSessionManager/Operational'; StartTime = $start } -MaxEvents $maxSessionEvents -ErrorAction SilentlyContinue

                            $sessionRows = New-Object System.Collections.Generic.List[object]
                            foreach ($evt in $sessionEvents) {
                                try {
                                    if ($evt.Id -notin $targetSessionLifecycleEventIds) {
                                        continue
                                    }

                                    $xml = [xml]$evt.ToXml()
                                    $data = @{}
                                    foreach ($d in $xml.Event.EventData.Data) { $data[$d.Name] = [string]$d.'#text' }

                                    $eventSessionId = $null
                                    if ($data.ContainsKey('SessionID') -and ($data.SessionID -match '^\d+$')) {
                                        $eventSessionId = [int]$data.SessionID
                                    } elseif ($data.ContainsKey('SessionId') -and ($data.SessionId -match '^\d+$')) {
                                        $eventSessionId = [int]$data.SessionId
                                    } elseif ($evt.Message -match '(?i)\bsession\s+(\d+)\b') {
                                        $eventSessionId = [int]$Matches[1]
                                    }

                                    if (($null -eq $eventSessionId) -or ($eventSessionId -ne $targetSessionId)) {
                                        continue
                                    }

                                    $sessionRows.Add([pscustomobject]@{
                                        TimeCreated = $evt.TimeCreated
                                        Id          = $evt.Id
                                        Message     = ([string]$evt.Message -replace '\r?\n', ' ')
                                    })
                                } catch {
                                    # Ignore malformed/partial event payloads.
                                }
                            }

                            if ($sessionRows.Count -gt 0) {
                                $orderedSessionRows = @($sessionRows | Sort-Object TimeCreated)
                                $firstSessionRow = $orderedSessionRows | Select-Object -First 1
                                $result['gui_target_session_creation_signal_utc'] = $firstSessionRow.TimeCreated.ToUniversalTime().ToString('o')
                                $result['gui_target_session_creation_signals'] = "Since $start (sampled up to $maxSessionEvents recent LocalSessionManager events; IDs=$($targetSessionLifecycleEventIds -join ','))`n" + ($orderedSessionRows | Select-Object -First 20 TimeCreated, Id, Message | Format-Table -AutoSize | Out-String)

                                $explorerStartUtcRaw = [string]$result['gui_target_session_explorer_earliest_start_utc']
                                if (-not [string]::IsNullOrWhiteSpace($explorerStartUtcRaw)) {
                                    try {
                                        $sessionCreateUtc = [datetime]::Parse([string]$result['gui_target_session_creation_signal_utc'])
                                        $explorerStartUtc = [datetime]::Parse($explorerStartUtcRaw)
                                        $result['gui_target_session_explorer_after_creation_known'] = $true
                                        $result['gui_target_session_explorer_after_creation'] = ($explorerStartUtc -ge $sessionCreateUtc.AddSeconds(-2))
                                    } catch {
                                        $result['gui_target_session_explorer_after_creation_known'] = $false
                                        $result['gui_target_session_explorer_after_creation'] = $false
                                    }
                                }
                            } else {
                                # Fallback for hosts where LocalSessionManager operational events are sparse or unavailable:
                                # use the latest Security 4624 LogonType=10 event for the target user as a session-start proxy.
                                $targetUserForFallback = $targetUserName
                                if (-not [string]::IsNullOrWhiteSpace($targetUserForFallback)) {
                                    $securityRows = New-Object System.Collections.Generic.List[object]
                                    foreach ($evt in (Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 4624; StartTime = $start } -MaxEvents 400 -ErrorAction SilentlyContinue)) {
                                        try {
                                            $xml = [xml]$evt.ToXml()
                                            $data = @{}
                                            foreach ($d in $xml.Event.EventData.Data) { $data[$d.Name] = [string]$d.'#text' }

                                            if (($data.LogonType -eq '10') -and ($data.TargetUserName -eq $targetUserForFallback)) {
                                                $securityRows.Add([pscustomobject]@{
                                                    TimeCreated  = $evt.TimeCreated
                                                    LogonProcess = $data.LogonProcessName
                                                    ProcessName  = $data.ProcessName
                                                    IpAddress    = $data.IpAddress
                                                })
                                            }
                                        } catch {
                                            # Ignore malformed event payloads.
                                        }
                                    }

                                    if ($securityRows.Count -gt 0) {
                                        $latestSecurityRow = @($securityRows | Sort-Object TimeCreated -Descending | Select-Object -First 1)[0]
                                        $result['gui_target_session_creation_signal_utc'] = $latestSecurityRow.TimeCreated.ToUniversalTime().ToString('o')
                                        $result['gui_target_session_creation_signals'] = "Fallback source: Security 4624 LogonType=10 for '$targetUserForFallback' since $start`n" + ($securityRows | Sort-Object TimeCreated -Descending | Select-Object -First 10 TimeCreated, LogonProcess, ProcessName, IpAddress | Format-Table -AutoSize | Out-String)

                                        $explorerStartUtcRaw = [string]$result['gui_target_session_explorer_earliest_start_utc']
                                        if (-not [string]::IsNullOrWhiteSpace($explorerStartUtcRaw)) {
                                            try {
                                                $sessionCreateUtc = [datetime]::Parse([string]$result['gui_target_session_creation_signal_utc'])
                                                $explorerStartUtc = [datetime]::Parse($explorerStartUtcRaw)
                                                $result['gui_target_session_explorer_after_creation_known'] = $true
                                                $result['gui_target_session_explorer_after_creation'] = ($explorerStartUtc -ge $sessionCreateUtc.AddSeconds(-2))
                                            } catch {
                                                $result['gui_target_session_explorer_after_creation_known'] = $false
                                                $result['gui_target_session_explorer_after_creation'] = $false
                                            }
                                        }
                                    }
                                }
                            }
                        } catch {
                            $result['gui_target_session_creation_signals'] = "Could not collect LocalSessionManager session events: $_"
                        }
                    } catch {
                        $result['gui_target_session_processes'] = "Could not enumerate target-session processes: $_"
                    }
                }

                try {
                    $start = $securityStartTime.AddMinutes(-1)
                    $maxExplorerLifecycleEvents = 500
                    $explorerLifecycleEvents = Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $start; Id = @(1000, 1001, 1002) } -MaxEvents $maxExplorerLifecycleEvents -ErrorAction SilentlyContinue |
                        Where-Object { $_.Message -match '(?i)explorer\.exe' }

                    $result['explorer_crash_event_count'] = @($explorerLifecycleEvents | Where-Object { $_.Id -eq 1000 }).Count
                    $result['explorer_wer_event_count'] = @($explorerLifecycleEvents | Where-Object { $_.Id -eq 1001 }).Count
                    $result['explorer_hang_event_count'] = @($explorerLifecycleEvents | Where-Object { $_.Id -eq 1002 }).Count

                    if ($explorerLifecycleEvents) {
                        $rows = $explorerLifecycleEvents |
                            Select-Object -First 20 TimeCreated, Id, ProviderName, Message
                        $result['explorer_lifecycle_events'] = "Since $start (sampled up to $maxExplorerLifecycleEvents recent Application events)`n" + ($rows | Format-Table -AutoSize | Out-String)
                    }
                } catch {
                    $result['explorer_crash_event_count'] = -1
                    $result['explorer_wer_event_count'] = -1
                    $result['explorer_hang_event_count'] = -1
                    $result['explorer_lifecycle_events'] = "Could not collect explorer lifecycle events from Application log: $_"
                }
            }

            if ($mode -eq 'Provider') {
                # Collect Security 4624 LogonType=10 for the configured RDP username since test start.
                try {
                    $start = $securityStartTime.AddMinutes(-1)
                    $maxSecurityEvents = 1000
                    $maxRows = 20
                    $rows = New-Object System.Collections.Generic.List[object]

                    foreach ($evt in (Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 4624; StartTime = $start } -MaxEvents $maxSecurityEvents -ErrorAction SilentlyContinue)) {
                        if ($rows.Count -ge $maxRows) {
                            break
                        }

                        try {
                            $xml = [xml]$evt.ToXml()
                            $data = @{}
                            foreach ($d in $xml.Event.EventData.Data) { $data[$d.Name] = [string]$d.'#text' }

                            $userMatches = [string]::IsNullOrWhiteSpace($targetUserName) -or ($data.TargetUserName -eq $targetUserName)
                            if ($userMatches -and $data.LogonType -eq '10') {
                                $rows.Add([pscustomobject]@{
                                    TimeCreated  = $evt.TimeCreated
                                    Domain       = $data.TargetDomainName
                                    LogonType    = $data.LogonType
                                    LogonProcess = $data.LogonProcessName
                                    ProcessName  = $data.ProcessName
                                    IpAddress    = $data.IpAddress
                                })
                            }
                        } catch {
                            # Ignore malformed event XML and continue sampling recent events.
                        }
                    }

                    if ($rows.Count -gt 0) {
                        $result['security_4624_type10_user_count'] = $rows.Count
                        $result['security_4624_type10_user'] = "Since $start (sampled up to $maxSecurityEvents recent 4624 events)`n" + ($rows | Format-Table -AutoSize | Out-String)
                    } else {
                        $result['security_4624_type10_user_count'] = 0
                        $result['security_4624_type10_user'] = "Since $start (sampled up to $maxSecurityEvents recent 4624 events)`n(no matching 4624 LogonType=10 events for user '$targetUserName')"
                    }
                } catch {
                    $result['security_4624_type10_user_count'] = -1
                    $result['security_4624_type10_user'] = "Could not collect Security 4624: $_"
                }
            }

            if ($mode -eq 'Provider') {
                # Collect recent TermService event log entries
                try {
                    $events = Get-WinEvent -LogName 'System' -MaxEvents 20 -ErrorAction SilentlyContinue |
                        Where-Object { $_.ProviderName -match 'TermService|TermDD|Remote Desktop' -or $_.Id -in @(1058, 1088, 1096, 1149, 22) }
                    $result['termservice_events'] = ($events | Select-Object TimeCreated, Id, LevelDisplayName, Message | Out-String)
                } catch {
                    $result['termservice_events'] = "Could not collect TermService events: $_"
                }

                # Collect authoritative RemoteConnectionManager operational signals.
                # 261: listener accepted a connection
                # 263: WDDM graphics mode enabled for the remote connection
                try {
                    $start = $securityStartTime.AddMinutes(-1)
                    $maxRemoteEvents = 200
                    $remoteEvents = Get-WinEvent -FilterHashtable @{ LogName = 'Microsoft-Windows-TerminalServices-RemoteConnectionManager/Operational'; StartTime = $start } -MaxEvents $maxRemoteEvents -ErrorAction SilentlyContinue

                    if ($remoteEvents) {
                        $connectionSignals = $remoteEvents | Where-Object { $_.Id -eq 261 }
                        $graphicsSignals = $remoteEvents | Where-Object { $_.Id -eq 263 }

                        $result['remote_connection_signal_count'] = ($connectionSignals | Measure-Object).Count
                        $result['remote_graphics_signal_count'] = ($graphicsSignals | Measure-Object).Count

                        $signalRows = $remoteEvents |
                            Where-Object { $_.Id -in @(261, 263, 1149, 20523) } |
                            Select-Object -First 40 TimeCreated, Id, LevelDisplayName, Message

                        if ($signalRows) {
                            $result['remote_connection_signals'] = "Since $start (sampled up to $maxRemoteEvents recent RemoteConnectionManager events)`n" + ($signalRows | Format-Table -AutoSize | Out-String)
                        } else {
                            $result['remote_connection_signals'] = "Since $start (sampled up to $maxRemoteEvents recent RemoteConnectionManager events)`n(no targeted IDs observed: 261, 263, 1149, 20523)"
                        }
                    } else {
                        $result['remote_connection_signals'] = "Since $start (sampled up to $maxRemoteEvents recent RemoteConnectionManager events)`n(no events returned)"
                    }
                } catch {
                    $result['remote_connection_signal_count'] = -1
                    $result['remote_graphics_signal_count'] = -1
                    $result['remote_connection_signals'] = "Could not collect RemoteConnectionManager operational events: $_"
                }

                # Collect activation state because notification mode can block shell readiness.
                try {
                    $windowsAppId = '{55c92734-d682-4d71-983e-d6ec3f16059f}'
                    $license = Get-CimInstance -ClassName SoftwareLicensingProduct -ErrorAction SilentlyContinue |
                        Where-Object { $_.ApplicationID -eq $windowsAppId -and $_.PartialProductKey } |
                        Select-Object -First 1 LicenseStatus, LicenseStatusReason

                    if ($null -ne $license) {
                        $status = [int]$license.LicenseStatus
                        $result['activation_license_status'] = $status

                        $reasonRaw = $license.LicenseStatusReason
                        if ($null -ne $reasonRaw) {
                            $reasonInt = [int64]$reasonRaw
                            if ($reasonInt -lt 0) {
                                $reasonInt = $reasonInt -band 0xFFFFFFFF
                            }
                            $result['activation_license_status_reason_hex'] = ('0x{0:X8}' -f $reasonInt)
                        }

                        # LicenseStatus=5 is notification mode.
                        $result['activation_notification_mode'] = ($status -eq 5)
                    }
                } catch {
                    $result['activation_license_status'] = -1
                    $result['activation_license_status_reason_hex'] = ''
                    $result['activation_notification_mode'] = $false
                }
            }

            $proc = Get-Process -Name 'ironrdp-termsrv' -ErrorAction SilentlyContinue
            $result['running'] = ($null -ne $proc)
            if ($null -ne $proc) {
                $result['pid'] = $proc.Id
            }

            try {
                $listeners = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
                $result['listening'] = ($listeners | Measure-Object).Count -gt 0
            } catch {
                $result['listening'] = $false
            }

            $result
        } -ArgumentList $Port, $testStartTime, $Mode, $dumpRemoteDir, $RdpUsername

        $remoteLogCollectionSucceeded = $true

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

        Write-Host "Service running: $isRunning$(if ($isRunning) { " (PID $($remoteLogs['pid']))" })"
        Write-Host "Port $Port listening: $isListening"

        $remoteLogDir = Join-Path $artifactsDir "remote-logs-$timestamp"
        New-Item -ItemType Directory -Path $remoteLogDir -Force | Out-Null

        if ($remoteLogs['stdout']) {
            $remoteLogs['stdout'] | Set-Content (Join-Path $remoteLogDir 'ironrdp-termsrv.log')
            Write-Host "`n---- ironrdp-termsrv.log (tail) ----" -ForegroundColor Yellow
            Write-Host $remoteLogs['stdout']
        }
        if ($remoteLogs['stderr']) {
            $remoteLogs['stderr'] | Set-Content (Join-Path $remoteLogDir 'ironrdp-termsrv.err.log')
            Write-Host "`n---- ironrdp-termsrv.err.log (tail) ----" -ForegroundColor Yellow
            Write-Host $remoteLogs['stderr']
        }
        if ($remoteLogs['dll_debug']) {
            $remoteLogs['dll_debug'] | Set-Content (Join-Path $remoteLogDir 'wts-provider-debug.log')
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
            Write-Host "IDD diagnostics signals: NotifyIddDriverLoaded=$iddDriverLoadedNotified ProviderEnableWddmIdd=$iddWddmEnabledSignalCount ProviderVideoHandleMarkers=$providerVideoHandleMarkerCount TermsrvIddMarkers=$termsrvIddMarkerCount ProviderLogonPolicyMarkers=$providerLogonPolicyMarkerCount"
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
    }
    finally {
        Remove-PSSession -Session $session
    }
} catch {
    Write-Warning "Failed to collect remote logs: $_"
    if ($StrictSessionProof.IsPresent) {
        throw
    }
}

# ── Summary ─────────────────────────────────────────────────────────────────
Write-Host "`n=== Summary ===" -ForegroundColor Cyan
Write-Host "Mode:       $Mode"
Write-Host "VM:         $Hostname"
Write-Host "Port:       $Port"
Write-Host "Strict:     $($StrictSessionProof.IsPresent)"
Write-Host "Screenshot: $(if (Test-Path $OutputPng) { "$OutputPng ($($(Get-Item $OutputPng).Length) bytes)" } else { 'NOT PRODUCED' })"

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

    Write-Host "Fresh cleanup: status=$cleanupStatus aggressive=$freshSessionCleanupAggressiveMode target_user=$(if ([string]::IsNullOrWhiteSpace($freshSessionCleanupTargetUser)) { 'n/a' } else { $freshSessionCleanupTargetUser }) discovered=$(if ([string]::IsNullOrWhiteSpace($freshSessionCleanupDiscoveredSessionIds)) { 'none' } else { $freshSessionCleanupDiscoveredSessionIds }) logged_off=$(if ([string]::IsNullOrWhiteSpace($freshSessionCleanupLoggedOffSessionIds)) { 'none' } else { $freshSessionCleanupLoggedOffSessionIds })"

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupExplorerMatches)) {
        $freshSessionCleanupExplorerMatches | Set-Content (Join-Path $artifactsDir 'fresh-session-cleanup-explorer-matches.log')
    }

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupQuserMatches)) {
        $freshSessionCleanupQuserMatches | Set-Content (Join-Path $artifactsDir 'fresh-session-cleanup-quser-matches.log')
    }

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupLogoffErrors)) {
        $freshSessionCleanupLogoffErrors | Set-Content (Join-Path $artifactsDir 'fresh-session-cleanup-logoff-errors.log')
    }

    if (-not [string]::IsNullOrWhiteSpace($freshSessionCleanupError)) {
        Write-Warning "Fresh-session cleanup error: $freshSessionCleanupError"
    }
}

$screenshotExists = Test-Path $OutputPng
$screenshotFileInfo = $null
$hasMeaningfulContent = $false
$analysisSucceeded = $false
$analysisMessage = ''

if ($screenshotExists) {
    $screenshotFileInfo = Get-Item $OutputPng

    try {
        Add-Type -AssemblyName System.Drawing -ErrorAction Stop

        $bitmap = [System.Drawing.Bitmap]::new($OutputPng)
        try {
            $stepX = [Math]::Max([int]($bitmap.Width / 256), 1)
            $stepY = [Math]::Max([int]($bitmap.Height / 256), 1)

            $sampleCount = 0
            $hasNonBlackPixel = $false
            $isUniform = $true

            $firstPixelSet = $false
            $firstR = 0
            $firstG = 0
            $firstB = 0

            for ($y = 0; $y -lt $bitmap.Height; $y += $stepY) {
                for ($x = 0; $x -lt $bitmap.Width; $x += $stepX) {
                    $pixel = $bitmap.GetPixel($x, $y)
                    $sampleCount++

                    if (-not $firstPixelSet) {
                        $firstR = $pixel.R
                        $firstG = $pixel.G
                        $firstB = $pixel.B
                        $firstPixelSet = $true
                    } elseif ($pixel.R -ne $firstR -or $pixel.G -ne $firstG -or $pixel.B -ne $firstB) {
                        $isUniform = $false
                    }

                    if ($pixel.R -ne 0 -or $pixel.G -ne 0 -or $pixel.B -ne 0) {
                        $hasNonBlackPixel = $true
                    }
                }
            }

            $hasMeaningfulContent = $hasNonBlackPixel -or (-not $isUniform)
            $analysisSucceeded = $true
            $analysisMessage = "sampled_pixels=$sampleCount uniform_rgb=$isUniform non_black_rgb=$hasNonBlackPixel"
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
} else {
    Write-Host "RESULT: FAIL (no screenshot produced)" -ForegroundColor Red
}

if ($StrictSessionProof.IsPresent) {
    $strictFailures = New-Object System.Collections.Generic.List[string]

    if (-not $screenshotExists) {
        $strictFailures.Add('screenshot not produced')
    } elseif ($screenshotFileInfo.Length -le 1000) {
        $strictFailures.Add("screenshot too small ($($screenshotFileInfo.Length) bytes)")
    } elseif (-not $analysisSucceeded) {
        $strictFailures.Add('screenshot analysis failed')
    } elseif (-not $hasMeaningfulContent) {
        $strictFailures.Add('screenshot content is blank or uniform')
    }

    if (-not $remoteLogCollectionSucceeded) {
        $strictFailures.Add('remote log collection failed')
    }

    if ($Mode -eq 'Provider') {
        $hasSecurityType10Signal = $false
        if (($null -ne $securityLogonType10Count) -and ($securityLogonType10Count -ge 1)) {
            $hasSecurityType10Signal = $true
        }

        if (-not $hasSecurityType10Signal) {
            $strictFailures.Add("mandatory logon proof missing: Security 4624 LogonType=10 not observed for configured user '$RdpUsername' (count=$securityLogonType10Count)")
        }

        if ($null -eq $termsrvFallbackMarkerCount) {
            $strictFailures.Add('termsrv fallback marker count was not collected')
        } elseif ($termsrvFallbackMarkerCount -gt 0) {
            $strictFailures.Add("termsrv fallback markers detected (count=$termsrvFallbackMarkerCount)")
        }

        if ($null -eq $providerSessionProofMarkerCount) {
            $strictFailures.Add('provider session proof marker count was not collected')
        } elseif ($providerSessionProofMarkerCount -lt 1) {
            $strictFailures.Add('provider session proof markers were not observed')
        }

        if ($null -eq $notifyCommandProcessCreatedCount) {
            $strictFailures.Add('shell transition marker count was not collected (NotifyCommandProcessCreated)')
        } elseif ($notifyCommandProcessCreatedCount -lt 1) {
            $strictFailures.Add('shell transition marker missing: NotifyCommandProcessCreated was not observed')
        }

        if ($null -eq $termsrvSessionProofMarkerCount) {
            $strictFailures.Add('termsrv session proof marker count was not collected')
        } elseif ($termsrvSessionProofMarkerCount -lt 1) {
            $strictFailures.Add('termsrv session proof markers were not observed')
        }

        if (-not $guiTargetSessionResolved) {
            $strictFailures.Add('GUI session gate missing: could not resolve target capture session id from provider/termsrv markers')
        } elseif (-not $guiTargetSessionProcessProof) {
            $strictFailures.Add("GUI session gate failed: explorer.exe not observed in target session $guiTargetSessionId (source=$guiTargetSessionSource, explorer=$guiTargetSessionExplorerCount, gui_processes=$guiTargetSessionGuiProcessCount, winlogon=$guiTargetSessionWinlogonCount, logonui=$guiTargetSessionLogonUiCount)")
        }

        if ($bitmapDumpCount -lt 1) {
            $strictFailures.Add('no bitmap dumps were downloaded')
        } elseif ($guiTargetSessionResolved -and (-not $bitmapTargetSessionHasGraphics)) {
            $observedBitmapSessions = if ($bitmapObservedSessionIds.Count -gt 0) { $bitmapObservedSessionIds -join ',' } else { 'none' }
            $strictFailures.Add("graphics-session proof missing: no bitmap dump matched target session id $guiTargetSessionId (observed_bitmap_sessions=$observedBitmapSessions)")
        }

        if (-not $type10GraphicsSessionConfirmed) {
            $strictFailures.Add("mandatory type10+graphics proof missing: Security4624Type10=$securityLogonType10Count target_session=$guiTargetSessionId bitmap_matches=$bitmapTargetSessionMatchCount")
        }

        if ($null -eq $providerVideoHandleMarkerCount) {
            $strictFailures.Add('IDD provider video-handle marker count was not collected')
        }

        if ($null -eq $termsrvIddMarkerCount) {
            $strictFailures.Add('IDD termsrv marker count was not collected')
        }

        if ($null -eq $providerLogonPolicyMarkerCount) {
            $strictFailures.Add('provider logon-policy marker count was not collected')
        }

        $hasIddDiagnosticsSignal = $iddDriverLoadedNotified -or (($null -ne $iddWddmEnabledSignalCount) -and ($iddWddmEnabledSignalCount -ge 1)) -or (($null -ne $remoteGraphicsSignalCount) -and ($remoteGraphicsSignalCount -ge 1)) -or (($null -ne $providerVideoHandleMarkerCount) -and ($providerVideoHandleMarkerCount -ge 1)) -or (($null -ne $termsrvIddMarkerCount) -and ($termsrvIddMarkerCount -ge 1))
        if (-not $hasIddDiagnosticsSignal) {
            $strictFailures.Add("IDD diagnostics gate missing (NotifyIddDriverLoaded=$iddDriverLoadedNotified, ProviderEnableWddmIdd=$iddWddmEnabledSignalCount, ProviderVideoHandleMarkers=$providerVideoHandleMarkerCount, TermsrvIddMarkers=$termsrvIddMarkerCount, RemoteGraphics263=$remoteGraphicsSignalCount)")
        }

        if ($activationNotificationMode) {
            $strictFailures.Add("environment blocker detected: Windows activation notification mode is enabled (LicenseStatus=$activationLicenseStatus, Reason=$activationLicenseStatusReasonHex)")
        }
    }

    if ($strictFailures.Count -gt 0) {
        Write-Host "STRICT RESULT: FAIL" -ForegroundColor Red
        foreach ($failure in $strictFailures) {
            Write-Host "  - $failure" -ForegroundColor Red
        }

        throw "strict session proof failed with $($strictFailures.Count) issue(s)"
    }

    Write-Host "STRICT RESULT: PASS" -ForegroundColor Green
}
