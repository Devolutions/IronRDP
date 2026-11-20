BeforeAll {
    . (Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\e2e-agent.helpers.ps1')
}

Describe 'Scoped server certificate trust' {
    BeforeEach {
        $script:agentCall = $null
        Mock Invoke-E2EAgentCommand {
            $script:agentCall = @{
                AgentPath = $AgentPath
                Arguments = $Arguments
                Environment = $Environment
            }
            [pscustomobject]@{ ExitCode = 0; Output = 'trusted fixture' }
        }
    }

    It 'pins the authenticated endpoint in an isolated known-certificate store' {
        # oracle: the trust command must bind a verified fingerprint to only the requested endpoint and store.
        $result = Invoke-E2EAgentCertificateTrust -AgentPath 'agent.exe' -Endpoint 'server.example:4489' `
            -Fingerprint ('ab' * 32) -KnownCertificatesPath 'C:\fixture\known-certificates'

        $result.ExitCode | Should -Be 0
        $script:agentCall.Arguments | Should -Be @('cert', 'trust', 'server.example:4489', ('ab' * 32))
        $script:agentCall.Environment.IRONRDP_AGENT_KNOWN_CERTIFICATES | Should -Be 'C:\fixture\known-certificates'
        Should -Invoke Invoke-E2EAgentCommand -Times 1
    }

    It 'rejects a malformed fingerprint before invoking the client' {
        # oracle: cert trust accepts only SHA-256 fingerprints of 32 bytes.
        {
            Invoke-E2EAgentCertificateTrust -AgentPath 'agent.exe' -Endpoint 'server.example:4489' `
                -Fingerprint 'not-a-fingerprint' -KnownCertificatesPath 'C:\fixture\known-certificates'
        } | Should -Throw
        Should -Invoke Invoke-E2EAgentCommand -Times 0
    }

    It 'rejects an empty certificate store path instead of using the persistent default' {
        # oracle: the trust command must update only the caller-selected isolated store.
        {
            Invoke-E2EAgentCertificateTrust -AgentPath 'agent.exe' -Endpoint 'server.example:4489' `
                -Fingerprint ('ab' * 32) -KnownCertificatesPath ''
        } | Should -Throw
        Should -Invoke Invoke-E2EAgentCommand -Times 0
    }
}

Describe 'Pinned certificate use' {
    It 'uses the isolated certificate store for every agent command in the screenshot session' {
        # oracle: the client must consult the same temporary store that the authenticated trust command updated.
        $script:agentEnvironments = [Collections.Generic.List[hashtable]]::new()
        Mock Invoke-E2EAgentCommand {
            $script:agentEnvironments.Add($Environment)
            $output = 'ok'
            if ($Arguments -contains 'status') {
                $output = 'state: Connected'
            }
            if ($Arguments -contains 'screenshot') {
                $screenshotIndex = [Array]::IndexOf($Arguments, 'screenshot')
                Set-Content -LiteralPath $Arguments[$screenshotIndex + 1] -Value 'fixture frame'
            }
            [pscustomobject]@{ ExitCode = 0; Output = $output }
        }

        $knownCertificatesPath = 'C:\fixture\known-certificates'
        $result = Invoke-E2EAgentScreenshot -AgentPath 'agent.exe' -Server 'server.example:4489' `
            -Username 'fixture-user' -Password 'fixture-password' -OutputPath (Join-Path $TestDrive 'pinned.png') `
            -LogPath (Join-Path $TestDrive 'pinned.log') -AfterFirstGraphicsSeconds 0 `
            -KnownCertificatesPath $knownCertificatesPath

        $result.ExitCode | Should -Be 0
        $script:agentEnvironments | Should -Not -BeNullOrEmpty
        foreach ($environment in $script:agentEnvironments) {
            $environment.IRONRDP_AGENT_KNOWN_CERTIFICATES | Should -Be $knownCertificatesPath
        }
    }
}

Describe 'Owned RDP observation' {
    It 'bounds the native command when a child retains redirected output' {
        # oracle: mutant inherited output handles prevent a command from honoring its timeout.
        $pidFile = Join-Path $TestDrive 'child.pid'
        $fixture = Join-Path $TestDrive 'inherited-output.ps1'
        @'
param($PidFile)
$child = Start-Process powershell.exe -ArgumentList '-NoProfile -Command Start-Sleep -Seconds 30' -NoNewWindow -PassThru
$child.Id | Set-Content -LiteralPath $PidFile
'@ | Set-Content -LiteralPath $fixture
        $clock = [Diagnostics.Stopwatch]::StartNew()
        try {
            { Invoke-E2EAgentCommand -AgentPath 'powershell.exe' -Arguments @('-NoProfile', '-File', $fixture, $pidFile) `
                -Environment @{} -TimeoutSeconds 5 } | Should -Throw '*output remained open*'
            $clock.Elapsed.TotalSeconds | Should -BeLessThan 10
        }
        finally {
            if (Test-Path $pidFile) { Stop-Process -Id ([int](Get-Content $pidFile)) -ErrorAction SilentlyContinue }
        }
    }

    It 'waits for a frame and stops only its unique endpoint without command-line credentials' {
        # oracle: mutant capture stops another endpoint or fails before the first frame arrives.
        $state = @{ Commands = [System.Collections.Generic.List[object]]::new(); Frames = 0 }
        $png = Join-Path $TestDrive 'frame.png'
        $run = {
            param($command, $timeout)
            $state.Commands.Add($command)
            $output = 'ok'
            $exitCode = 0
            switch ($command[2]) {
                'status' { $output = 'state: Connected' }
                'screenshot' {
                    $state.Frames++
                    if ($state.Frames -eq 1) {
                        $output = 'no frame available yet'
                        $exitCode = 1
                    } else {
                        Set-Content -LiteralPath $command[3] -Value 'current frame'
                    }
                }
            }
            [pscustomobject]@{ ExitCode = $exitCode; Output = $output }
        }
        $result = Invoke-E2EAgentScreenshot -Server 'localhost:4489' -Username 'test' -Password 'test-secret' `
            -OutputPath $png -LogPath (Join-Path $TestDrive 'agent.log') -AfterFirstGraphicsSeconds 0 -CommandRunner $run `
            -ConnectionObserver { '[127.0.0.1]:4567' }
        $result.ExitCode | Should -Be 0
        $result.ClientEndpoint | Should -Be '[127.0.0.1]:4567'
        Get-Content -LiteralPath $png | Should -Be 'current frame'
        @($state.Commands | ForEach-Object { $_[1] } | Select-Object -Unique) | Should -HaveCount 1
        $state.Commands[-1][2..3] -join ' ' | Should -Be 'daemon stop'
        ($state.Commands | ForEach-Object { $_ -join ' ' }) -join ' ' | Should -Not -Match 'test-secret'
    }

    It 'fails if the connection drops after the first frame instead of publishing an early success' {
        # oracle: mutant capture declares success before observing a later disconnect.
        $state = @{ StatusCalls = 0 }
        $run = {
            param($command, $timeout)
            $output = 'ok'
            if ($command[2] -eq 'status') {
                $state.StatusCalls++
                $output = if ($state.StatusCalls -eq 1) { 'state: Connected' } else { 'state: Disconnected' }
            }
            [pscustomobject]@{ ExitCode = 0; Output = $output }
        }
        $result = Invoke-E2EAgentScreenshot -Server 'localhost:4489' -Username 'test' -Password 'test-secret' `
            -OutputPath (Join-Path $TestDrive 'early.png') -LogPath (Join-Path $TestDrive 'dropped.log') `
            -AfterFirstGraphicsSeconds 5 -CommandRunner $run
        $result.ExitCode | Should -Be 1
    }

    It 'finishes the requested observation interval while frames continue arriving' {
        # oracle: mutant each new frame resets the observation deadline and prevents completion.
        $state = @{ FirstFrameAt = $null }
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $run = {
            param($command, $timeout)
            if ($command[2] -eq 'screenshot' -and $null -eq $state.FirstFrameAt) {
                $state.FirstFrameAt = $clock.Elapsed.TotalSeconds
            }
            [pscustomobject]@{ ExitCode = 0; Output = $(if ($command[2] -eq 'status') { 'state: Connected' } else { 'ok' }) }
        }
        $result = Invoke-E2EAgentScreenshot -Server 'localhost:4489' -Username 'test' -Password 'fixture' `
            -OutputPath (Join-Path $TestDrive 'continuous.png') -LogPath (Join-Path $TestDrive 'continuous.log') `
            -AfterFirstGraphicsSeconds 1 -TimeoutSeconds 5 -CommandRunner $run
        $result.ExitCode | Should -Be 0
        ($clock.Elapsed.TotalSeconds - $state.FirstFrameAt) | Should -BeGreaterOrEqual 1
    }

    It 'rejects a retained frame when the connection drops during the final screenshot request' {
        # oracle: mutant a successful screenshot command masks a disconnect during the observation.
        $state = @{ Connected = $true }
        $run = {
            param($command, $timeout)
            $output = 'ok'
            if ($command[2] -eq 'status') {
                $output = if ($state.Connected) { 'state: Connected' } else { 'state: Disconnected' }
            } elseif ($command[2] -eq 'screenshot') {
                $state.Connected = $false
                Set-Content -LiteralPath $command[3] -Value 'retained frame'
            }
            [pscustomobject]@{ ExitCode = 0; Output = $output }
        }
        $result = Invoke-E2EAgentScreenshot -Server 'localhost:4489' -Username 'test' -Password 'fixture' `
            -OutputPath (Join-Path $TestDrive 'last-frame.png') -LogPath (Join-Path $TestDrive 'last-frame.log') `
            -AfterFirstGraphicsSeconds 0 -CommandRunner $run
        $result.ExitCode | Should -Be 1
    }
}
