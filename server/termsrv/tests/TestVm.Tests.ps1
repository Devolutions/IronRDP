BeforeDiscovery {
    Import-Module (Join-Path $PSScriptRoot '..\scripts\TestVm.psm1') -Force
}

Describe 'Test VM credentials and remoting' {
    InModuleScope TestVm {
        BeforeAll {
            $script:previousHostname = $env:IRONRDP_TEST_VM_HOSTNAME
            $env:IRONRDP_TEST_VM_HOSTNAME = 'fixture.invalid'
            $script:credential = [pscredential]::new('fixture-user', (ConvertTo-SecureString 'fixture-password' -AsPlainText -Force))
        }

        AfterAll {
            if ($null -eq $script:previousHostname) {
                Remove-Item Env:IRONRDP_TEST_VM_HOSTNAME -ErrorAction SilentlyContinue
            } else {
                $env:IRONRDP_TEST_VM_HOSTNAME = $script:previousHostname
            }
        }

        It 'uses the supplied credential without prompting or replacing its password' {
            # oracle: mutant a supplied credential is replaced or prompts for another password.
            Mock Read-Host { throw 'Unexpected password prompt' }
            $actual = Resolve-TestVmCredential -Credential $script:credential -Username 'other-user' -Password 'other-password' -PromptPassword
            [object]::ReferenceEquals($actual, $script:credential) | Should -BeTrue
            Should -Invoke Read-Host -Times 0
        }

        It 'uses an explicit password before a prompt or environment value' {
            # oracle: mutant an environment or prompted password overrides an explicit password.
            Mock Read-Host { throw 'Unexpected password prompt' }
            $actual = Resolve-TestVmCredential -Username 'fixture-user' -Password 'explicit-password' -PromptPassword
            $actual.GetNetworkCredential().Password | Should -Be 'explicit-password'
        }

        It 'uses the named environment variable only when no password was supplied' {
            # oracle: mutant credential resolution ignores the selected environment variable.
            $name = 'IRONRDP_TEST_CREDENTIAL_' + [guid]::NewGuid().ToString('N')
            [Environment]::SetEnvironmentVariable($name, 'environment-password')
            try {
                $actual = Resolve-TestVmCredential -Username 'fixture-user' -PasswordEnvVar $name
                $actual.GetNetworkCredential().Password | Should -Be 'environment-password'
            } finally {
                [Environment]::SetEnvironmentVariable($name, $null)
            }
        }

        It 'prompts before falling back to the environment' {
            # oracle: mutant a configured fallback is used before the requested prompt.
            Mock Read-Host { ConvertTo-SecureString 'prompt-password' -AsPlainText -Force }
            $actual = Resolve-TestVmCredential -Username 'fixture-user' -PromptPassword
            $actual.GetNetworkCredential().Password | Should -Be 'prompt-password'
        }

        It 'rejects a missing password without connecting or prompting' {
            # oracle: mutant a missing password reaches remoting or an unexpected prompt.
            Mock Read-Host { throw 'Unexpected password prompt' }
            { Resolve-TestVmCredential -Username 'fixture-user' -PasswordEnvVar ('UNSET_' + [guid]::NewGuid().ToString('N')) } |
                Should -Throw '*Password is required*'
        }

        It 'rejects a missing username instead of using an empty identity' {
            # oracle: mutant an empty username becomes a remoting credential.
            { Resolve-TestVmCredential -Username '' -Password 'fixture-password' } | Should -Throw '*Username is required*'
        }

        It 'requires an explicit test VM hostname instead of silently targeting the lab VM' {
            # oracle: a caller must identify the target rather than inherit a machine-specific default.
            $previous = $env:IRONRDP_TEST_VM_HOSTNAME
            Remove-Item Env:IRONRDP_TEST_VM_HOSTNAME -ErrorAction SilentlyContinue
            try {
                { Resolve-TestVmCredential -Username 'fixture-user' -Password 'fixture-password' } |
                    Should -Throw
            } finally {
                if ($null -ne $previous) { $env:IRONRDP_TEST_VM_HOSTNAME = $previous }
            }
        }

        It 'uses the Cargo metadata target directory instead of assuming a repository target folder' {
            # oracle: Cargo metadata reports the active target directory, including CARGO_TARGET_DIR overrides.
            $workspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
            $previous = $env:CARGO_TARGET_DIR
            $expected = Join-Path $TestDrive 'cargo target'
            $env:CARGO_TARGET_DIR = $expected
            try {
                Get-CargoTargetDirectory -WorkspaceRoot $workspaceRoot | Should -Be ([IO.Path]::GetFullPath($expected))
            } finally {
                if ($null -eq $previous) {
                    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
                } else {
                    $env:CARGO_TARGET_DIR = $previous
                }
            }
        }

        It 'includes an environment-selected Cargo build target in artifact paths' {
            # oracle: Cargo places artifacts for CARGO_BUILD_TARGET beneath a target-specific directory.
            $workspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
            $previous = $env:CARGO_BUILD_TARGET
            $env:CARGO_BUILD_TARGET = 'x86_64-pc-windows-msvc'
            try {
                $expected = Join-Path (Get-CargoTargetDirectory -WorkspaceRoot $workspaceRoot) $env:CARGO_BUILD_TARGET
                Get-CargoBuildOutputDirectory -WorkspaceRoot $workspaceRoot | Should -Be $expected
            } finally {
                if ($null -eq $previous) {
                    Remove-Item Env:CARGO_BUILD_TARGET -ErrorAction SilentlyContinue
                } else {
                    $env:CARGO_BUILD_TARGET = $previous
                }
            }
        }

        It 'uses HTTP first without weakening its session options' {
            # oracle: mutant initial HTTP remoting disables transport protections or uses another host.
            Mock New-PSSessionOption { [pscustomobject]@{} }
            Mock New-PSSession { [pscustomobject]@{ Id = 12 } }
            (New-TestVmSession -Hostname 'fixture.invalid' -Credential $script:credential).Id | Should -Be 12
            Should -Invoke New-PSSession -Times 1 -ParameterFilter { -not $UseSSL -and $ComputerName -eq 'fixture.invalid' }
            Should -Invoke New-PSSessionOption -Times 0 -ParameterFilter { $SkipCACheck -or $SkipCNCheck -or $SkipRevocationCheck }
        }

        It 'validates the HTTPS fallback certificate and never prints the HTTP exception' {
            # oracle: mutant HTTPS fallback disables certificate validation or exposes the failed HTTP diagnostic.
            Mock New-PSSessionOption { [pscustomobject]@{} }
            Mock New-PSSession {
                if (-not $UseSSL) { throw 'sensitive-fixture-diagnostic' }
                [pscustomobject]@{ Id = 13 }
            }
            $messages = @(New-TestVmSession -Hostname 'fixture.invalid' -Credential $script:credential 3>&1)
            ($messages | Out-String) | Should -Not -Match 'sensitive-fixture-diagnostic'
            Should -Invoke New-PSSession -Times 1 -ParameterFilter { $UseSSL -and $Port -eq 5986 }
            Should -Invoke New-PSSessionOption -Times 1 -ParameterFilter { -not $SkipCACheck -and -not $SkipCNCheck -and -not $SkipRevocationCheck }
        }

        It 'closes an unusable reconnected session before retrying' {
            # oracle: mutant readiness retry leaks a failed remoting session.
            $script:session = New-MockObject -Type System.Management.Automation.Runspaces.PSSession
            Mock New-TestVmSession { $script:session }
            $script:probes = 0
            Mock Invoke-Command {
                $script:probes++
                if ($script:probes -eq 1) { throw 'Not ready' }
                'ok'
            }
            Mock Remove-PSSession {}
            Mock Start-Sleep {}
            $actual = Wait-TestVmSession -Hostname 'fixture.invalid' -Credential $script:credential -TimeoutSeconds 5
            [object]::ReferenceEquals($actual, $script:session) | Should -BeTrue
            Should -Invoke Remove-PSSession -Times 1
        }

        It 'bounds reconnect attempts and preserves the caller-owned successful session' {
            # oracle: mutant reconnect polling runs without a deadline or closes a caller-owned session.
            Mock New-TestVmSession { throw 'Unavailable' }
            Mock Remove-PSSession {}
            $script:now = [datetime]'2026-01-01'
            Mock Get-Date { $script:now = $script:now.AddSeconds(1); $script:now }
            Mock Start-Sleep {}
            { Wait-TestVmSession -Hostname 'fixture.invalid' -Credential $script:credential -TimeoutSeconds 3 } | Should -Throw '*WinRM*'
            Should -Invoke New-TestVmSession -Times 1
            Should -Invoke Remove-PSSession -Times 0
        }
    }
}

