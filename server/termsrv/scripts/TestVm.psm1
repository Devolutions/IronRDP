Set-StrictMode -Version Latest

function Resolve-TestVmCredential {
    [CmdletBinding()]
    param(
        [pscredential]$Credential,
        [string]$Username = $env:RDP_USERNAME,
        [string]$Password = '',
        [switch]$PromptPassword,
        [string]$PasswordEnvVar = 'RDP_PASSWORD',
        [string]$Hostname = $env:IRONRDP_TEST_VM_HOSTNAME
    )

    if ([string]::IsNullOrWhiteSpace($Hostname)) {
        throw 'Hostname is required (pass -Hostname or set IRONRDP_TEST_VM_HOSTNAME)'
    }
    if ($null -ne $Credential) { return $Credential }
    if ([string]::IsNullOrWhiteSpace($Username)) {
        throw 'Username is required (pass -Username, pass -Credential, or set RDP_USERNAME)'
    }
    if ([string]::IsNullOrWhiteSpace($Password) -and $PromptPassword) {
        $secure = Read-Host -Prompt "Password for $Username@$Hostname" -AsSecureString
    } else {
        if ([string]::IsNullOrWhiteSpace($Password)) {
            $Password = [Environment]::GetEnvironmentVariable($PasswordEnvVar)
        }
        if ([string]::IsNullOrWhiteSpace($Password)) {
            throw "Password is required (pass -Password, pass -Credential, or set $PasswordEnvVar)"
        }
        $secure = ConvertTo-SecureString $Password -AsPlainText -Force
    }
    [pscredential]::new($Username, $secure)
}

function Get-CargoTargetDirectory {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][string]$WorkspaceRoot)

    $manifestPath = Join-Path $WorkspaceRoot 'Cargo.toml'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "Cargo manifest not found: $manifestPath"
    }

    $cargo = Get-Command cargo -ErrorAction Stop
    $metadataOutput = & $cargo.Source metadata --no-deps --format-version 1 --manifest-path $manifestPath
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed (exit $LASTEXITCODE)"
    }

    try {
        $metadata = ($metadataOutput -join [Environment]::NewLine) | ConvertFrom-Json -ErrorAction Stop
    } catch {
        throw "cargo metadata returned invalid JSON: $($_.Exception.Message)"
    }

    if ([string]::IsNullOrWhiteSpace([string]$metadata.target_directory)) {
        throw 'cargo metadata did not report a target directory'
    }

    [System.IO.Path]::GetFullPath([string]$metadata.target_directory)
}

function Get-CargoBuildOutputDirectory {
    [CmdletBinding()]
    param([Parameter(Mandatory = $true)][string]$WorkspaceRoot)

    $targetDirectory = Get-CargoTargetDirectory -WorkspaceRoot $WorkspaceRoot
    if (-not [string]::IsNullOrWhiteSpace($env:CARGO_BUILD_TARGET)) {
        return Join-Path $targetDirectory $env:CARGO_BUILD_TARGET
    }

    $targetDirectory
}

function New-TestVmSession {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$Hostname,
        [Parameter(Mandatory = $true)][pscredential]$Credential,
        [ValidateRange(1, 2147483647)][int]$TimeoutMilliseconds = 30000
    )

    $deadline = (Get-Date).AddMilliseconds($TimeoutMilliseconds)
    $attemptTimeout = [Math]::Max(1, [int]($TimeoutMilliseconds / 2))
    $options = New-PSSessionOption -OpenTimeout $attemptTimeout -OperationTimeout $attemptTimeout
    try {
        return New-PSSession -ComputerName $Hostname -Credential $Credential -SessionOption $options -ErrorAction Stop
    } catch {
        Write-Warning "WinRM over HTTP failed for $Hostname; trying WinRM over HTTPS (5986)"
        $remaining = [Math]::Max(1, [int]($deadline - (Get-Date)).TotalMilliseconds)
        $options = New-PSSessionOption -OpenTimeout $remaining -OperationTimeout $remaining
        return New-PSSession -ComputerName $Hostname -Credential $Credential -UseSSL -Port 5986 -SessionOption $options -ErrorAction Stop
    }
}

function Wait-TestVmSession {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$Hostname,
        [Parameter(Mandatory = $true)][pscredential]$Credential,
        [ValidateRange(1, 3600)][int]$TimeoutSeconds = 180
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while (($remaining = ($deadline - (Get-Date)).TotalMilliseconds) -gt 0) {
        $session = $null
        try {
            $session = New-TestVmSession -Hostname $Hostname -Credential $Credential -TimeoutMilliseconds ([Math]::Max(1, [int][Math]::Min(5000, $remaining / 2)))
            $null = Invoke-Command -Session $session -ScriptBlock { 'ok' } -ErrorAction Stop
            return $session
        } catch {
            if ($null -ne $session) { Remove-PSSession -Session $session -ErrorAction SilentlyContinue }
        }
        $remaining = ($deadline - (Get-Date)).TotalMilliseconds
        if ($remaining -gt 0) { Start-Sleep -Milliseconds ([int][Math]::Min(1000, $remaining)) }
    }
    throw "WinRM did not become ready on $Hostname within ${TimeoutSeconds}s"
}

