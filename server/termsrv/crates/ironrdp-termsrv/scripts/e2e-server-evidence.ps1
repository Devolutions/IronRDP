<#
.SYNOPSIS
    Collects diagnostics without accepting historical or unrelated connection evidence.
.PARAMETER CaptureBaseline
    Run on the target immediately before connecting, then pass the returned object as EvidenceBaseline.
    The baseline records target UTC and byte offsets without clearing shared logs.
.PARAMETER EvidenceBaseline
    The pre-connect baseline, including log-prefix hashes that reject truncation or replacement.
.PARAMETER ExpectedClientEndpoint
    The tested client's actual local TCP address and port, as seen by the server.
    Strict proof requires evidence_correlated to be true in addition to the session and IDD checks.
    Missing socket identity or a Security 4624 event without the matching source port cannot prove correlation.
.PARAMETER LogPaths
    Optional stdout, stderr, and provider log paths; provider is an array of candidate paths.
#>
param(
    [Parameter(Position = 0)]$port,
    [Parameter(Position = 1)]$securityStartTime,
    [Parameter(Position = 2)]$mode,
    [Parameter(Position = 3)]$dumpDir,
    [Parameter(Position = 4)]$rdpUsernameForAudit,
    [Parameter(Position = 5)]$iddRuntimeStatePath,
    [Parameter(Position = 6)]$iddDebugTracePath,
    [Parameter(Position = 7)]$iddOpenProbeLogPath,
    [Parameter(Position = 8)]$EvidenceBaseline,
    [Parameter(Position = 9)][string]$ExpectedClientEndpoint,
    [Parameter(Position = 10)][switch]$CaptureBaseline,
    [Parameter(Position = 11)][hashtable]$LogPaths,
    [Parameter(Position = 12)][string]$RemoteRoot = $(if ($env:IRONRDP_DEPLOY_ROOT) { $env:IRONRDP_DEPLOY_ROOT } else { 'C:\IronRDPDeploy' })
)

if ($null -eq $LogPaths) {
    $LogPaths = @{
        stdout = Join-Path $RemoteRoot 'logs\ironrdp-termsrv.log'
        stderr = Join-Path $RemoteRoot 'logs\ironrdp-termsrv.err.log'
        provider = @((Join-Path $RemoteRoot 'logs\wts-provider-debug.log'), 'C:\Windows\Temp\wts-provider-debug.log')
    }
}

function Read-EvidenceLog {
    param([string]$Path)
    $snapshot = @{
        path = $Path; exists = $false; valid = $true; reason = ''
        length = 0L; creation_utc = ''; sha256 = ''; ends_with_newline = $true
        bytes = [byte[]]@(); raw = ''
    }
    $stream = $null
    $hasher = $null
    try {
        if (-not (Test-Path -LiteralPath $Path -ErrorAction Stop)) { return $snapshot }
        $snapshot.exists = $true
        $stream = [System.IO.File]::Open($Path, 'Open', 'Read', 'ReadWrite, Delete')
        $snapshot.creation_utc = [System.IO.File]::GetCreationTimeUtc($Path).ToString('o')
        $snapshot.length = $stream.Length
        $bytes = New-Object byte[] $stream.Length
        $read = 0
        while ($read -lt $bytes.Length) {
            $count = $stream.Read($bytes, $read, $bytes.Length - $read)
            if ($count -eq 0) { throw 'Log changed while being read' }
            $read += $count
        }
        $snapshot.bytes = $bytes
        $snapshot.raw = [System.Text.Encoding]::UTF8.GetString($bytes)
        $snapshot.ends_with_newline = ($bytes.Length -eq 0) -or ($bytes[$bytes.Length - 1] -eq 10)
        $hasher = [System.Security.Cryptography.SHA256]::Create()
        $snapshot.sha256 = [Convert]::ToBase64String($hasher.ComputeHash($bytes))
    } catch {
        $snapshot.valid = $false
        $snapshot.reason = "Could not read '$Path': $_"
    } finally {
        if ($null -ne $stream) { $stream.Dispose() }
        if ($null -ne $hasher) { $hasher.Dispose() }
    }
    return $snapshot
}

function Get-EvidenceTimestamp {
    param([string]$Line)
    if ($Line -match '^(\d{13})\s') {
        return ([datetime]'1970-01-01T00:00:00Z').ToUniversalTime().AddMilliseconds([long]$Matches[1])
    }
    if ($Line -match '^(\d{4}-\d{2}-\d{2}T\S+)') {
        $parsed = [datetimeoffset]::MinValue
        if ([datetimeoffset]::TryParse($Matches[1], [ref]$parsed)) { return $parsed.UtcDateTime }
    }
    return [datetime]::MinValue
}

function Get-FreshEvidenceLines {
    param($Snapshot, $BaselineEntry, [datetime]$StartUtc, [switch]$AllowUntimestamped)
    $result = @{ valid = $false; lines = @(); reason = '' }
    try {
        if (($null -eq $BaselineEntry) -or -not $BaselineEntry.valid) { throw 'Log baseline unavailable' }
        if (-not $Snapshot.valid) { throw $Snapshot.reason }
        $offset = [long]$BaselineEntry.length
        if ($BaselineEntry.exists) {
            if (-not $Snapshot.exists -or $Snapshot.creation_utc -ne $BaselineEntry.creation_utc -or $Snapshot.length -lt $offset) {
                throw 'Log was removed, replaced, or truncated'
            }
            $hasher = [System.Security.Cryptography.SHA256]::Create()
            try {
                $prefixHash = [Convert]::ToBase64String($hasher.ComputeHash($Snapshot.bytes, 0, $offset))
            } finally { $hasher.Dispose() }
            if ($prefixHash -ne $BaselineEntry.sha256) { throw 'Log prefix changed after baseline' }
        }
        $text = [System.Text.Encoding]::UTF8.GetString($Snapshot.bytes, $offset, $Snapshot.length - $offset)
        if (-not $BaselineEntry.ends_with_newline) {
            $firstNewline = $text.IndexOf("`n")
            $text = if ($firstNewline -ge 0) { $text.Substring($firstNewline + 1) } else { '' }
        }
        $lastNewline = $text.LastIndexOf("`n")
        $text = if ($lastNewline -ge 0) { $text.Substring(0, $lastNewline) } else { '' }
        $result.lines = @(
            foreach ($line in ($text -split '\r?\n')) {
                $clean = $line -replace '\x1B\[[0-?]*[ -/]*[@-~]', ''
                $timestamp = Get-EvidenceTimestamp $clean
                if (($AllowUntimestamped -and $timestamp -eq [datetime]::MinValue) -or
                    ($timestamp -ge $StartUtc -and $timestamp -le [datetime]::UtcNow)) {
                    $clean
                }
            }
        )
        $result.valid = $true
    } catch { $result.reason = [string]$_ }
    return $result
}

function ConvertTo-EvidenceEndpoint {
    param([string]$Value)
    if ($Value -notmatch '^\[?(.+?)\]?:(\d+)$') { return '' }
    $addressText = $Matches[1]
    $portText = $Matches[2]
    $address = $null
    if (-not [System.Net.IPAddress]::TryParse($addressText, [ref]$address)) { return '' }
    $number = 0
    if (-not [int]::TryParse($portText, [ref]$number) -or $number -lt 1 -or $number -gt 65535) { return '' }
    if ($address.IsIPv4MappedToIPv6) { $address = $address.MapToIPv4() }
    return "[$address]:$number"
}

function Get-EvidenceField {
    param([string]$Line, [string]$Name)
    $field = [regex]::Escape($Name)
    if ($Line -match "(?:^|\s)$field=(\d+)(?=\s|$)") { return $Matches[1] }
    return ''
}

