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

.EXAMPLE
    .\e2e-test-screenshot.ps1 -Mode Standalone
    .\e2e-test-screenshot.ps1 -Mode Provider
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
    [int]$AfterFirstGraphicsSeconds = 20
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
                Stop-Service -Name TermService -Force -ErrorAction SilentlyContinue
                Start-Sleep -Seconds 2
            }

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
                    -RestartTermService

                & $firewallScript -Mode Add -PortNumber $Port

                & $waitScript -PortNumber $Port -TimeoutSeconds 90
            } -ArgumentList $remoteProviderDir, $Port
        }
        finally {
            Remove-PSSession -Session $session
        }
    }

    Write-Host "Deploy succeeded" -ForegroundColor Green
} else {
    Write-Host "`n=== Step 2: Deploy skipped ===" -ForegroundColor Yellow
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
        # Provider mode uses TLS-only to capture plaintext credentials from ClientInfo.
        # CredSSP/NLA will be rejected (no Hybrid advertised).
        $screenshotArgs += @('--tls-enabled', 'true', '--credssp-enabled', 'false')
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
            param($port, $securityStartTime, $mode, $dumpDir)
            $result = @{}
            $logOut = 'C:\IronRDPDeploy\logs\ironrdp-termsrv.log'
            $logErr = 'C:\IronRDPDeploy\logs\ironrdp-termsrv.err.log'
            $dllDebugLog = 'C:\IronRDPDeploy\logs\wts-provider-debug.log'

            if (Test-Path $logOut) {
                $result['stdout'] = Get-Content $logOut -Tail 150 -ErrorAction SilentlyContinue | Out-String
            }
            if (Test-Path $logErr) {
                $result['stderr'] = Get-Content $logErr -Tail 50 -ErrorAction SilentlyContinue | Out-String
            }
            if ($mode -eq 'Provider' -and (Test-Path $dllDebugLog)) {
                # The provider debug log can be extremely noisy due to polling. Capture a signal-focused view.
                $dllTail = Get-Content $dllDebugLog -Tail 5000 -ErrorAction SilentlyContinue
                $patterns = @(
                    'IWRdsProtocolManager::',
                    'IWRdsProtocolListener::',
                    'IWRdsProtocolListenerCallback::',
                    'IWRdsProtocolConnection::',
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
            }

            if ($mode -eq 'Provider') {
                # Collect Security 4624 for Administrator (Type 2 vs Type 10) since test start
                try {
                    $start = $securityStartTime.AddMinutes(-1)
                    $rows = Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 4624; StartTime = $start } -ErrorAction SilentlyContinue |
                        ForEach-Object {
                            $xml = [xml]$_.ToXml()
                            $data = @{}
                            foreach ($d in $xml.Event.EventData.Data) { $data[$d.Name] = [string]$d.'#text' }

                            if ($data.TargetUserName -eq 'Administrator' -and ($data.LogonType -in '2', '10')) {
                                [pscustomobject]@{
                                    TimeCreated  = $_.TimeCreated
                                    Domain       = $data.TargetDomainName
                                    LogonType    = $data.LogonType
                                    LogonProcess = $data.LogonProcessName
                                    ProcessName  = $data.ProcessName
                                    IpAddress    = $data.IpAddress
                                }
                            }
                        } |
                        Sort-Object TimeCreated -Descending |
                        Select-Object -First 20

                    if ($rows) {
                        $result['security_4624_admin'] = "Since $start`n" + ($rows | Format-Table -AutoSize | Out-String)
                    } else {
                        $result['security_4624_admin'] = "Since $start`n(no matching 4624 events for Administrator with LogonType 2/10)"
                    }
                } catch {
                    $result['security_4624_admin'] = "Could not collect Security 4624: $_"
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
        } -ArgumentList $Port, $testStartTime, $Mode, $dumpRemoteDir

        $isRunning = $remoteLogs['running']
        $isListening = $remoteLogs['listening']
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

        if ($remoteLogs['security_4624_admin']) {
            $remoteLogs['security_4624_admin'] | Set-Content (Join-Path $remoteLogDir 'security-4624-admin.log')
            Write-Host "`n---- Security 4624 (Administrator Type 2/10) ----" -ForegroundColor Yellow
            Write-Host $remoteLogs['security_4624_admin']
        }
        if ($remoteLogs['termservice_events']) {
            Write-Host "`n---- TermService event log (recent) ----" -ForegroundColor Yellow
            Write-Host $remoteLogs['termservice_events']
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
                $bmps = Get-ChildItem -LiteralPath $dumpLocalDir -Filter '*.bmp' -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending
                if ($bmps -and $bmps.Count -gt 0) {
                    $latest = $bmps | Select-Object -First 1
                    Copy-Item -LiteralPath $latest.FullName -Destination (Join-Path $artifactsDir 'latest-bitmap-dump.bmp') -Force
                    Write-Host "`nBitmap dumps: $($bmps.Count) file(s) downloaded to $dumpLocalDir" -ForegroundColor Green
                    Write-Host "Latest dump: $($latest.Name) ($([math]::Round($latest.Length / 1MB, 2)) MB)" -ForegroundColor Green
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
}

# ── Summary ─────────────────────────────────────────────────────────────────
Write-Host "`n=== Summary ===" -ForegroundColor Cyan
Write-Host "Mode:       $Mode"
Write-Host "VM:         $Hostname"
Write-Host "Port:       $Port"
Write-Host "Screenshot: $(if (Test-Path $OutputPng) { "$OutputPng ($($(Get-Item $OutputPng).Length) bytes)" } else { 'NOT PRODUCED' })"

if (Test-Path $OutputPng) {
    $fileInfo = Get-Item $OutputPng
    $hasMeaningfulContent = $false
    $analysisSucceeded = $false
    $analysisMessage = ''

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

    if ($fileInfo.Length -le 1000) {
        Write-Host "RESULT: WARN (screenshot produced but suspiciously small: $($fileInfo.Length) bytes)" -ForegroundColor Yellow
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