Describe 'Bounded listener readiness' {
    InModuleScope TestVm {
        BeforeEach { Mock Start-Sleep {} }

        It 'waits through a temporarily missing listener' {
            # oracle: mutant transient listener absence aborts readiness before the deadline.
            $script:queries = 0
            Mock Get-NetTCPConnection {
                $script:queries++
                if ($script:queries -eq 1) {
                    $exception = [InvalidOperationException]::new('No matching listener')
                    $record = [System.Management.Automation.ErrorRecord]::new(
                        $exception, 'CmdletizationQuery_NotFound_Port', 'ObjectNotFound', $null)
                    throw $record
                }
                [pscustomobject]@{ LocalPort = 4489; OwningProcess = 90 }
            }
            (Wait-TestVmListener -Port 4489 -OwningProcess 90 -TimeoutSeconds 5).OwningProcess | Should -Be 90
            Should -Invoke Get-NetTCPConnection -Times 2
        }

        It 'does not turn a listener query failure into absence' {
            # oracle: mutant an access error is treated as an absent listener and retried.
            Mock Get-NetTCPConnection { throw [UnauthorizedAccessException]::new('Access denied') }
            { Wait-TestVmListener -Port 4489 -TimeoutSeconds 5 } | Should -Throw '*Access denied*'
            Should -Invoke Get-NetTCPConnection -Times 1
        }

        It 'rejects a listener owned by another process at the deadline' {
            # oracle: mutant a different process can satisfy the listener ownership check.
            Mock Get-NetTCPConnection { [pscustomobject]@{ LocalPort = 4489; OwningProcess = 91 } }
            $script:now = [datetime]'2026-01-01'
            Mock Get-Date { $script:now = $script:now.AddSeconds(1); $script:now }
            { Wait-TestVmListener -Port 4489 -OwningProcess 90 -TimeoutSeconds 3 } | Should -Throw '*4489*'
        }

        It 'does not ignore unrelated object-not-found query errors' {
            # oracle: mutant an unrelated provider error is mistaken for normal listener absence.
            Mock Get-NetTCPConnection {
                throw [System.Management.Automation.ErrorRecord]::new(
                    [InvalidOperationException]::new('Provider not found'), 'MissingProvider', 'ObjectNotFound', $null)
            }
            { Wait-TestVmListener -Port 4489 -TimeoutSeconds 5 } | Should -Throw '*Provider not found*'
        }
    }
}

