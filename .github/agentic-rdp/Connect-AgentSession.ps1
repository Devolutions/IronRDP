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

    [string] $Endpoint = "pipe:ironrdp-agent-ci-$PID",

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

    & $AgentPath --endpoint $Endpoint --no-spawn-daemon @Arguments
}

function Get-SessionStatus {
    param(
        [Parameter(Mandatory)]
        [string] $SessionId
    )

    $statusJson = Invoke-Agent status --session $SessionId
    $statusJson | Set-Content -Path (Join-Path $ArtifactsDir 'agent-session-status.json') -Encoding utf8NoBOM
    return ($statusJson | ConvertFrom-Json)
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
$passwordEnvironmentVariable = 'IRONRDP_AGENT_LOCAL_RDP_PASSWORD'
$env:IRONRDP_AGENT_LOCAL_RDP_PASSWORD = $Password

try {
    $destination = "${HostName}:$Port"
    $connectJson = Invoke-Agent connect $destination `
        --username $UserName `
        --password-env $passwordEnvironmentVariable `
        --desktop-size $DesktopSize `
        --no-credssp `
        --autologon `
        --compression-enabled false `
        --color-depth 16 `
        --no-server-pointer
    $connectJson | Set-Content -Path (Join-Path $ArtifactsDir 'agent-connect.json') -Encoding utf8NoBOM

    $connect = $connectJson | ConvertFrom-Json
    $sessionId = [string] $connect.session_id

    Invoke-Agent wait-frame --session $sessionId --timeout-ms 120000 | Out-Null
    $status = Get-SessionStatus -SessionId $sessionId

    if ([int] $status.width -ne $expectedWidth -or [int] $status.height -ne $expectedHeight) {
        $beforeResizeFrame = [uint64] $status.frame_sequence
        Invoke-Agent resize --session $sessionId --width $expectedWidth --height $expectedHeight --scale 100 | Out-Null
        Invoke-Agent wait-frame --session $sessionId --timeout-ms 60000 --after-frame $beforeResizeFrame | Out-Null
        $status = Get-SessionStatus -SessionId $sessionId
    }

    Assert-DesktopSize -Status $status -ExpectedWidth $expectedWidth -ExpectedHeight $expectedHeight

    $state = [pscustomobject]@{
        SessionId = $sessionId
        Endpoint = $Endpoint
        HostName = $HostName
        Port = $Port
        UserName = $UserName
        RequestedDesktopSize = $DesktopSize
        Width = $status.width
        Height = $status.height
        FrameSequence = $status.frame_sequence
        StatePath = $StatePath
    }

    $state | ConvertTo-Json -Depth 6 | Set-Content -Path $StatePath -Encoding utf8NoBOM
    $state | ConvertTo-Json -Compress
}
finally {
    Remove-Item Env:\IRONRDP_AGENT_LOCAL_RDP_PASSWORD -ErrorAction SilentlyContinue
}
