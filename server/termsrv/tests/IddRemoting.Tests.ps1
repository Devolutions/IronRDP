# oracle: mutant lost credentials, leaked sessions, omitted remote dependencies, and colliding diagnostic output.
BeforeAll {
    $script:previousTestVmHostname = $env:IRONRDP_TEST_VM_HOSTNAME
    $env:IRONRDP_TEST_VM_HOSTNAME = 'fixture.invalid'
    $script:modulePath = Join-Path $PSScriptRoot '..\scripts\TestVm.psm1'
    $script:iddScripts = Join-Path $PSScriptRoot '..\crates\ironrdp-idd\scripts'
    Import-Module $script:modulePath -Force
    $script:credential = [pscredential]::new('fixture-user', (ConvertTo-SecureString 'fixture-password' -AsPlainText -Force))
}

AfterAll {
    if ($null -eq $script:previousTestVmHostname) {
        Remove-Item Env:IRONRDP_TEST_VM_HOSTNAME -ErrorAction SilentlyContinue
    } else {
        $env:IRONRDP_TEST_VM_HOSTNAME = $script:previousTestVmHostname
    }
}

Describe 'IDD remoting callers' {
    BeforeEach {
        Mock Import-Module {}
        Mock New-TestVmSession { New-MockObject -Type System.Management.Automation.Runspaces.PSSession }
        Mock Wait-TestVmSession { New-MockObject -Type System.Management.Automation.Runspaces.PSSession }
        Mock Remove-PSSession {}
        Mock Invoke-Command {}
        Mock Copy-Item {}
        Mock Start-Sleep {}
    }

    It 'forwards credentials and releases the session when checking the RDS role' {
        # oracle: mutant a remoting caller drops the caller's credential or leaks its session.
        & (Join-Path $script:iddScripts 'ensure-rdsh-role.ps1') -VmHostname 'fixture.invalid' -Username 'fixture-user' -Password '' -Credential $script:credential | Out-Null
        Should -Invoke New-TestVmSession -Times 1 -ParameterFilter {
            $Hostname -eq 'fixture.invalid' -and $Credential.UserName -eq 'fixture-user' -and
            $Credential.GetNetworkCredential().Password -eq 'fixture-password'
        }
        Should -Invoke Invoke-Command -Times 1
        Should -Invoke Remove-PSSession -Times 1 -ParameterFilter {
            $Session.Count -eq 1 -and $Session[0] -is [System.Management.Automation.Runspaces.PSSession]
        }
    }

    It 'closes the session when the remote diagnostic collector fails' {
        # oracle: mutant a remote failure bypasses diagnostic session cleanup.
        Mock Invoke-Command { throw 'Remote collector failed' }
        { & (Join-Path $script:iddScripts 'collect-idd-diagnostics.ps1') -Credential $script:credential } | Should -Throw '*Remote collector failed*'
        Should -Invoke Remove-PSSession -Times 1
        Should -Invoke Copy-Item -Times 0
    }

    It 'downloads the remote collector result to the requested path rather than a fixed archive' {
        # oracle: mutant diagnostics are copied from a fixed path or to a different requested destination.
        Mock Invoke-Command { [pscustomobject]@{ ArchivePath = 'C:\fixture\unique-diagnostics.zip'; Warnings = @() } }
        & (Join-Path $script:iddScripts 'collect-idd-diagnostics.ps1') -Credential $script:credential -LocalOutZip '.\fixture-result.zip' -RemoteDiagRoot 'C:\fixture'
        Should -Invoke Copy-Item -Times 1 -ParameterFilter {
            $Path -eq 'C:\fixture\unique-diagnostics.zip' -and $Destination -eq '.\fixture-result.zip' -and
            $FromSession -is [System.Management.Automation.Runspaces.PSSession]
        }
        Should -Invoke Remove-PSSession -Times 1
    }

    It 'isolates each diagnostic collection below the requested remote root' {
        # oracle: mutant successive collections reuse or escape their caller-owned remote directory.
        $paths = [Collections.Generic.List[string]]::new()
        Mock Invoke-Command -MockWith ({
            param($ArgumentList)
            $paths.Add($ArgumentList[0])
            $paths.Add($ArgumentList[1])
            [pscustomobject]@{ ArchivePath = $ArgumentList[1]; Warnings = @() }
        }.GetNewClosure())
        & (Join-Path $script:iddScripts 'collect-idd-diagnostics.ps1') -Credential $script:credential -LocalOutZip '.\one.zip' -RemoteDiagRoot 'C:\fixture'
        & (Join-Path $script:iddScripts 'collect-idd-diagnostics.ps1') -Credential $script:credential -LocalOutZip '.\two.zip' -RemoteDiagRoot 'C:\fixture'
        $paths.Count | Should -Be 4
        $paths[0] | Should -Not -Be $paths[2]
        $paths[1] | Should -Not -Be $paths[3]
        foreach ($path in $paths) { (Split-Path -Parent $path) | Should -Be 'C:\fixture' }
        $paths[1] | Should -Be ($paths[0] + '.zip')
        $paths[3] | Should -Be ($paths[2] + '.zip')
    }

    It 'returns structured artifact paths only when requested and the download succeeds' {
        # oracle: mutant pass-through results are missing, malformed, or returned after a failed download.
        Mock Invoke-Command { [pscustomobject]@{ ArchivePath = $ArgumentList[1]; Warnings = @('fixture collector warning') } }
        $result = & (Join-Path $script:iddScripts 'collect-idd-diagnostics.ps1') -Credential $script:credential -LocalOutZip '.\fixture-result.zip' -RemoteDiagRoot 'C:\fixture' -PassThru
        $result.LocalArchivePath | Should -Be ($ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath('.\fixture-result.zip'))
        $result.RemoteArchivePath | Should -Be ($result.RemoteDirectory + '.zip')
        $result.Warnings | Should -Contain 'fixture collector warning'
        Mock Copy-Item { throw 'Download failed' }
        { & (Join-Path $script:iddScripts 'collect-idd-diagnostics.ps1') -Credential $script:credential -PassThru } | Should -Throw '*Download failed*'
    }

    It 'copies the lifecycle module before asking the target to restart RDS' {
        # oracle: mutant RDS restart runs before the lifecycle module is installed on the target.
        $script:copied = $false
        Mock Copy-Item { $script:copied = $true }
        Mock Invoke-Command {
            if ($null -ne $ArgumentList -and $ArgumentList.Count -gt 0) {
                $script:copied | Should -BeTrue
                $ArgumentList[3] | Should -Be 'C:\ProgramData\IronRDP\TestVm.psm1'
            }
        }

        & (Join-Path $script:iddScripts 'restart-rds-services.ps1') -Credential $script:credential
        Should -Invoke Copy-Item -Times 1 -ParameterFilter {
            (Split-Path -Leaf $LiteralPath) -eq 'TestVm.psm1' -and $Destination -eq 'C:\ProgramData\IronRDP\TestVm.psm1'
        }
        Should -Invoke Invoke-Command -Times 2
        Should -Invoke Remove-PSSession -Times 1
    }

    It 'closes the initial session when reboot readiness reconnect fails' {
        # oracle: mutant failed readiness reconnect leaves the initial remoting session open.
        Mock Wait-TestVmSession { throw 'WinRM readiness timed out' }
        { & (Join-Path $script:iddScripts 'restart-vm.ps1') -Credential $script:credential -TimeoutSeconds 45 } | Should -Throw '*WinRM readiness timed out*'
        Should -Invoke Remove-PSSession -Times 1
        Should -Invoke Wait-TestVmSession -Times 1 -ParameterFilter { $TimeoutSeconds -eq 25 }
    }
}