Describe 'TermService lifecycle recovery' {
    InModuleScope TestVm {
        BeforeEach {
            $script:mode = 'demand'
            $script:status = 'Running'
            $script:events = [Collections.Generic.List[string]]::new()
            Mock Get-CimInstance {
                [pscustomobject]@{ StartMode = 'Manual'; State = $script:status; ProcessId = 77 }
            }
            Mock Get-Service { [pscustomobject]@{ Status = $script:status } }
            Mock Set-TermServiceStartupType {
                $script:mode = $StartupType
                $script:events.Add("mode:$StartupType")
            }
            Mock Invoke-TermServiceControl {
                switch ($Arguments[0]) {
                    'start' {
                        if ($script:mode -eq 'disabled') { throw 'Cannot start a disabled service' }
                        $script:status = 'Running'
                        $script:events.Add('start')
                    }
                    'stop' {
                        $script:status = 'Stopped'
                        $script:events.Add('stop')
                    }
                }
            }
            Mock Wait-TermServiceState {}
        }

        It 'temporarily enables a disabled-but-running service before restoring its startup mode' {
            # oracle: mutant a disabled running service cannot be started for recovery or stays enabled.
            $script:mode = 'disabled'
            $script:status = 'Stopped'
            Restore-TermServiceLifecycle -StartupType disabled -WasRunning $true
            $script:status | Should -Be 'Running'
            $script:mode | Should -Be 'disabled'
            ($script:events -join ',') | Should -Be 'mode:demand,start,mode:disabled'
        }

        It 'restores the disabled startup mode even when starting fails' {
            # oracle: mutant a failed TermService start leaves its startup mode changed.
            $script:status = 'Stopped'
            Mock Invoke-TermServiceControl { throw 'Start failed' }
            { Restore-TermServiceLifecycle -StartupType disabled -WasRunning $true } | Should -Throw '*Start failed*'
            $script:mode | Should -Be 'disabled'
        }

        It 'restores the original stopped state instead of leaving a service running' {
            # oracle: mutant recovery starts a service that was stopped before the operation.
            Restore-TermServiceLifecycle -StartupType auto -WasRunning $false
            $script:mode | Should -Be 'auto'
            $script:status | Should -Be 'Stopped'
        }

        It 'restores startup mode even if querying the service during recovery fails' {
            # oracle: mutant a failed recovery status query prevents startup-mode restoration.
            Mock Get-Service { throw 'Query failed' }
            { Restore-TermServiceLifecycle -StartupType auto -WasRunning $true } | Should -Throw '*Query failed*'
            $script:mode | Should -Be 'auto'
        }

        It 'reports both recovery and configuration errors without masking the first failure' {
            # oracle: mutant startup-mode restoration hides the primary recovery error.
            Mock Get-Service { throw 'Query failed' }
            Mock Set-TermServiceStartupType { throw 'Config failed' }
            { Restore-TermServiceLifecycle -StartupType auto -WasRunning $true } | Should -Throw '*Query failed*Config failed*'
        }

        It 'preserves automatic delayed startup in its snapshot' {
            # oracle: mutant lifecycle snapshot collapses delayed automatic startup to automatic.
            Mock Get-CimInstance { [pscustomobject]@{ StartMode = 'Auto'; State = 'Running'; ProcessId = 77 } }
            Mock Get-ItemProperty { [pscustomobject]@{ DelayedAutoStart = 1 } }
            $state = Get-TermServiceLifecycle
            $state.StartupType | Should -Be 'delayed-auto'
            $state.WasRunning | Should -BeTrue
        }

        It 'restores service state after a native stop failure' {
            # oracle: mutant a failed native stop leaves TermService altered.
            Mock Invoke-TermServiceControl { throw 'sc.exe stop failed (exit 5)' }
            { Stop-TermServiceSafely } | Should -Throw '*exit 5*'
            $script:mode | Should -Be 'demand'
            $script:status | Should -Be 'Running'
        }

        It 'returns the original lifecycle and keeps the service disabled for replacement' {
            # oracle: mutant replacement startup receives no original lifecycle or an enabled service.
            $state = Stop-TermServiceSafely
            $state.StartupType | Should -Be 'demand'
            $state.WasRunning | Should -BeTrue
            $script:status | Should -Be 'Stopped'
            $script:mode | Should -Be 'disabled'
        }

        It 'restores the running state after a stop wait fails' {
            # oracle: mutant stop timeout leaves a previously running service stopped.
            Mock Wait-TermServiceState { throw [TimeoutException]::new('Stop timeout') } -ParameterFilter { $Status -eq 'Stopped' }
            { Stop-TermServiceSafely } | Should -Throw '*Stop timeout*'
            $script:status | Should -Be 'Running'
            $script:mode | Should -Be 'demand'
        }

        It 'restores the requested startup mode after a start wait fails' {
            # oracle: mutant failed service start leaves the requested startup mode changed.
            Mock Wait-TermServiceState { throw [TimeoutException]::new('Start timeout') }
            { Start-TermServiceSafely -StartupType disabled } | Should -Throw '*Start timeout*'
            $script:mode | Should -Be 'disabled'
        }

        It 'reports both native startup and mode restoration failures' {
            # oracle: mutant startup-mode restoration hides the native startup failure.
            Mock Invoke-TermServiceControl { throw 'Start failed (exit 5)' }
            Mock Set-TermServiceStartupType { throw 'Config failed (exit 6)' } -ParameterFilter { $StartupType -eq 'disabled' }
            { Start-TermServiceSafely -StartupType disabled } | Should -Throw '*Start failed (exit 5)*Config failed (exit 6)*'
        }

        It 'restores the original mode when restart startup fails' {
            # oracle: mutant failed replacement startup leaves the original TermService mode or state changed.
            Mock Get-TermServiceLifecycle { @{ StartupType = 'auto'; WasRunning = $true } }
            Mock Stop-TermServiceSafely { $script:status = 'Stopped'; $script:mode = 'disabled' }
            Mock Start-TermServiceSafely { throw 'Startup failed' }
            { Restart-TermServiceSafely } | Should -Throw '*Startup failed*'
            $script:mode | Should -Be 'auto'
            $script:status | Should -Be 'Running'
        }

        It 'successfully restarts a disabled-but-running service without changing its startup mode' {
            # oracle: mutant restart leaves a disabled running service stopped or changes its startup mode.
            $script:mode = 'disabled'
            Mock Get-TermServiceLifecycle { @{ StartupType = 'disabled'; WasRunning = $true } }
            Restart-TermServiceSafely
            $script:mode | Should -Be 'disabled'
            $script:status | Should -Be 'Running'
        }

        It 'does not kill a service host shared with another running service' {
            # oracle: mutant forced recovery kills an svchost that owns another service.
            Mock Get-TermServiceLifecycle { @{ StartupType = 'demand'; WasRunning = $true } }
            Mock Invoke-TermServiceControl {}
            Mock Wait-TermServiceState { throw [TimeoutException]::new('Timed out') }
            Mock Get-CimInstance { @(
                [pscustomobject]@{ Name = 'TermService'; ProcessId = 77; State = 'Running' }
                [pscustomobject]@{ Name = 'OtherService'; ProcessId = 77; State = 'Running' }
            ) } -ParameterFilter { $Filter -eq 'ProcessId=77' }
            Mock Stop-Process { throw 'Must not kill shared host' }
            { Stop-TermServiceSafely -Force } | Should -Throw '*shared*'
            Should -Invoke Stop-Process -Times 0
            $script:mode | Should -Be 'demand'
        }

        It 'does not kill a replacement service host after a PID change' {
            # oracle: mutant forced recovery kills a new host after TermService changes its PID.
            Mock Wait-TermServiceState { throw [TimeoutException]::new('Timed out') }
            Mock Get-TermServiceLifecycle { @{ StartupType = 'demand'; WasRunning = $true } }
            Mock Get-CimInstance { @() } -ParameterFilter { $Filter -eq 'ProcessId=77' }
            $script:pidQueries = 0
            Mock Get-CimInstance {
                $script:pidQueries++
                [pscustomobject]@{ ProcessId = 76 + $script:pidQueries }
            } -ParameterFilter { $Filter -eq "Name='TermService'" }
            Mock Stop-Process { throw 'Must not kill a replacement host' }
            { Stop-TermServiceSafely -Force } | Should -Throw '*host changed*'
            Should -Invoke Stop-Process -Times 0
            $script:mode | Should -Be 'demand'
        }
    }
}