function Wait-TestVmListener {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][ValidateRange(1, 65535)][int]$Port,
        [ValidateRange(0, 2147483647)][int]$OwningProcess = 0,
        [ValidateRange(1, 3600)][int]$TimeoutSeconds = 30
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        $listeners = @()
        try {
            $listeners = @(Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction Stop)
        } catch {
            if ($_.CategoryInfo.Category -ne 'ObjectNotFound' -or
                $_.FullyQualifiedErrorId -notlike 'CmdletizationQuery_NotFound*') { throw }
        }
        $listener = $listeners | Where-Object { $OwningProcess -eq 0 -or $_.OwningProcess -eq $OwningProcess } | Select-Object -First 1
        if ($null -ne $listener) { return $listener }
        $remaining = ($deadline - (Get-Date)).TotalMilliseconds
        if ($remaining -le 0) { break }
        Start-Sleep -Milliseconds ([int][Math]::Min(500, $remaining))
    } while ($true)
    throw "No matching listener on port $Port within ${TimeoutSeconds}s (expected PID $OwningProcess)"
}

function Invoke-TermServiceControl {
    param([string[]]$Arguments, [int[]]$SuccessExitCodes = @(0))

    $output = & sc.exe @Arguments 2>&1
    if ($LASTEXITCODE -notin $SuccessExitCodes) {
        throw "sc.exe $($Arguments -join ' ') failed (exit $LASTEXITCODE): $(($output | Out-String).Trim())"
    }
}

function Set-TermServiceStartupType {
    param([ValidateSet('auto', 'delayed-auto', 'demand', 'disabled')][string]$StartupType)
    Invoke-TermServiceControl -Arguments @('config', 'TermService', 'start=', $StartupType)
}

function Get-TermServiceLifecycle {
    [CmdletBinding()]
    param()

    $service = Get-CimInstance Win32_Service -Filter "Name='TermService'" -ErrorAction Stop
    if ($null -eq $service) { throw 'TermService not found' }
    $startup = switch ($service.StartMode) {
        'Auto' {
            $settings = Get-ItemProperty -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Services\TermService' -ErrorAction Stop
            $delayed = $settings.PSObject.Properties['DelayedAutoStart']
            if ($null -ne $delayed -and $delayed.Value -eq 1) { 'delayed-auto' } else { 'auto' }
        }
        'Manual' { 'demand' }
        'Disabled' { 'disabled' }
        default { throw "Unsupported TermService startup mode: $($service.StartMode)" }
    }
    @{ StartupType = $startup; WasRunning = $service.State -eq 'Running' }
}

function Wait-TermServiceState {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][ValidateSet('Running', 'Stopped')][string]$Status,
        [ValidateRange(1, 3600)][int]$TimeoutSeconds = 30
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        if ((Get-Service -Name TermService -ErrorAction Stop).Status -eq $Status) { return }
        $remaining = ($deadline - (Get-Date)).TotalMilliseconds
        if ($remaining -le 0) { break }
        Start-Sleep -Milliseconds ([int][Math]::Min(500, $remaining))
    } while ($true)
    throw [TimeoutException]::new("TermService did not reach $Status within ${TimeoutSeconds}s")
}

function Restore-TermServiceLifecycle {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][ValidateSet('auto', 'delayed-auto', 'demand', 'disabled')][string]$StartupType,
        [Parameter(Mandatory = $true)][bool]$WasRunning,
        [scriptblock]$SetStartupType = { param($Value) Set-TermServiceStartupType -StartupType $Value },
        [scriptblock]$GetStatus = { (Get-Service -Name TermService -ErrorAction Stop).Status.ToString() },
        [scriptblock]$StartService = { Invoke-TermServiceControl -Arguments @('start', 'TermService') -SuccessExitCodes @(0, 1056) },
        [scriptblock]$StopService = { Invoke-TermServiceControl -Arguments @('stop', 'TermService') -SuccessExitCodes @(0, 1062) },
        [scriptblock]$WaitForStatus = { param($Status, $Timeout) Wait-TermServiceState -Status $Status -TimeoutSeconds $Timeout }
    )

    $failure = $null
    try {
        $status = [string](& $GetStatus)
        if ($WasRunning -and $status -ne 'Running') {
            $startMode = if ($StartupType -eq 'disabled') { 'demand' } else { $StartupType }
            $null = & $SetStartupType $startMode
            $null = & $StartService
            $null = & $WaitForStatus 'Running' 30
        } elseif (-not $WasRunning -and $status -ne 'Stopped') {
            $null = & $StopService
            $null = & $WaitForStatus 'Stopped' 30
        }
    } catch {
        $failure = $_
        throw
    } finally {
        try { $null = & $SetStartupType $StartupType } catch {
            if ($null -ne $failure) {
                throw "TermService recovery failed: $($failure.Exception.Message); startup mode restoration failed: $($_.Exception.Message)"
            }
            throw
        }
    }
}

