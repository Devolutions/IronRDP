BeforeAll {
    . (Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\session-capture.helpers.ps1')
}

Describe 'Session-local screenshot identity' {
    It 'does not prefer another users IRDP desktop over an owned session' {
        # oracle: mutant capture selects another user's desktop.
        $rows = @(
            [pscustomobject]@{ Id = 4; State = 'Active'; SessionName = 'rdp-tcp#1' }
            [pscustomobject]@{ Id = 8; State = 'Active'; SessionName = 'irdp-tcp#2' }
            [pscustomobject]@{ Id = 10; State = 'Active'; SessionName = 'irdp-tcp#3' }
        )
        (Select-CaptureSession -Rows $rows -OwnedSessionIds @(4, 8) -PreferIrdpTcpSession $true).Id | Should -Be 8
    }

    It 'chooses the newest eligible IRDP session and falls back to the newest owned session' {
        # oracle: mutant session ordering selects a stale IRDP session or an ineligible desktop.
        $rows = @(
            [pscustomobject]@{ Id = 4; State = 'Active'; SessionName = 'rdp-tcp#1' }
            [pscustomobject]@{ Id = 8; State = 'Conn'; SessionName = 'irdp-tcp#2' }
            [pscustomobject]@{ Id = 9; State = 'Disconnected'; SessionName = 'irdp-tcp#3' }
            [pscustomobject]@{ Id = 12; State = 'Active'; SessionName = 'rdp-tcp#4' }
        )
        (Select-CaptureSession -Rows $rows -OwnedSessionIds @(4, 8, 9, 12) -PreferIrdpTcpSession $true).Id | Should -Be 8
        (Select-CaptureSession -Rows $rows -OwnedSessionIds @(4, 9, 12) -PreferIrdpTcpSession $true).Id | Should -Be 12
        (Select-CaptureSession -Rows $rows -OwnedSessionIds @(4, 8, 9, 12) -PreferIrdpTcpSession $false).Id | Should -Be 12
    }

    It 'returns no session when no owned session is active or connecting' {
        # oracle: mutant capture starts against a disconnected or unowned Windows session.
        $rows = @(
            [pscustomobject]@{ Id = 4; State = 'Disconnected'; SessionName = 'irdp-tcp#1' }
            [pscustomobject]@{ Id = 8; State = 'Active'; SessionName = 'rdp-tcp#2' }
        )
        Select-CaptureSession -Rows $rows -OwnedSessionIds @(4) -PreferIrdpTcpSession $true | Should -BeNullOrEmpty
    }

    It 'rejects output from a different session even for the same user' {
        # oracle: mutant capture accepts output from a different session.
        { Assert-CaptureSession -ExpectedSessionId 4 -ExpectedUserSid 'S-1-5-21-1' `
            -Capture ([pscustomobject]@{ SessionId = 8; UserSid = 'S-1-5-21-1' }) } | Should -Throw '*different Windows session*'
    }

    It 'rejects output from a different or missing user SID in the expected session' {
        # oracle: mutant capture trusts a same-session payload from another user or without an identity.
        { Assert-CaptureSession -ExpectedSessionId 4 -ExpectedUserSid 'S-1-5-21-1' `
            -Capture ([pscustomobject]@{ SessionId = 4; UserSid = 'S-1-5-21-2' }) } | Should -Throw '*different Windows session or user*'
        { Assert-CaptureSession -ExpectedSessionId 4 -ExpectedUserSid 'S-1-5-21-1' `
            -Capture ([pscustomobject]@{ SessionId = 4; UserSid = '' }) } | Should -Throw '*different Windows session or user*'
    }

    It 'accepts output only when both the selected session and user SID match' {
        # oracle: mutant valid capture output is rejected despite matching both ownership identities.
        {
            Assert-CaptureSession -ExpectedSessionId 4 -ExpectedUserSid 'S-1-5-21-1' `
                -Capture ([pscustomobject]@{ SessionId = 4; UserSid = 'S-1-5-21-1' })
        } | Should -Not -Throw
    }
}
