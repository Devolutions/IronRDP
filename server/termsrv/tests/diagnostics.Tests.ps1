BeforeAll {
    . (Join-Path $PSScriptRoot '..\scripts\diagnostics.helpers.ps1')
}

Describe 'Diagnostic bundle failure visibility' {
    It 'retains successful sections and reports failed and warning collectors' {
        # oracle: per-collector failures must remain visible without discarding independent diagnostic evidence.
        $collectors = [ordered]@{
            provider = {
                param($path)
                '{"listener":"IRDP-Tcp"}' | Set-Content (Join-Path $path 'provider.json')
                [pscustomobject]@{ Warnings = @() }
            }
            idd = { param($path); throw 'IDD event log unavailable' }
            logon = {
                param($path)
                '{}' | Set-Content (Join-Path $path 'logon.json')
                [pscustomobject]@{ Warnings = @('Security log access denied') }
            }
        }
        $result = Invoke-TermSrvDiagnosticBundle -OutputDirectory (Join-Path $TestDrive 'bundle') `
            -ComputerName 'localhost' -Collectors $collectors
        $result.Status | Should -Be 'partial'
        $manifest = Get-Content $result.ManifestPath -Raw | ConvertFrom-Json
        $manifest.Sections[0].Status | Should -Be 'ok'
        Test-Path (Join-Path $result.OutputDirectory $manifest.Sections[0].Artifacts[0]) | Should -BeTrue
        $manifest.Sections[1].Error | Should -Be 'IDD event log unavailable'
        $manifest.Sections[2].Warnings | Should -Contain 'Security log access denied'
    }

    It 'does not label a collector with no output successful' {
        # oracle: a diagnostic section needs actual artifacts, not just a successful command exit.
        $result = Invoke-TermSrvDiagnosticBundle -OutputDirectory (Join-Path $TestDrive 'empty') `
            -ComputerName 'localhost' -Collectors ([ordered]@{ idd = { [pscustomobject]@{ Warnings = @() } } })
        $result.Status | Should -Be 'partial'
        (Get-Content $result.ManifestPath -Raw | ConvertFrom-Json).Sections[0].Error | Should -Match 'no artifacts'
    }
}
