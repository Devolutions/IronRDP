[CmdletBinding()]
param(
    [string] $AgentPath = (Join-Path $env:GITHUB_WORKSPACE 'target\release\ironrdp-agent.exe'),

    [string] $Endpoint = "pipe:ironrdp-agent-ci-$PID",

    [string] $ArtifactsDir = (Join-Path $env:GITHUB_WORKSPACE 'artifacts\agentic-rdp'),

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

    & $AgentPath --endpoint $Endpoint --no-spawn-daemon @Arguments
}

New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null

$logPath = Join-Path $ArtifactsDir 'ironrdp-agent.log'
$stdoutPath = Join-Path $ArtifactsDir 'ironrdp-agent.stdout.log'
$stderrPath = Join-Path $ArtifactsDir 'ironrdp-agent.stderr.log'

$process = Start-Process `
    -FilePath $AgentPath `
    -ArgumentList @('--endpoint', $Endpoint, '--log-level', $LogLevel, '--log-file', $logPath, '--no-spawn-daemon', 'daemon') `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath `
    -PassThru

$deadline = (Get-Date).AddSeconds(30)
do {
    try {
        Invoke-Agent status | Out-Null
        $state = [pscustomobject]@{
            ProcessId = $process.Id
            Endpoint = $Endpoint
            AgentPath = $AgentPath
            LogPath = $logPath
            StandardOutputPath = $stdoutPath
            StandardErrorPath = $stderrPath
        }
        $state | ConvertTo-Json -Depth 4 | Set-Content -Path $StatePath -Encoding utf8NoBOM
        $state | ConvertTo-Json -Compress
        return
    }
    catch {
        Start-Sleep -Milliseconds 250
    }
} while ((Get-Date) -lt $deadline)

throw "Timed out waiting for ironrdp-agent daemon on $Endpoint"
