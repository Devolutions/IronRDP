BeforeAll {
    $termsrvRoot = Split-Path -Parent $PSScriptRoot
    $sourceScripts = Join-Path $termsrvRoot 'crates\ironrdp-wtsprotocol-provider\scripts'
    $workspaceRoot = (Resolve-Path (Join-Path $termsrvRoot '..\..')).Path
    $fixtureRoot = Join-Path $workspaceRoot ('artifacts\package-tests-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
    New-Item -ItemType Directory -Path $fixtureRoot | Out-Null

    $stubBodies = @{}
    foreach ($name in @(
        'preflight-side-by-side', 'backup-side-by-side-state', 'install-side-by-side',
        'verify-side-by-side', 'smoke-test-side-by-side', 'wait-termservice-ready',
        'configure-side-by-side-firewall', 'collect-side-by-side-diagnostics',
        'uninstall-side-by-side', 'restore-side-by-side-state', 'build-provider-dll'
    )) {
        $tokens = $null
        $parseErrors = $null
        $ast = [System.Management.Automation.Language.Parser]::ParseFile(
            (Join-Path $sourceScripts "$name.ps1"), [ref]$tokens, [ref]$parseErrors)
        if ($parseErrors.Count -gt 0) { throw $parseErrors[0] }

        # Keep the real parameter binder, but never copy a host-mutating script body.
        $stubBodies[$name] = $ast.ParamBlock.Extent.Text + @'

$arguments = @{}
foreach ($entry in $PSBoundParameters.GetEnumerator()) {
    $value = $entry.Value
    if ($value -is [switch]) { $value = $value.IsPresent }
    $arguments[$entry.Key] = $value
}
[ordered]@{ Name = $MyInvocation.MyCommand.Name; Arguments = $arguments } |
    ConvertTo-Json -Compress |
    Add-Content -LiteralPath (Join-Path $PSScriptRoot 'calls.jsonl') -Encoding UTF8
if ($MyInvocation.MyCommand.Name -eq 'backup-side-by-side-state.ps1') {
    Join-Path $PSScriptRoot 'saved backup'
}
if ($MyInvocation.MyCommand.Name -eq 'build-provider-dll.ps1') {
    Join-Path $PSScriptRoot "..\..\..\..\..\target\$Profile\ironrdp_wtsprotocol_provider.dll"
}
'@
    }
}

AfterAll {
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
}

Describe 'First-run configuration package' {
    BeforeEach {
        $caseRoot = Join-Path $fixtureRoot ([guid]::NewGuid().ToString('N').Substring(0, 8))
        $fixtureWorkspace = Join-Path $caseRoot 'checkout [1] '' $literal'
        $fixtureScripts = Join-Path $fixtureWorkspace 'server\termsrv\crates\ironrdp-wtsprotocol-provider\scripts'
        New-Item -ItemType Directory -Path $fixtureScripts -Force | Out-Null
        foreach ($name in 'prepare-first-run-package', 'side-by-side-defaults', 'new-side-by-side-mstsc-file') {
            Copy-Item -LiteralPath (Join-Path $sourceScripts "$name.ps1") -Destination $fixtureScripts
        }
        foreach ($name in $stubBodies.Keys) {
            Set-Content -LiteralPath (Join-Path $fixtureScripts "$name.ps1") -Value $stubBodies[$name] -Encoding UTF8
        }
        $preparePackage = Join-Path $fixtureScripts 'prepare-first-run-package.ps1'
        $callsPath = Join-Path $fixtureScripts 'calls.jsonl'
        $providerDll = Join-Path $fixtureWorkspace ('provider [1] '' $dollar `ticks &;' + [char]0xE9 + '.dll')
        Set-Content -LiteralPath $providerDll -Value 'package generation does not load this file'
        $outputDirectory = Join-Path $caseRoot 'package [1] '' $literal'
    }

    It 'replaces executable launchers and command copies with configuration and a guide' {
        # oracle: mutant obsolete launchers survive alongside the configuration.
        & $preparePackage -TargetHost localhost -ProviderDllPath $providerDll `
            -PortNumber 4489 -CreateZip:$false -OutputDirectory $outputDirectory

        $files = @(Get-ChildItem -LiteralPath $outputDirectory -File)
        @($files | Where-Object Extension -In '.ps1', '.cmd', '.txt').Count | Should -Be 0
        $files.Name | Should -Contain 'package.json'
        $files.Name | Should -Contain 'START-HERE.md'
        $files.Name | Should -Contain 'irdp-side-by-side.rdp'
    }

    It 'round-trips caller values into independently callable commands with restart=<RestartRequested>' -ForEach @(
        @{ RestartRequested = $false; Port = 4495; Endpoint = '127.0.0.1' }
        @{ RestartRequested = $true; Port = 65535; Endpoint = '[::1]' }
    ) {
        # oracle: property round-trip of chosen inputs through JSON and documented PowerShell calls.
        $listener = 'Custom '' $listener `name'
        $clsid = '{11111111-2222-3333-4444-555555555555}'
        Push-Location -LiteralPath $fixtureWorkspace
        try {
            & $preparePackage -TargetHost $Endpoint -ProviderDllPath ('.\' + (Split-Path $providerDll -Leaf)) `
                -ListenerName $listener -ProtocolManagerClsid $clsid -PortNumber $Port `
                -RestartTermService:$RestartRequested -CreateZip:$false `
                -OutputDirectory (Join-Path '..' (Split-Path $outputDirectory -Leaf))
        } finally {
            Pop-Location
        }

        $stored = Get-Content -LiteralPath (Join-Path $outputDirectory 'package.json') -Raw | ConvertFrom-Json
        $stored.workspaceRoot | Should -BeExactly $fixtureWorkspace
        $stored.providerDllPath | Should -BeExactly $providerDll
        $stored.targetHost | Should -BeExactly $Endpoint
        $stored.listenerName | Should -BeExactly $listener
        $stored.protocolManagerClsid | Should -BeExactly $clsid
        $stored.portNumber | Should -Be $Port
        $stored.restartTermService | Should -BeOfType ([bool])
        $stored.restartTermService | Should -Be $RestartRequested
        $stored.rdpFilePath | Should -BeExactly (Join-Path $outputDirectory 'irdp-side-by-side.rdp')
        Get-Content -LiteralPath $stored.rdpFilePath | Should -Contain "full address:s:${Endpoint}:$Port"

        $guide = Get-Content -LiteralPath (Join-Path $outputDirectory 'START-HERE.md') -Raw
        $blocks = [regex]::Matches($guide, '(?ms)^```powershell\r?\n(.*?)^```')
        $blocks.Count | Should -BeGreaterThan 0
        Push-Location -LiteralPath $outputDirectory
        try {
            & {
                foreach ($block in $blocks) {
                    . ([scriptblock]::Create($block.Groups[1].Value))
                }
            } | Out-Null
        } finally {
            Pop-Location
        }

        $calls = @(Get-Content -LiteralPath $callsPath | ForEach-Object { $_ | ConvertFrom-Json })
        foreach ($name in 'preflight-side-by-side', 'install-side-by-side', 'verify-side-by-side',
            'smoke-test-side-by-side', 'collect-side-by-side-diagnostics') {
            $call = @($calls | Where-Object Name -EQ "$name.ps1")
            $call.Count | Should -Be 1
            $call[0].Arguments.ProviderDllPath | Should -BeExactly $providerDll
            $call[0].Arguments.ListenerName | Should -BeExactly $listener
            $call[0].Arguments.ProtocolManagerClsid | Should -BeExactly $clsid
            $call[0].Arguments.PortNumber | Should -Be $Port
        }
        foreach ($name in 'backup-side-by-side-state', 'uninstall-side-by-side') {
            $call = @($calls | Where-Object Name -EQ "$name.ps1")
            $call.Count | Should -Be 1
            $call[0].Arguments.ListenerName | Should -BeExactly $listener
            $call[0].Arguments.ProtocolManagerClsid | Should -BeExactly $clsid
        }
        ($calls | Where-Object Name -EQ 'install-side-by-side.ps1').Arguments.RestartTermService |
            Should -Be $RestartRequested
        ($calls | Where-Object Name -EQ 'restore-side-by-side-state.ps1').Arguments.BackupDirectory |
            Should -BeExactly (Join-Path $fixtureScripts 'saved backup')
        ($calls | Where-Object Name -EQ 'wait-termservice-ready.ps1').Arguments.PortNumber | Should -Be $Port
        $firewall = @($calls | Where-Object Name -EQ 'configure-side-by-side-firewall.ps1')
        $firewall.Count | Should -Be 2
        $firewall[0].Arguments.Mode | Should -BeExactly 'Add'
        $firewall[1].Arguments.Mode | Should -BeExactly 'Remove'
        $firewall[0].Arguments.PortNumber | Should -Be $Port
        $firewall[1].Arguments.PortNumber | Should -Be $Port

        # oracle: mutant runtime smoke silently falls back to registration-only checks.
        ($calls | Where-Object Name -EQ 'smoke-test-side-by-side.ps1').Arguments.CheckLocalPortListener |
            Should -BeTrue
    }

    It 'archives the generated files and writes a matching SHA256 by default' {
        # oracle: property round-trip of package bytes through ZIP storage.
        & $preparePackage -TargetHost localhost -ProviderDllPath $providerDll `
            -PortNumber 4495 -OutputDirectory ($outputDirectory + '\')

        $zipPath = $outputDirectory + '.zip'
        Add-Type -AssemblyName System.IO.Compression.FileSystem
        $archive = [System.IO.Compression.ZipFile]::OpenRead($zipPath)
        try {
            $entries = @($archive.Entries | Where-Object { $_.Name })
            ($entries.Name | Sort-Object) -join ',' |
                Should -BeExactly 'irdp-side-by-side.rdp,package.json,START-HERE.md'
            foreach ($entry in $entries) {
                $stream = $entry.Open()
                $hash = [System.Security.Cryptography.SHA256]::Create()
                try {
                    $archivedHash = [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '')
                    $archivedHash | Should -BeExactly (
                        Get-FileHash -LiteralPath (Join-Path $outputDirectory $entry.Name) -Algorithm SHA256).Hash
                } finally {
                    $hash.Dispose()
                    $stream.Dispose()
                }
            }
        } finally {
            $archive.Dispose()
        }
        (Get-Content -LiteralPath ($zipPath + '.sha256.txt') -Raw).Trim() |
            Should -BeExactly (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash
    }

    It 'does not create an archive when disabled' {
        # oracle: property round-trip of the caller's archive choice to filesystem output.
        & $preparePackage -TargetHost localhost -ProviderDllPath $providerDll `
            -PortNumber 4495 -CreateZip:$false -OutputDirectory $outputDirectory

        Test-Path -LiteralPath ($outputDirectory + '.zip') | Should -BeFalse
        Test-Path -LiteralPath ($outputDirectory + '.zip.sha256.txt') | Should -BeFalse
    }

    It 'rejects nonempty output without deleting existing files or leaving stale launchers active' {
        # oracle: mutant generation silently mixes new configuration with obsolete launchers.
        New-Item -ItemType Directory -Path $outputDirectory | Out-Null
        $previousLauncher = Join-Path $outputDirectory 'run-install.ps1'
        Set-Content -LiteralPath $previousLauncher -Value 'existing content'

        { & $preparePackage -TargetHost localhost -ProviderDllPath $providerDll `
            -PortNumber 4495 -CreateZip:$false -OutputDirectory $outputDirectory } |
            Should -Throw '*must be empty*'
        (Get-Content -LiteralPath $previousLauncher -Raw).Trim() | Should -BeExactly 'existing content'
        @(Get-ChildItem -LiteralPath $outputDirectory).Count | Should -Be 1
    }

    It 'rejects a missing DLL before creating output' {
        # oracle: mutant a generated configuration points to a nonexistent provider.
        { & $preparePackage -TargetHost localhost -ProviderDllPath (Join-Path $caseRoot 'missing.dll') `
            -PortNumber 4495 -CreateZip:$false -OutputDirectory $outputDirectory } |
            Should -Throw '*Provider DLL does not exist*'
        Test-Path -LiteralPath $outputDirectory | Should -BeFalse
    }

    It 'detects the provider under the requested build profile without building' {
        # oracle: property round-trip of the selected profile to the recorded artifact path.
        $artifactDirectory = Join-Path $fixtureWorkspace 'target\debug'
        New-Item -ItemType Directory -Path $artifactDirectory -Force | Out-Null
        $expectedDll = Join-Path $artifactDirectory 'ironrdp_wtsprotocol_provider.dll'
        Set-Content -LiteralPath $expectedDll -Value 'existing debug build'

        & $preparePackage -TargetHost localhost -BuildProfile debug `
            -PortNumber 4495 -CreateZip:$false -OutputDirectory $outputDirectory

        $stored = Get-Content -LiteralPath (Join-Path $outputDirectory 'package.json') -Raw | ConvertFrom-Json
        $stored.providerDllPath | Should -BeExactly $expectedDll
        Test-Path -LiteralPath $callsPath | Should -BeFalse
    }

    It 'forwards build choices and records the returned DLL path' {
        # oracle: property round-trip of the caller's profile and lock choice through the build helper.
        $artifactDirectory = Join-Path $fixtureWorkspace 'target\release'
        New-Item -ItemType Directory -Path $artifactDirectory -Force | Out-Null
        $expectedDll = Join-Path $artifactDirectory 'ironrdp_wtsprotocol_provider.dll'
        Set-Content -LiteralPath $expectedDll -Value 'local build fixture'

        & $preparePackage -TargetHost localhost -BuildProvider -BuildProfile release -LockedBuild:$false `
            -PortNumber 4495 -CreateZip:$false -OutputDirectory $outputDirectory

        $call = Get-Content -LiteralPath $callsPath -Raw | ConvertFrom-Json
        $call.Name | Should -BeExactly 'build-provider-dll.ps1'
        $call.Arguments.Profile | Should -BeExactly 'release'
        $call.Arguments.Locked | Should -BeFalse
        $stored = Get-Content -LiteralPath (Join-Path $outputDirectory 'package.json') -Raw | ConvertFrom-Json
        $stored.providerDllPath | Should -BeExactly $expectedDll
    }
}
