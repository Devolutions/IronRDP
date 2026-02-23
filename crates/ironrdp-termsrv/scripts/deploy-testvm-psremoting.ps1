[CmdletBinding()]
param(
    [Parameter()]
    [string]$Hostname = 'IT-HELP-TEST',

    [Parameter()]
    [string]$Username = 'IT-HELP\Administrator',

    [Parameter()]
    [securestring]$Password,

    [Parameter()]
    [string]$PasswordPlainText = '',

    [Parameter()]
    [switch]$PromptPassword,

    [Parameter()]
    [string]$PasswordEnvVar = 'IRONRDP_TESTVM_PASSWORD',

    [Parameter()]
    [string]$RdpUsername = 'test',

    [Parameter()]
    [string]$RdpPassword = '',

    [Parameter()]
    [string]$RdpPasswordEnvVar = 'IRONRDP_TESTVM_RDP_PASSWORD',

    [Parameter()]
    [string]$RdpDomain = '',

    [Parameter()]
    [string]$RemoteRoot = 'C:\IronRDPDeploy',

    [Parameter()]
    [string]$TaskName = 'IronRdpTermSrvSystem',

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
    [string]$CaptureSessionId = '',

    [Parameter()]
    [string]$DumpBitmapUpdatesDir = '',

    [Parameter()]
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',

    [Parameter()]
    [switch]$SkipBuild,

    # When set, the deploy script will NOT start TermService after the companion is started.
    # Use this when a separate step (e.g. provider DLL install) will start TermService, to avoid
    # a double-start that triggers StopListen → the companion's TCP listener task is aborted.
    [Parameter()]
    [switch]$NoTermServiceStart,

    [Parameter()]
    [int]$TailLines = 80
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Convert-SecureStringToPlainText {
    param(
        [Parameter(Mandatory = $true)]
        [securestring]$Value
    )

    $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($Value)
    try {
        [Runtime.InteropServices.Marshal]::PtrToStringUni($bstr)
    }
    finally {
        [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr)
    }
}

function Resolve-WorkspaceRoot {
    $here = $PSScriptRoot
    return (Get-Item -LiteralPath (Join-Path $here '..\..\..')).FullName
}

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

$workspaceRoot = Resolve-WorkspaceRoot
$profileDir = if ($Configuration -eq 'Release') { 'release' } else { 'debug' }
$exeLocal = Join-Path $workspaceRoot "target\$profileDir\ironrdp-termsrv.exe"

if (-not $SkipBuild.IsPresent) {
    Push-Location $workspaceRoot
    try {
        if ($Configuration -eq 'Release') {
            cargo build -p ironrdp-termsrv --release
        } else {
            cargo build -p ironrdp-termsrv
        }
    }
    finally {
        Pop-Location
    }
}

if (-not (Test-Path -LiteralPath $exeLocal -PathType Leaf)) {
    throw "Service executable not found: $exeLocal"
}

$resolvedPassword = if ($Password) {
    $Password
}
elseif (-not [string]::IsNullOrWhiteSpace($PasswordPlainText)) {
    ConvertTo-SecureString -String $PasswordPlainText -AsPlainText -Force
}
elseif ($PromptPassword.IsPresent) {
    Read-Host -Prompt "Password for $Username@$Hostname" -AsSecureString
}
else {
    $plain = [Environment]::GetEnvironmentVariable($PasswordEnvVar)
    if ([string]::IsNullOrWhiteSpace($plain)) {
        throw "Missing password: pass -Password, use -PromptPassword, or set env:$PasswordEnvVar"
    }
    ConvertTo-SecureString -String $plain -AsPlainText -Force
}

$cred = [pscredential]::new($Username, $resolvedPassword)

$wtsLogonUsername = ''
$wtsLogonDomain = ''
if ($WtsProvider.IsPresent) {
    $m = [regex]::Match($Username, '^(?<domain>[^\\]+)\\(?<user>.+)$')
    if ($m.Success) {
        $wtsLogonDomain = $m.Groups['domain'].Value
        $wtsLogonUsername = $m.Groups['user'].Value
    }
    else {
        $wtsLogonUsername = $Username
        $wtsLogonDomain = ''
    }
}

$resolvedRdpPassword = if ($PSBoundParameters.ContainsKey('RdpPassword') -and -not [string]::IsNullOrWhiteSpace($RdpPassword)) {
    Write-Warning "-RdpPassword is passed as plaintext; prefer env:$RdpPasswordEnvVar to avoid leaking credentials via shell history."
    $RdpPassword
}
else {
    $fromEnv = [Environment]::GetEnvironmentVariable($RdpPasswordEnvVar)
    if (-not [string]::IsNullOrWhiteSpace($fromEnv)) {
        $fromEnv
    } else {
        # Back-compat / convenience fallback: if the admin password env var is set, reuse it.
        [Environment]::GetEnvironmentVariable('IRONRDP_TESTVM_PASSWORD')
    }
}

if ([string]::IsNullOrWhiteSpace($resolvedRdpPassword)) {
    # Convenience: if the user provided/prompted an admin password (SecureString), reuse it for RDP
    # so Hybrid/CredSSP can be enabled without passing any plaintext password on the command line.
    $resolvedRdpPassword = Convert-SecureStringToPlainText -Value $resolvedPassword
}

if ([string]::IsNullOrWhiteSpace($resolvedRdpPassword)) {
    Write-Warning "RDP password is not configured (pass -RdpPassword or set env:$RdpPasswordEnvVar). Standard security connections will be rejected."
}

$session = $null
$session = New-TestVmSession -Hostname $Hostname -Credential $cred
try {
    Invoke-Command -Session $session -ScriptBlock {
        param($RemoteRoot)
        New-Item -ItemType Directory -Path $RemoteRoot -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $RemoteRoot 'bin') -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $RemoteRoot 'secrets') -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $RemoteRoot 'logs') -Force | Out-Null
    } -ArgumentList $RemoteRoot

    $exeRemote = Join-Path $RemoteRoot 'bin\ironrdp-termsrv.exe'
    $runnerRemote = Join-Path $RemoteRoot 'bin\run-ironrdp-termsrv.ps1'
    $rdpPasswordRemote = Join-Path $RemoteRoot 'secrets\rdp_password.txt'
    $logOut = Join-Path $RemoteRoot 'logs\ironrdp-termsrv.log'
    $logErr = Join-Path $RemoteRoot 'logs\ironrdp-termsrv.err.log'

    Invoke-Command -Session $session -ScriptBlock {
        param($TaskName, $RdpPasswordPath, $LogOut, $LogErr)

        # Stop TermService first so the DLL is unloaded and stops connecting to the pipe
        Write-Host "Stopping TermService..."
        Stop-Service -Name 'TermService' -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2

        # Stop ceviche-service if present - it uses the same named pipe and causes accept_connection to fail
        Write-Host "Stopping ceviche-service (if running)..."
        Get-Process -Name 'ceviche-service' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

        try { Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue } catch {}
        try { Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue } catch {}

        # Kill ALL ironrdp-termsrv processes and wait for them to exit
        $procs = Get-Process -Name 'ironrdp-termsrv' -ErrorAction SilentlyContinue
        if ($procs) {
            Write-Host "Killing $($procs.Count) ironrdp-termsrv process(es): $($procs.Id -join ', ')"
            $procs | Stop-Process -Force -ErrorAction SilentlyContinue
            $deadline = (Get-Date).AddSeconds(10)
            do {
                Start-Sleep -Milliseconds 500
                $remaining = Get-Process -Name 'ironrdp-termsrv' -ErrorAction SilentlyContinue
            } while ($remaining -and ((Get-Date) -lt $deadline))
            if ($remaining) {
                Write-Warning "Failed to kill all ironrdp-termsrv processes: $($remaining.Id -join ', ')"
            }
        }

        # Wait for kernel to release named pipe handles (first_pipe_instance requires exclusive ownership)
        Write-Host "Waiting 5s for pipe handles to be released..."
        Start-Sleep -Seconds 5

        Remove-Item -LiteralPath $RdpPasswordPath -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $LogOut -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $LogErr -Force -ErrorAction SilentlyContinue
        # Clear the DLL debug log so each run has a fresh log
        Remove-Item -LiteralPath 'C:\IronRDPDeploy\logs\wts-provider-debug.log' -Force -ErrorAction SilentlyContinue
    } -ArgumentList $TaskName, $rdpPasswordRemote, $logOut, $logErr

    $copyAttempts = 0
    $maxCopyAttempts = 4
    while ($true) {
        $copyAttempts++
        try {
            Copy-Item -ToSession $session -Path $exeLocal -Destination $exeRemote -Force
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
    [string]$ListenerAddr,

    [Parameter(Mandatory = $true)]
    [string]$CaptureIpc,

    [Parameter()]
    [switch]$AutoListen,

    [Parameter()]
    [switch]$WtsProvider,

    [Parameter()]
    [string]$CaptureSessionId = '',

    [Parameter()]
    [string]$DumpBitmapUpdatesDir = '',

    [Parameter()]
    [string]$RdpUsername = '',

    [Parameter()]
    [string]$RdpDomain = '',

    [Parameter()]
    [string]$RdpPasswordFile = '',

    [Parameter()]
    [string]$WtsLogonUsername = '',

    [Parameter()]
    [string]$WtsLogonDomain = '',

    [Parameter()]
    [string]$WtsLogonPasswordFile = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$env:IRONRDP_WTS_LISTEN_ADDR = $ListenerAddr
$env:IRONRDP_WTS_CAPTURE_IPC = $CaptureIpc
$env:IRONRDP_WTS_AUTO_LISTEN = if ($AutoListen.IsPresent) { '1' } else { '0' }
$env:IRONRDP_WTS_PROVIDER = if ($WtsProvider.IsPresent) { '1' } else { '0' }
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

if (-not [string]::IsNullOrWhiteSpace($RdpUsername)) {
    $env:IRONRDP_RDP_USERNAME = $RdpUsername
} else {
    Remove-Item Env:IRONRDP_RDP_USERNAME -ErrorAction SilentlyContinue
}

if (-not [string]::IsNullOrWhiteSpace($RdpDomain)) {
    $env:IRONRDP_RDP_DOMAIN = $RdpDomain
} else {
    Remove-Item Env:IRONRDP_RDP_DOMAIN -ErrorAction SilentlyContinue
}

$rdpPassword = ''
if (-not [string]::IsNullOrWhiteSpace($RdpPasswordFile) -and (Test-Path -LiteralPath $RdpPasswordFile -PathType Leaf)) {
    $rdpPassword = [System.IO.File]::ReadAllText($RdpPasswordFile, [System.Text.Encoding]::UTF8).TrimEnd("`r", "`n")
}

if (-not [string]::IsNullOrWhiteSpace($rdpPassword)) {
    $env:IRONRDP_RDP_PASSWORD = $rdpPassword
} else {
    Remove-Item Env:IRONRDP_RDP_PASSWORD -ErrorAction SilentlyContinue
}

$wtsPassword = ''
if (-not [string]::IsNullOrWhiteSpace($WtsLogonPasswordFile) -and (Test-Path -LiteralPath $WtsLogonPasswordFile -PathType Leaf)) {
    $wtsPassword = [System.IO.File]::ReadAllText($WtsLogonPasswordFile, [System.Text.Encoding]::UTF8).TrimEnd("`r", "`n")
}

if (-not [string]::IsNullOrWhiteSpace($WtsLogonUsername) -and -not [string]::IsNullOrWhiteSpace($wtsPassword)) {
    $env:IRONRDP_WTS_LOGON_USERNAME = $WtsLogonUsername
    if (-not [string]::IsNullOrWhiteSpace($WtsLogonDomain)) {
        $env:IRONRDP_WTS_LOGON_DOMAIN = $WtsLogonDomain
    } else {
        Remove-Item Env:IRONRDP_WTS_LOGON_DOMAIN -ErrorAction SilentlyContinue
    }
    $env:IRONRDP_WTS_LOGON_PASSWORD = $wtsPassword
} else {
    Remove-Item Env:IRONRDP_WTS_LOGON_USERNAME -ErrorAction SilentlyContinue
    Remove-Item Env:IRONRDP_WTS_LOGON_DOMAIN -ErrorAction SilentlyContinue
    Remove-Item Env:IRONRDP_WTS_LOGON_PASSWORD -ErrorAction SilentlyContinue
}

$logDir = Split-Path -Parent $LogOut
if (-not [string]::IsNullOrWhiteSpace($logDir)) {
    New-Item -ItemType Directory -Path $logDir -Force | Out-Null
}

try {
    # Start the server and exit; the child process inherits env vars.
    Start-Process -FilePath $ExePath -NoNewWindow -RedirectStandardOutput $LogOut -RedirectStandardError $LogErr | Out-Null
}
finally {
    # Best-effort cleanup: remove the password file after startup.
    if (-not [string]::IsNullOrWhiteSpace($RdpPasswordFile)) {
        Remove-Item -LiteralPath $RdpPasswordFile -Force -ErrorAction SilentlyContinue
    }
    if (-not [string]::IsNullOrWhiteSpace($WtsLogonPasswordFile) -and ($WtsLogonPasswordFile -ne $RdpPasswordFile)) {
        Remove-Item -LiteralPath $WtsLogonPasswordFile -Force -ErrorAction SilentlyContinue
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

            [System.IO.File]::WriteAllText($SecretPath, $Secret, [System.Text.Encoding]::UTF8)

            # Lock down the secret so it's not readable by standard users.
            icacls $secretDir /inheritance:r /grant:r "SYSTEM:(OI)(CI)F" "Administrators:(OI)(CI)F" | Out-Null
            icacls $SecretPath /inheritance:r /grant:r "SYSTEM:F" "Administrators:F" | Out-Null
        } -ArgumentList $rdpPasswordRemote, $resolvedRdpPassword
    }

    Invoke-Command -Session $session -ScriptBlock {
        param($Port)

        $ruleName = "IronRDP TermSrv $Port"
        if (-not (Get-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue)) {
            New-NetFirewallRule -DisplayName $ruleName -Direction Inbound -Action Allow -Protocol TCP -LocalPort $Port -Profile Any | Out-Null
        }
    } -ArgumentList ([int]($ListenerAddr.Split(':')[-1]))

    Invoke-Command -Session $session -ScriptBlock {
        param($TaskName, $ExePath, $RunnerPath, $SecretPath, $LogOut, $LogErr, $ListenerAddr, $CaptureIpc, $AutoListen, $WtsProvider, $CaptureSessionId, $DumpBitmapUpdatesDir, $RdpUsername, $RdpDomain, $WtsLogonUsername, $WtsLogonDomain)

        $arguments = @(
            '-NoProfile',
            '-ExecutionPolicy', 'Bypass',
            '-File', $RunnerPath,
            '-ExePath', $ExePath,
            '-LogOut', $LogOut,
            '-LogErr', $LogErr,
            '-ListenerAddr', $ListenerAddr,
            '-CaptureIpc', $CaptureIpc
        )

        if ($AutoListen) {
            $arguments += @('-AutoListen')
        }

        if ($WtsProvider) {
            $arguments += @('-WtsProvider')
            if (-not [string]::IsNullOrWhiteSpace($WtsLogonUsername)) {
                $arguments += @('-WtsLogonUsername', $WtsLogonUsername)
            }
            if (-not [string]::IsNullOrWhiteSpace($WtsLogonDomain)) {
                $arguments += @('-WtsLogonDomain', $WtsLogonDomain)
            }
            $arguments += @('-WtsLogonPasswordFile', $SecretPath)
        }

        if (-not [string]::IsNullOrWhiteSpace($CaptureSessionId)) {
            $arguments += @('-CaptureSessionId', $CaptureSessionId)
        }

        if (-not [string]::IsNullOrWhiteSpace($DumpBitmapUpdatesDir)) {
            $arguments += @('-DumpBitmapUpdatesDir', $DumpBitmapUpdatesDir)
        }

        if (-not [string]::IsNullOrWhiteSpace($RdpUsername)) {
            $arguments += @('-RdpUsername', $RdpUsername)
        }

        if (-not [string]::IsNullOrWhiteSpace($RdpDomain)) {
            $arguments += @('-RdpDomain', $RdpDomain)
        }

        $arguments += @('-RdpPasswordFile', $SecretPath)

        $argumentString = ($arguments | ForEach-Object {
            if ($_ -match '[\s"'']') { '"' + ($_ -replace '"', '\\"') + '"' } else { $_ }
        }) -join ' '

        $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $argumentString
        $trigger = New-ScheduledTaskTrigger -AtStartup
        $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
        $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew

        Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
        Start-ScheduledTask -TaskName $TaskName

        $proc = $null
        $deadline = (Get-Date).AddSeconds(10)
        do {
            Start-Sleep -Milliseconds 250
            $proc = Get-Process -Name 'ironrdp-termsrv' -ErrorAction SilentlyContinue
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
            Pid = $proc.Id
        }
    } -ArgumentList $TaskName, $exeRemote, $runnerRemote, $rdpPasswordRemote, $logOut, $logErr, $ListenerAddr, $CaptureIpc, $AutoListen.IsPresent, $WtsProvider.IsPresent, $CaptureSessionId, $DumpBitmapUpdatesDir, $RdpUsername, $RdpDomain, $wtsLogonUsername, $wtsLogonDomain | Format-List

    Invoke-Command -Session $session -ScriptBlock {
        param($ListenerAddr, $LogOut, $LogErr, $TailLines, $NoTermServiceStart)

        if (-not $NoTermServiceStart) {
            # Restart TermService NOW (after companion is running) so the DLL connects to THIS instance.
            # Skip in Provider mode: the provider DLL install step will restart TermService exactly once.
            # A double-restart causes StopListen IPC → companion aborts its TCP listener task.
            Write-Host "Starting TermService..."
            Start-Service -Name 'TermService' -ErrorAction SilentlyContinue
            Start-Sleep -Seconds 3
        } else {
            Write-Host "Skipping TermService start (NoTermServiceStart flag set)"
        }

        $port = [int]($ListenerAddr.Split(':')[-1])
        $listening = $false
        try {
            $listening = (Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue | Measure-Object).Count -gt 0
        } catch {}

        [pscustomobject]@{
            ListenerAddr = $ListenerAddr
            Listening = $listening
        } | Format-List

        if (Test-Path -LiteralPath $LogErr -PathType Leaf) {
            Write-Host "---- ironrdp-termsrv.err.log (tail) ----" -ForegroundColor Yellow
            Get-Content -LiteralPath $LogErr -Tail $TailLines -ErrorAction SilentlyContinue
        }

        if (Test-Path -LiteralPath $LogOut -PathType Leaf) {
            Write-Host "---- ironrdp-termsrv.log (tail) ----" -ForegroundColor Yellow
            Get-Content -LiteralPath $LogOut -Tail $TailLines -ErrorAction SilentlyContinue
        }
    } -ArgumentList $ListenerAddr, $logOut, $logErr, $TailLines, $NoTermServiceStart.IsPresent

    Write-Host "Ready: $Hostname ($ListenerAddr)" -ForegroundColor Green
    $rdpIdentity = if ([string]::IsNullOrWhiteSpace($RdpDomain)) { $RdpUsername } else { "$RdpDomain\\$RdpUsername" }
    $rdpDomainArgs = if ([string]::IsNullOrWhiteSpace($RdpDomain)) { '' } else { " -d $RdpDomain" }
    $port = $ListenerAddr.Split(':')[-1]
    if ([string]::IsNullOrWhiteSpace($resolvedRdpPassword)) {
        Write-Host "RDP test credentials: $rdpIdentity (password NOT configured)" -ForegroundColor Yellow
        Write-Host "Connect with: ironrdp-client.exe ${Hostname}:$port -u $RdpUsername$rdpDomainArgs -p <password>" -ForegroundColor Yellow
    } else {
        Write-Host "RDP test credentials: $rdpIdentity (password provided out-of-band)" -ForegroundColor Green
        Write-Host "Connect with: ironrdp-client.exe ${Hostname}:$port -u $RdpUsername$rdpDomainArgs -p <password>" -ForegroundColor Green
    }
}
finally {
    if ($null -ne $session) {
        Remove-PSSession -Session $session
    }
}
