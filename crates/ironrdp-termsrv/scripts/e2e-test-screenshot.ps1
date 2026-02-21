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
    [string]$Hostname = 'IT-HELP-TEST',

    [Parameter()]
    [int]$Port = 4489,

    [Parameter()]
    [string]$AdminUsername = 'IT-HELP\Administrator',

    [Parameter()]
    [string]$AdminPassword = 'DevoLabs123!',

    [Parameter()]
    [string]$RdpUsername = 'Administrator',

    [Parameter()]
    [string]$RdpPassword = 'DevoLabs123!',

    [Parameter()]
    [string]$RdpDomain = 'ad.it-help.ninja',

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
    [int]$ScreenshotTimeoutSeconds = 30
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$workspaceRoot = (Resolve-Path (Join-Path $scriptRoot '..\..\..')).Path
$artifactsDir = Join-Path $workspaceRoot 'artifacts'
New-Item -ItemType Directory -Path $artifactsDir -Force | Out-Null

$timestamp = Get-Date -Format 'yyyyMMdd-HHmmss'
if ([string]::IsNullOrWhiteSpace($OutputPng)) {
    $OutputPng = Join-Path $artifactsDir "screenshot-$timestamp.png"
}

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
        Username         = $AdminUsername
        PasswordPlainText = $AdminPassword
        RdpUsername      = $RdpUsername
        RdpPassword      = $RdpPassword
        RdpDomain        = $RdpDomain
        Configuration    = $Configuration
        SkipBuild        = $true
        ListenerAddr     = "0.0.0.0:$Port"
        CaptureIpc       = 'tcp'
    }

    if ($Mode -eq 'Standalone') {
        $deployArgs.AutoListen = $true
    }

    & $deployScript @deployArgs

    if ($Mode -eq 'Provider') {
        Write-Host "Installing side-by-side WTS provider on $Hostname..." -ForegroundColor Cyan

        $securePwd = ConvertTo-SecureString -String $AdminPassword -AsPlainText -Force
        $cred = [pscredential]::new($AdminUsername, $securePwd)
        $session = New-PSSession -ComputerName $Hostname -Credential $cred
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
        '-p', $RdpPassword,
        '-o', $OutputPng
    )
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

$securePwd = ConvertTo-SecureString -String $AdminPassword -AsPlainText -Force
$cred = [pscredential]::new($AdminUsername, $securePwd)

try {
    $session = New-PSSession -ComputerName $Hostname -Credential $cred
    try {
        $remoteLogs = Invoke-Command -Session $session -ScriptBlock {
            $result = @{}
            $logOut = 'C:\IronRDPDeploy\logs\ironrdp-termsrv.log'
            $logErr = 'C:\IronRDPDeploy\logs\ironrdp-termsrv.err.log'

            if (Test-Path $logOut) {
                $result['stdout'] = Get-Content $logOut -Tail 100 -ErrorAction SilentlyContinue | Out-String
            }
            if (Test-Path $logErr) {
                $result['stderr'] = Get-Content $logErr -Tail 100 -ErrorAction SilentlyContinue | Out-String
            }

            $proc = Get-Process -Name 'ironrdp-termsrv' -ErrorAction SilentlyContinue
            $result['running'] = ($null -ne $proc)
            if ($null -ne $proc) {
                $result['pid'] = $proc.Id
            }

            try {
                $listeners = Get-NetTCPConnection -State Listen -LocalPort 4489 -ErrorAction SilentlyContinue
                $result['listening'] = ($listeners | Measure-Object).Count -gt 0
            } catch {
                $result['listening'] = $false
            }

            $result
        }

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
    if ($fileInfo.Length -gt 1000) {
        Write-Host "RESULT: PASS (screenshot produced with content)" -ForegroundColor Green
    } else {
        Write-Host "RESULT: WARN (screenshot produced but suspiciously small: $($fileInfo.Length) bytes)" -ForegroundColor Yellow
    }
} else {
    Write-Host "RESULT: FAIL (no screenshot produced)" -ForegroundColor Red
}
