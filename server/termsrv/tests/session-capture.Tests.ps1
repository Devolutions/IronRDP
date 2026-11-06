BeforeAll {
    . (Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\session-capture.helpers.ps1')
}

Describe 'Session-local screenshot identity' {
    It 'does not prefer another users IRDP desktop over an owned session' {
        # oracle: mutant capture selects another user's desktop.
        $rows = @(
            [pscustomobject]@{ Id = 4; State = 'Active'; SessionName = 'rdp-tcp#1' }
            [pscustomobject]@{ Id = 8; State = 'Active'; SessionName = 'irdp-tcp#2' }
        )
        (Select-CaptureSession -Rows $rows -OwnedSessionIds @(4) -PreferIrdpTcpSession $true).Id | Should -Be 4
    }

    It 'rejects output from a different session even for the same user' {
        # oracle: mutant capture accepts output from a different session.
        { Assert-CaptureSession -ExpectedSessionId 4 -ExpectedUserSid 'S-1-5-21-1' `
            -Capture ([pscustomobject]@{ SessionId = 8; UserSid = 'S-1-5-21-1' }) } | Should -Throw '*different Windows session*'
    }
}
