BeforeAll {
    $termsrvRoot = Split-Path -Parent $PSScriptRoot
    $sourceScripts = Join-Path $termsrvRoot 'crates\ironrdp-wtsprotocol-provider\scripts'
    $workspaceRoot = (Resolve-Path (Join-Path $termsrvRoot '..\..')).Path
    $fixtureRoot = Join-Path $workspaceRoot ('artifacts\first-run-tests-' + [guid]::NewGuid().ToString('N'))

    $stubNames = @(
        'backup-side-by-side-state',
        'configure-side-by-side-firewall',
        'install-side-by-side',
        'preflight-side-by-side',
        'restore-side-by-side-state',
        'smoke-test-side-by-side',
        'uninstall-side-by-side',
        'verify-side-by-side',
        'wait-termservice-ready'
    )
}

Describe 'First-run side-by-side orchestration' {
    BeforeEach {
        $fixtureWorkspace = Join-Path $fixtureRoot ([guid]::NewGuid().ToString('N'))
        $fixtureScripts = Join-Path $fixtureWorkspace 'server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts'
        New-Item -ItemType Directory -Path $fixtureScripts -Force | Out-Null
        $scriptPath = Join-Path $fixtureScripts 'first-run-side-by-side.ps1'
        $callsPath = Join-Path $fixtureScripts 'calls.jsonl'
        $providerDll = Join-Path $fixtureWorkspace 'provider.dll'
        Set-Content -LiteralPath $providerDll -Value 'fixture'
        Copy-Item -LiteralPath (Join-Path $sourceScripts 'first-run-side-by-side.ps1') -Destination $scriptPath
        Copy-Item -LiteralPath (Join-Path $sourceScripts 'side-by-side-defaults.ps1') -Destination $fixtureScripts

        foreach ($name in $stubNames) {
            $sourcePath = Join-Path $sourceScripts "$name.ps1"
            $tokens = $null
            $parseErrors = $null
            $ast = [System.Management.Automation.Language.Parser]::ParseFile(
                $sourcePath, [ref]$tokens, [ref]$parseErrors)
            if ($parseErrors.Count -gt 0) { throw $parseErrors[0] }

            $stub = $ast.ParamBlock.Extent.Text + @'

$arguments = @{}
foreach ($entry in $PSBoundParameters.GetEnumerator()) {
    $value = $entry.Value
    if ($value -is [switch]) { $value = $value.IsPresent }
    $arguments[$entry.Key] = $value
}
[ordered]@{ Name = [IO.Path]::GetFileName($MyInvocation.MyCommand.Path); Arguments = $arguments } |
    ConvertTo-Json -Compress |
    Add-Content -LiteralPath $env:FIRST_RUN_TEST_CALLS -Encoding UTF8
if ([IO.Path]::GetFileName($MyInvocation.MyCommand.Path) -eq 'backup-side-by-side-state.ps1') {
    Write-Output (Join-Path $PSScriptRoot 'saved backup')
}
'@
            Set-Content -LiteralPath (Join-Path $fixtureScripts "$name.ps1") -Value $stub -Encoding UTF8
        }

        Set-Content -LiteralPath (Join-Path $fixtureScripts 'TestVm.psm1') -Value @'
function Restart-TermServiceSafely {
    [ordered]@{ Name = 'restart-termservice'; Arguments = @{} } |
        ConvertTo-Json -Compress |
        Add-Content -LiteralPath $env:FIRST_RUN_TEST_CALLS -Encoding UTF8
}
Export-ModuleMember -Function Restart-TermServiceSafely
'@ -Encoding UTF8
        $previousCallPath = $env:FIRST_RUN_TEST_CALLS
        $env:FIRST_RUN_TEST_CALLS = $callsPath
    }

    AfterEach {
        $env:FIRST_RUN_TEST_CALLS = $previousCallPath
        Remove-Module TestVm -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $fixtureWorkspace -Recurse -Force -ErrorAction SilentlyContinue
    }

    AfterAll {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force -ErrorAction SilentlyContinue
    }

    It 'backs up the selected CLSID and waits for readiness before the smoke test' {
        # oracle: mutant readiness-gated smoke runs before the requested listener readiness check.
        & $scriptPath -Mode Install -ProviderDllPath $providerDll -ProtocolManagerClsid '{11111111-2222-3333-4444-555555555555}' `
            -PortNumber 4495 -RestartTermService -WaitForServiceReadyAfterRestart -SkipFirewall

        $calls = @(Get-Content -LiteralPath $callsPath | ForEach-Object { $_ | ConvertFrom-Json })
        $backup = $calls | Where-Object Name -EQ 'backup-side-by-side-state.ps1'
        $backup.Arguments.ProtocolManagerClsid | Should -BeExactly '{11111111-2222-3333-4444-555555555555}'
        $waitIndex = -1
        $smokeIndex = -1
        for ($index = 0; $index -lt $calls.Count; $index++) {
            if ($calls[$index].Name -eq 'wait-termservice-ready.ps1') { $waitIndex = $index }
            if ($calls[$index].Name -eq 'smoke-test-side-by-side.ps1') { $smokeIndex = $index }
        }
        $waitIndex | Should -BeGreaterThan -1
        $smokeIndex | Should -BeGreaterThan $waitIndex
    }

    It 'restores the registry backup before restarting TermService on rollback' {
        # oracle: mutant rollback starts TermService before restoring the saved registration.
        & $scriptPath -Mode Rollback -ProtocolManagerClsid '{11111111-2222-3333-4444-555555555555}' `
            -PortNumber 4495 -RestartTermService -RestoreBackupOnRollback `
            -BackupDirectory (Join-Path $fixtureWorkspace 'backup') -SkipFirewall

        $calls = @(Get-Content -LiteralPath $callsPath | ForEach-Object { $_ | ConvertFrom-Json })
        ($calls.Name -join ',') | Should -BeExactly 'uninstall-side-by-side.ps1,restore-side-by-side-state.ps1,restart-termservice'
        ($calls | Where-Object Name -EQ 'uninstall-side-by-side.ps1').Arguments.RestartTermService | Should -BeNullOrEmpty
    }
}
