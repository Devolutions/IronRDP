BeforeAll {
    $preflightPath = Join-Path $PSScriptRoot '..\crates\ironrdp-termsrv\scripts\e2e-idd-preflight.ps1'
    function bcdedit {}
    function pnputil {}
    function sc.exe {}
}

Describe 'IDD observation preparation' {
    BeforeEach {
        $reflector = @{ Running = $false }
        Mock New-Item {}
        Mock Remove-Item { throw 'Unexpected deletion' }
        Mock Set-Content {}
        Mock Test-Path { $false }
        Mock Test-Path { $true } -ParameterFilter { $LiteralPath -eq 'C:\fixture\IronRdpIdd.inf' }
        Mock Get-Content { 'UmdfExtensions=IddCx0102' }
        Mock Get-ItemProperty { [pscustomobject]@{ ProductName = 'Windows'; InstallationType = 'Client' } }
        Mock bcdedit { 'testsigning Yes'; $global:LASTEXITCODE = 0 }
        Mock pnputil { 'IronRdpIdd.inf'; $global:LASTEXITCODE = 0 }
        Mock sc.exe { $global:LASTEXITCODE = 0 }
        Mock Get-ChildItem {}
        Mock Get-PnpDevice {}
        Mock Get-CimInstance {}
        Mock Get-Command {}
        Mock Get-Service -MockWith ({
            [pscustomobject]@{ Status = $(if ($reflector.Running) { 'Running' } else { 'Stopped' }); StartType = 'Manual' }
        }.GetNewClosure())
        Mock Start-Service -MockWith ({ $reflector.Running = $true }.GetNewClosure())
        Mock Remove-Item {} -ParameterFilter { $LiteralPath -eq 'C:\fixture\debug.log' }
    }

    It 'starts an idle reflector before checking readiness without removing display devices' {
        # oracle: mutant a normally idle WUDFRd is rejected before preparation can start it.
        $result = & $preflightPath 'C:\fixture' 'C:\fixture\runtime-config.txt' 'C:\fixture\debug.log' 'C:\fixture\new-dumps' 'C:\fixture\IronRdpIdd.inf'
        $result.wudfrd_running | Should -BeTrue
        $result.issues | Should -Not -Contain 'WUDFRd is not running (status=Stopped)'
        Should -Invoke Start-Service -Times 1 -ParameterFilter { $Name -eq 'WUDFRd' }
        Should -Invoke pnputil -Times 1
        Should -Invoke Set-Content -Times 1 -ParameterFilter {
            $Path -eq 'C:\fixture\runtime-config.txt' -and $Value -contains 'dump_dir=C:\fixture\new-dumps'
        }
    }

    It 'propagates a failed native reflector configuration instead of continuing' {
        # oracle: mutant a rejected native service configuration is reported as completed preparation.
        Mock sc.exe { $global:LASTEXITCODE = 5 }
        { & $preflightPath 'C:\fixture' 'C:\fixture\runtime-config.txt' 'C:\fixture\debug.log' 'C:\fixture\new-dumps' 'C:\fixture\IronRdpIdd.inf' } |
            Should -Throw '*Could not configure WUDFRd (exit 5)*'
        Should -Invoke Start-Service -Times 0
    }
}
