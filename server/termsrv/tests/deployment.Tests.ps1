BeforeAll {
    $deployScript = Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\deploy-testvm-psremoting.ps1'
    Import-Module (Join-Path $PSScriptRoot '..\scripts\TestVm.psm1') -Force
    function Install-FixtureProvider { param($ListenerName, $PortNumber) }
}

Describe 'Unified provider deployment' {
    BeforeEach {
        $remoteRoot = Join-Path $TestDrive 'remote'
        $events = [Collections.Generic.List[string]]::new()
        $runnerArguments = [Collections.Generic.List[string]]::new()
        Mock Import-Module {}
        Mock New-TestVmSession { New-MockObject -Type System.Management.Automation.Runspaces.PSSession }
        Mock Remove-PSSession {}
        Mock Invoke-Command {
            $RemoteRoot = $null
            & $ScriptBlock @ArgumentList
        }
        Mock Start-Sleep {}
        Mock Remove-Item {}
        Mock Test-Path { [IO.File]::Exists($LiteralPath) -or [IO.Directory]::Exists($LiteralPath) }
        Mock Test-Path { $true } -ParameterFilter { $LiteralPath -match '\\target\\release\\ironrdp[-_]'}
        Mock Get-TermServiceLifecycle { @{ StartupType = 'auto'; WasRunning = $true } }
        Mock Stop-TermServiceSafely ({ $events.Add('stop'); @{ StartupType = 'auto'; WasRunning = $true } }.GetNewClosure())
        Mock Restore-TermServiceLifecycle ({ param($StartupType, $WasRunning); $events.Add("restore:$StartupType/$WasRunning") }.GetNewClosure())
        Mock Get-ScheduledTask {}
        Mock Register-ScheduledTask ({
            param($Action)
            $runnerArguments.Add([string]$Action.Arguments)
        }.GetNewClosure())
        Mock Start-ScheduledTask ({ $events.Add('companion') }.GetNewClosure())
        Mock Stop-Process {}
        Mock Get-CimInstance ({
            if ($events.Contains('companion')) {
                [pscustomobject]@{ Name = 'ironrdp-termsrv.next.exe'; ExecutablePath = (Join-Path $remoteRoot 'bin\ironrdp-termsrv.next.exe'); ProcessId = 4242 }
            }
        }.GetNewClosure())
        Mock Get-Service { [pscustomobject]@{ Status = 'Running'; StartType = 'Automatic' } }
        Mock Get-NetFirewallRule { [pscustomobject]@{ DisplayName = 'fixture' } }
        Mock Start-TermServiceSafely ({ $events.Add('start') }.GetNewClosure())
        Mock Wait-TestVmListener ({ param($OwningProcess); $events.Add("listener:$OwningProcess") }.GetNewClosure())
        Mock Install-FixtureProvider ({ $events.Add('provider') }.GetNewClosure())
        Mock Copy-Item ({
            param($LiteralPath, $Path, $Destination)
            $sources = if ($LiteralPath) { @($LiteralPath) } else { @($Path) }
            foreach ($source in $sources) {
                $name = Split-Path -Leaf $source
                if ($name -eq 'TestVm.psm1') { $events.Add("copy-module:$Destination") }
                if ($name -eq 'install-side-by-side.ps1') {
                    'param($ProviderDllPath, $ListenerName, $PortNumber); Install-FixtureProvider -ListenerName $ListenerName -PortNumber $PortNumber' |
                        Set-Content -LiteralPath (Join-Path $Destination $name)
                }
            }
        }.GetNewClosure())
        $arguments = @{
            Hostname = 'localhost'
            Username = 'fixture'
            Password = (ConvertTo-SecureString 'fixture' -AsPlainText -Force)
            WtsProvider = $true
            SkipBuild = $true
            RemoteRoot = $remoteRoot
        }
    }

    It 'installs once and starts the companion before TermService and checks its PID' {
        # oracle: deployment requires stop, artifact registration, companion start, service start, then owned-listener readiness.
        & $deployScript @arguments
        @($events | Where-Object { $_ -notlike 'copy-module:*' }) -join ',' | Should -Be 'stop,provider,companion,start,listener:4242'
        $events | Should -Contain "copy-module:$(Join-Path $remoteRoot 'provider')"
        Should -Invoke Install-FixtureProvider -Times 1 -ParameterFilter { $ListenerName -eq 'IRDP-Tcp' -and $PortNumber -eq 4489 }
    }

    It 'passes the configured deployment root to the companion process' {
        # oracle: custom deployment roots must reach the process that writes runtime logs.
        & $deployScript @arguments

        $runnerArguments.Count | Should -Be 1
        $pattern = '-DeployRoot\s+"?{0}"?(?:\s|$)' -f [regex]::Escape($remoteRoot)
        $runnerArguments[0] | Should -Match $pattern
    }

    It 'converts the legacy plaintext password parameter to a remoting credential' {
        # oracle: the documented compatibility parameter must authenticate with its supplied password.
        $seenCredentials = [Collections.Generic.List[pscredential]]::new()
        Mock New-TestVmSession {
            $seenCredentials.Add($Credential)
            New-MockObject -Type System.Management.Automation.Runspaces.PSSession
        }
        $caseArguments = $arguments.Clone()
        $caseArguments.Remove('Password')
        $caseArguments.PasswordPlainText = 'legacy-fixture-password'

        & $deployScript @caseArguments

        $seenCredentials | Should -Not -BeNullOrEmpty
        $seenCredentials[0].UserName | Should -Be 'fixture'
        $seenCredentials[0].GetNetworkCredential().Password | Should -Be 'legacy-fixture-password'
    }

    It 'still installs the provider when startup is intentionally deferred' {
        # oracle: NoTermServiceStart defers startup, not artifact installation.
        & $deployScript @arguments -NoTermServiceStart
        $events | Should -Contain 'provider'
        Should -Invoke Start-TermServiceSafely -Times 0
        Should -Invoke Wait-TestVmListener -Times 0
        $events | Should -Contain 'restore:demand/False'
    }

    It 'restores the original lifecycle after readiness fails' {
        # oracle: a deployment failure cannot leave TermService disabled or replace its pre-deployment running state.
        Mock Wait-TestVmListener { throw 'listener timeout' }
        { & $deployScript @arguments } | Should -Throw '*listener timeout*'
        $events | Should -Contain 'restore:auto/True'
    }
}
