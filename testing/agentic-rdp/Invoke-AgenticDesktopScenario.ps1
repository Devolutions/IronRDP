[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $SessionId,

    [string] $AgentPath = (Join-Path $env:GITHUB_WORKSPACE 'target\release\ironrdp-agent.exe'),

    [string] $Endpoint = "pipe:ironrdp-agent-ci-$PID",

    [string] $ArtifactsDir = (Join-Path $env:GITHUB_WORKSPACE 'artifacts\agentic-rdp'),

    [string] $DesktopSize = '1920x1080'
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

function Get-AgentStatus {
    $statusJson = Invoke-Agent status --session $SessionId
    return ($statusJson | ConvertFrom-Json)
}

function Test-PngScreenshot {
    param(
        [Parameter(Mandatory)]
        [string] $Path,

        [Parameter(Mandatory)]
        [int] $ExpectedWidth,

        [Parameter(Mandatory)]
        [int] $ExpectedHeight
    )

    Add-Type -AssemblyName System.Drawing
    $bitmap = [System.Drawing.Bitmap]::new($Path)
    try {
        if ($bitmap.Width -ne $ExpectedWidth -or $bitmap.Height -ne $ExpectedHeight) {
            throw "Screenshot is $($bitmap.Width)x$($bitmap.Height), expected ${ExpectedWidth}x${ExpectedHeight}"
        }

        $firstPixel = $bitmap.GetPixel(0, 0).ToArgb()
        $hasDifferentPixel = $false
        $stepX = [Math]::Max(1, [Math]::Floor($bitmap.Width / 64))
        $stepY = [Math]::Max(1, [Math]::Floor($bitmap.Height / 64))

        for ($y = 0; $y -lt $bitmap.Height; $y += $stepY) {
            for ($x = 0; $x -lt $bitmap.Width; $x += $stepX) {
                if ($bitmap.GetPixel($x, $y).ToArgb() -ne $firstPixel) {
                    $hasDifferentPixel = $true
                    break
                }
            }

            if ($hasDifferentPixel) {
                break
            }
        }

        if (-not $hasDifferentPixel) {
            throw 'Screenshot appears uniform; the framebuffer is likely blank'
        }
    }
    finally {
        $bitmap.Dispose()
    }
}

New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null

if ($DesktopSize -notmatch '^(?<width>[1-9][0-9]*)[xX](?<height>[1-9][0-9]*)$') {
    throw "DesktopSize must use WxH format, got '$DesktopSize'"
}

$expectedWidth = [int] $Matches.width
$expectedHeight = [int] $Matches.height

$initialStatus = Get-AgentStatus
$initialFrameSequence = [uint64] $initialStatus.frame_sequence

Invoke-Agent mouse --session $SessionId move --x 200 --y 200 | Out-Null
Invoke-Agent mouse --session $SessionId click --button left | Out-Null
Invoke-Agent keyboard --session $SessionId shortcut --scancodes '0xE05B,0x13' | Out-Null
Start-Sleep -Seconds 1
Invoke-Agent keyboard --session $SessionId text --text 'msedge.exe about:blank' | Out-Null
Invoke-Agent keyboard --session $SessionId key --scancode 0x1c | Out-Null
Invoke-Agent keyboard --session $SessionId key --scancode 0x1c --release | Out-Null

Start-Sleep -Seconds 8
Invoke-Agent wait-frame --session $SessionId --timeout-ms 60000 --after-frame $initialFrameSequence | Out-Null

$screenshotPath = Join-Path $ArtifactsDir 'agent-desktop.png'
Invoke-Agent screenshot --session $SessionId --output $screenshotPath | Out-Null
Test-PngScreenshot -Path $screenshotPath -ExpectedWidth $expectedWidth -ExpectedHeight $expectedHeight

$finalStatus = Get-AgentStatus
$result = [pscustomobject]@{
    SessionId = $SessionId
    InitialFrameSequence = $initialFrameSequence
    FinalFrameSequence = $finalStatus.frame_sequence
    Width = $finalStatus.width
    Height = $finalStatus.height
    ScreenshotPath = $screenshotPath
}

$result | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $ArtifactsDir 'agentic-desktop-scenario.json') -Encoding utf8NoBOM
$result | ConvertTo-Json -Compress
