[CmdletBinding(DefaultParameterSetName = 'Register')]
param(
    [Parameter(ParameterSetName = 'Register')]
    [switch] $Register,

    [Parameter(ParameterSetName = 'RunServer')]
    [switch] $RunServer,

    [Parameter(ParameterSetName = 'Wait')]
    [switch] $Wait,

    [Parameter(ParameterSetName = 'Register')]
    [Parameter(ParameterSetName = 'RunServer')]
    [Parameter(ParameterSetName = 'Wait')]
    [string] $EndpointPath = (Join-Path $env:GITHUB_WORKSPACE 'artifacts\agentic-rdp\pshost-endpoint.json'),

    [Parameter(ParameterSetName = 'Register')]
    [Parameter(ParameterSetName = 'RunServer')]
    [int] $Port = 45985,

    [Parameter(ParameterSetName = 'Register')]
    [Parameter(ParameterSetName = 'RunServer')]
    [string] $ArtifactsDir = (Join-Path $env:GITHUB_WORKSPACE 'artifacts\agentic-rdp'),

    [Parameter(ParameterSetName = 'Register')]
    [string] $TaskName = 'IronRdpAgenticPSHost',

    [Parameter(ParameterSetName = 'Wait')]
    [int] $TimeoutSeconds = 120
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Ensure-AwakeCodingModule {
    if (Get-Module -ListAvailable -Name AwakeCoding.PSRemoting) {
        return
    }

    $repository = Get-PSRepository -Name PSGallery -ErrorAction SilentlyContinue
    if ($null -ne $repository -and $repository.InstallationPolicy -ne 'Trusted') {
        Set-PSRepository -Name PSGallery -InstallationPolicy Trusted
    }

    $installModuleParameters = @{
        Name = 'AwakeCoding.PSRemoting'
        Scope = 'CurrentUser'
        Repository = 'PSGallery'
        Force = $true
        AllowClobber = $true
    }

    if ((Get-Command Install-Module).Parameters.ContainsKey('AcceptLicense')) {
        $installModuleParameters['AcceptLicense'] = $true
    }

    Install-Module @installModuleParameters
}

if ($Register) {
    New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null
    Remove-Item -Path $EndpointPath -Force -ErrorAction SilentlyContinue
    Ensure-AwakeCodingModule

    $pwshPath = (Get-Command pwsh -ErrorAction Stop).Source
    $actionArguments = @(
        '-NoLogo',
        '-NoProfile',
        '-ExecutionPolicy', 'Bypass',
        '-File', "`"$PSCommandPath`"",
        '-RunServer',
        '-EndpointPath', "`"$EndpointPath`"",
        '-Port', $Port,
        '-ArtifactsDir', "`"$ArtifactsDir`""
    ) -join ' '

    $taskAction = New-ScheduledTaskAction -Execute $pwshPath -Argument $actionArguments
    $taskTrigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:COMPUTERNAME\$env:USERNAME"
    $taskPrincipal = New-ScheduledTaskPrincipal -UserId "$env:COMPUTERNAME\$env:USERNAME" -LogonType Interactive -RunLevel Highest
    $taskSettings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero)
    Register-ScheduledTask -TaskName $TaskName -Action $taskAction -Trigger $taskTrigger -Principal $taskPrincipal -Settings $taskSettings -Force | Out-Null

    [pscustomobject]@{
        TaskName = $TaskName
        EndpointPath = $EndpointPath
        Port = $Port
    } | ConvertTo-Json -Compress
    return
}

if ($RunServer) {
    New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null
    $serverLogPath = Join-Path $ArtifactsDir 'pshost-server.log'

    try {
        Start-Transcript -Path $serverLogPath -Force | Out-Null
        Import-Module AwakeCoding.PSRemoting -ErrorAction Stop
        $server = Start-PSHostServer -TransportType TCP -Port $Port
        $process = Get-Process -Id $PID

        [pscustomobject]@{
            Transport = 'TCP'
            HostName = '127.0.0.1'
            Port = $Port
            ProcessId = $PID
            SessionId = $process.SessionId
            UserName = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
            State = [string] $server.State
            StartedAt = (Get-Date).ToString('o')
        } | ConvertTo-Json -Depth 4 | Set-Content -Path $EndpointPath -Encoding utf8NoBOM

        while ($true) {
            Start-Sleep -Seconds 5
            $current = Get-PSHostServer -Port $Port -ErrorAction SilentlyContinue
            if ($null -eq $current -or [string] $current.State -ne 'Running') {
                throw "PSHostServer on port $Port is no longer running"
            }
        }
    }
    catch {
        [pscustomobject]@{
            Error = $_.Exception.Message
            ProcessId = $PID
            FailedAt = (Get-Date).ToString('o')
        } | ConvertTo-Json -Depth 4 | Set-Content -Path $EndpointPath -Encoding utf8NoBOM
        throw
    }
    finally {
        Stop-Transcript -ErrorAction SilentlyContinue | Out-Null
    }
}

if ($Wait) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        if (Test-Path $EndpointPath) {
            $endpoint = Get-Content -Path $EndpointPath -Raw | ConvertFrom-Json
            if ($endpoint.PSObject.Properties.Name -contains 'Error') {
                throw "Interactive PSHostServer failed: $($endpoint.Error)"
            }

            $endpoint | ConvertTo-Json -Compress
            return
        }

        Start-Sleep -Seconds 2
    } while ((Get-Date) -lt $deadline)

    throw "Timed out waiting for interactive PSHostServer endpoint marker: $EndpointPath"
}
