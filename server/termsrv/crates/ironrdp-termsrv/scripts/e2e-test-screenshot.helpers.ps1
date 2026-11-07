function Assert-E2ESafetyOptions {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$EnableFreshSessionCleanup,

        [Parameter(Mandatory = $true)]
        [bool]$AggressiveFreshSessionCleanup,

        [Parameter(Mandatory = $true)]
        [int]$FreshSessionCleanupSessionId,

        [Parameter(Mandatory = $true)]
        [bool]$AllowHyperVPowerCycle,

        [Parameter()]
        [string]$HyperVVmName = ''
    )

    if ($AggressiveFreshSessionCleanup -and -not $EnableFreshSessionCleanup) {
        throw '-AggressiveFreshSessionCleanup requires -EnableFreshSessionCleanup'
    }

    if ($EnableFreshSessionCleanup -and $FreshSessionCleanupSessionId -lt 1) {
        throw '-EnableFreshSessionCleanup requires a specific -FreshSessionCleanupSessionId greater than 0'
    }

    if (-not $EnableFreshSessionCleanup -and $FreshSessionCleanupSessionId -gt 0) {
        throw '-FreshSessionCleanupSessionId requires -EnableFreshSessionCleanup'
    }

    if ($AllowHyperVPowerCycle -and [string]::IsNullOrWhiteSpace($HyperVVmName)) {
        throw '-AllowHyperVPowerCycle requires an explicit -HyperVVmName'
    }
}

function Publish-ScreenshotOutput {
    param(
        [Parameter(Mandatory = $true)]
        [int]$ExitCode,

        [Parameter(Mandatory = $true)]
        [string]$TemporaryPath,

        [Parameter(Mandatory = $true)]
        [string]$OutputPath
    )

    if (($ExitCode -ne 0) -or (-not (Test-Path -LiteralPath $TemporaryPath))) {
        Remove-Item -LiteralPath $TemporaryPath -Force -ErrorAction SilentlyContinue
        return $false
    }

    Move-Item -LiteralPath $TemporaryPath -Destination $OutputPath -Force
    return $true
}

function Test-ScreenshotHasMeaningfulContent {
    param(
        [Parameter(Mandatory = $true)]
        [bool]$HasNonBlackPixel,

        [Parameter(Mandatory = $true)]
        [bool]$IsUniform
    )

    return $HasNonBlackPixel -and (-not $IsUniform)
}

function Get-ScreenshotPixelAnalysis {
    param([Parameter(Mandatory = $true)]$Bitmap)
    if (($Bitmap.Width -le 0) -or ($Bitmap.Height -le 0)) { throw 'Screenshot dimensions must be positive' }

    $stepX = [Math]::Max([int]($Bitmap.Width / 256), 1)
    $stepY = [Math]::Max([int]($Bitmap.Height / 256), 1)
    $sampleCount = 0
    $hasNonBlackPixel = $false
    $isUniform = $true
    $firstPixelSet = $false
    $firstR = 0
    $firstG = 0
    $firstB = 0

    for ($y = 0; $y -lt $Bitmap.Height; $y += $stepY) {
        for ($x = 0; $x -lt $Bitmap.Width; $x += $stepX) {
            $pixel = $Bitmap.GetPixel($x, $y)
            $sampleCount++

            if (-not $firstPixelSet) {
                $firstR = $pixel.R
                $firstG = $pixel.G
                $firstB = $pixel.B
                $firstPixelSet = $true
            } elseif ($pixel.R -ne $firstR -or $pixel.G -ne $firstG -or $pixel.B -ne $firstB) {
                $isUniform = $false
            }

            if ($pixel.R -ne 0 -or $pixel.G -ne 0 -or $pixel.B -ne 0) {
                $hasNonBlackPixel = $true
            }
        }
    }

    return @{
        SampleCount = $sampleCount
        HasNonBlackPixel = $hasNonBlackPixel
        IsUniform = $isUniform
        HasMeaningfulContent = Test-ScreenshotHasMeaningfulContent `
            -HasNonBlackPixel $hasNonBlackPixel -IsUniform $isUniform
    }
}

function Assert-E2EOutput {
    param([hashtable]$Evidence, [bool]$SkipScreenshot)
    if ($Evidence.preflightBlockedConnectionAttempt) { throw 'E2E connection was blocked by preflight' }
    if (-not $Evidence.remoteLogCollectionSucceeded) { throw 'E2E remote evidence collection failed' }
    if ($SkipScreenshot) { return }
    if (-not $Evidence.screenshotExists) { throw 'E2E screenshot was not produced by this run' }
    if ($Evidence.screenshotFileInfo.Length -le 1000) { throw 'E2E screenshot is suspiciously small' }
    if (-not $Evidence.analysisSucceeded) { throw 'E2E screenshot analysis failed' }
    if (-not $Evidence.hasMeaningfulContent) { throw 'E2E screenshot is blank or uniform' }
}

function Get-E2EAgentClientEndpoint {
    param([string]$Endpoint, [string]$AgentPath, [string]$Hostname, [int]$Port)
    $daemons = @(Get-CimInstance Win32_Process -Filter "Name='ironrdp-agent.exe'" -ErrorAction Stop |
        Where-Object {
            $_.ExecutablePath -eq [IO.Path]::GetFullPath($AgentPath) -and
            $_.CommandLine -match '\bdaemon-child\b' -and $_.CommandLine.Contains($Endpoint)
        })
    if ($daemons.Count -ne 1) { throw 'Could not uniquely identify the owned RDP daemon process' }
    $addresses = @([Net.Dns]::GetHostAddresses($Hostname) | ForEach-Object { $_.ToString() })
    $connections = @(Get-NetTCPConnection -OwningProcess $daemons[0].ProcessId -State Established -ErrorAction Stop |
        Where-Object { $_.RemotePort -eq $Port -and $_.RemoteAddress -in $addresses })
    if ($connections.Count -ne 1) { throw 'Could not uniquely identify the direct RDP TCP connection' }
    "[$($connections[0].LocalAddress)]:$($connections[0].LocalPort)"
}
