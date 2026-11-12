BeforeAll {
    $path = Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\e2e-test-screenshot.ps1'
    $ast = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$null, [ref]$null)
    $cleanup = $ast.Find({
        param($node)
        $node -is [Management.Automation.Language.ScriptBlockExpressionAst] -and
        $null -ne $node.ScriptBlock.ParamBlock -and
        $node.ScriptBlock.ParamBlock.Parameters.Name.VariablePath.UserPath -contains 'RdpUsernameRaw'
    }, $true).ScriptBlock.GetScriptBlock()
    function quser {}
    function qwinsta {}
    function logoff {}
    function rwinsta {}
}

Describe 'Destructive session cleanup ownership' {
    BeforeEach {
        Mock Get-Process {}
        Mock quser { ' USERNAME SESSIONNAME ID STATE IDLE TIME'; ' bob irdp-tcp#1 42 Disc 1 10:00' }
        Mock qwinsta { ' SESSIONNAME USERNAME ID STATE'; ' irdp-tcp#1 bob 42 Disc' }
        Mock logoff { $global:LASTEXITCODE = 1; 'logoff failed' }
        Mock rwinsta { $global:LASTEXITCODE = 0 }
    }

    It 'does not reset another users session after rejecting its logoff' {
        # oracle: mutant aggressive reset bypasses the session ownership rejection.
        { & $cleanup 'alice' '' $true 42 } | Should -Throw '*not discovered for target_user=alice*'
        Should -Invoke logoff -Times 0
        Should -Invoke rwinsta -Times 0
    }

    It 'can reset the selected owned session when its logoff fails' {
        # oracle: mutant aggressive cleanup no longer recovers a selected owned session.
        Mock quser { ' USERNAME SESSIONNAME ID STATE IDLE TIME'; ' alice irdp-tcp#1 42 Disc 1 10:00' }
        Mock Get-Process {
            [pscustomobject]@{ UserName = "$env:COMPUTERNAME\alice"; SessionId = 42; Id = 100; ProcessName = 'explorer'; StartTime = [datetime]::Now }
        }
        $result = & $cleanup 'alice' '' $true 42
        $result.reset_session_ids | Should -Be '42'
        Should -Invoke logoff -Times 1
        Should -Invoke rwinsta -Times 1
    }

    It 'does not infer domain ownership from the short name in quser' {
        # oracle: mutant matching short usernames authorizes cleanup of a foreign domain's session.
        Mock quser { ' USERNAME SESSIONNAME ID STATE IDLE TIME'; ' alice irdp-tcp#1 42 Disc 1 10:00' }
        Mock Get-Process {
            [pscustomobject]@{ UserName = 'FOREIGN\alice'; SessionId = 42; Id = 100; ProcessName = 'explorer'; StartTime = [datetime]::Now }
        }
        { & $cleanup 'alice' 'EXPECTED' $true 42 } | Should -Throw '*not discovered for target_user=alice*'
        Should -Invoke logoff -Times 0
        Should -Invoke rwinsta -Times 0
    }
}
