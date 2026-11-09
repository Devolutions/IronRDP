[CmdletBinding()]
param(
    [Parameter()]
    [string]$Hostname = $env:IRONRDP_TEST_VM_HOSTNAME,

    [Parameter()]
    [string]$Username = $env:RDP_USERNAME,

    [Parameter()]
    [securestring]$Password,

    [Parameter()]
    [string]$PasswordPlainText = '',

    [Parameter()]
    [switch]$PromptPassword,

    [Parameter()]
    [string]$PasswordEnvVar = 'RDP_PASSWORD',

    [Parameter()]
    [string]$RdpUsername = $env:RDP_USERNAME,

    [Parameter()]
    [string]$RdpPassword = '',

    [Parameter()]
    [string]$RdpPasswordEnvVar = 'RDP_PASSWORD',

    [Parameter()]
    [string]$RdpDomain = '',

    [Parameter()]
    [string]$RemoteRoot = $(if ($env:IRONRDP_DEPLOY_ROOT) { $env:IRONRDP_DEPLOY_ROOT } else { 'C:\IronRDPDeploy' }),

    [Parameter()]
    [string]$IddRemotePath = $(if ($env:IRONRDP_IDD_REMOTE_PATH) { $env:IRONRDP_IDD_REMOTE_PATH } else { 'C:\Program Files\IronRDP\idd' }),

    [Parameter()]
    [string]$TestCertificatesDirectory = '',

    [Parameter()]
    [string]$TaskName = 'IronRdpTermSrvSystem',

    [Parameter()]
    [string]$ListenerName = 'IRDP-Tcp',

    [Parameter()]
    [string]$ListenerAddr = '0.0.0.0:4489',

    [Parameter()]
    [ValidateSet('tcp', 'shm', 'sharedmem', 'shared-memory')]
    [string]$CaptureIpc = 'tcp',

    [Parameter()]
    [switch]$AutoListen,

    [Parameter()]
    [switch]$WtsProvider,

    [Parameter()]
    [switch]$InstallIdd,

    [Parameter()]
    [switch]$Clipboard,

    [Parameter()]
    [switch]$RemoteApp,

    [Parameter()]
    [switch]$AutoSendSas,

    [Parameter()]
    [string]$CaptureSessionId = '',

    [Parameter()]
    [string]$DumpBitmapUpdatesDir = '',

    [Parameter()]
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',

    [Parameter()]
    [switch]$SkipBuild,

    # Keep TermService stopped after staging all artifacts when the caller owns its eventual start.
    [Parameter()]
    [switch]$NoTermServiceStart,

    [Parameter()]
    [int]$TailLines = 80,

    [Parameter()]
    [ValidateRange(5, 600)]
    [int]$TermServiceStopTimeoutSeconds = 60
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$testVmModule = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..\scripts\TestVm.psm1')).Path
Import-Module $testVmModule -Force

if ($RemoteApp.IsPresent -and -not $WtsProvider.IsPresent) {
    throw '-RemoteApp requires -WtsProvider'
}

if ($InstallIdd.IsPresent -and -not $WtsProvider.IsPresent) {
    throw '-InstallIdd requires -WtsProvider'
}

function Resolve-WorkspaceRoot {
    $here = $PSScriptRoot
    return (Get-Item -LiteralPath (Join-Path $here '..\..\..\..\..')).FullName
}

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

$workspaceRoot = Resolve-WorkspaceRoot
$cargoTargetDirectory = Get-CargoBuildOutputDirectory -WorkspaceRoot $workspaceRoot
$profileDir = if ($Configuration -eq 'Release') { 'release' } else { 'debug' }
$exeLocal = Join-Path $cargoTargetDirectory "$profileDir\ironrdp-termsrv.exe"
$providerDllLocal = Join-Path $cargoTargetDirectory "$profileDir\ironrdp_wtsprotocol_provider.dll"
$iddDllLocal = Join-Path $cargoTargetDirectory "$profileDir\ironrdp_idd.dll"

if (-not $Password -and -not [string]::IsNullOrWhiteSpace($PasswordPlainText)) {
    Write-Warning '-PasswordPlainText is passed as plaintext; prefer -Password or the configured environment variable.'
    $Password = ConvertTo-SecureString $PasswordPlainText -AsPlainText -Force
}

if ($InstallIdd.IsPresent) {
    $null = Get-Command psign-tool -ErrorAction Stop
    $kitRoot = if ($env:IRONRDP_WINDOWS_KITS_ROOT) {
        $env:IRONRDP_WINDOWS_KITS_ROOT
    } else {
        'C:\Program Files (x86)\Windows Kits\10'
    }
    $findWdkTools = Join-Path $workspaceRoot 'server\termsrv\crates\ironrdp-idd\scripts\find-wdk-tools.ps1'
    $wdkTools = & $findWdkTools -WindowsKitsRoot $kitRoot | ConvertFrom-Json
    if (@($wdkTools.Inf2Cat).Count -eq 0) {
        throw "Inf2Cat.exe not found under '$kitRoot'; install the Windows Driver Kit"
    }
    if (@($wdkTools.IddCxLib).Count -eq 0 -and @($wdkTools.IddCxStubLib).Count -eq 0) {
        throw "IddCx import libraries not found under '$kitRoot'; install the Windows Driver Kit"
    }
}

if (-not $SkipBuild.IsPresent) {
    Push-Location $workspaceRoot
    try {
        if ($Configuration -eq 'Release') {
            cargo build -p ironrdp-termsrv --release
        } else {
            cargo build -p ironrdp-termsrv
        }
        if ($LASTEXITCODE -ne 0) { throw "Companion build failed (exit $LASTEXITCODE)" }

        if ($WtsProvider.IsPresent) {
            if ($Configuration -eq 'Release') {
                cargo build -p ironrdp-wtsprotocol-provider --release
            } else {
                cargo build -p ironrdp-wtsprotocol-provider
            }
            if ($LASTEXITCODE -ne 0) { throw "WTS provider build failed (exit $LASTEXITCODE)" }
        }

        if ($InstallIdd.IsPresent) {
            $previousIronRdpIddLink = $env:IRONRDP_IDD_LINK
            try {
                $env:IRONRDP_IDD_LINK = '1'
                if ($Configuration -eq 'Release') {
                    cargo build -p ironrdp-idd --release
                } else {
                    cargo build -p ironrdp-idd
                }
                if ($LASTEXITCODE -ne 0) { throw "IDD build failed (exit $LASTEXITCODE)" }
            }
            finally {
                if ([string]::IsNullOrWhiteSpace($previousIronRdpIddLink)) {
                    Remove-Item Env:IRONRDP_IDD_LINK -ErrorAction SilentlyContinue
                } else {
                    $env:IRONRDP_IDD_LINK = $previousIronRdpIddLink
                }
            }
        }
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $exeLocal -PathType Leaf)) {
    throw "Service executable not found: $exeLocal"
}

if ($WtsProvider.IsPresent -and -not (Test-Path -LiteralPath $providerDllLocal -PathType Leaf)) {
    throw "WTS provider DLL not found: $providerDllLocal"
}

if ($InstallIdd.IsPresent) {
    if (-not (Test-Path -LiteralPath $iddDllLocal -PathType Leaf)) {
        throw "IDD DLL not found: $iddDllLocal"
    }

    $iddSignScript = Join-Path $workspaceRoot 'server\termsrv\crates\ironrdp-idd\scripts\sign-driver.ps1'
    $iddArtifactRoot = Join-Path $cargoTargetDirectory 'termsrv\idd'
    & $iddSignScript -DriverPath $iddDllLocal -OutputDirectory $iddArtifactRoot `
        -TestCertificatesDirectory $TestCertificatesDirectory

    foreach ($artifactName in 'IronRdpIdd.dll', 'IronRdpIdd.inf', 'IronRdpIdd.cat') {
        $artifactPath = Join-Path $iddArtifactRoot $artifactName
        if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
            throw "Signed IDD package artifact not found: $artifactPath"
        }
    }
}

$cred = if ($Password) {
    Resolve-TestVmCredential -Credential ([pscredential]::new($Username, $Password)) -Hostname $Hostname
} else {
    Resolve-TestVmCredential -Username $Username -Password $PasswordPlainText `
        -PromptPassword:$PromptPassword -PasswordEnvVar $PasswordEnvVar -Hostname $Hostname
}

$resolvedRdpIdentity = Resolve-RdpIdentityParts -Username $RdpUsername -Domain $RdpDomain
$RdpUsername = [string]$resolvedRdpIdentity.Username
$RdpDomain = [string]$resolvedRdpIdentity.Domain

$resolvedRdpPassword = if ($WtsProvider.IsPresent) {
    ''
}
elseif ($PSBoundParameters.ContainsKey('RdpPassword') -and -not [string]::IsNullOrWhiteSpace($RdpPassword)) {
    Write-Warning "-RdpPassword is passed as plaintext; prefer env:$RdpPasswordEnvVar to avoid leaking credentials via shell history."
    $RdpPassword
}
else {
    $fromEnv = [Environment]::GetEnvironmentVariable($RdpPasswordEnvVar)
    if (-not [string]::IsNullOrWhiteSpace($fromEnv)) {
        $fromEnv
    } else {
        # Back-compat / convenience fallback: if the admin password env var is set, reuse it.
        [Environment]::GetEnvironmentVariable($PasswordEnvVar)
    }
}

if (-not $WtsProvider.IsPresent -and [string]::IsNullOrWhiteSpace($resolvedRdpPassword)) {
    # Convenience: if the user provided/prompted an admin password (SecureString), reuse it for RDP
    # so Hybrid/CredSSP can be enabled without passing any plaintext password on the command line.
    $resolvedRdpPassword = $cred.GetNetworkCredential().Password
}

if (-not $WtsProvider.IsPresent -and [string]::IsNullOrWhiteSpace($resolvedRdpPassword)) {
    Write-Warning "RDP password is not configured (pass -RdpPassword or set env:$RdpPasswordEnvVar). Standard security connections will be rejected."
}

$termServiceOriginalState = $null
$deploymentSucceeded = $false
$session = $null
$session = New-TestVmSession -Hostname $Hostname -Credential $cred
try {
    Invoke-Command -Session $session -ScriptBlock {
        param($RemoteRoot)
        New-Item -ItemType Directory -Path $RemoteRoot -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $RemoteRoot 'bin') -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $RemoteRoot 'secrets') -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $RemoteRoot 'logs') -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $RemoteRoot 'tools') -Force | Out-Null
    } -ArgumentList $RemoteRoot

    $remoteTestVmModule = Join-Path $RemoteRoot 'tools\TestVm.psm1'
    Copy-Item -ToSession $session -LiteralPath $testVmModule -Destination $remoteTestVmModule -Force
    $exeRemote = Join-Path $RemoteRoot 'bin\ironrdp-termsrv.exe'
    $exeRemoteNext = Join-Path $RemoteRoot 'bin\ironrdp-termsrv.next.exe'
    $runnerRemote = Join-Path $RemoteRoot 'bin\run-ironrdp-termsrv.ps1'
    $rdpPasswordRemote = Join-Path $RemoteRoot 'secrets\rdp_password.txt'
    $logOut = Join-Path $RemoteRoot 'logs\ironrdp-termsrv.log'
    $logErr = Join-Path $RemoteRoot 'logs\ironrdp-termsrv.err.log'

    $termServiceOriginalState = Invoke-Command -Session $session -ScriptBlock {
        param($ModulePath)
        Import-Module $ModulePath -Force
        Get-TermServiceLifecycle
    } -ArgumentList $remoteTestVmModule

    Invoke-Command -Session $session -ScriptBlock {
        param($TaskName, $RdpPasswordPath, $LogOut, $LogErr, $StopTimeoutSeconds, $ModulePath, $OwnedExePaths, $DeployRoot)
        $ErrorActionPreference = 'Stop'
        Import-Module $ModulePath -Force
        $null = Stop-TermServiceSafely -TimeoutSeconds $StopTimeoutSeconds -Force
        $task = Get-ScheduledTask -TaskPath '\' -ErrorAction Stop | Where-Object TaskName -eq $TaskName
        if ($null -ne $task) {
            $task | Stop-ScheduledTask -ErrorAction Stop
            $task | Unregister-ScheduledTask -Confirm:$false -ErrorAction Stop
        }

        # Kill any staged or primary ironrdp-termsrv process before restaging the binary.
        $procs = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object { $_.ExecutablePath -in $OwnedExePaths })
        if ($procs) {
            Write-Host "Killing $($procs.Count) ironrdp-termsrv process(es): $(@($procs | ForEach-Object { $_.ProcessId }) -join ', ')"
            $procs | ForEach-Object {
                Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
            }
            $deadline = (Get-Date).AddSeconds(10)
            do {
                Start-Sleep -Milliseconds 500
                $remaining = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object { $_.ExecutablePath -in $OwnedExePaths })
            } while ($remaining -and ((Get-Date) -lt $deadline))
            if ($remaining) {
                throw "Failed to stop deployed companion processes: $(@($remaining | ForEach-Object { $_.ProcessId }) -join ', ')"
            }
        }

        # Wait for kernel to release named pipe handles (first_pipe_instance requires exclusive ownership)
        Write-Host "Waiting 5s for pipe handles to be released..."
        Start-Sleep -Seconds 5

        Remove-Item -LiteralPath $RdpPasswordPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $LogOut -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $LogErr -Force -ErrorAction SilentlyContinue
        # Clear all known DLL debug log paths so each run has a fresh provider log.
        Remove-Item -LiteralPath (Join-Path $DeployRoot 'logs\wts-provider-debug.log') -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath 'C:\Windows\Temp\wts-provider-debug.log' -Force -ErrorAction SilentlyContinue

    } -ArgumentList $TaskName, $rdpPasswordRemote, $logOut, $logErr, $TermServiceStopTimeoutSeconds, $remoteTestVmModule, @($exeRemote, $exeRemoteNext), $RemoteRoot

    if ($InstallIdd.IsPresent) {
        # UNTESTED: Local tests do not exercise PSRemoting reconnect after reboot or target-side IDD loading.
        # Verify both before changing this sequence because TermService remains disabled until deployment completes.
        $iddScriptsRoot = Join-Path $workspaceRoot 'server\termsrv\crates\ironrdp-idd\scripts'
        $iddDeployScript = Join-Path $iddScriptsRoot 'deploy-idd.ps1'
        $iddRestartScript = Join-Path $iddScriptsRoot 'restart-vm.ps1'
        $iddInstallScript = Join-Path $iddScriptsRoot 'install-idd-remote.ps1'

        $iddDeployment = & $iddDeployScript -VmHostname $Hostname -Credential $cred `
            -TestCertificatesDirectory $TestCertificatesDirectory
        if ($iddDeployment.RebootRequired) {
            Write-Host "Rebooting $Hostname to apply test-signing policy..." -ForegroundColor Yellow
            & $iddRestartScript -VmHostname $Hostname -Credential $cred

            Remove-PSSession -Session $session -ErrorAction SilentlyContinue
            $session = New-TestVmSession -Hostname $Hostname -Credential $cred
        }

        & $iddInstallScript -VmHostname $Hostname -Credential $cred -RemotePath $IddRemotePath `
            -TestCertificatesDirectory $TestCertificatesDirectory
    }

    if ($WtsProvider.IsPresent) {
        # UNTESTED: Local tests only parse this remote path; verify provider registration and the companion-owned listener before changing its ordering.
        $providerRemoteDir = Join-Path $RemoteRoot 'provider'
        $providerDllRemote = Join-Path $providerRemoteDir 'ironrdp_wtsprotocol_provider.dll'
        $providerScriptsLocal = Join-Path $workspaceRoot 'server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts'
        $providerScriptsRemote = @(
            (Join-Path $providerScriptsLocal 'install-side-by-side.ps1'),
            (Join-Path $providerScriptsLocal 'side-by-side-defaults.ps1'),
            $testVmModule
        )
        $portNumber = [int]($ListenerAddr.Split(':')[-1])

        Invoke-Command -Session $session -ScriptBlock {
            param($ProviderDir)
            New-Item -ItemType Directory -Path $ProviderDir -Force | Out-Null
        } -ArgumentList $providerRemoteDir

        Invoke-Command -Session $session -ScriptBlock {
            param($DllPath)

            if (-not (Test-Path -LiteralPath $DllPath -PathType Leaf)) {
                return
            }

            $deadline = (Get-Date).AddSeconds(30)
            while ((Get-Date) -lt $deadline) {
                try {
                    $stream = [System.IO.File]::Open(
                        $DllPath,
                        [System.IO.FileMode]::Open,
                        [System.IO.FileAccess]::ReadWrite,
                        [System.IO.FileShare]::None)
                    $stream.Dispose()
                    return
                }
                catch {
                    Start-Sleep -Milliseconds 500
                }
            }

            throw "Provider DLL '$DllPath' remained locked after TermService stopped"
        } -ArgumentList $providerDllRemote

        Copy-Item -ToSession $session -Path $providerDllLocal -Destination $providerDllRemote -Force
        Copy-Item -ToSession $session -Path $providerScriptsRemote -Destination $providerRemoteDir -Force

        Invoke-Command -Session $session -ScriptBlock {
            param($ProviderDir, $DllPath, $Listener, $Port, $DeployRoot)

            $installScript = Join-Path $ProviderDir 'install-side-by-side.ps1'
            & $installScript -ProviderDllPath $DllPath -ListenerName $Listener -PortNumber $Port -RemoteRoot $DeployRoot
        } -ArgumentList $providerRemoteDir, $providerDllRemote, $ListenerName, $portNumber, $RemoteRoot
    }

    $copyAttempts = 0
    $maxCopyAttempts = 4
    while ($true) {
        $copyAttempts++
        try {
            Copy-Item -ToSession $session -Path $exeLocal -Destination $exeRemoteNext -Force
            break
        }
        catch {
            Write-Warning "Copy-Item failed (attempt $copyAttempts/$maxCopyAttempts): $_"
            if ($copyAttempts -ge $maxCopyAttempts) {
                throw
            }

            try {
                if ($null -ne $session) {
                    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
                    $session = $null
                }
            }
            catch { }

            Start-Sleep -Seconds (2 * $copyAttempts)
            $session = New-TestVmSession -Hostname $Hostname -Credential $cred
        }
    }

    Invoke-Command -Session $session -ScriptBlock {
        param($RunnerPath)

        Set-StrictMode -Version Latest
        $ErrorActionPreference = 'Stop'

        $runner = @'
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ExePath,

    [Parameter(Mandatory = $true)]
    [string]$LogOut,

    [Parameter(Mandatory = $true)]
    [string]$LogErr,

    [Parameter(Mandatory = $true)]
    [string]$DeployRoot,

    [Parameter(Mandatory = $true)]
    [string]$ListenerAddr,

    [Parameter(Mandatory = $true)]
    [string]$CaptureIpc,

    [Parameter()]
    [switch]$AutoListen,

    [Parameter()]
    [switch]$WtsProvider,

    [Parameter()]
    [switch]$Clipboard,

    [Parameter()]
    [switch]$RemoteApp,

    [Parameter()]
    [switch]$AutoSendSas,

    [Parameter()]
    [string]$CaptureSessionId = '',

    [Parameter()]
    [string]$DumpBitmapUpdatesDir = '',

    [Parameter()]
    [string]$RdpUsername = '',

    [Parameter()]
    [string]$RdpDomain = '',

    [Parameter()]
    [string]$RdpPasswordFile = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$env:IRONRDP_WTS_LISTEN_ADDR = $ListenerAddr
$env:IRONRDP_WTS_CAPTURE_IPC = $CaptureIpc
$env:IRONRDP_WTS_AUTO_LISTEN = if ($AutoListen.IsPresent) { '1' } else { '0' }
$env:IRONRDP_WTS_PROVIDER = if ($WtsProvider.IsPresent) { '1' } else { '0' }
$env:IRONRDP_WTS_REMOTEAPP = if ($RemoteApp.IsPresent) { '1' } else { '0' }
if ($Clipboard.IsPresent) {
    $env:IRONRDP_WTS_CLIPBOARD = '1'
}
$env:IRONRDP_WTS_AUTO_SEND_SAS = if ($AutoSendSas.IsPresent) { '1' } else { '0' }
$env:IRONRDP_WTS_DEPLOY_ROOT = $DeployRoot
$env:IRONRDP_LOG = 'info'
$env:RUST_BACKTRACE = '1'

if (-not [string]::IsNullOrWhiteSpace($CaptureSessionId)) {
    $env:IRONRDP_WTS_CAPTURE_SESSION_ID = $CaptureSessionId
} else {
    Remove-Item Env:IRONRDP_WTS_CAPTURE_SESSION_ID -ErrorAction SilentlyContinue
}

if (-not [string]::IsNullOrWhiteSpace($DumpBitmapUpdatesDir)) {
    $env:IRONRDP_WTS_DUMP_BITMAP_UPDATES_DIR = $DumpBitmapUpdatesDir
} else {
    Remove-Item Env:IRONRDP_WTS_DUMP_BITMAP_UPDATES_DIR -ErrorAction SilentlyContinue
}

if (-not $WtsProvider.IsPresent -and -not [string]::IsNullOrWhiteSpace($RdpUsername)) {
    $env:IRONRDP_RDP_USERNAME = $RdpUsername
} else {
    Remove-Item Env:IRONRDP_RDP_USERNAME -ErrorAction SilentlyContinue
}

if (-not $WtsProvider.IsPresent -and -not [string]::IsNullOrWhiteSpace($RdpDomain)) {
    $env:IRONRDP_RDP_DOMAIN = $RdpDomain
} else {
    Remove-Item Env:IRONRDP_RDP_DOMAIN -ErrorAction SilentlyContinue
}

$rdpPassword = ''
if (-not $WtsProvider.IsPresent -and -not [string]::IsNullOrWhiteSpace($RdpPasswordFile) -and (Test-Path -LiteralPath $RdpPasswordFile -PathType Leaf)) {
    $rdpPassword = [System.IO.File]::ReadAllText($RdpPasswordFile, [System.Text.Encoding]::UTF8).TrimEnd("`r", "`n")
}

if (-not [string]::IsNullOrWhiteSpace($rdpPassword)) {
    $env:IRONRDP_RDP_PASSWORD = $rdpPassword
} else {
    Remove-Item Env:IRONRDP_RDP_PASSWORD -ErrorAction SilentlyContinue
}

Remove-Item Env:IRONRDP_WTS_LOGON_USERNAME -ErrorAction SilentlyContinue
Remove-Item Env:IRONRDP_WTS_LOGON_DOMAIN -ErrorAction SilentlyContinue
Remove-Item Env:IRONRDP_WTS_LOGON_PASSWORD -ErrorAction SilentlyContinue

$logDir = Split-Path -Parent $LogOut
if (-not [string]::IsNullOrWhiteSpace($logDir)) {
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
}

$exeDir = Split-Path -Parent $ExePath
$exeName = [System.IO.Path]::GetFileNameWithoutExtension($ExePath)
$exeExt = [System.IO.Path]::GetExtension($ExePath)
$nextExePath = if (-not [string]::IsNullOrWhiteSpace($exeDir) -and -not [string]::IsNullOrWhiteSpace($exeName)) {
    Join-Path $exeDir ($exeName + '.next' + $exeExt)
} else {
    ''
}
$resolvedExePath = $ExePath
if (-not [string]::IsNullOrWhiteSpace($nextExePath) -and (Test-Path -LiteralPath $nextExePath -PathType Leaf)) {
    $resolvedExePath = $nextExePath
}

try {
    # Start the server and exit; the child process inherits env vars.
    Start-Process -FilePath $resolvedExePath -NoNewWindow -RedirectStandardOutput $LogOut -RedirectStandardError $LogErr | Out-Null
}
finally {
    # Best-effort cleanup: remove the password file after startup.
    if (-not [string]::IsNullOrWhiteSpace($RdpPasswordFile)) {
        Remove-Item -LiteralPath $RdpPasswordFile -Force -ErrorAction SilentlyContinue
    }
}
'@

        [System.IO.File]::WriteAllText($RunnerPath, $runner, [System.Text.Encoding]::UTF8)
    } -ArgumentList $runnerRemote

    if (-not [string]::IsNullOrWhiteSpace($resolvedRdpPassword)) {
        Invoke-Command -Session $session -ScriptBlock {
            param($SecretPath, $Secret)

            Set-StrictMode -Version Latest
            $ErrorActionPreference = 'Stop'

            $secretDir = Split-Path -Parent $SecretPath
            if (-not [string]::IsNullOrWhiteSpace($secretDir)) {
                New-Item -ItemType Directory -Path $secretDir -Force | Out-Null
            }

            icacls $secretDir /inheritance:r /grant:r "SYSTEM:(OI)(CI)F" "Administrators:(OI)(CI)F" | Out-Null
            if ($LASTEXITCODE -ne 0) { throw "Could not restrict the credential directory (exit $LASTEXITCODE)" }
            [System.IO.File]::WriteAllText($SecretPath, $Secret, [System.Text.Encoding]::UTF8)
            icacls $SecretPath /inheritance:r /grant:r "SYSTEM:F" "Administrators:F" | Out-Null
            if ($LASTEXITCODE -ne 0) { throw "Could not restrict the credential file (exit $LASTEXITCODE)" }
        } -ArgumentList $rdpPasswordRemote, $resolvedRdpPassword
    }

    Invoke-Command -Session $session -ScriptBlock {
        param($Port)

        $ruleName = "IronRDP TermSrv $Port"
        if (-not (Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue)) {
            New-NetFirewallRule -DisplayName $ruleName -Direction Inbound -Action Allow -Protocol TCP -LocalPort $Port -Profile Any | Out-Null
        }
    } -ArgumentList ([int]($ListenerAddr.Split(':')[-1]))

    $companion = Invoke-Command -Session $session -ScriptBlock {
        param($TaskName, $ExePath, $RunnerPath, $SecretPath, $LogOut, $LogErr, $DeployRoot, $ListenerAddr, $CaptureIpc, $AutoListen, $WtsProvider, $Clipboard, $RemoteApp, $AutoSendSas, $CaptureSessionId, $DumpBitmapUpdatesDir, $RdpUsername, $RdpDomain)

        $arguments = @(
            '-NoProfile',
            '-ExecutionPolicy', 'Bypass',
            '-File', $RunnerPath,
            '-ExePath', $ExePath,
            '-LogOut', $LogOut,
            '-LogErr', $LogErr,
            '-DeployRoot', $DeployRoot,
            '-ListenerAddr', $ListenerAddr,
            '-CaptureIpc', $CaptureIpc
        )

        if ($AutoListen) {
            $arguments += @('-AutoListen')
        }

        if ($WtsProvider) {
            $arguments += @('-WtsProvider')
        }

        if ($AutoSendSas) {
            $arguments += @('-AutoSendSas')
        }

        if ($Clipboard) {
            $arguments += @('-Clipboard')
        }

        if ($RemoteApp) {
            $arguments += @('-RemoteApp')
        }

        if (-not [string]::IsNullOrWhiteSpace($CaptureSessionId)) {
            $arguments += @('-CaptureSessionId', $CaptureSessionId)
        }

        if (-not [string]::IsNullOrWhiteSpace($DumpBitmapUpdatesDir)) {
            $arguments += @('-DumpBitmapUpdatesDir', $DumpBitmapUpdatesDir)
        }

        if (-not $WtsProvider -and -not [string]::IsNullOrWhiteSpace($RdpUsername)) {
            $arguments += @('-RdpUsername', $RdpUsername)
        }

        if (-not $WtsProvider -and -not [string]::IsNullOrWhiteSpace($RdpDomain)) {
            $arguments += @('-RdpDomain', $RdpDomain)
        }

        if (-not $WtsProvider) {
            $arguments += @('-RdpPasswordFile', $SecretPath)
        }

        $argumentString = ($arguments | ForEach-Object {
            if ($_ -match '[\s"'']') { '"' + ($_ -replace '"', '\\"') + '"' } else { $_ }
        }) -join ' '

        $scheduleService = Get-Service -Name 'Schedule' -ErrorAction SilentlyContinue
        if ($null -eq $scheduleService) {
            throw 'Task Scheduler service is missing'
        }
        if ($scheduleService.Status -ne 'Running') {
            Start-Service -Name 'Schedule'
            $scheduleService.WaitForStatus('Running', [TimeSpan]::FromSeconds(15))
        }

        try {
            $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $argumentString
            $trigger = New-ScheduledTaskTrigger -AtStartup
            $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
            $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew

            Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
            Start-ScheduledTask -TaskName $TaskName
        }
        catch {
            Write-Warning "ScheduledTasks cmdlets failed for ${TaskName}: $($_.Exception.Message). Falling back to schtasks.exe"
            $taskCommand = "powershell.exe $argumentString"
            & schtasks.exe /Create /TN $TaskName /SC ONSTART /RU SYSTEM /RL HIGHEST /TR $taskCommand /F | Out-Null
            if ($LASTEXITCODE -ne 0) {
                throw "schtasks.exe /Create failed for $TaskName (exit $LASTEXITCODE)"
            }
            & schtasks.exe /Run /TN $TaskName | Out-Null
            if ($LASTEXITCODE -ne 0) {
                throw "schtasks.exe /Run failed for $TaskName (exit $LASTEXITCODE)"
            }
        }

        $proc = $null
        $deadline = (Get-Date).AddSeconds(10)
        do {
            Start-Sleep -Milliseconds 250
            $proc = Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
                $_.ExecutablePath -in @($ExePath, [IO.Path]::ChangeExtension($ExePath, 'next.exe'))
            } | Select-Object -First 1
        } while (($null -eq $proc) -and ((Get-Date) -lt $deadline))

        if ($null -eq $proc) {
            if (Test-Path -LiteralPath $LogErr -PathType Leaf) {
                Write-Host "---- ironrdp-termsrv.err.log (tail) ----" -ForegroundColor Yellow
                Get-Content -LiteralPath $LogErr -Tail 40 -ErrorAction SilentlyContinue
            }
            if (Test-Path -LiteralPath $LogOut -PathType Leaf) {
                Write-Host "---- ironrdp-termsrv.log (tail) ----" -ForegroundColor Yellow
                Get-Content -LiteralPath $LogOut -Tail 40 -ErrorAction SilentlyContinue
            }
            throw "ironrdp-termsrv did not start (check logs: $LogOut / $LogErr)"
        }

        [pscustomobject]@{
            ExePath = $ExePath
            LogOut = $LogOut
            LogErr = $LogErr
            Pid = $proc.ProcessId
        }
    } -ArgumentList $TaskName, $exeRemote, $runnerRemote, $rdpPasswordRemote, $logOut, $logErr, $RemoteRoot, $ListenerAddr, $CaptureIpc, $AutoListen.IsPresent, $WtsProvider.IsPresent, $Clipboard.IsPresent, $RemoteApp.IsPresent, $AutoSendSas.IsPresent, $CaptureSessionId, $DumpBitmapUpdatesDir, $RdpUsername, $RdpDomain
    $companion | Format-List

    Invoke-Command -Session $session -ScriptBlock {
        param($ListenerAddr, $LogOut, $LogErr, $TailLines, $NoTermServiceStart, $WtsProvider, $AutoListen, $ModulePath, $CompanionPid)
        Import-Module $ModulePath -Force

        if (-not $NoTermServiceStart) {
            # Start TermService only after the companion is running so the provider connects to this instance.
            Write-Host "Starting TermService..."
            Start-TermServiceSafely -StartupType demand -TimeoutSeconds 30

            if ($WtsProvider) {
                $wudfrd = Get-Service -Name 'WUDFRd' -ErrorAction SilentlyContinue
                if ($null -ne $wudfrd -and $wudfrd.StartType -ne 'Automatic') {
                    & sc.exe config WUDFRd start= auto | Out-Null
                    if ($LASTEXITCODE -ne 0) { throw "Could not configure WUDFRd (exit $LASTEXITCODE)" }
                }
                if ($null -ne $wudfrd -and $wudfrd.Status -ne 'Running') {
                    Start-Service -Name 'WUDFRd' -ErrorAction Stop
                    $wudfrd = Get-Service -Name 'WUDFRd' -ErrorAction Stop
                    $wudfrd.WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
                }
            }
        } else {
            Restore-TermServiceLifecycle -StartupType demand -WasRunning $false
            Write-Host "Skipping TermService start (NoTermServiceStart flag set)"
        }

        $port = [int]($ListenerAddr.Split(':')[-1])
        $listening = $false
        $listenerExpected = $WtsProvider -or $AutoListen
        if (-not $NoTermServiceStart -and $listenerExpected) {
            $null = Wait-TestVmListener -Port $port -OwningProcess $CompanionPid -TimeoutSeconds 30
            $listening = $true
        }

        [pscustomobject]@{
            ListenerAddr = $ListenerAddr
            Listening = $listening
            ListenerExpected = $listenerExpected
            StartupDeferred = $NoTermServiceStart
        } | Format-List

        if (Test-Path -LiteralPath $LogErr -PathType Leaf) {
            Write-Host "---- ironrdp-termsrv.err.log (tail) ----" -ForegroundColor Yellow
            Get-Content -LiteralPath $LogErr -Tail $TailLines -ErrorAction SilentlyContinue
        }

        if (Test-Path -LiteralPath $LogOut -PathType Leaf) {
            Write-Host "---- ironrdp-termsrv.log (tail) ----" -ForegroundColor Yellow
            Get-Content -LiteralPath $LogOut -Tail $TailLines -ErrorAction SilentlyContinue
        }
    } -ArgumentList $ListenerAddr, $logOut, $logErr, $TailLines, $NoTermServiceStart.IsPresent, $WtsProvider.IsPresent, $AutoListen.IsPresent, $remoteTestVmModule, $companion.Pid

    if ($NoTermServiceStart.IsPresent) {
        Write-Host "Deployment staged: TermService startup intentionally deferred on $Hostname" -ForegroundColor Yellow
    } elseif ($WtsProvider.IsPresent -or $AutoListen.IsPresent) {
        Write-Host "Ready: $Hostname ($ListenerAddr)" -ForegroundColor Green
    } else {
        Write-Host "Companion started; no listener was requested on $Hostname" -ForegroundColor Green
    }
    $port = $ListenerAddr.Split(':')[-1]
    if (-not $WtsProvider.IsPresent -and [string]::IsNullOrWhiteSpace($resolvedRdpPassword)) {
        Write-Warning 'RDP password is not configured'
    } else {
        Write-Host "Connect with: ironrdp-agent connect --server ${Hostname}:$port --no-prompt (credentials from environment)" -ForegroundColor Green
    }

    $deploymentSucceeded = $true
}
finally {
    if (-not $deploymentSucceeded -and $null -ne $termServiceOriginalState) {
        # UNTESTED: Local tests cover restoration logic, but not this PSRemoting finally path.
        try {
            if ($null -eq $session -or $session.State -ne 'Opened') {
                if ($null -ne $session) {
                    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
                }
                $session = New-TestVmSession -Hostname $Hostname -Credential $cred
            }

            Invoke-Command -Session $session -ScriptBlock {
                param($StartupType, $WasRunning, $ModulePath)
                $ErrorActionPreference = 'Stop'
                Import-Module $ModulePath -Force
                Restore-TermServiceLifecycle -StartupType $StartupType -WasRunning $WasRunning
            } -ArgumentList $termServiceOriginalState.StartupType, $termServiceOriginalState.WasRunning, $remoteTestVmModule | Out-Null
        }
        catch {
            Write-Warning "TermService recovery after deployment failure failed: $($_.Exception.Message)"
        }
    }

    if ($null -ne $session) {
        Remove-PSSession -Session $session
    }
}
