BeforeDiscovery {
    $termsrvRoot = Split-Path -Parent $PSScriptRoot
    Import-Module (Join-Path $termsrvRoot 'scripts\TestVm.psm1') -Force
}

Describe 'TermService lifecycle recovery' {
    InModuleScope TestVm {
        BeforeEach {
            $script:state = @{
                StartupType = 'disabled'
                Status = 'Stopped'
            }
            $script:events = [Collections.Generic.List[string]]::new()

            Mock Get-Service { [pscustomobject]@{ Status = $script:state.Status } }
            Mock Set-TermServiceStartupType {
                param($StartupType)
                $script:state.StartupType = $StartupType
                $script:events.Add("mode:$StartupType")
            }
            Mock Invoke-TermServiceControl {
                param($Arguments)
                $script:state.Status = if ($Arguments[0] -eq 'start') { 'Running' } else { 'Stopped' }
                $script:events.Add($Arguments[0])
            }
            Mock Wait-TermServiceState {}
        }

        It 'restores the original startup type and running state through production dependencies' {
            # oracle: mutant recovery leaves TermService disabled or stopped.
            Restore-TermServiceLifecycle -StartupType disabled -WasRunning $true

            $script:state.StartupType | Should -Be 'disabled'
            $script:state.Status | Should -Be 'Running'
            ($script:events -join ',') | Should -Be 'mode:demand,start,mode:disabled'
        }

        It 'restores a previously stopped service without starting it' {
            # oracle: mutant recovery starts TermService when it was previously stopped.
            $script:state.Status = 'Running'

            Restore-TermServiceLifecycle -StartupType demand -WasRunning $false

            $script:state.StartupType | Should -Be 'demand'
            $script:state.Status | Should -Be 'Stopped'
            ($script:events -join ',') | Should -Be 'stop,mode:demand'
        }
    }
}
