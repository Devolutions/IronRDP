BeforeAll {
    $termsrvRoot = Split-Path -Parent $PSScriptRoot
    $screenshotHelpers = Join-Path $termsrvRoot 'crates\ironrdp-termsrv\scripts\e2e-test-screenshot.helpers.ps1'
    . $screenshotHelpers
}

Describe 'Owned client socket identity' {
    It 'uses only the daemon at the owned endpoint and reports its TCP source' {
        # oracle: mutant an unrelated agent daemon supplies the connection tuple.
        $agentPath = Join-Path $TestDrive 'ironrdp-agent.exe'
        Mock Get-CimInstance ({
            [pscustomobject]@{ ExecutablePath = $agentPath; CommandLine = 'ironrdp-agent --endpoint owned-pipe daemon-child'; ProcessId = 42 }
            [pscustomobject]@{ ExecutablePath = $agentPath; CommandLine = 'ironrdp-agent --endpoint foreign-pipe daemon-child'; ProcessId = 99 }
        }.GetNewClosure())
        Mock Get-NetTCPConnection {
            param($OwningProcess)
            if ($OwningProcess -ne 42) { throw 'Wrong daemon' }
            [pscustomobject]@{ RemoteAddress = '127.0.0.1'; RemotePort = 4489; LocalAddress = '127.0.0.1'; LocalPort = 4567 }
        }
        Get-E2EAgentClientEndpoint -Endpoint 'owned-pipe' -AgentPath $agentPath -Hostname '127.0.0.1' -Port 4489 |
            Should -Be '[127.0.0.1]:4567'
    }
}

Describe 'Screenshot publication' {
    It 'does not publish output after a failed client run' {
        # oracle: mutant nonzero client exit publishes output
        $outputPath = Join-Path $TestDrive 'screenshot.png'
        $temporaryPath = Join-Path $TestDrive 'screenshot-run.tmp.png'
        [System.IO.File]::WriteAllText($outputPath, 'previous successful screenshot')
        [System.IO.File]::WriteAllText($temporaryPath, 'partial failed screenshot')

        $published = Publish-ScreenshotOutput -ExitCode 1 -TemporaryPath $temporaryPath -OutputPath $outputPath

        $published | Should -BeFalse
        Get-Content -LiteralPath $outputPath -Raw | Should -Be 'previous successful screenshot'
        Test-Path -LiteralPath $temporaryPath | Should -BeFalse
    }

    It 'publishes a current screenshot after a successful client run' {
        # oracle: mutant successful client output is not moved to the requested path
        $outputPath = Join-Path $TestDrive 'screenshot.png'
        $temporaryPath = Join-Path $TestDrive 'screenshot-run.tmp.png'
        [System.IO.File]::WriteAllText($temporaryPath, 'current successful screenshot')

        $published = Publish-ScreenshotOutput -ExitCode 0 -TemporaryPath $temporaryPath -OutputPath $outputPath

        $published | Should -BeTrue
        Get-Content -LiteralPath $outputPath -Raw | Should -Be 'current successful screenshot'
        Test-Path -LiteralPath $temporaryPath | Should -BeFalse
    }
}

