BeforeAll {
    Import-Module (Join-Path $PSScriptRoot '..\scripts\TestVm.psm1') -Force
    $path = Join-Path $PSScriptRoot '..\crates\ironrdp-idd\scripts\restart-rds-services.ps1'
    $ast = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$null)
    $restart = $ast.Find({
        param($node)
        $node -is [Management.Automation.Language.ScriptBlockExpressionAst] -and
        $null -ne $node.ScriptBlock.ParamBlock -and
        $node.ScriptBlock.ParamBlock.Parameters.Name.VariablePath.UserPath -contains 'SkipIronRdpTermSrv'
    }, $true).ScriptBlock.GetScriptBlock()
}

Describe 'Scheduled companion restart' {
    BeforeEach {
        $script:events = [Collections.Generic.List[string]]::new()
        $script:processes = @(
            [pscustomobject]@{ Name = 'ironrdp-termsrv.next.exe'; ExecutablePath = 'C:\fixture\bin\ironrdp-termsrv.next.exe'; ProcessId = 123 }
            [pscustomobject]@{ Name = 'ironrdp-termsrv.exe'; ExecutablePath = 'C:\unrelated\ironrdp-termsrv.exe'; ProcessId = 900 }
        )
        Mock Import-Module {}
        Mock Get-TermServiceLifecycle { @{ StartupType = 'disabled'; WasRunning = $true } }
        Mock Get-ScheduledTask { [pscustomobject]@{ State = 'Ready' } }
        Mock Get-CimInstance {
            param($Filter)
            $script:processes | Where-Object { $Filter.Contains("Name='$($_.Name)'") }
        }
        Mock Get-Service { [pscustomobject]@{ Status = 'Running' } }
        Mock Start-Sleep { throw 'Unexpected readiness delay' }
        Mock Stop-TermServiceSafely { $script:events.Add('stop-term') }
        Mock Start-TermServiceSafely { $script:events.Add('start-term') }
        Mock Restore-TermServiceLifecycle { $script:events.Add('restore-term') }
        Mock Stop-ScheduledTask { $script:events.Add('stop-task') }
        Mock Stop-Process {
            param($Id)
            $script:events.Add("stop-$Id")
            $script:processes = @($script:processes | Where-Object { $_.ProcessId -notin $Id })
        }
        Mock Start-ScheduledTask {
            $script:events.Add('start-task')
            $script:processes += [pscustomobject]@{
                Name = 'ironrdp-termsrv.next.exe'; ExecutablePath = 'C:\fixture\bin\ironrdp-termsrv.next.exe'; ProcessId = 456
            }
        }
    }

    It 'restarts only deployment-owned processes before TermService even when the launcher has exited' {
        # oracle: mutant restart ignores the deployed .next.exe child or starts the provider before its companion.
        $result = & $restart $false $false 'FixtureTask' 'ignored.psm1' 'C:\fixture'
        ($script:events -join ',') | Should -Be 'stop-term,stop-task,stop-123,start-task,start-term'
        $result.CompanionProcessId | Should -Be 456
        $result.CompanionTask | Should -Be 'Ready'
        $script:processes.ProcessId | Should -Contain 900
        Should -Invoke Start-TermServiceSafely -Times 1 -ParameterFilter { $StartupType -eq 'disabled' }
    }

    It 'restores a previously running child before restoring TermService after launch failure' {
        # oracle: mutant task Ready is mistaken for a stopped child during recovery.
        $script:launchAttempts = 0
        Mock Start-ScheduledTask {
            $script:events.Add('start-task')
            $script:launchAttempts++
            if ($script:launchAttempts -eq 1) { throw 'Launch failed' }
            $script:processes += [pscustomobject]@{
                Name = 'ironrdp-termsrv.next.exe'; ExecutablePath = 'C:\fixture\bin\ironrdp-termsrv.next.exe'; ProcessId = 456
            }
        }
        { & $restart $false $false 'FixtureTask' 'ignored.psm1' 'C:\fixture' } | Should -Throw '*Launch failed*'
        ($script:events -join ',') | Should -Be 'stop-term,stop-task,stop-123,start-task,start-task,restore-term'
        $script:processes.ProcessId | Should -Contain 456
        Should -Invoke Restore-TermServiceLifecycle -Times 1 -ParameterFilter { $StartupType -eq 'disabled' -and $WasRunning }
    }

    It 'removes a newly started child on failure when no owned companion was originally running' {
        # oracle: mutant failed restart leaves a previously stopped companion running or stops an unrelated process.
        $script:processes = @($script:processes | Where-Object { $_.ProcessId -eq 900 })
        Mock Get-TermServiceLifecycle { @{ StartupType = 'demand'; WasRunning = $false } }
        Mock Start-TermServiceSafely { throw 'Service start failed' }
        { & $restart $false $false 'FixtureTask' 'ignored.psm1' 'C:\fixture' } | Should -Throw '*Service start failed*'
        $script:processes | Should -HaveCount 1
        $script:processes[0].ProcessId | Should -Be 900
        Should -Invoke Restore-TermServiceLifecycle -Times 1 -ParameterFilter { $StartupType -eq 'demand' -and -not $WasRunning }
    }
}