Describe 'Native TermService command results' {
    InModuleScope TestVm {
        BeforeAll {
            $script:scAlias = Get-Alias sc -ErrorAction SilentlyContinue
            if ($null -ne $script:scAlias) {
                # Pester also aliases sc.exe as sc, which conflicts with Windows PowerShell's read-only alias.
                Remove-Item Alias:\sc -Force
            }
        }

        AfterAll {
            if ($null -ne $script:scAlias) {
                Set-Alias -Name sc -Value $script:scAlias.Definition -Option $script:scAlias.Options -Scope Global -Force
            }
        }

        It 'reports a failed startup mode change instead of silently continuing' {
            # oracle: mutant native startup-mode failure is reported as successful.
            Mock sc.exe { $global:LASTEXITCODE = 5; 'Access denied' }
            { Set-TermServiceStartupType -StartupType demand } | Should -Throw '*exit 5*'
        }

        It 'accepts the native already-stopped result only when explicitly allowed' {
            # oracle: mutant native stop success-code policy accepts unrelated failure codes.
            Mock sc.exe { $global:LASTEXITCODE = 1062 }
            { Invoke-TermServiceControl -Arguments @('stop', 'TermService') -SuccessExitCodes @(0, 1062) } | Should -Not -Throw
            { Invoke-TermServiceControl -Arguments @('config', 'TermService', 'start=', 'disabled') } | Should -Throw '*exit 1062*'
        }

        It 'bounds a service-state wait without swallowing status query errors' {
            # oracle: mutant service-state wait hides a status-query failure.
            Mock Get-Service { throw 'Service query failed' }
            { Wait-TermServiceState -Status Stopped -TimeoutSeconds 1 } | Should -Throw '*Service query failed*'
        }

        It 'times out when the service never reaches the requested state' {
            # oracle: mutant service-state wait exceeds its deadline for a permanently pending service.
            Mock Get-Service { [pscustomobject]@{ Status = 'StopPending' } }
            Mock Start-Sleep {}
            $script:now = [datetime]'2026-01-01'
            Mock Get-Date { $script:now = $script:now.AddSeconds(1); $script:now }
            { Wait-TermServiceState -Status Stopped -TimeoutSeconds 3 } | Should -Throw '*did not reach Stopped*'
        }
    }
}
