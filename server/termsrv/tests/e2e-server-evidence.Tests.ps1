BeforeAll {
    $collectorPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'crates\ironrdp-termsrv\scripts\e2e-server-evidence.ps1'

    function Add-ConnectionFixture {
        param($Paths, [int]$Connection = 7, [int]$Session = 12, [string]$Endpoint = '192.0.2.1:51000')
        # Provider timestamps have millisecond precision; write beyond the baseline's millisecond.
        Start-Sleep -Milliseconds 2
        $utc = [datetime]::UtcNow.ToString('o')
        $unix = [datetimeoffset]::UtcNow.ToUnixTimeMilliseconds()
        $stdoutText = @"
$utc INFO Registered incoming TCP connection connection_id=$Connection listener_name=IRDP-Tcp peer_addr=Some("$Endpoint")
$utc INFO SESSION_PROOF_TERMSRV_SET_CAPTURE_SESSION_ID_APPLIED connection_id=$Connection session_id=$Session
"@ + "`n"
        $writer = [System.IO.File]::Open($Paths.stdout, 'Append', 'Write', 'ReadWrite')
        try {
            $bytes = [System.Text.Encoding]::UTF8.GetBytes($stdoutText)
            $writer.Write($bytes, 0, $bytes.Length)
        } finally {
            $writer.Dispose()
        }
        [System.IO.File]::AppendAllText($Paths.provider[0], @"
$unix SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK source=logon_notify connection_id=$Connection session_id=$Session
$unix IWRdsProtocolConnection::NotifyCommandProcessCreated called connection_id=$Connection session_id=$Session userinit=false explorer=true logonui=false winlogon=true
$unix SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_LOAD connection_id=$Connection session_id=$Session handle_nonzero=true callback=props
$unix SESSION_PROOF_PROVIDER_VIDEO_HANDLE_SELECTED connection_id=$Connection source=driver_load_callback driver_load_session=$Session
"@ + "`n")
    }

    function New-SecurityFixture {
        param([datetime]$Time = [datetime]::UtcNow, [string]$IpPort = '51000')
        $event = [pscustomobject]@{
            Id = 4624
            TimeCreated = $Time
            Xml = "<Event><EventData><Data Name='LogonType'>10</Data><Data Name='TargetUserName'>test-user</Data><Data Name='IpAddress'>192.0.2.1</Data><Data Name='IpPort'>$IpPort</Data></EventData></Event>"
        }
        $event | Add-Member ScriptMethod ToXml { $this.Xml }
        return $event
    }
}