function Select-ConnectionEvidence {
    param([string[]]$Lines, [string]$ConnectionId, [string]$SessionId, [datetime]$StartUtc)
    foreach ($line in $Lines) {
        if ((Get-EvidenceTimestamp $line) -lt $StartUtc) { continue }
        $connection = Get-EvidenceField $line 'connection_id'
        $session = Get-EvidenceField $line 'session_id'
        $driverSession = Get-EvidenceField $line 'driver_load_session'
        if ($connection -and $connection -ne $ConnectionId) { continue }
        if ($session -and $session -ne $SessionId) { continue }
        if ($driverSession -and $driverSession -ne $SessionId) { continue }
        if (-not $connection -and -not $session -and $line -match 'SESSION_PROOF_') { continue }
        $line
    }
}

function Test-EvidenceAccount {
    param($Data, [string]$Account)
    if ([string]::IsNullOrWhiteSpace($Account)) { return $false }
    if ($Account -match '^([^\\]+)\\(.+)$') {
        $domain = if ($Matches[1] -eq '.') { $env:COMPUTERNAME } else { $Matches[1] }
        return $Data.TargetUserName -eq $Matches[2] -and $Data.TargetDomainName -eq $domain
    }
    if ($Account -match '^(.+)@([^@]+)$') {
        return ($Data.TargetUserName -eq $Account) -or
            ($Data.TargetUserName -eq $Matches[1] -and $Data.TargetDomainName -eq $Matches[2])
    }
    return $Data.TargetUserName -eq $Account
}

$evidenceLogPaths = @($LogPaths.stdout, $LogPaths.stderr) + @($LogPaths.provider)
if (-not [string]::IsNullOrWhiteSpace($iddDebugTracePath) -and $iddDebugTracePath -notin $evidenceLogPaths) {
    $evidenceLogPaths += $iddDebugTracePath
}
$logSnapshots = @{}
foreach ($path in $evidenceLogPaths) {
    $logSnapshots[$path] = Read-EvidenceLog $path
}
if ($CaptureBaseline) {
    $baselines = @{}
    foreach ($path in $logSnapshots.Keys) {
        $entry = @{} + $logSnapshots[$path]
        $entry.Remove('bytes')
        $entry.Remove('raw')
        $baselines[$path] = $entry
    }
    return @{
        version = 1; run_id = [guid]::NewGuid().ToString()
        started_utc = [datetime]::UtcNow.ToString('o'); logs = $baselines
    }
}

$correlationReason = 'Pre-connect baseline unavailable'
$baselineValid = $false
try {
    if ($EvidenceBaseline.version -eq 1 -and $EvidenceBaseline.run_id) {
        $securityStartTime = [datetimeoffset]::Parse($EvidenceBaseline.started_utc).UtcDateTime
        $baselineValid = $securityStartTime -le [datetime]::UtcNow
    }
} catch { $correlationReason = "Invalid pre-connect baseline: $_" }
$freshLogs = @{}
foreach ($path in $logSnapshots.Keys) {
    $baselineEntry = if ($baselineValid) { $EvidenceBaseline.logs[$path] } else { $null }
    $allowUntimestamped = $path -eq $iddDebugTracePath
    $freshLogs[$path] = Get-FreshEvidenceLines $logSnapshots[$path] $baselineEntry $securityStartTime -AllowUntimestamped:$allowUntimestamped
}

