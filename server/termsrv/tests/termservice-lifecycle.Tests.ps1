BeforeAll {
    $termsrvRoot = Split-Path -Parent $PSScriptRoot
    Import-Module (Join-Path $termsrvRoot 'scripts\TestVm.psm1') -Force
}

Describe 'TermService lifecycle recovery' {
    It 'restores the original startup type and running state' {
        # oracle: mutant recovery leaves TermService disabled or stopped
        $state = @{
            StartupType = 'disabled'
            Status = 'Stopped'
        }
        $setStartupType = { param($value) $state.StartupType = $value }
        $getStatus = { $state.Status }
        $startService = { $state.Status = 'Running' }
        $waitForStatus = {
            param($status, $timeout)
            if ($state.Status -ne $status) {
                throw "unexpected service status $($state.Status)"
            }
        }

        Restore-TermServiceLifecycle -StartupType 'auto' -WasRunning $true `
            -SetStartupType $setStartupType -GetStatus $getStatus -StartService $startService `
            -WaitForStatus $waitForStatus

        $state.StartupType | Should -Be 'auto'
        $state.Status | Should -Be 'Running'
    }

    It 'restores a previously stopped service without starting it' {
        # oracle: mutant recovery starts TermService when it was previously stopped
        $state = @{
            StartupType = 'disabled'
            Status = 'Running'
            StartCount = 0
        }
        $setStartupType = { param($value) $state.StartupType = $value }
        $getStatus = { $state.Status }
        $startService = { $state.StartCount++ }
        $stopService = { $state.Status = 'Stopped' }
        $waitForStatus = {
            param($status, $timeout)
            if ($state.Status -ne $status) {
                throw "unexpected service status $($state.Status)"
            }
        }

        Restore-TermServiceLifecycle -StartupType 'demand' -WasRunning $false `
            -SetStartupType $setStartupType -GetStatus $getStatus `
            -StartService $startService -StopService $stopService -WaitForStatus $waitForStatus

        $state.StartupType | Should -Be 'demand'
        $state.Status | Should -Be 'Stopped'
        $state.StartCount | Should -Be 0
    }
}