Describe 'E2E safety options' {
    It 'requires an exact session ID before enabling session cleanup' {
        # oracle: mutant cleanup is allowed without a selected test session
        {
            Assert-E2ESafetyOptions -EnableFreshSessionCleanup $true `
                -AggressiveFreshSessionCleanup $false -FreshSessionCleanupSessionId 0 `
                -AllowHyperVPowerCycle $false
        } | Should -Throw '*requires a specific -FreshSessionCleanupSessionId*'
    }

    It 'requires session cleanup to be enabled before accepting a session ID' {
        # oracle: mutant a supplied session ID bypasses the cleanup opt-in
        {
            Assert-E2ESafetyOptions -EnableFreshSessionCleanup $false `
                -AggressiveFreshSessionCleanup $false -FreshSessionCleanupSessionId 8 `
                -AllowHyperVPowerCycle $false
        } | Should -Throw '*requires -EnableFreshSessionCleanup*'
    }

    It 'requires basic cleanup opt-in before aggressive cleanup' {
        # oracle: mutant aggressive cleanup is authorized without basic cleanup opt-in
        {
            Assert-E2ESafetyOptions -EnableFreshSessionCleanup $false `
                -AggressiveFreshSessionCleanup $true -FreshSessionCleanupSessionId 0 `
                -AllowHyperVPowerCycle $false
        } | Should -Throw '*AggressiveFreshSessionCleanup requires -EnableFreshSessionCleanup*'
    }

    It 'requires an explicit VM name before allowing a Hyper-V power cycle' {
        # oracle: mutant Hyper-V restart is allowed using an inferred VM name
        {
            Assert-E2ESafetyOptions -EnableFreshSessionCleanup $false `
                -AggressiveFreshSessionCleanup $false -FreshSessionCleanupSessionId 0 `
                -AllowHyperVPowerCycle $true -HyperVVmName '   '
        } | Should -Throw '*requires an explicit -HyperVVmName*'
    }

    It 'accepts session ID 1 as an explicit interactive session' {
        # oracle: mutant valid interactive session ID 1 is rejected
        {
            Assert-E2ESafetyOptions -EnableFreshSessionCleanup $true `
                -AggressiveFreshSessionCleanup $false -FreshSessionCleanupSessionId 1 `
                -AllowHyperVPowerCycle $false
        } | Should -Not -Throw
    }

    It 'accepts cleanup and power-cycle authorization only with explicit targets' {
        # oracle: mutant explicit cleanup and restart targets are rejected
        {
            Assert-E2ESafetyOptions -EnableFreshSessionCleanup $true `
                -AggressiveFreshSessionCleanup $true -FreshSessionCleanupSessionId 8 `
                -AllowHyperVPowerCycle $true -HyperVVmName 'MyTestVm'
        } | Should -Not -Throw
    }
}

Describe 'Screenshot content validation' {
    BeforeAll { Add-Type -AssemblyName System.Drawing }

    It 'rejects a black image based on its actual pixels' {
        # oracle: mutant black screenshot passes the pixel-content classifier.
        $bitmap = [System.Drawing.Bitmap]::new(2, 2)
        try {
            $analysis = Get-ScreenshotPixelAnalysis -Bitmap $bitmap
            $analysis.HasNonBlackPixel | Should -BeFalse
            $analysis.IsUniform | Should -BeTrue
            $analysis.HasMeaningfulContent | Should -BeFalse
        } finally {
            $bitmap.Dispose()
        }
    }

    It 'rejects a uniform nonblack image based on its actual pixels' {
        # oracle: mutant uniform nonblack screenshot passes the pixel-content classifier.
        $bitmap = [System.Drawing.Bitmap]::new(2, 2)
        try {
            for ($y = 0; $y -lt $bitmap.Height; $y++) {
                for ($x = 0; $x -lt $bitmap.Width; $x++) {
                    $bitmap.SetPixel($x, $y, [System.Drawing.Color]::White)
                }
            }
            $analysis = Get-ScreenshotPixelAnalysis -Bitmap $bitmap
            $analysis.HasNonBlackPixel | Should -BeTrue
            $analysis.IsUniform | Should -BeTrue
            $analysis.HasMeaningfulContent | Should -BeFalse
        } finally {
            $bitmap.Dispose()
        }
    }

    It 'accepts a varying nonblack image based on its actual pixels' {
        # oracle: mutant meaningful screenshot is rejected by the pixel-content classifier.
        $bitmap = [System.Drawing.Bitmap]::new(2, 2)
        try {
            $bitmap.SetPixel(0, 0, [System.Drawing.Color]::White)
            $analysis = Get-ScreenshotPixelAnalysis -Bitmap $bitmap
            $analysis.SampleCount | Should -Be 4
            $analysis.HasNonBlackPixel | Should -BeTrue
            $analysis.IsUniform | Should -BeFalse
            $analysis.HasMeaningfulContent | Should -BeTrue
        } finally {
            $bitmap.Dispose()
        }
    }
}
