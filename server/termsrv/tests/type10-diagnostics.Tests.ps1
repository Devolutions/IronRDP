BeforeAll {
    $type10Script = Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\confirm-rdp-type10-logon.ps1'
    Import-Module (Join-Path $PSScriptRoot '..\scripts\TestVm.psm1') -Force
    $credential = [pscredential]::new('fixture', (ConvertTo-SecureString 'fixture' -AsPlainText -Force))
}

Describe 'Type10 diagnostic query failures' {
    BeforeEach {
        Mock Import-Module {}
        Mock New-TestVmSession { New-MockObject -Type System.Management.Automation.Runspaces.PSSession }
        Mock Remove-PSSession {}
        Mock Invoke-Command {
            param($ScriptBlock, $ArgumentList)
            $remote = [powershell]::Create()
            try {
                [void]$remote.AddScript($remoteFixture).AddStatement().AddScript($ScriptBlock.ToString())
                foreach ($argument in $ArgumentList) { [void]$remote.AddArgument($argument) }
                $output = $remote.Invoke()
                if ($remote.Streams.Error.Count -gt 0) { throw ($remote.Streams.Error | Out-String) }
                $output
            } finally {
                $remote.Dispose()
            }
        }
    }

    It 'returns query errors rather than treating inaccessible logs as empty' {
        # oracle: mutant inaccessible remote logs are reported as empty or a caller-scoped accumulator throws.
        $remoteFixture = 'function Get-WinEvent { throw "access denied" }'
        $result = & $type10Script -ComputerName 'localhost' -Credential $credential -PassThru
        $result | Should -HaveCount 1
        $result.Latest4624Type10 | Should -BeNullOrEmpty
        $result.Warnings | Should -Contain 'Security 4624 query failed: access denied'
        $result.Warnings | Should -Contain 'LSM query failed: access denied'
    }

    It 'returns an empty warning collection from a separate runspace when queries succeed' {
        # oracle: mutant a successful empty query returns a null warning that breaks diagnostic output.
        $remoteFixture = 'function Get-WinEvent {}'
        $result = & $type10Script -ComputerName 'localhost' -Credential $credential -PassThru
        $result.Warnings | Should -HaveCount 0
        $result.Latest4624Type10 | Should -BeNullOrEmpty
    }

    It 'correlates a UserData-only LSM event with its Security logon' {
        # oracle: mutant UserData-only LSM events lose fields and cannot correlate with a matching Security logon.
        $remoteFixture = @'
function Get-WinEvent {
    param($FilterHashtable)
    $xml = switch ($FilterHashtable.LogName) {
        'Security' {
            if ($FilterHashtable.Id -ne 4624) { return }
            '<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event"><EventData><Data Name="LogonType">10</Data><Data Name="TargetUserName">alice</Data><Data Name="TargetDomainName">LAB</Data><Data Name="TargetLogonId">0x123</Data><Data Name="IpAddress">192.0.2.1</Data><Data Name="IpPort">12345</Data></EventData></Event>'
        }
        'Microsoft-Windows-TerminalServices-LocalSessionManager/Operational' {
            '<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event"><UserData><EventXML xmlns="Event_NS"><User>LAB\alice</User><SessionID>42</SessionID><Address>192.0.2.1</Address></EventXML></UserData></Event>'
        }
        default { throw "Unexpected log query: $($FilterHashtable.LogName)" }
    }
    $event = [pscustomobject]@{ Xml = $xml; TimeCreated = [datetime]'2026-01-01T00:00:00Z'; Id = 21; Message = 'fixture' }
    $event | Add-Member ScriptMethod ToXml { $this.Xml }
    $event
}
'@
        $result = & $type10Script -ComputerName 'localhost' -Credential $credential -UserLike 'alice' -PassThru
        $result.Warnings | Should -HaveCount 0
        $result.CorrelatedLsm21.User | Should -Be 'LAB\alice'
        $result.CorrelatedLsm21.SessionId | Should -Be '42'
        $result.CorrelatedLsm21.Address | Should -Be '192.0.2.1'
        $result.Latest4624Type10.LogonId | Should -Be '0x123'
    }
}
