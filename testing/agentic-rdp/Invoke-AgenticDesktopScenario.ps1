[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string] $SessionId,

    [string] $AgentPath = (Join-Path $env:GITHUB_WORKSPACE 'target\release\ironrdp-agent.exe'),

    [string] $Endpoint = "\\.\pipe\ironrdp-agent-ci-$PID",

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

    & $AgentPath --endpoint $Endpoint @Arguments
}

function Get-AgentStatus {
    $statusText = (Invoke-Agent status) -join "`n"
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

function Send-Scancode {
    param([string] $Scancode, [bool] $Pressed)
    Invoke-Agent key-scancode --scancode $Scancode --pressed $Pressed.ToString().ToLowerInvariant() | Out-Null
}

function Send-Text {
    param([string] $Text)
    foreach ($character in $Text.ToCharArray()) {
        Invoke-Agent key-unicode --char ([string] $character) --pressed true | Out-Null
        Invoke-Agent key-unicode --char ([string] $character) --pressed false | Out-Null
    }
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

Invoke-Agent mouse-move --x 200 --y 200 | Out-Null
Invoke-Agent mouse-button --button left --pressed true | Out-Null
Invoke-Agent mouse-button --button left --pressed false | Out-Null
Send-Scancode -Scancode '0xE05B' -Pressed $true
Send-Scancode -Scancode '0x13' -Pressed $true
Send-Scancode -Scancode '0x13' -Pressed $false
Send-Scancode -Scancode '0xE05B' -Pressed $false
Start-Sleep -Seconds 1
Send-Text -Text 'msedge.exe about:blank'
Send-Scancode -Scancode '0x1c' -Pressed $true
Send-Scancode -Scancode '0x1c' -Pressed $false

Start-Sleep -Seconds 8

$screenshotPath = Join-Path $ArtifactsDir 'agent-desktop.png'
Invoke-Agent screenshot $screenshotPath | Out-Null
Test-PngScreenshot -Path $screenshotPath -ExpectedWidth $expectedWidth -ExpectedHeight $expectedHeight

$finalStatus = Get-AgentStatus
$result = [pscustomobject]@{
    SessionId = $SessionId
    Width = $finalStatus.width
    Height = $finalStatus.height
    ScreenshotPath = $screenshotPath
}

$result | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $ArtifactsDir 'agentic-desktop-scenario.json') -Encoding utf8NoBOM
$result | ConvertTo-Json -Compress
