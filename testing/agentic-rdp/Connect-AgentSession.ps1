[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $UserName,

    [Parameter(Mandatory)]
    [string] $Password,

    [string] $HostName = '127.0.0.1',

    [int] $Port = 3389,

    [string] $DesktopSize = '1920x1080',

    [string] $AgentPath = (Join-Path $env:GITHUB_WORKSPACE 'target\release\ironrdp-agent.exe'),

    [string] $Endpoint = "\\.\pipe\ironrdp-agent-ci-$PID",

    [string] $ArtifactsDir = (Join-Path $env:GITHUB_WORKSPACE 'artifacts\agentic-rdp'),

    [string] $StatePath = (Join-Path $ArtifactsDir 'agent-session.json')
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

function Get-SessionStatus {
    $statusOutput = Invoke-Agent status
    $statusText = $statusOutput -join "`n"
    $statusText | Set-Content -Path (Join-Path $ArtifactsDir 'agent-session-status.txt') -Encoding utf8NoBOM

    $state = if ($statusText -match '(?m)^state:\s*(\S+)') { $Matches[1] } else { $null }
    $width = $null
    $height = $null
    if ($statusText -match '(?m)^resolution:\s*(\d+)x(\d+)') {
        $width = [int] $Matches[1]
        $height = [int] $Matches[2]
    }

    return [pscustomobject]@{
        state = $state
        width = $width
        height = $height
    }
}

function Assert-DesktopSize {
    param(
        [Parameter(Mandatory)]
        [object] $Status,

        [Parameter(Mandatory)]
        [int] $ExpectedWidth,

        [Parameter(Mandatory)]
        [int] $ExpectedHeight
    )

    if ([int] $Status.width -ne $ExpectedWidth -or [int] $Status.height -ne $ExpectedHeight) {
        throw "RDP framebuffer is $($Status.width)x$($Status.height), expected ${ExpectedWidth}x${ExpectedHeight}"
    }
}

New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null

if ($DesktopSize -notmatch '^(?<width>[1-9][0-9]*)[xX](?<height>[1-9][0-9]*)$') {
    throw "DesktopSize must use WxH format, got '$DesktopSize'"
}

$expectedWidth = [int] $Matches.width
$expectedHeight = [int] $Matches.height
$destination = "${HostName}:$Port"
$connectOutput = Invoke-Agent connect `
        --server $destination `
        --username $UserName `
        --password $Password `
        --prop "desktopwidth:i:$expectedWidth" `
        --prop "desktopheight:i:$expectedHeight" `
        --prop 'enablecredsspsupport:i:0' `
        --prop 'ironrdp_autologon:i:1' `
        --prop 'compression:i:0' `
        --prop 'ironrdp_colordepth:i:16' `
        --prop 'ironrdp_serverpointer:i:0'
$connectOutput | Set-Content -Path (Join-Path $ArtifactsDir 'agent-connect.txt') -Encoding utf8NoBOM

$deadline = (Get-Date).AddSeconds(120)
do {
    $status = Get-SessionStatus
    if ($status.state -eq 'Connected' -and $null -ne $status.width -and $null -ne $status.height) {
        break
    }

    if ($status.state -eq 'Failed') {
        throw 'RDP connection failed; inspect the agent session log'
    }

    Start-Sleep -Milliseconds 500
} while ((Get-Date) -lt $deadline)

if ($status.state -ne 'Connected') {
    throw 'Timed out waiting for the RDP session to produce a frame'
}

Assert-DesktopSize -Status $status -ExpectedWidth $expectedWidth -ExpectedHeight $expectedHeight

$state = [pscustomobject]@{
    SessionId = 'current'
    Endpoint = $Endpoint
    HostName = $HostName
    Port = $Port
    UserName = $UserName
    RequestedDesktopSize = $DesktopSize
    Width = $status.width
    Height = $status.height
    StatePath = $StatePath
}

$state | ConvertTo-Json -Depth 6 | Set-Content -Path $StatePath -Encoding utf8NoBOM
$state | ConvertTo-Json -Compress
