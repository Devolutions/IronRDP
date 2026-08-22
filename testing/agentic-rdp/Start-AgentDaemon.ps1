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

    [switch] $SkipCertificateCheck
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-Agent {
    param(
        [Parameter(ValueFromRemainingArguments)]
        [string[]] $Arguments
    )

    & $AgentPath --endpoint $Endpoint @Arguments
}

New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null

$stdoutPath = Join-Path $ArtifactsDir 'ironrdp-agent.stdout.log'
$stderrPath = Join-Path $ArtifactsDir 'ironrdp-agent.stderr.log'

$previousLogFilter = $env:IRONRDP_LOG
$env:IRONRDP_LOG = $LogLevel
try {
    $daemonArguments = @('--endpoint', $Endpoint, 'daemon-start')
    if ($SkipCertificateCheck) {
        $daemonArguments += '--skip-certificate-check'
    }
    $process = Start-Process `
        -FilePath $AgentPath `
        -ArgumentList $daemonArguments `
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
    CertificateCheckSkipped = $SkipCertificateCheck.IsPresent
}
$state | ConvertTo-Json -Depth 4 | Set-Content -Path $StatePath -Encoding utf8NoBOM

$deadline = (Get-Date).AddSeconds(30)
do {
    try {
        Invoke-Agent status | Out-Null
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

throw "Timed out waiting for ironrdp-agent daemon on $Endpoint"
