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

    [string] $LogLevel = 'debug'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-Agent {
    param(
        [Parameter(ValueFromRemainingArguments)]
        [string[]] $Arguments
    )

    & $AgentPath --endpoint $Endpoint @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "ironrdp-agent command failed with exit code $LASTEXITCODE"
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
    ProcessPath = $process.Path
    ProcessStartTimeUtcTicks = $process.StartTime.ToUniversalTime().Ticks
    Endpoint = $Endpoint
    AgentPath = $AgentPath
    LogPath = $stderrPath
    StandardOutputPath = $stdoutPath
    StandardErrorPath = $stderrPath
}
$state | ConvertTo-Json -Depth 4 | Set-Content -Path $StatePath -Encoding utf8NoBOM

$deadline = (Get-Date).AddSeconds(30)
do {
    try {
        Invoke-Agent status 2>$null | Out-Null
        $state | ConvertTo-Json -Compress
        return
    }
    catch {
        Start-Sleep -Milliseconds 250
    }
} while ((Get-Date) -lt $deadline)

try {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
    }
}
catch {
    Write-Warning "Could not stop ironrdp-agent after startup timeout: $($_.Exception.Message)"
}
finally {
    Remove-Item -Path $StatePath -Force -ErrorAction SilentlyContinue
}

throw "Timed out waiting for ironrdp-agent daemon on $Endpoint"
