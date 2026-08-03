[CmdletBinding()]
param(
    [string] $WorkspaceRoot = $(
        if ([string]::IsNullOrWhiteSpace($env:GITHUB_WORKSPACE)) {
            [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
        }
        else {
            $env:GITHUB_WORKSPACE
        }
    ),

    [string] $AgentPath = (Join-Path $WorkspaceRoot 'target\release\ironrdp-agent.exe'),

    [string] $Endpoint = "\\.\pipe\ironrdp-agent-ci-$PID",

    [string] $ArtifactsDir = (Join-Path $WorkspaceRoot 'artifacts\agentic-rdp'),

    [string] $StatePath = (Join-Path $ArtifactsDir 'agent-daemon.json'),

    [string] $LogLevel = 'debug',

    [ValidateRange(1, 30000)]
    [int] $ProbeTimeoutMilliseconds = 3000
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Test-AgentReady {
    param(
        [Parameter(Mandatory)]
        [int] $TimeoutMilliseconds
    )

    $probe = Start-Process `
        -FilePath $AgentPath `
        -ArgumentList @('--endpoint', $Endpoint, 'status') `
        -PassThru
    try {
        if (-not $probe.WaitForExit($TimeoutMilliseconds)) {
            Stop-Process -Id $probe.Id -Force
            $probe.WaitForExit()
            return $false
        }

        return $probe.ExitCode -eq 0
    }
    finally {
        if (-not $probe.HasExited) {
            Stop-Process -Id $probe.Id -Force
        }
    }
}

New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null

$stdoutPath = Join-Path $ArtifactsDir 'ironrdp-agent.stdout.log'
$stderrPath = Join-Path $ArtifactsDir 'ironrdp-agent.stderr.log'

$previousLogFilter = $env:IRONRDP_LOG
$env:IRONRDP_LOG = $LogLevel
try {
    $process = Start-Process `
        -FilePath $AgentPath `
        -ArgumentList @('--endpoint', $Endpoint, 'daemon-start') `
        -RedirectStandardOutput $stdoutPath `
        -RedirectStandardError $stderrPath `
        -PassThru
}
finally {
    $env:IRONRDP_LOG = $previousLogFilter
}

$state = [pscustomobject]@{
    ProcessId = $process.Id
    Endpoint = $Endpoint
    AgentPath = $AgentPath
    LogPath = $stderrPath
    StandardOutputPath = $stdoutPath
    StandardErrorPath = $stderrPath
}
$state | ConvertTo-Json -Depth 4 | Set-Content -Path $StatePath -Encoding utf8NoBOM

$deadline = (Get-Date).AddSeconds(30)
do {
    if ($process.HasExited) {
        $errorOutput = if (Test-Path $stderrPath) {
            Get-Content -Path $stderrPath -Raw
        }
        else {
            ''
        }
        throw "ironrdp-agent daemon exited before it became ready: $errorOutput"
    }

    try {
        if (Test-AgentReady -TimeoutMilliseconds $ProbeTimeoutMilliseconds) {
            $state | ConvertTo-Json -Compress
            return
        }
    }
    catch {
        Write-Warning "Could not probe ironrdp-agent daemon: $($_.Exception.Message)"
    }
    Start-Sleep -Milliseconds 250
} while ((Get-Date) -lt $deadline)

try {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
    }
}
catch {
    Write-Warning "Could not stop ironrdp-agent after startup timeout: $($_.Exception.Message)"
}

throw "Timed out waiting for ironrdp-agent daemon on $Endpoint"