function Stop-TermServiceSafely {
    <#
    .SYNOPSIS
    Stops and disables TermService, returning the original lifecycle for restoration.
    #>
    [CmdletBinding()]
    param(
        [ValidateRange(1, 3600)][int]$TimeoutSeconds = 60,
        [switch]$Force
    )

    $state = Get-TermServiceLifecycle
    try {
        Set-TermServiceStartupType -StartupType disabled
        Invoke-TermServiceControl -Arguments @('stop', 'TermService') -SuccessExitCodes @(0, 1062)
        $grace = if ($Force) { [Math]::Max(1, [int]($TimeoutSeconds / 2)) } else { $TimeoutSeconds }
        try {
            Wait-TermServiceState -Status Stopped -TimeoutSeconds $grace
        } catch [TimeoutException] {
            if (-not $Force -or $TimeoutSeconds -le $grace) { throw }
            $service = Get-CimInstance Win32_Service -Filter "Name='TermService'" -ErrorAction Stop
            $processId = [int]$service.ProcessId
            if ($processId -le 0) { throw 'TermService stop timed out without a host PID' }
            $shared = @(Get-CimInstance Win32_Service -Filter "ProcessId=$processId" -ErrorAction Stop |
                Where-Object { $_.Name -ne 'TermService' })
            if ($shared.Count -gt 0) { throw "Refusing to stop shared TermService host PID $processId" }
            # UNTESTED: Recheck the dedicated TermService host before killing its PID; validate SCM/PID races on the target when changing forced recovery.
            $current = Get-CimInstance Win32_Service -Filter "Name='TermService'" -ErrorAction Stop
            if ([int]$current.ProcessId -ne $processId) { throw 'TermService host changed during forced stop' }
            Stop-Process -Id $processId -Force -ErrorAction Stop
            Wait-TermServiceState -Status Stopped -TimeoutSeconds ($TimeoutSeconds - $grace)
        }
        return $state
    } catch {
        $failure = $_
        try { Restore-TermServiceLifecycle @state } catch {
            throw "TermService stop failed: $($failure.Exception.Message); recovery failed: $($_.Exception.Message)"
        }
        throw $failure
    }
}

function Start-TermServiceSafely {
    [CmdletBinding()]
    param(
        [ValidateSet('auto', 'delayed-auto', 'demand', 'disabled')][string]$StartupType = 'demand',
        [ValidateRange(1, 3600)][int]$TimeoutSeconds = 60
    )

    $failure = $null
    try {
        $startMode = if ($StartupType -eq 'disabled') { 'demand' } else { $StartupType }
        Set-TermServiceStartupType -StartupType $startMode
        Invoke-TermServiceControl -Arguments @('start', 'TermService') -SuccessExitCodes @(0, 1056)
        Wait-TermServiceState -Status Running -TimeoutSeconds $TimeoutSeconds
    } catch {
        $failure = $_
        throw
    } finally {
        try { Set-TermServiceStartupType -StartupType $StartupType } catch {
            if ($null -ne $failure) {
                throw "TermService start failed: $($failure.Exception.Message); startup mode restoration failed: $($_.Exception.Message)"
            }
            throw
        }
    }
}

function Restart-TermServiceSafely {
    [CmdletBinding()]
    param(
        [ValidateRange(1, 3600)][int]$StopTimeoutSeconds = 60,
        [ValidateRange(1, 3600)][int]$StartTimeoutSeconds = 60,
        [switch]$Force
    )

    $state = Get-TermServiceLifecycle
    try {
        $null = Stop-TermServiceSafely -TimeoutSeconds $StopTimeoutSeconds -Force:$Force
        Start-TermServiceSafely -StartupType $state.StartupType -TimeoutSeconds $StartTimeoutSeconds
    } catch {
        $failure = $_
        try { Restore-TermServiceLifecycle @state } catch {
            throw "TermService restart failed: $($failure.Exception.Message); recovery failed: $($_.Exception.Message)"
        }
        throw $failure
    }
}

Export-ModuleMember -Function Resolve-TestVmCredential, Get-CargoTargetDirectory, Get-CargoBuildOutputDirectory, New-TestVmSession, Wait-TestVmSession, Wait-TestVmListener,
    Get-TermServiceLifecycle, Wait-TermServiceState, Restore-TermServiceLifecycle, Stop-TermServiceSafely,
    Start-TermServiceSafely, Restart-TermServiceSafely
