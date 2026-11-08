function Invoke-E2EAgentCommand {
    param(
        [string]$AgentPath,
        [string[]]$Arguments,
        [hashtable]$Environment,
        [ValidateRange(1, 600)][int]$TimeoutSeconds
    )

    $start = New-Object System.Diagnostics.ProcessStartInfo
    $start.FileName = $AgentPath
    $start.Arguments = ($Arguments | ForEach-Object {
        '"' + [regex]::Replace([regex]::Replace($_, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
    }) -join ' '
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($key in $Environment.Keys) {
        $start.EnvironmentVariables[$key] = [string]$Environment[$key]
    }

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $start
    $started = $false
    try {
        if (-not $process.Start()) {
            throw 'Could not start ironrdp-agent'
        }
        $started = $true
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $process.Kill()
            [void]$process.WaitForExit(2000)
            throw "ironrdp-agent command timed out after ${TimeoutSeconds}s"
        }
        $drain = [System.Threading.Tasks.Task]::WhenAll([System.Threading.Tasks.Task[]]@($stdout, $stderr))
        if (-not $drain.Wait(2000)) {
            throw 'ironrdp-agent exited but its redirected output remained open'
        }
        [pscustomobject]@{
            ExitCode = $process.ExitCode
            Output = $stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult()
        }
    }
    finally {
        if ($started) {
            $process.StandardOutput.Dispose()
            $process.StandardError.Dispose()
        }
        $process.Dispose()
    }
}

function Invoke-E2EAgentCertificateTrust {
    param(
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$AgentPath,
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$Endpoint,
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$Fingerprint,
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$KnownCertificatesPath
    )

    $normalizedFingerprint = $Fingerprint -replace '[:\s]', ''
    if ($normalizedFingerprint -notmatch '^[0-9a-fA-F]{64}$') {
        throw 'Server certificate fingerprint must contain 64 hexadecimal digits'
    }

    Invoke-E2EAgentCommand -AgentPath $AgentPath -Arguments @(
        'cert', 'trust', $Endpoint, $normalizedFingerprint
    ) -Environment @{
        IRONRDP_AGENT_KNOWN_CERTIFICATES = $KnownCertificatesPath
    } -TimeoutSeconds 30
}

function Invoke-E2EAgentScreenshot {
    param(
        [string]$AgentPath,
        [string]$Server,
        [string]$Username,
        [string]$Password,
        [string]$Domain = '',
        [string]$OutputPath,
        [string]$LogPath,
        [ValidateRange(1, 600)][int]$TimeoutSeconds = 60,
        [ValidateRange(1, 600)][int]$NoGraphicsTimeoutSeconds = 20,
        [ValidateRange(0, 600)][int]$AfterFirstGraphicsSeconds = 20,
        [string]$KnownCertificatesPath = '',
        [scriptblock]$ConnectionObserver,
        [scriptblock]$CommandRunner = {
            param($Arguments, $RemainingSeconds)
            $environment = @{ RDP_USERNAME = $Username; RDP_PASSWORD = $Password }
            if (-not [string]::IsNullOrWhiteSpace($KnownCertificatesPath)) {
                $environment.IRONRDP_AGENT_KNOWN_CERTIFICATES = $KnownCertificatesPath
            }
            Invoke-E2EAgentCommand -AgentPath $AgentPath -Arguments $Arguments `
                -Environment $environment `
                -TimeoutSeconds $RemainingSeconds
        }
    )

    $endpoint = '\\.\pipe\ironrdp-e2e-' + [guid]::NewGuid().ToString('N')
    $clock = [System.Diagnostics.Stopwatch]::StartNew()
    $firstFrameAt = $null
    $connectionObserved = $false
    $clientEndpoint = ''
    $succeeded = $false
    $message = ''
    $invoke = {
        param([string[]]$Command)
        $remaining = [Math]::Ceiling($TimeoutSeconds - $clock.Elapsed.TotalSeconds)
        if ($remaining -le 0) {
            throw "RDP observation timed out after ${TimeoutSeconds}s"
        }
        $reply = & $CommandRunner (@('--endpoint', $endpoint) + $Command) ([int]$remaining)
        Add-Content -LiteralPath $LogPath -Value $reply.Output
        return $reply
    }

    try {
        $connect = @('connect', '--server', $Server, '--no-prompt', '--auto-start',
            '--prop', 'ironrdp_autologon:i:1')
        if (-not [string]::IsNullOrWhiteSpace($Domain)) {
            $connect += @('--domain', $Domain)
        }
        $reply = & $invoke $connect
        if ($reply.ExitCode -ne 0) { throw "RDP connect failed: $($reply.Output)" }

        while ($true) {
            $status = & $invoke @('status')
            if ($status.ExitCode -ne 0 -or $status.Output -match '(?im)^state:\s*(Failed|Disconnected)\s*$') {
                throw "RDP connection failed during observation: $($status.Output)"
            }
            if ($status.Output -match '(?im)^state:\s*Connected\s*$') {
                if (-not $connectionObserved -and $null -ne $ConnectionObserver) {
                    $connectionObserved = $true
                    try { $clientEndpoint = [string](& $ConnectionObserver $endpoint) }
                    catch { Write-Warning "Client socket identity unavailable: $($_.Exception.Message)" }
                }
                $frame = & $invoke @('screenshot', $OutputPath)
                if ($frame.ExitCode -eq 0) {
                    if ($null -eq $firstFrameAt) { $firstFrameAt = $clock.Elapsed.TotalSeconds }
                    if ($clock.Elapsed.TotalSeconds - $firstFrameAt -ge $AfterFirstGraphicsSeconds) {
                        $finalStatus = & $invoke @('status')
                        if ($finalStatus.ExitCode -ne 0 -or $finalStatus.Output -notmatch '(?im)^state:\s*Connected\s*$') {
                            throw "RDP connection failed at the end of observation: $($finalStatus.Output)"
                        }
                        $succeeded = $true
                        break
                    }
                } elseif ($frame.Output -notmatch 'no frame available yet') {
                    throw "RDP screenshot failed: $($frame.Output)"
                }
            }
            if ($null -eq $firstFrameAt -and $clock.Elapsed.TotalSeconds -ge $NoGraphicsTimeoutSeconds) {
                throw "No RDP frame arrived within ${NoGraphicsTimeoutSeconds}s"
            }
            Start-Sleep -Milliseconds 200
        }
    }
    catch {
        $message = $_.Exception.Message
        Write-Warning $message
    }
    finally {
        try {
            $logs = & $CommandRunner @('--endpoint', $endpoint, 'query-logs', '--last', '500') 10
            Add-Content -LiteralPath $LogPath -Value $logs.Output
            if ($logs.ExitCode -ne 0) { Write-Warning 'Could not collect the owned agent session logs' }
        } catch {
            Write-Warning "Could not collect the owned agent session logs: $($_.Exception.Message)"
        }
        try {
            $stop = & $CommandRunner @('--endpoint', $endpoint, 'daemon', 'stop') 10
            if ($stop.ExitCode -ne 0) { throw $stop.Output }
        } catch {
            $succeeded = $false
            $message = "Could not stop the owned agent endpoint '$endpoint': $($_.Exception.Message)"
            Write-Warning $message
        }
    }

    [pscustomobject]@{
        ExitCode = $(if ($succeeded) { 0 } else { 1 })
        Message = $message
        Endpoint = $endpoint
        ClientEndpoint = $clientEndpoint
    }
}