Describe 'Fresh server evidence' {
    BeforeEach {
        Mock Test-Path {
            $filePath = if ($LiteralPath) { $LiteralPath } else { $Path }
            $filePath.StartsWith($TestDrive) -and [System.IO.File]::Exists($filePath)
        }
        Mock Get-Process {}
        Mock Get-NetTCPConnection {}
        Mock Get-CimInstance {
            [pscustomobject]@{
                ApplicationID = '{55c92734-d682-4d71-983e-d6ec3f16059f}'
                LicenseStatus = 1
                Name = 'Windows'
            }
        }
        Mock Get-WinEvent {}
        $fixtureDirectory = Join-Path $TestDrive ([guid]::NewGuid().ToString())
        New-Item -ItemType Directory -Path $fixtureDirectory | Out-Null
        $logPaths = @{
            stdout = Join-Path $fixtureDirectory 'stdout.log'
            stderr = Join-Path $fixtureDirectory 'stderr.log'
            provider = @(Join-Path $fixtureDirectory 'provider.log')
        }
        $arguments = @{
            port = 3390
            securityStartTime = [datetime]::UtcNow
            mode = 'Provider'
            rdpUsernameForAudit = 'test-user'
            iddOpenProbeLogPath = Join-Path $TestDrive 'missing-probe.log'
            iddDebugTracePath = Join-Path $fixtureDirectory 'idd-debug.log'
        }
        $securityEvents = @()
        Mock Get-WinEvent { $securityEvents } -ParameterFilter { $FilterHashtable.LogName -eq 'Security' }
    }

    It 'does not count a Security logon from before the connection window' {
        # oracle: mutant stale E2E proof accepts Security events from before test start.
        $start = [datetime]::UtcNow
        $oldLogon = [pscustomobject]@{ Id = 4624; TimeCreated = $start.AddSeconds(-10) }
        $oldLogon | Add-Member ScriptMethod ToXml {
            '<Event><EventData><Data Name="LogonType">10</Data><Data Name="TargetUserName">test-user</Data></EventData></Event>'
        }
        Mock Get-WinEvent { $oldLogon } -ParameterFilter { $FilterHashtable.LogName -eq 'Security' }

        $result = & $collectorPath -port 3390 -securityStartTime $start -mode Provider `
            -rdpUsernameForAudit 'test-user' -iddOpenProbeLogPath 'C:\missing-evidence-probe.log'

        $result.security_4624_type10_user_count | Should -Be 0
    }

    It 'retains old log text without accepting it as current proof' {
        # oracle: mutant historical E2E connection and session markers satisfy a later run.
        [System.IO.File]::WriteAllText($logPaths.stdout, @'
2026-01-01T00:00:00Z INFO Registered incoming TCP connection connection_id=7 peer_addr=Some("192.0.2.1:51000")
2026-01-01T00:00:00Z INFO SESSION_PROOF_TERMSRV_SET_CAPTURE_SESSION_ID_APPLIED connection_id=7 session_id=12
'@ + "`n")
        [System.IO.File]::WriteAllText($logPaths.provider[0], @'
1767225600000 SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK source=logon_notify connection_id=7 session_id=12
'@ + "`n")

        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.termsrv_session_proof_marker_count | Should -Be 0
        $result.provider_session_proof_marker_count | Should -Be 0
        $result.stdout_raw | Should -Match 'connection_id=7'
        $result.dll_debug_raw | Should -Match 'session_id=12'
    }

    It 'correlates a fresh socket, provider acknowledgement, companion binding, and Security logon' {
        # oracle: mutant strict E2E proof accepts evidence from another connection or session.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeTrue
        $result.evidence_connection_id | Should -Be '7'
        $result.evidence_session_id | Should -Be '12'
        $result.security_4624_type10_correlated_count | Should -Be 1
        $result.provider_on_driver_load_count | Should -Be 1
        $result.notify_command_process_created_count | Should -Be 1
        $result.gui_target_session_id | Should -Be 12
    }

    It 'rejects a provider acknowledgement for another connection' {
        # oracle: mutant mixed connections jointly satisfy strict E2E proof.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $providerText = [System.IO.File]::ReadAllText($logPaths.provider[0]).Replace('connection_id=7', 'connection_id=8')
        [System.IO.File]::WriteAllText($logPaths.provider[0], $providerText)
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.gui_target_session_resolved | Should -BeFalse
        $result.dll_debug_raw | Should -Match 'connection_id=8'
    }

    It 'rejects disagreement between provider and companion session identities' {
        # oracle: mutant mixed sessions jointly satisfy strict E2E proof.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $providerText = [System.IO.File]::ReadAllText($logPaths.provider[0]).Replace('session_id=12', 'session_id=13')
        [System.IO.File]::WriteAllText($logPaths.provider[0], $providerText)
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.gui_target_session_resolved | Should -BeFalse
    }

    It 'keeps concurrent connections in raw diagnostics but excludes them from selected proof counts' {
        # oracle: mutant concurrent E2E runs supply each other's session or IDD proof.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        Add-ConnectionFixture $logPaths -Connection 8 -Session 13 -Endpoint '192.0.2.1:51001'
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeTrue
        $result.gui_target_session_id | Should -Be 12
        $result.provider_on_driver_load_count | Should -Be 1
        $result.provider_video_handle_driver_load_count | Should -Be 1
        $result.notify_command_process_created_count | Should -Be 1
        $result.provider_session_proof_markers | Should -Not -Match 'connection_id=8'
        $result.stdout_raw | Should -Match 'connection_id=8'
        $result.dll_debug_raw | Should -Match 'session_id=13'
    }

    It 'does not infer the tested socket from a sole unrelated fresh connection' {
        # oracle: mutant another run proves a client that never reached the server.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths -Endpoint '192.0.2.1:51001'
        $securityEvents = @(New-SecurityFixture -IpPort '51001')

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.security_4624_type10_correlated_count | Should -Be 0
        $result.security_4624_type10_user_count | Should -Be 1
    }

    It 'requires the actual client endpoint even when fresh markers agree' {
        # oracle: mutant coincident fresh logs identify the tested connection.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.evidence_correlation_reason | Should -Match 'endpoint unavailable'
    }

    It 'does not correlate Security evidence with source port <Port>' -ForEach @(
        @{ Port = '51001' }
        @{ Port = '0' }
        @{ Port = '-' }
    ) {
        # oracle: mutant an unrelated or unidentified Security logon proves the tested socket.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $securityEvents = @(New-SecurityFixture -IpPort $Port)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_log_correlated | Should -BeTrue
        $result.evidence_correlated | Should -BeFalse
        $result.security_4624_type10_correlated_count | Should -Be 0
        $result.security_4624_type10_user_count | Should -Be 1
    }

    It 'uses the target baseline rather than the caller clock for Security freshness' {
        # oracle: mutant a host-clock window admits a Security logon older than the target baseline.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $arguments.securityStartTime = [datetime]::UtcNow.AddHours(-1)
        $securityEvents = @(New-SecurityFixture -Time ([datetime]::Parse($baseline.started_utc).AddSeconds(-1)))

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.security_4624_type10_user_count | Should -Be 0
    }

    It 'rejects a rewritten prefix even when the log is longer than its baseline' {
        # oracle: mutant offset reuse after log replacement establishes fresh E2E evidence.
        [System.IO.File]::WriteAllText($logPaths.stdout, "old prefix`n")
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        [System.IO.File]::WriteAllText($logPaths.stdout, "new prefix`n")
        Add-ConnectionFixture $logPaths
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.evidence_log_errors | Should -Match 'prefix changed'
        $result.stdout_raw | Should -Match 'Registered incoming TCP connection'
    }

    It 'does not use a record that began before the baseline' {
        # oracle: mutant appending to an old partial record makes the record fresh.
        $utc = [datetime]::UtcNow.AddSeconds(1).ToString('o')
        [System.IO.File]::WriteAllText($logPaths.stdout, "$utc INFO Registered incoming TCP connection connection_id=7 ")
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        [System.IO.File]::AppendAllText($logPaths.stdout, "peer_addr=Some(`"192.0.2.1:51000`")`n")
        $unix = [datetimeoffset]::UtcNow.ToUnixTimeMilliseconds()
        [System.IO.File]::AppendAllText($logPaths.stdout, "$utc INFO SESSION_PROOF_TERMSRV_SET_CAPTURE_SESSION_ID_APPLIED connection_id=7 session_id=12`n")
        [System.IO.File]::WriteAllText($logPaths.provider[0], "$unix SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK source=logon_notify connection_id=7 session_id=12`n")
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.stdout_fresh_raw | Should -Not -Match 'Registered incoming TCP connection'
    }

    It 'preserves correlation through a remoting-style baseline serialization' {
        # oracle: mutant baseline serialization loses the target's byte offsets or prefix hashes.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        $baseline = [System.Management.Automation.PSSerializer]::Deserialize(
            [System.Management.Automation.PSSerializer]::Serialize($baseline, 10))
        Add-ConnectionFixture $logPaths
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '[::ffff:192.0.2.1]:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeTrue
    }

    It 'counts only fresh IDD trace markers for the correlated session' {
        # oracle: mutant stale or cross-session IDD markers satisfy strict graphics proof.
        [System.IO.File]::WriteAllText($arguments.iddDebugTracePath, "SESSION_PROOF_IDD_MONITOR_ARRIVED session_id=12`n")
        $baseline = & $collectorPath -iddDebugTracePath $arguments.iddDebugTracePath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        [System.IO.File]::AppendAllText($arguments.iddDebugTracePath, @'
SESSION_PROOF_IDD_MONITOR_ARRIVED session_id=12
SESSION_PROOF_IDD_COMMIT_MODES active_paths=1 session_id=999
SESSION_PROOF_IDD_FIRST_FRAME session_id=12
'@ + "`n")
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.idd_monitor_arrival_marker_count | Should -Be 1
        $result.idd_commit_modes_marker_count | Should -Be 0
        $result.idd_first_frame_marker_count | Should -Be 1
        $result.idd_debug_trace_lines | Should -Match 'session_id=12'
        $result.idd_debug_trace_lines | Should -Not -Match 'session_id=999'
    }

    It 'retains diagnostics but fails closed when no baseline was supplied' {
        # oracle: mutant fresh-looking log timestamps alone prove an unbounded E2E run.
        Add-ConnectionFixture $logPaths
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.stdout_raw | Should -Match 'Registered incoming TCP connection'
        $result.dll_debug_raw | Should -Match 'SET_CAPTURE_SESSION_ID_ACK'
    }

    It 'does not accept an old timestamp merely because its record was appended after the baseline' {
        # oracle: mutant delayed historical log writes prove a fresh connection.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $providerText = [System.IO.File]::ReadAllText($logPaths.provider[0]) -replace '(?m)^\d{13}', '1767225600000'
        [System.IO.File]::WriteAllText($logPaths.provider[0], $providerText)
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.dll_debug_fresh_raw | Should -Not -Match 'SET_CAPTURE_SESSION_ID_ACK'
        $result.dll_debug_raw | Should -Match 'SET_CAPTURE_SESSION_ID_ACK'
    }

    It 'rejects log truncation and keeps the remaining diagnostic text' {
        # oracle: mutant truncated shared logs reuse their previous offsets as proof.
        [System.IO.File]::WriteAllText($logPaths.stdout, ('x' * 4096) + "`n")
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        [System.IO.File]::WriteAllText($logPaths.stdout, '')
        Add-ConnectionFixture $logPaths
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.evidence_log_errors | Should -Match 'truncated'
        $result.stdout_raw | Should -Match 'SET_CAPTURE_SESSION_ID_APPLIED'
    }

    It 'rejects conflicting session acknowledgements for the same connection' {
        # oracle: mutant the last session marker hides a conflicting session identity.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $unix = [datetimeoffset]::UtcNow.ToUnixTimeMilliseconds()
        [System.IO.File]::AppendAllText($logPaths.provider[0], "$unix SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK source=logon_notify connection_id=7 session_id=13`n")
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.gui_target_session_resolved | Should -BeFalse
    }

    It 'rejects competing provider log identities instead of choosing the newest file' {
        # oracle: mutant multiple provider log candidates are combined into session proof.
        $alternate = Join-Path $fixtureDirectory 'provider-alternate.log'
        $logPaths.provider += $alternate
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $unix = [datetimeoffset]::UtcNow.ToUnixTimeMilliseconds()
        [System.IO.File]::WriteAllText($alternate, "$unix SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK source=logon_notify connection_id=7 session_id=13`n")
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.provider_logs_raw | Should -Match 'session_id=12'
        $result.provider_logs_raw | Should -Match 'session_id=13'
    }

    It 'does not borrow IDD callbacks from another connection' {
        # oracle: mutant unrelated IDD success satisfies the selected connection's proof counts.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $providerLines = [System.IO.File]::ReadAllLines($logPaths.provider[0]) | Where-Object { $_ -notmatch 'IDD_ON_DRIVER_LOAD|VIDEO_HANDLE_SELECTED' }
        [System.IO.File]::WriteAllLines($logPaths.provider[0], $providerLines)
        Add-ConnectionFixture $logPaths -Connection 8 -Session 13 -Endpoint '192.0.2.1:51001'
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeTrue
        $result.provider_on_driver_load_count | Should -Be 0
        $result.provider_video_handle_driver_load_count | Should -Be 0
        $result.dll_debug_raw | Should -Match 'IDD_ON_DRIVER_LOAD connection_id=8'
    }

    It 'does not correlate another domain with the configured account name' {
        # oracle: mutant a matching account leaf proves a different configured domain.
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $arguments.rdpUsernameForAudit = 'TEST-DOMAIN\test-user'
        $securityEvents = @(New-SecurityFixture)
        $securityEvents[0].Xml = $securityEvents[0].Xml.Replace('</EventData>', '<Data Name="TargetDomainName">OTHER-DOMAIN</Data></EventData>')

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.security_4624_type10_user_count | Should -Be 1
    }

    It 'captures the baseline through the serialized script block without running diagnostics' {
        # oracle: mutant the remote collector caller cannot use the pre-connect baseline.
        $scriptBlock = [scriptblock]::Create([System.IO.File]::ReadAllText($collectorPath))
        $captureArguments = @($null, $null, $null, $null, $null, $null, $null, $null, $null, $null, $true, $logPaths)

        $baseline = & $scriptBlock @captureArguments

        $baseline.started_utc | Should -Not -BeNullOrEmpty
        $baseline.logs[$logPaths.stdout].length | Should -Be 0
        Should -Invoke Get-WinEvent -Times 0 -Exactly
        Should -Invoke Get-Process -Times 0 -Exactly
        Should -Invoke Get-CimInstance -Times 0 -Exactly
        Should -Invoke Get-NetTCPConnection -Times 0 -Exactly
    }

    It 'reads active shared logs without clearing or locking out their writer' {
        # oracle: mutant baseline capture clears shared logs or blocks their writers.
        [System.IO.File]::WriteAllText($logPaths.stdout, "old text`n")
        $writer = [System.IO.File]::Open($logPaths.stdout, 'Open', 'ReadWrite', 'ReadWrite')
        try {
            $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
            Add-ConnectionFixture $logPaths
            $securityEvents = @(New-SecurityFixture)
            $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
                -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

            $result.evidence_correlated | Should -BeTrue
            $result.stdout_raw | Should -Match '^old text'
            $writer.CanWrite | Should -BeTrue
        } finally {
            $writer.Dispose()
        }
    }

    It 'fails closed if another provider candidate was unreadable at baseline' {
        # oracle: mutant unavailable logs allow proof despite competing connection or session identities.
        $alternate = Join-Path $fixtureDirectory 'unreadable-provider.log'
        $logPaths.provider += $alternate
        Mock Test-Path { throw 'Access denied' } -ParameterFilter { $LiteralPath -eq $alternate }
        $baseline = & $collectorPath -CaptureBaseline -LogPaths $logPaths
        Add-ConnectionFixture $logPaths
        $securityEvents = @(New-SecurityFixture)

        $result = & $collectorPath @arguments -EvidenceBaseline $baseline `
            -ExpectedClientEndpoint '192.0.2.1:51000' -LogPaths $logPaths

        $result.evidence_correlated | Should -BeFalse
        $result.evidence_log_errors | Should -Match 'baseline unavailable'
        $result.dll_debug_raw | Should -Match 'SET_CAPTURE_SESSION_ID_ACK'
    }
}
