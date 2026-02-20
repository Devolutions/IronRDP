[CmdletBinding()]
param(
    [Parameter()]
    [string]$Hostname = 'IT-HELP-TEST',

    [Parameter()]
    [string]$Username = 'IT-HELP\Administrator',

    [Parameter()]
    [securestring]$Password,

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
    [string]$CaptureSessionId = '',

    [Parameter()]
    [ValidateSet('Debug', 'Release')]
    [string]$Configuration = 'Release',

    [Parameter()]
    [switch]$SkipBuild

    ,

    [Parameter()]
    [int]$TailLines = 80
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-WorkspaceRoot {
    $here = $PSScriptRoot
    return (Get-Item -LiteralPath (Join-Path $here '..\..\..')).FullName
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
    Write-Warning "RDP password is not configured (pass -RdpPassword or set env:$RdpPasswordEnvVar). Standard security connections will be rejected."
}

$session = New-PSSession -ComputerName $Hostname -Credential $cred
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
        param($TaskName, $RdpPasswordPath)

        try { Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue } catch {}
        try { Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue } catch {}

        Get-Process -Name 'ironrdp-termsrv' -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $RdpPasswordPath -Force -ErrorAction SilentlyContinue
    } -ArgumentList $TaskName, $rdpPasswordRemote

    Copy-Item -ToSession $session -Path $exeLocal -Destination $exeRemote -Force

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
    [string]$CaptureSessionId = '',

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
$env:IRONRDP_WTS_AUTO_LISTEN = '1'
$env:IRONRDP_LOG = 'info'

if (-not [string]::IsNullOrWhiteSpace($CaptureSessionId)) {
    $env:IRONRDP_WTS_CAPTURE_SESSION_ID = $CaptureSessionId
} else {
    Remove-Item Env:IRONRDP_WTS_CAPTURE_SESSION_ID -ErrorAction SilentlyContinue
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
        param($TaskName, $ExePath, $RunnerPath, $SecretPath, $LogOut, $LogErr, $ListenerAddr, $CaptureIpc, $CaptureSessionId, $RdpUsername, $RdpDomain)

        $arguments = @(
            '-NoProfile',
            '-ExecutionPolicy', 'Bypass',
            '-File', $RunnerPath,
            '-ExePath', $ExePath,
            '-LogOut', $LogOut,
            '-LogErr', $LogErr,
            '-ListenerAddr', $ListenerAddr,
            '-CaptureIpc', $CaptureIpc,
            '-CaptureSessionId', $CaptureSessionId,
            '-RdpUsername', $RdpUsername,
            '-RdpDomain', $RdpDomain,
            '-RdpPasswordFile', $SecretPath
        )

        $argumentString = ($arguments | ForEach-Object {
            if ($_ -match '[\s"'']') { '"' + ($_ -replace '"', '\\"') + '"' } else { $_ }
        }) -join ' '

        $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $argumentString
        $trigger = New-ScheduledTaskTrigger -AtStartup
        $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
        $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew

        Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
        Start-ScheduledTask -TaskName $TaskName

        Start-Sleep -Seconds 1
        $proc = Get-Process -Name 'ironrdp-termsrv' -ErrorAction SilentlyContinue
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
    } -ArgumentList $TaskName, $exeRemote, $runnerRemote, $rdpPasswordRemote, $logOut, $logErr, $ListenerAddr, $CaptureIpc, $CaptureSessionId, $RdpUsername, $RdpDomain | Format-List

    Invoke-Command -Session $session -ScriptBlock {
        param($ListenerAddr, $LogOut, $LogErr, $TailLines)

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
    } -ArgumentList $ListenerAddr, $logOut, $logErr, $TailLines

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
