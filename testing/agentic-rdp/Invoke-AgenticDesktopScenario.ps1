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

    $output = & $AgentPath --endpoint $Endpoint @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        $message = ($output | Out-String).Trim()
        throw "ironrdp-agent $($Arguments -join ' ') failed: $message"
    }
    return $output
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

function Test-ScreenshotCenterColor {
    param(
        [Parameter(Mandatory)]
        [string] $Path,

        [Parameter(Mandatory)]
        [System.Drawing.Color] $Expected,

        [int] $Tolerance = 16
    )

    $bitmap = [System.Drawing.Bitmap]::new($Path)
    try {
        $pixel = $bitmap.GetPixel([int] ($bitmap.Width / 2), [int] ($bitmap.Height / 2))
        return ([Math]::Abs($pixel.R - $Expected.R) -le $Tolerance) -and
            ([Math]::Abs($pixel.G - $Expected.G) -le $Tolerance) -and
            ([Math]::Abs($pixel.B - $Expected.B) -le $Tolerance)
    }
    finally {
        $bitmap.Dispose()
    }
}

# Edge's first-run welcome dialog would otherwise cover the page we navigate to.
function Set-EdgeFirstRunPolicy {
    $policyPath = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge'
    $previous = $null
    try {
        if (Test-Path $policyPath) {
            # GetValue returns $null for a missing value instead of tripping StrictMode on property access.
            $previous = (Get-Item -Path $policyPath).GetValue('HideFirstRunExperience', $null)
        }
        else {
            New-Item -Path $policyPath -Force | Out-Null
        }
        New-ItemProperty -Path $policyPath -Name 'HideFirstRunExperience' -PropertyType DWord -Value 1 -Force | Out-Null
    }
    catch {
        Write-Warning "Could not set Edge HideFirstRunExperience policy: $($_.Exception.Message)"
    }

    return [pscustomobject]@{
        Path = $policyPath
        Previous = $previous
    }
}

function Restore-EdgeFirstRunPolicy {
    param([Parameter(Mandatory)] $State)

    try {
        if ($null -eq $State.Previous) {
            Remove-ItemProperty -Path $State.Path -Name 'HideFirstRunExperience' -ErrorAction SilentlyContinue
        }
        else {
            Set-ItemProperty -Path $State.Path -Name 'HideFirstRunExperience' -Value $State.Previous
        }
    }
    catch {
        Write-Warning "Could not restore Edge HideFirstRunExperience policy: $($_.Exception.Message)"
    }
}

New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null

if ($DesktopSize -notmatch '^(?<width>[1-9][0-9]*)[xX](?<height>[1-9][0-9]*)$') {
    throw "DesktopSize must use WxH format, got '$DesktopSize'"
}

$expectedWidth = [int] $Matches.width
$expectedHeight = [int] $Matches.height

Add-Type -AssemblyName System.Drawing

# The page is filled with a known color so the screenshot proves that the typed command
# reached the session and the browser rendered it, not just that the framebuffer is non-blank.
$markerColor = [System.Drawing.Color]::FromArgb(31, 111, 235)
$markerPage = "data:text/html,<body style=background:rgb($($markerColor.R),$($markerColor.G),$($markerColor.B));color:white;font-family:sans-serif><h1>IronRDP agentic RDP</h1></body>"
$screenshotPath = Join-Path $ArtifactsDir 'agent-desktop.png'

$edgePolicy = Set-EdgeFirstRunPolicy
try {
    Invoke-Agent mouse-move --x 200 --y 200 | Out-Null
    Invoke-Agent mouse-button --button left --pressed true | Out-Null
    Invoke-Agent mouse-button --button left --pressed false | Out-Null
    Send-Scancode -Scancode '0xE05B' -Pressed $true
    Send-Scancode -Scancode '0x13' -Pressed $true
    Send-Scancode -Scancode '0x13' -Pressed $false
    Send-Scancode -Scancode '0xE05B' -Pressed $false
    Start-Sleep -Seconds 1
    Send-Text -Text "msedge.exe --start-maximized --no-first-run --no-default-browser-check `"$markerPage`""
    Send-Scancode -Scancode '0x1c' -Pressed $true
    Send-Scancode -Scancode '0x1c' -Pressed $false

    $deadline = (Get-Date).AddSeconds(45)
    $markerVisible = $false
    do {
        Start-Sleep -Seconds 3
        Invoke-Agent screenshot $screenshotPath | Out-Null
        $markerVisible = Test-ScreenshotCenterColor -Path $screenshotPath -Expected $markerColor
    } while (-not $markerVisible -and (Get-Date) -lt $deadline)

    if (-not $markerVisible) {
        throw "Marker page is not visible at the center of $screenshotPath"
    }
}
finally {
    Restore-EdgeFirstRunPolicy -State $edgePolicy
}

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