$connectionId = ''
$sessionId = ''
$correlatedProviderPath = ''
$logCorrelated = $false
$expectedEndpoint = ConvertTo-EvidenceEndpoint $ExpectedClientEndpoint
$freshStdout = @($freshLogs[$LogPaths.stdout].lines)
if ($baselineValid) {
    $correlationReason = 'Tested client TCP endpoint unavailable'
    if ($expectedEndpoint) {
        $registrations = @($freshStdout | Where-Object {
            if ($_ -notmatch 'Registered incoming TCP connection') { return $false }
            if ($_ -notmatch 'peer_addr=Some\("([^"]+)"\)') { return $false }
            (ConvertTo-EvidenceEndpoint $Matches[1]) -eq $expectedEndpoint
        })
        $correlationReason = "Expected exactly one fresh TCP registration for the tested endpoint; found $($registrations.Count)"
        if ($registrations.Count -eq 1) {
            $connectionId = Get-EvidenceField $registrations[0] 'connection_id'
            $registrationUtc = Get-EvidenceTimestamp $registrations[0]
            # Provider timestamps cannot distinguish events within the same millisecond.
            $registrationUtc = $registrationUtc.AddTicks(-($registrationUtc.Ticks % [timespan]::TicksPerMillisecond))
            $sameIdRegistrations = @($freshStdout | Where-Object {
                $_ -match 'Registered incoming TCP connection' -and (Get-EvidenceField $_ 'connection_id') -eq $connectionId
            })
            $applied = @($freshStdout | Where-Object {
                $_ -match 'SESSION_PROOF_TERMSRV_SET_CAPTURE_SESSION_ID_APPLIED' -and
                    (Get-EvidenceField $_ 'connection_id') -eq $connectionId -and
                    (Get-EvidenceTimestamp $_) -ge $registrationUtc
            })
            $sessionIds = @($applied | ForEach-Object { Get-EvidenceField $_ 'session_id' } | Sort-Object -Unique)
            $correlationReason = 'Missing or ambiguous capture-session binding for the tested connection'
            if ($connectionId -and $sameIdRegistrations.Count -eq 1 -and $sessionIds.Count -eq 1 -and $sessionIds[0] -match '^[1-9]\d*$') {
                $sessionId = $sessionIds[0]
                $providerAcks = @{}
                foreach ($path in $LogPaths.provider) {
                    $acks = @($freshLogs[$path].lines | Where-Object {
                        $_ -match 'SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK' -and
                            (Get-EvidenceField $_ 'connection_id') -eq $connectionId -and
                            (Get-EvidenceTimestamp $_) -ge $registrationUtc
                    })
                    if ($acks.Count -gt 0) { $providerAcks[$path] = $acks }
                }
                $providerPaths = @($providerAcks.Keys)
                $invalidProviderLogs = @($LogPaths.provider | Where-Object { -not $freshLogs[$_].valid })
                $correlationReason = 'Provider acknowledgement does not uniquely match the companion connection and session'
                if ($providerPaths.Count -eq 1 -and $invalidProviderLogs.Count -eq 0 -and $freshLogs[$LogPaths.stdout].valid) {
                    $ackSessions = @($providerAcks[$providerPaths[0]] | ForEach-Object { Get-EvidenceField $_ 'session_id' } | Sort-Object -Unique)
                    if ($ackSessions.Count -eq 1 -and $ackSessions[0] -eq $sessionId) {
                        $correlatedProviderPath = $providerPaths[0]
                        $logCorrelated = $true
                        $correlationReason = 'Matching connection and session; matching Security logon not yet observed'
                    }
                }
            }
        }
    }
}
            $result = @{}
            $result['evidence_correlated'] = $false
            $result['evidence_log_correlated'] = $logCorrelated
            $result['evidence_correlation_reason'] = $correlationReason
            $result['evidence_run_id'] = if ($baselineValid) { [string]$EvidenceBaseline.run_id } else { '' }
            $result['evidence_started_utc'] = if ($baselineValid) { $securityStartTime.ToString('o') } else { '' }
            $result['evidence_connection_id'] = $connectionId
            $result['evidence_session_id'] = $sessionId
            $result['evidence_client_endpoint'] = $expectedEndpoint
            $result['evidence_log_errors'] = @($freshLogs.Keys | Where-Object { -not $freshLogs[$_].valid } | ForEach-Object {
                "$_`: $($freshLogs[$_].reason)"
            }) -join "`n"
            $result['idd_debug_trace_valid'] = $false
            $result['idd_debug_trace_lines'] = ''
            $result['idd_monitor_arrival_marker_count'] = 0
            $result['idd_display_config_update_request_count'] = 0
            $result['idd_display_config_update_success_count'] = 0
            $result['idd_commit_modes_marker_count'] = 0
            $result['idd_commit_modes_active_path_count'] = 0
            $result['idd_swapchain_assigned_count'] = 0
            $result['idd_session_ready_marker_count'] = 0
            $result['idd_first_frame_marker_count'] = 0
            if (-not [string]::IsNullOrWhiteSpace($iddDebugTracePath) -and $freshLogs.ContainsKey($iddDebugTracePath)) {
                $iddTrace = $freshLogs[$iddDebugTracePath]
                $result['idd_debug_trace_valid'] = $iddTrace.valid
                if ($iddTrace.valid -and $sessionId) {
                    $iddLines = @(Select-ConnectionEvidence -Lines $iddTrace.lines -ConnectionId '' -SessionId $sessionId -StartUtc ([datetime]::MinValue))
                    $iddText = $iddLines -join "`n"
                    $result['idd_debug_trace_lines'] = $iddText
                    $result['idd_monitor_arrival_marker_count'] = [regex]::Matches($iddText, 'SESSION_PROOF_IDD_MONITOR_ARRIVED').Count
                    $result['idd_display_config_update_request_count'] = [regex]::Matches($iddText, 'SESSION_PROOF_IDD_DISPLAY_CONFIG_UPDATE_REQUEST').Count
                    $result['idd_display_config_update_success_count'] = [regex]::Matches($iddText, 'SESSION_PROOF_IDD_DISPLAY_CONFIG_UPDATE_RESULT[^\r\n]*status=0x00000000').Count
                    $result['idd_commit_modes_marker_count'] = [regex]::Matches($iddText, 'SESSION_PROOF_IDD_COMMIT_MODES').Count
                    $result['idd_commit_modes_active_path_count'] = [regex]::Matches($iddText, 'SESSION_PROOF_IDD_COMMIT_MODES[^\r\n]*active_paths=([1-9]\d*)').Count
                    $result['idd_swapchain_assigned_count'] = [regex]::Matches($iddText, 'SESSION_PROOF_IDD_SWAPCHAIN_ASSIGNED').Count
                    $result['idd_session_ready_marker_count'] = [regex]::Matches($iddText, 'SESSION_PROOF_IDD_SESSION_READY_FOR_CAPTURE').Count
                    $result['idd_first_frame_marker_count'] = [regex]::Matches($iddText, 'SESSION_PROOF_IDD_FIRST_FRAME').Count
                }
            }
            $result['security_4624_type10_correlated_count'] = 0
            $logOut = $LogPaths.stdout
            $logErr = $LogPaths.stderr
            $dllDebugLogCandidates = @($LogPaths.provider)
            $dllDebugLog = $null
            $dllDebugLogLastWriteUtc = [datetime]::MinValue
            $dllDebugLogLength = -1
            $stdoutLines = @()
            $stdoutTail = $null
            $dllLines = @()

            $result['security_4624_type10_user_count'] = 0
            $result['termsrv_fallback_marker_count'] = 0
            $result['termsrv_fallback_markers'] = ''
            $result['provider_session_proof_marker_count'] = 0
            $result['provider_session_proof_markers'] = ''
            $result['termsrv_session_proof_marker_count'] = 0
            $result['termsrv_session_proof_markers'] = ''
            $result['idd_driver_loaded_notified'] = $false
            $result['idd_wddm_enabled_signal_count'] = 0
            $result['provider_video_handle_marker_count'] = 0
            $result['provider_video_handle_markers'] = ''
            $result['termsrv_idd_marker_count'] = 0
            $result['termsrv_idd_markers'] = ''
            $result['provider_logon_policy_marker_count'] = 0
            $result['provider_logon_policy_markers'] = ''
            $result['provider_get_hardware_id_count'] = 0
            $result['provider_on_driver_load_count'] = 0
            $result['provider_assume_present_count'] = 0
            $result['provider_video_handle_driver_load_count'] = 0
            $result['remote_connection_signal_count'] = 0
            $result['remote_graphics_signal_count'] = 0
            $result['remote_connection_signals'] = ''
            $result['gui_target_session_id'] = -1
            $result['gui_target_session_source'] = ''
            $result['gui_target_session_resolved'] = $false
            $result['gui_target_session_process_proof'] = $false
            $result['gui_target_session_explorer_count'] = 0
            $result['gui_target_session_gui_process_count'] = 0
            $result['gui_target_session_winlogon_count'] = 0
            $result['gui_target_session_logonui_count'] = 0
            $result['gui_target_session_processes'] = ''
            $result['gui_target_session_explorer_bootstrap_pid'] = 0
            $result['gui_target_session_explorer_bootstrap_time_utc'] = ''
            $result['gui_target_session_explorer_bootstrap_pid_alive'] = $false
            $result['gui_target_session_explorer_earliest_start_utc'] = ''
            $result['gui_target_session_explorer_after_creation_known'] = $false
            $result['gui_target_session_explorer_after_creation'] = $false
            $result['gui_target_session_creation_signal_utc'] = ''
            $result['gui_target_session_creation_signals'] = ''
            $result['gui_target_session_explorer_details'] = ''
            $result['explorer_crash_event_count'] = 0
            $result['explorer_hang_event_count'] = 0
            $result['explorer_wer_event_count'] = 0
            $result['explorer_lifecycle_events'] = ''
            $result['dll_debug_path'] = ''
            $result['idd_open_probe'] = ''

            $termsrvSessionProofLines = @()
            $providerSessionProofLines = @()

            foreach ($candidate in $dllDebugLogCandidates) {
                if (-not (Test-Path $candidate)) {
                    continue
                }

                $candidateLength = 0
                $candidateLastWriteUtc = [datetime]::MinValue
                try {
                    $candidateInfo = Get-Item -LiteralPath $candidate -ErrorAction Stop
                    $candidateLength = [int64]$candidateInfo.Length
                    $candidateLastWriteUtc = $candidateInfo.LastWriteTimeUtc
                }
                catch {
                    $candidateLength = 0
                    $candidateLastWriteUtc = [datetime]::MinValue
                }

                if (($null -eq $dllDebugLog) -or
                    ($candidateLastWriteUtc -gt $dllDebugLogLastWriteUtc) -or
                    (($candidateLastWriteUtc -eq $dllDebugLogLastWriteUtc) -and ($candidateLength -gt $dllDebugLogLength))) {
                    $dllDebugLog = $candidate
                    $dllDebugLogLastWriteUtc = $candidateLastWriteUtc
                    $dllDebugLogLength = $candidateLength
                }
            }

            if ($correlatedProviderPath) { $dllDebugLog = $correlatedProviderPath }
            if ($null -ne $dllDebugLog) {
                $result['dll_debug_path'] = [string]$dllDebugLog
            }
            $result['provider_logs_raw'] = (@($dllDebugLogCandidates | ForEach-Object {
                "$_`n$($logSnapshots[$_].raw)"
            }) -join "`n")

            $targetUserName = $rdpUsernameForAudit
            if (-not [string]::IsNullOrWhiteSpace($targetUserName)) {
                if ($targetUserName.Contains('\')) {
                    $targetUserName = $targetUserName.Split('\')[-1]
                }
                if ($targetUserName.Contains('@')) {
                    $targetUserName = $targetUserName.Split('@')[0]
                }
            }

            if (Test-Path $logOut) {
                $stdoutLines = $freshStdout
                if ($logCorrelated) { $stdoutLines = @(Select-ConnectionEvidence $stdoutLines $connectionId $sessionId $registrationUtc) }
                $stdoutTail = $stdoutLines | Select-Object -Last 200
                $result['stdout'] = ($logSnapshots[$logOut].raw -split '\r?\n' | Select-Object -Last 200 | Out-String)
                $result['stdout_raw'] = $logSnapshots[$logOut].raw
                $result['stdout_fresh_raw'] = ($freshStdout | Out-String)
            }
            if (Test-Path $logErr) {
                $stderrLines = $logSnapshots[$logErr].raw -split '\r?\n'
                $result['stderr'] = ($stderrLines | Select-Object -Last 80 | Out-String)
                $result['stderr_raw'] = $logSnapshots[$logErr].raw
            }
            if ($mode -eq 'Provider' -and $stdoutLines) {
                $fallbackPatterns = @(
                    'falling back to guessed session',
                    'sending synthetic test pattern',
                    'falling back to in-process GDI',
                    'prelogon token fallback disabled after initial helper start'
                )
                $fallbackHits = $stdoutLines | Select-String -SimpleMatch -Pattern $fallbackPatterns -ErrorAction SilentlyContinue
                if ($fallbackHits) {
                    $result['termsrv_fallback_marker_count'] = ($fallbackHits | Measure-Object).Count
                    $result['termsrv_fallback_markers'] = ($fallbackHits | Select-Object -Last 20 | ForEach-Object { $_.Line } | Out-String)
                }

                $termsrvSessionProofHits = $stdoutLines | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_TERMSRV_' -ErrorAction SilentlyContinue
                if ($termsrvSessionProofHits) {
                    $termsrvSessionProofLines = $termsrvSessionProofHits | ForEach-Object { $_.Line }
                    $result['termsrv_session_proof_marker_count'] = ($termsrvSessionProofHits | Measure-Object).Count
                    $result['termsrv_session_proof_markers'] = ($termsrvSessionProofLines | Select-Object -Last 40 | Out-String)
                }

                $termsrvShellBootstrapHits = $stdoutLines | Select-String -SimpleMatch -Pattern @(
                    'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_ATTEMPT',
                    'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS',
                    'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_ERROR'
                ) -ErrorAction SilentlyContinue
                if ($termsrvShellBootstrapHits) {
                    $termsrvShellBootstrapLines = $termsrvShellBootstrapHits | ForEach-Object { $_.Line }
                    $existing = [string]$result['termsrv_session_proof_markers']
                    $bootstrap = ($termsrvShellBootstrapLines | Select-Object -Last 40 | Out-String)
                    $result['termsrv_session_proof_markers'] = ($existing + $bootstrap)
                }

                $termsrvIddHits = $stdoutLines | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_TERMSRV_IDD_' -ErrorAction SilentlyContinue
                if ($termsrvIddHits) {
                    $termsrvIddLines = $termsrvIddHits | ForEach-Object { $_.Line }
                    $result['termsrv_idd_marker_count'] = ($termsrvIddHits | Measure-Object).Count
                    $result['termsrv_idd_markers'] = ($termsrvIddLines | Select-Object -Last 40 | Out-String)
                }
            }
            if ($mode -eq 'Provider' -and $null -ne $dllDebugLog -and (Test-Path $dllDebugLog)) {
                # The provider debug log can be extremely noisy due to polling. Capture a signal-focused view.
                $dllLines = @($freshLogs[$dllDebugLog].lines)
                $result['dll_debug_fresh_raw'] = ($dllLines | Out-String)
                if ($logCorrelated) { $dllLines = @(Select-ConnectionEvidence $dllLines $connectionId $sessionId $registrationUtc) }
                $patterns = @(
                    'IWRdsProtocolManager::',
                    'IWRdsProtocolListener::',
                    'IWRdsProtocolListenerCallback::',
                    'IWRdsProtocolConnection::',
                    'SESSION_PROOF_PROVIDER_',
                    'SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_LOAD',
                    'IWRdsWddmIddProps::',
                    'GetUserCredentials ok',
                    'NotifyCommandProcessCreated',
                    'IsUserAllowedToLogon',
                    'LogonNotify',
                    'SessionArbitrationEnumeration',
                    'DisconnectNotify',
                    'Close called',
                    'PreDisconnect'
                )

                $result['dll_debug'] = ($logSnapshots[$dllDebugLog].raw -split '\r?\n' | Select-Object -Last 200 | Out-String)
                $result['dll_debug_raw'] = $logSnapshots[$dllDebugLog].raw
                $matchLines = $dllLines | Select-String -SimpleMatch -Pattern $patterns -ErrorAction SilentlyContinue
                if ($matchLines) {
                    $result['dll_debug_key'] = ($matchLines | Select-Object -Last 400 | ForEach-Object { $_.Line } | Out-String)
                }

                $providerSessionProofHits = $dllLines | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_PROVIDER_' -ErrorAction SilentlyContinue
                if ($providerSessionProofHits) {
                    $providerSessionProofLines = $providerSessionProofHits | ForEach-Object { $_.Line }
                    $result['provider_session_proof_marker_count'] = ($providerSessionProofHits | Measure-Object).Count
                    $result['provider_session_proof_markers'] = ($providerSessionProofLines | Select-Object -Last 40 | Out-String)
                }

                $providerVideoHandleHits = $dllLines | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_PROVIDER_VIDEO_HANDLE_' -ErrorAction SilentlyContinue
                if ($providerVideoHandleHits) {
                    $providerVideoHandleLines = $providerVideoHandleHits | ForEach-Object { $_.Line }
                    $result['provider_video_handle_marker_count'] = ($providerVideoHandleHits | Measure-Object).Count
                    $result['provider_video_handle_markers'] = ($providerVideoHandleLines | Select-Object -Last 40 | Out-String)
                }

                $providerLogonPolicyHits = $dllLines | Select-String -SimpleMatch -Pattern @(
                    'SESSION_PROOF_PROVIDER_SUPPRESS_LOGON_UI',
                    'SESSION_PROOF_PROVIDER_UNIVERSAL_APPS',
                    'SESSION_PROOF_PROVIDER_LOGON_GATE'
                ) -ErrorAction SilentlyContinue
                if ($providerLogonPolicyHits) {
                    $providerLogonPolicyLines = $providerLogonPolicyHits | ForEach-Object { $_.Line }
                    $result['provider_logon_policy_marker_count'] = ($providerLogonPolicyHits | Measure-Object).Count
                    $result['provider_logon_policy_markers'] = ($providerLogonPolicyLines | Select-Object -Last 40 | Out-String)
                }

                $iddLoadHit = $dllLines | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_LOAD' -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($iddLoadHit) {
                    $result['idd_driver_loaded_notified'] = $true
                }

                $iddWddmEnableHits = $dllLines | Select-String -SimpleMatch -Pattern 'IWRdsWddmIddProps::EnableWddmIdd enabled=true' -ErrorAction SilentlyContinue
                if ($iddWddmEnableHits) {
                    $result['idd_wddm_enabled_signal_count'] = ($iddWddmEnableHits | Measure-Object).Count
                }

                $notifyCommandProcessCreatedHits = $dllLines | Select-String -SimpleMatch -Pattern 'IWRdsProtocolConnection::NotifyCommandProcessCreated called' -ErrorAction SilentlyContinue
                if ($notifyCommandProcessCreatedHits) {
                    $result['notify_command_process_created_count'] = ($notifyCommandProcessCreatedHits | Measure-Object).Count
                } else {
                    $result['notify_command_process_created_count'] = 0
                }

                $providerGetHardwareIdHits = $dllLines | Select-String -Pattern "IWRdsWddmIddProps::GetHardwareId wrote 'IronRdpIdd'" -ErrorAction SilentlyContinue
                if ($providerGetHardwareIdHits) {
                    $result['provider_get_hardware_id_count'] = ($providerGetHardwareIdHits | Measure-Object).Count
                }

                $providerOnDriverLoadHits = $dllLines | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_PROVIDER_IDD_ON_DRIVER_LOAD' -ErrorAction SilentlyContinue
                if ($providerOnDriverLoadHits) {
                    $result['provider_on_driver_load_count'] = @($providerOnDriverLoadHits | Where-Object { $_.Line -match 'handle_nonzero=true' }).Count
                }

                $providerAssumePresentHits = $dllLines | Select-String -SimpleMatch -Pattern 'ASSUME_PRESENT' -ErrorAction SilentlyContinue
                if ($providerAssumePresentHits) {
                    $result['provider_assume_present_count'] = ($providerAssumePresentHits | Measure-Object).Count
                }

                $providerVideoHandleDriverLoadHits = $dllLines | Select-String -SimpleMatch -Pattern 'SESSION_PROOF_PROVIDER_VIDEO_HANDLE_SELECTED' -ErrorAction SilentlyContinue |
                    Where-Object { $_.Line -match 'source=driver_load_callback' }
                if ($providerVideoHandleDriverLoadHits) {
                    $result['provider_video_handle_driver_load_count'] = ($providerVideoHandleDriverLoadHits | Measure-Object).Count
                }
            }

            if ($mode -eq 'Provider') {
                $targetSessionId = $null
                $targetSessionSource = ''

                $termsrvBootstrapSuccessLines = $termsrvSessionProofLines | Where-Object {
                    $_ -match 'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS'
                }
                if ($termsrvBootstrapSuccessLines) {
                    $lastBootstrapLine = $termsrvBootstrapSuccessLines | Select-Object -Last 1
                    if ($lastBootstrapLine -match 'explorer_pid=(\d+)') {
                        $result['gui_target_session_explorer_bootstrap_pid'] = [int]$Matches[1]
                    }
                    if ($lastBootstrapLine -match '^(\d{4}-\d{2}-\d{2}T[^\s]+Z)') {
                        $result['gui_target_session_explorer_bootstrap_time_utc'] = [string]$Matches[1]
                    }
                }

                $termsrvProofText = [string]$result['termsrv_session_proof_markers']
                if (-not [string]::IsNullOrWhiteSpace($termsrvProofText)) {
                    $bootstrapPidMatches = [regex]::Matches($termsrvProofText, 'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS[^\r\n]*explorer_pid=(\d+)')
                    if ($bootstrapPidMatches.Count -gt 0) {
                        $lastPidMatch = $bootstrapPidMatches[$bootstrapPidMatches.Count - 1]
                        $result['gui_target_session_explorer_bootstrap_pid'] = [int]$lastPidMatch.Groups[1].Value
                    }

                    if ($result['gui_target_session_explorer_bootstrap_pid'] -le 0) {
                        $genericPidMatches = [regex]::Matches($termsrvProofText, 'explorer_pid=(\d+)')
                        if ($genericPidMatches.Count -gt 0) {
                            $lastGenericPid = $genericPidMatches[$genericPidMatches.Count - 1]
                            $result['gui_target_session_explorer_bootstrap_pid'] = [int]$lastGenericPid.Groups[1].Value
                        }
                    }

                    $bootstrapTimeMatches = [regex]::Matches($termsrvProofText, '(\d{4}-\d{2}-\d{2}T[^\s]+Z)[^\r\n]*SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS')
                    if ($bootstrapTimeMatches.Count -gt 0) {
                        $lastTimeMatch = $bootstrapTimeMatches[$bootstrapTimeMatches.Count - 1]
                        $result['gui_target_session_explorer_bootstrap_time_utc'] = [string]$lastTimeMatch.Groups[1].Value
                    }
                }

                if (($result['gui_target_session_explorer_bootstrap_pid'] -le 0) -and $stdoutTail) {
                    $rawBootstrapHits = $stdoutTail | Select-String -Pattern 'SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS.*explorer_pid=(\d+)' -ErrorAction SilentlyContinue
                    if ($rawBootstrapHits) {
                        $lastRawBootstrap = $rawBootstrapHits | Select-Object -Last 1
                        if ($lastRawBootstrap.Matches.Count -gt 0) {
                            $result['gui_target_session_explorer_bootstrap_pid'] = [int]$lastRawBootstrap.Matches[0].Groups[1].Value
                        }
                        if ($lastRawBootstrap.Line -match '^(\d{4}-\d{2}-\d{2}T[^\s]+Z)') {
                            $result['gui_target_session_explorer_bootstrap_time_utc'] = [string]$Matches[1]
                        }
                    }
                }

                $providerAckLines = $providerSessionProofLines | Where-Object {
                    $_ -match 'SESSION_PROOF_PROVIDER_SET_CAPTURE_SESSION_ID_ACK'
                }

                if ($logCorrelated -and $providerAckLines) {
                    foreach ($line in $providerAckLines) {
                        if ($line -match 'source=([^\s]+)') {
                            $targetSessionSource = [string]$Matches[1]
                        }
                        if ($line -match 'session_id=(\d+)') {
                            $targetSessionId = [int]$Matches[1]
                        }
                    }
                }

                if ($logCorrelated -and $null -eq $targetSessionId) {
                    $termsrvApplyLines = $termsrvSessionProofLines | Where-Object {
                        $_ -match 'SESSION_PROOF_TERMSRV_SET_CAPTURE_SESSION_ID_APPLIED'
                    }

                    if ($termsrvApplyLines) {
                        foreach ($line in $termsrvApplyLines) {
                            if ($line -match 'session_id=(\d+)') {
                                $targetSessionId = [int]$Matches[1]
                            }
                        }

                        if ($null -ne $targetSessionId) {
                            $targetSessionSource = 'termsrv_applied'
                        }
                    }
                }

                if ($null -ne $targetSessionId) {
                    $result['gui_target_session_id'] = $targetSessionId
                    $result['gui_target_session_source'] = $targetSessionSource
                    $result['gui_target_session_resolved'] = $true

                    try {
                        $sessionProcesses = Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $targetSessionId }
                        $explorerProcesses = @($sessionProcesses | Where-Object { $_.ProcessName -ieq 'explorer' })

                        $explorerCount = ($explorerProcesses | Measure-Object).Count
                        $guiProcessCount = ($sessionProcesses | Where-Object {
                            $_.ProcessName -in @('explorer', 'dwm', 'ShellExperienceHost', 'sihost')
                        } | Measure-Object).Count
                        $winlogonCount = ($sessionProcesses | Where-Object { $_.ProcessName -ieq 'winlogon' } | Measure-Object).Count
                        $logonUiCount = ($sessionProcesses | Where-Object { $_.ProcessName -ieq 'LogonUI' } | Measure-Object).Count

                        $result['gui_target_session_explorer_count'] = $explorerCount
                        $result['gui_target_session_gui_process_count'] = $guiProcessCount
                        $result['gui_target_session_winlogon_count'] = $winlogonCount
                        $result['gui_target_session_logonui_count'] = $logonUiCount
                        $result['gui_target_session_process_proof'] = ($explorerCount -ge 1)
                        $result['gui_target_session_processes'] = ($sessionProcesses | Select-Object -First 40 Id, ProcessName, SessionId | Sort-Object ProcessName, Id | Format-Table -AutoSize | Out-String)

                        if ($explorerCount -ge 1) {
                            $explorerRows = New-Object System.Collections.Generic.List[object]
                            $explorerEarliestStartUtc = $null

                            foreach ($explorerProc in ($explorerProcesses | Sort-Object Id)) {
                                $startUtc = ''
                                try {
                                    $startLocal = $explorerProc.StartTime
                                    if ($null -ne $startLocal) {
                                        $startUtcDate = $startLocal.ToUniversalTime()
                                        $startUtc = $startUtcDate.ToString('o')
                                        if (($null -eq $explorerEarliestStartUtc) -or ($startUtcDate -lt $explorerEarliestStartUtc)) {
                                            $explorerEarliestStartUtc = $startUtcDate
                                        }
                                    }
                                } catch {
                                    # Best-effort only: process may exit while sampling details.
                                }

                                $cpuSeconds = 0.0
                                try {
                                    if ($null -ne $explorerProc.CPU) {
                                        $cpuSeconds = [double]$explorerProc.CPU
                                    }
                                } catch {
                                    $cpuSeconds = 0.0
                                }

                                $respondingValue = 'n/a'
                                try {
                                    if (($null -ne $explorerProc.MainWindowHandle) -and ($explorerProc.MainWindowHandle -ne 0)) {
                                        $respondingValue = [string]([bool]$explorerProc.Responding)
                                    }
                                } catch {
                                    $respondingValue = 'n/a'
                                }

                                $explorerRows.Add([pscustomobject]@{
                                    Id           = $explorerProc.Id
                                    StartTimeUtc = $startUtc
                                    CpuSec       = [math]::Round($cpuSeconds, 2)
                                    Handles      = $explorerProc.HandleCount
                                    Responding   = $respondingValue
                                })
                            }

                            if ($null -ne $explorerEarliestStartUtc) {
                                $result['gui_target_session_explorer_earliest_start_utc'] = $explorerEarliestStartUtc.ToString('o')
                            }

                            $result['gui_target_session_explorer_details'] = ($explorerRows | Sort-Object StartTimeUtc, Id | Format-Table -AutoSize | Out-String)

                            if ($result['gui_target_session_explorer_bootstrap_pid'] -le 0) {
                                $bootstrapTimeUtcRaw = [string]$result['gui_target_session_explorer_bootstrap_time_utc']
                                if (-not [string]::IsNullOrWhiteSpace($bootstrapTimeUtcRaw)) {
                                    try {
                                        $bootstrapTimeUtc = [datetime]::Parse($bootstrapTimeUtcRaw)
                                        $closestExplorer = $null
                                        $closestDelta = [timespan]::MaxValue

                                        foreach ($row in $explorerRows) {
                                            if ([string]::IsNullOrWhiteSpace([string]$row.StartTimeUtc)) {
                                                continue
                                            }

                                            $rowStartUtc = [datetime]::Parse([string]$row.StartTimeUtc)
                                            $delta = ($rowStartUtc - $bootstrapTimeUtc).Duration()
                                            if ($delta -lt $closestDelta) {
                                                $closestDelta = $delta
                                                $closestExplorer = $row
                                            }
                                        }

                                        if (($null -ne $closestExplorer) -and ($closestDelta.TotalSeconds -le 5)) {
                                            $result['gui_target_session_explorer_bootstrap_pid'] = [int]$closestExplorer.Id
                                        }
                                    } catch {
                                        # Best-effort only.
                                    }
                                }
                            }
                        }

                        $bootstrapExplorerPid = [int]$result['gui_target_session_explorer_bootstrap_pid']
                        if ($bootstrapExplorerPid -gt 0) {
                            $bootstrapExplorerProc = Get-Process -Id $bootstrapExplorerPid -ErrorAction SilentlyContinue
                            $result['gui_target_session_explorer_bootstrap_pid_alive'] = ($null -ne $bootstrapExplorerProc)
                        }

                        try {
                            $start = $securityStartTime
                            $maxSessionEvents = 500
                            $targetSessionLifecycleEventIds = @(21, 22, 24, 25)
                            $sessionEvents = Get-WinEvent -FilterHashtable @{ LogName = 'Microsoft-Windows-TerminalServices-LocalSessionManager/Operational'; StartTime = $start } -MaxEvents $maxSessionEvents -ErrorAction SilentlyContinue

                            $sessionRows = New-Object System.Collections.Generic.List[object]
                            foreach ($evt in $sessionEvents) {
                                try {
                                    if (($evt.Id -notin $targetSessionLifecycleEventIds) -or ($evt.TimeCreated.ToUniversalTime() -lt $start)) {
                                        continue
                                    }

                                    $xml = [xml]$evt.ToXml()
                                    $data = @{}
                                    foreach ($d in $xml.Event.EventData.Data) { $data[$d.Name] = [string]$d.'#text' }

                                    $eventSessionId = $null
                                    if ($data.ContainsKey('SessionID') -and ($data.SessionID -match '^\d+$')) {
                                        $eventSessionId = [int]$data.SessionID
                                    } elseif ($data.ContainsKey('SessionId') -and ($data.SessionId -match '^\d+$')) {
                                        $eventSessionId = [int]$data.SessionId
                                    } elseif ($evt.Message -match '(?i)\bsession\s+(\d+)\b') {
                                        $eventSessionId = [int]$Matches[1]
                                    }

                                    if (($null -eq $eventSessionId) -or ($eventSessionId -ne $targetSessionId)) {
                                        continue
                                    }

                                    $sessionRows.Add([pscustomobject]@{
                                        TimeCreated = $evt.TimeCreated
                                        Id          = $evt.Id
                                        Message     = ([string]$evt.Message -replace '\r?\n', ' ')
                                    })
                                } catch {
                                    # Ignore malformed/partial event payloads.
                                }
                            }

                            if ($sessionRows.Count -gt 0) {
                                $orderedSessionRows = @($sessionRows | Sort-Object TimeCreated)
                                $firstSessionRow = $orderedSessionRows | Select-Object -First 1
                                $result['gui_target_session_creation_signal_utc'] = $firstSessionRow.TimeCreated.ToUniversalTime().ToString('o')
                                $result['gui_target_session_creation_signals'] = "Since $start (sampled up to $maxSessionEvents recent LocalSessionManager events; IDs=$($targetSessionLifecycleEventIds -join ','))`n" + ($orderedSessionRows | Select-Object -First 20 TimeCreated, Id, Message | Format-Table -AutoSize | Out-String)

                                $explorerStartUtcRaw = [string]$result['gui_target_session_explorer_earliest_start_utc']
                                if (-not [string]::IsNullOrWhiteSpace($explorerStartUtcRaw)) {
                                    try {
                                        $sessionCreateUtc = [datetime]::Parse([string]$result['gui_target_session_creation_signal_utc'])
                                        $explorerStartUtc = [datetime]::Parse($explorerStartUtcRaw)
                                        $result['gui_target_session_explorer_after_creation_known'] = $true
                                        $result['gui_target_session_explorer_after_creation'] = ($explorerStartUtc -ge $sessionCreateUtc.AddSeconds(-2))
                                    } catch {
                                        $result['gui_target_session_explorer_after_creation_known'] = $false
                                        $result['gui_target_session_explorer_after_creation'] = $false
                                    }
                                }
                            } else {
                                # Fallback for hosts where LocalSessionManager operational events are sparse or unavailable:
                                # use the latest Security 4624 LogonType=10 event for the target user as a session-start proxy.
                                $targetUserForFallback = $targetUserName
                                if (-not [string]::IsNullOrWhiteSpace($targetUserForFallback)) {
                                    $securityRows = New-Object System.Collections.Generic.List[object]
                                    foreach ($evt in (Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 4624; StartTime = $start } -MaxEvents 400 -ErrorAction SilentlyContinue)) {
                                        try {
                                            if ($evt.TimeCreated.ToUniversalTime() -lt $start) { continue }
                                            $xml = [xml]$evt.ToXml()
                                            $data = @{}
                                            foreach ($d in $xml.Event.EventData.Data) { $data[$d.Name] = [string]$d.'#text' }

                                            $eventEndpoint = ConvertTo-EvidenceEndpoint "[$($data.IpAddress)]:$($data.IpPort)"
                                            if (($data.LogonType -eq '10') -and ($data.TargetUserName -eq $targetUserForFallback) -and
                                                $eventEndpoint -and ($eventEndpoint -eq $expectedEndpoint) -and
                                                ($evt.TimeCreated.ToUniversalTime() -ge $registrationUtc) -and
                                                (Test-EvidenceAccount $data $rdpUsernameForAudit)) {
                                                $securityRows.Add([pscustomobject]@{
                                                    TimeCreated  = $evt.TimeCreated
                                                    LogonProcess = $data.LogonProcessName
                                                    ProcessName  = $data.ProcessName
                                                    IpAddress    = $data.IpAddress
                                                })
                                            }
                                        } catch {
                                            # Ignore malformed event payloads.
                                        }
                                    }

                                    if ($securityRows.Count -gt 0) {
                                        $latestSecurityRow = @($securityRows | Sort-Object TimeCreated -Descending | Select-Object -First 1)[0]
                                        $result['gui_target_session_creation_signal_utc'] = $latestSecurityRow.TimeCreated.ToUniversalTime().ToString('o')
                                        $result['gui_target_session_creation_signals'] = "Fallback source: Security 4624 LogonType=10 for '$targetUserForFallback' since $start`n" + ($securityRows | Sort-Object TimeCreated -Descending | Select-Object -First 10 TimeCreated, LogonProcess, ProcessName, IpAddress | Format-Table -AutoSize | Out-String)

                                        $explorerStartUtcRaw = [string]$result['gui_target_session_explorer_earliest_start_utc']
                                        if (-not [string]::IsNullOrWhiteSpace($explorerStartUtcRaw)) {
                                            try {
                                                $sessionCreateUtc = [datetime]::Parse([string]$result['gui_target_session_creation_signal_utc'])
                                                $explorerStartUtc = [datetime]::Parse($explorerStartUtcRaw)
                                                $result['gui_target_session_explorer_after_creation_known'] = $true
                                                $result['gui_target_session_explorer_after_creation'] = ($explorerStartUtc -ge $sessionCreateUtc.AddSeconds(-2))
                                            } catch {
                                                $result['gui_target_session_explorer_after_creation_known'] = $false
                                                $result['gui_target_session_explorer_after_creation'] = $false
                                            }
                                        }
                                    }
                                }
                            }
                        } catch {
                            $result['gui_target_session_creation_signals'] = "Could not collect LocalSessionManager session events: $_"
                        }
                    } catch {
                        $result['gui_target_session_processes'] = "Could not enumerate target-session processes: $_"
                    }
                }

                try {
                    $start = $securityStartTime
                    $maxExplorerLifecycleEvents = 500
                    $explorerLifecycleEvents = Get-WinEvent -FilterHashtable @{ LogName = 'Application'; StartTime = $start; Id = @(1000, 1001, 1002) } -MaxEvents $maxExplorerLifecycleEvents -ErrorAction SilentlyContinue |
                        Where-Object { $_.Message -match '(?i)explorer\.exe' }

                    $result['explorer_crash_event_count'] = @($explorerLifecycleEvents | Where-Object { $_.Id -eq 1000 }).Count
                    $result['explorer_wer_event_count'] = @($explorerLifecycleEvents | Where-Object { $_.Id -eq 1001 }).Count
                    $result['explorer_hang_event_count'] = @($explorerLifecycleEvents | Where-Object { $_.Id -eq 1002 }).Count

                    if ($explorerLifecycleEvents) {
                        $rows = $explorerLifecycleEvents |
                            Select-Object -First 20 TimeCreated, Id, ProviderName, Message
                        $result['explorer_lifecycle_events'] = "Since $start (sampled up to $maxExplorerLifecycleEvents recent Application events)`n" + ($rows | Format-Table -AutoSize | Out-String)
                    }
                } catch {
                    $result['explorer_crash_event_count'] = -1
                    $result['explorer_wer_event_count'] = -1
                    $result['explorer_hang_event_count'] = -1
                    $result['explorer_lifecycle_events'] = "Could not collect explorer lifecycle events from Application log: $_"
                }
            }

            if ($mode -eq 'Provider') {
                # Collect Security 4624 LogonType=10 for the configured RDP username since test start.
                try {
                    $start = $securityStartTime
                    $maxSecurityEvents = 1000
                    $maxRows = 20
                    $rows = New-Object System.Collections.Generic.List[object]

                    foreach ($evt in (Get-WinEvent -FilterHashtable @{ LogName = 'Security'; Id = 4624; StartTime = $start } -MaxEvents $maxSecurityEvents -ErrorAction SilentlyContinue)) {
                        if ($rows.Count -ge $maxRows) {
                            break
                        }

                        try {
                            if ($evt.TimeCreated.ToUniversalTime() -lt $start) { continue }
                            $xml = [xml]$evt.ToXml()
                            $data = @{}
                            foreach ($d in $xml.Event.EventData.Data) { $data[$d.Name] = [string]$d.'#text' }

                            $userMatches = [string]::IsNullOrWhiteSpace($targetUserName) -or ($data.TargetUserName -eq $targetUserName)
                            if ($userMatches -and $data.LogonType -eq '10') {
                                $eventEndpoint = ConvertTo-EvidenceEndpoint "[$($data.IpAddress)]:$($data.IpPort)"
                                if ($logCorrelated -and -not [string]::IsNullOrWhiteSpace($targetUserName) -and
                                    $eventEndpoint -and $eventEndpoint -eq $expectedEndpoint -and
                                    $evt.TimeCreated.ToUniversalTime() -ge $registrationUtc -and
                                    (Test-EvidenceAccount $data $rdpUsernameForAudit)) {
                                    $result['security_4624_type10_correlated_count']++
                                }
                                $rows.Add([pscustomobject]@{
                                    TimeCreated  = $evt.TimeCreated
                                    Domain       = $data.TargetDomainName
                                    LogonType    = $data.LogonType
                                    LogonProcess = $data.LogonProcessName
                                    ProcessName  = $data.ProcessName
                                    IpAddress    = $data.IpAddress
                                    IpPort       = $data.IpPort
                                })
                            }
                        } catch {
                            # Ignore malformed event XML and continue sampling recent events.
                        }
                    }

                    if ($rows.Count -gt 0) {
                        $result['security_4624_type10_user_count'] = $rows.Count
                        $result['security_4624_type10_user'] = "Since $start (sampled up to $maxSecurityEvents recent 4624 events)`n" + ($rows | Format-Table -AutoSize | Out-String)
                    } else {
                        $result['security_4624_type10_user_count'] = 0
                        $result['security_4624_type10_user'] = "Since $start (sampled up to $maxSecurityEvents recent 4624 events)`n(no matching 4624 LogonType=10 events for user '$targetUserName')"
                    }
                } catch {
                    $result['security_4624_type10_user_count'] = -1
                    $result['security_4624_type10_user'] = "Could not collect Security 4624: $_"
                }
            }

            if ($mode -eq 'Provider') {
                # Collect recent TermService event log entries
                try {
                    $events = Get-WinEvent -LogName 'System' -MaxEvents 20 -ErrorAction SilentlyContinue |
                        Where-Object { $_.ProviderName -match 'TermService|TermDD|Remote Desktop' -or $_.Id -in @(1058, 1088, 1096, 1149, 22) }
                    $result['termservice_events'] = ($events | Select-Object TimeCreated, Id, LevelDisplayName, Message | Out-String)
                } catch {
                    $result['termservice_events'] = "Could not collect TermService events: $_"
                }

                # Collect authoritative RemoteConnectionManager operational signals.
                # 261: listener accepted a connection
                # 263: WDDM graphics mode enabled for the remote connection
                try {
                    $start = $securityStartTime
                    $maxRemoteEvents = 200
                    $remoteEvents = Get-WinEvent -FilterHashtable @{ LogName = 'Microsoft-Windows-TerminalServices-RemoteConnectionManager/Operational'; StartTime = $start } -MaxEvents $maxRemoteEvents -ErrorAction SilentlyContinue

                    if ($remoteEvents) {
                        $connectionSignals = $remoteEvents | Where-Object { $_.Id -eq 261 }
                        $graphicsSignals = $remoteEvents | Where-Object { $_.Id -eq 263 }

                        $result['remote_connection_signal_count'] = ($connectionSignals | Measure-Object).Count
                        $result['remote_graphics_signal_count'] = ($graphicsSignals | Measure-Object).Count

                        $signalRows = $remoteEvents |
                            Where-Object { $_.Id -in @(261, 263, 1149, 20523) } |
                            Select-Object -First 40 TimeCreated, Id, LevelDisplayName, Message

                        if ($signalRows) {
                            $result['remote_connection_signals'] = "Since $start (sampled up to $maxRemoteEvents recent RemoteConnectionManager events)`n" + ($signalRows | Format-Table -AutoSize | Out-String)
                        } else {
                            $result['remote_connection_signals'] = "Since $start (sampled up to $maxRemoteEvents recent RemoteConnectionManager events)`n(no targeted IDs observed: 261, 263, 1149, 20523)"
                        }
                    } else {
                        $result['remote_connection_signals'] = "Since $start (sampled up to $maxRemoteEvents recent RemoteConnectionManager events)`n(no events returned)"
                    }
                } catch {
                    $result['remote_connection_signal_count'] = -1
                    $result['remote_graphics_signal_count'] = -1
                    $result['remote_connection_signals'] = "Could not collect RemoteConnectionManager operational events: $_"
                }

                # Collect activation state because notification mode can block shell readiness.
                try {
                    $windowsAppId = '{55c92734-d682-4d71-983e-d6ec3f16059f}'
                    $activationSortOrder = @(
                        @{ Expression = { if ($_.PartialProductKey) { 0 } else { 1 } } }
                        @{ Expression = { if ([string]$_.Description -like 'Windows(R) Operating System*') { 0 } else { 1 } } }
                        @{ Expression = { if ([string]$_.Name -like 'Windows(R), *edition') { 0 } else { 1 } } }
                        @{ Expression = { if ([int]$_.LicenseStatus -eq 5) { 0 } else { 1 } } }
                    )
                    $licenseRows = @(Get-CimInstance -ClassName SoftwareLicensingProduct -ErrorAction SilentlyContinue |
                        Where-Object { $_.ApplicationID -eq $windowsAppId })
                    $license = if ($licenseRows.Count -gt 0) {
                        @($licenseRows | Sort-Object -Property $activationSortOrder | Select-Object -First 1)[0]
                    } else {
                        $null
                    }
                    if ($null -ne $license) {
                        $status = [int]$license.LicenseStatus
                        $result['activation_license_status'] = $status
                        $result['activation_license_name'] = [string]$license.Name
                        $result['activation_license_description'] = [string]$license.Description
                        $result['activation_source'] = 'cim'

                        $reasonRaw = $license.LicenseStatusReason
                        if ($null -ne $reasonRaw) {
                            $reasonInt = [int64]$reasonRaw
                            if ($reasonInt -lt 0) {
                                $reasonInt = $reasonInt -band 0xFFFFFFFF
                            }
                            $result['activation_license_status_reason_hex'] = ('0x{0:X8}' -f $reasonInt)
                        }

                        # LicenseStatus=5 is notification mode.
                        $result['activation_notification_mode'] = ($status -eq 5)
                    } elseif (Get-Command cscript.exe -ErrorAction SilentlyContinue) {
                        $slmgrText = (& cscript.exe //NoLogo "$env:SystemRoot\System32\slmgr.vbs" /dlv 2>$null | Out-String)
                        if (-not [string]::IsNullOrWhiteSpace($slmgrText)) {
                            $result['activation_source'] = 'slmgr'

                            if ($slmgrText -match '(?im)^License Status:\s*Notification\b') {
                                $result['activation_license_status'] = 5
                                $result['activation_notification_mode'] = $true
                            } elseif ($slmgrText -match '(?im)^License Status:\s*Licensed\b') {
                                $result['activation_license_status'] = 1
                                $result['activation_notification_mode'] = $false
                            }

                            if ($slmgrText -match '(?im)^Notification Reason:\s*(0x[0-9A-F]+)') {
                                $result['activation_license_status_reason_hex'] = $Matches[1].ToUpperInvariant()
                            }
                        }
                    }
                } catch {
                    $result['activation_license_status'] = -1
                    $result['activation_license_status_reason_hex'] = ''
                    $result['activation_notification_mode'] = $false
                }
            }

            if ($logCorrelated -and $result['security_4624_type10_correlated_count'] -gt 0) {
                $result['evidence_correlated'] = $true
                $result['evidence_correlation_reason'] = 'Fresh TCP endpoint, provider/companion session binding, and Security logon agree'
            }

            $maxResultTextLength = 12000
            $preserveFullTextKeys = @(
                'stdout_raw',
                'stderr_raw',
                'dll_debug_raw',
                'stdout_fresh_raw',
                'dll_debug_fresh_raw',
                'provider_logs_raw'
            )
            foreach ($key in @($result.Keys)) {
                $value = $result[$key]
                if ($value -isnot [string]) {
                    continue
                }

                if ($key -in $preserveFullTextKeys) {
                    continue
                }

                if ($value.Length -le $maxResultTextLength) {
                    continue
                }

                $headLength = [Math]::Min(4000, [Math]::Floor($maxResultTextLength / 2))
                $tailLength = [Math]::Min(4000, $maxResultTextLength - $headLength)
                $omittedLength = $value.Length - $headLength - $tailLength

                $result[$key] = $value.Substring(0, $headLength) +
                    "`n... [truncated $omittedLength characters] ...`n" +
                    $value.Substring($value.Length - $tailLength, $tailLength)
            }

            $proc = Get-Process -Name 'ironrdp-termsrv' -ErrorAction SilentlyContinue
            $result['running'] = ($null -ne $proc)
            if ($null -ne $proc) {
                $result['pid'] = $proc.Id
            }

            try {
                $listeners = Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue
                $result['listening'] = ($listeners | Measure-Object).Count -gt 0
            } catch {
                $result['listening'] = $false
            }

            if (Test-Path -LiteralPath $iddOpenProbeLogPath) {
                $result['idd_open_probe'] = Get-Content -LiteralPath $iddOpenProbeLogPath -Tail 1000 -ErrorAction SilentlyContinue | Out-String
            }

            $result