Describe 'IDD diagnostic payload unit without WinRM' {
    BeforeAll {
        $ast = [System.Management.Automation.Language.Parser]::ParseFile(
            (Join-Path $script:iddScripts 'collect-idd-diagnostics.ps1'), [ref]$null, [ref]$null)
        $payloadAst = $ast.Find({
            param($node)
            $node -is [System.Management.Automation.Language.ScriptBlockExpressionAst]
        }, $true)
        $script:payload = $payloadAst.ScriptBlock.GetScriptBlock()
    }

    BeforeEach {
        Mock New-Item {}
        Mock Out-File {}
        Mock Compress-Archive {}
        Mock pnputil.exe { $global:LASTEXITCODE = 0; 'Device list' }
        Mock Get-WinEvent {}
        Mock Get-ChildItem {} -ParameterFilter { $Path -like '*DriverStore*' }
    }

    It 'reports native and query failures while preserving a usable diagnostic archive' {
        # oracle: mutant collector failures disappear or prevent the diagnostic archive from being returned.
        Mock pnputil.exe { $global:LASTEXITCODE = 87; 'Invalid parameter' }
        Mock Get-WinEvent { throw 'Event query denied' }
        Mock Get-ChildItem { throw 'Driver store denied' } -ParameterFilter { $Path -like '*DriverStore*' }
        $result = & $script:payload -diagPath '.\fixture-diagnostics' -zipPath '.\fixture-diagnostics.zip'
        $result.ArchivePath | Should -Be '.\fixture-diagnostics.zip'
        $result.Warnings.Count | Should -Be 3
        $result.Warnings[0] | Should -Match 'pnputil.*exit 87'
        $result.Warnings[1] | Should -Match 'Event query denied'
        $result.Warnings[2] | Should -Match 'Driver store denied'
        Should -Invoke Compress-Archive -Times 1 -ParameterFilter {
            $Path -eq '.\fixture-diagnostics' -and $DestinationPath -eq '.\fixture-diagnostics.zip'
        }
    }

    It 'returns no warnings when all collectors succeed' {
        # oracle: mutant successful diagnostic collection is reported with spurious warnings.
        $result = & $script:payload -diagPath '.\fixture-diagnostics' -zipPath '.\fixture-diagnostics.zip'
        $result.Warnings.Count | Should -Be 0
    }
}
