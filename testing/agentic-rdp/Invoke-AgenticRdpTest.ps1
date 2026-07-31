[CmdletBinding()]
param(
    [ValidateSet('rdp', 'daemon', 'connect', 'remoting-server', 'remote-command', 'desktop-scenario')]
    [string] $MaxStage = 'desktop-scenario',

    [string] $DesktopSize = '1920x1080',

    [string] $WorkspaceRoot = $(
        if ([string]::IsNullOrWhiteSpace($env:GITHUB_WORKSPACE)) {
            [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
        }
        else {
            $env:GITHUB_WORKSPACE
        }
    ),

    [string] $AgentPath = (Join-Path $WorkspaceRoot 'target\release\ironrdp-agent.exe'),

    [string] $ArtifactsDir = (Join-Path $WorkspaceRoot 'artifacts\agentic-rdp'),

    [switch] $CleanupOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$stageOrder = @('rdp', 'daemon', 'connect', 'remoting-server', 'remote-command', 'desktop-scenario')
$maxStageIndex = [array]::IndexOf($stageOrder, $MaxStage)
$scriptsRoot = $PSScriptRoot
$endpoint = "\\.\pipe\ironrdp-agent-ci-$PID"
$rdpStatePath = Join-Path $ArtifactsDir 'rdp-state.json'
$daemonStatePath = Join-Path $ArtifactsDir 'agent-daemon.json'
$sessionStatePath = Join-Path $ArtifactsDir 'agent-session.json'
$psHostEndpointPath = Join-Path $ArtifactsDir 'pshost-endpoint.json'
$taskName = 'IronRdpAgenticPSHost'
$psHostPort = 45985

function Test-ShouldRunStage {
    param(
        [Parameter(Mandatory)]
        [string] $Stage
    )

    return ([array]::IndexOf($stageOrder, $Stage) -le $maxStageIndex)
}

function Stop-ProcessFromState {
    param(
        [Parameter(Mandatory)]
        [string] $Path
    )

    if (-not (Test-Path $Path)) {
        return
    }

    try {
        $state = Get-Content -Path $Path -Raw | ConvertFrom-Json
        if ($null -eq $state.ProcessId `
            -or $state.PSObject.Properties.Name -notcontains 'ProcessPath' `
            -or $state.PSObject.Properties.Name -notcontains 'ProcessStartTimeUtcTicks') {
            Write-Warning "Skipping process cleanup for incomplete state file: $Path"
            return
        }

        $process = Get-Process -Id ([int] $state.ProcessId) -ErrorAction SilentlyContinue
        if ($null -eq $process) {
            return
        }

        $expectedPath = [System.IO.Path]::GetFullPath([string] $state.ProcessPath)
        $actualPath = [System.IO.Path]::GetFullPath($process.Path)
        $expectedStartTime = [int64] $state.ProcessStartTimeUtcTicks
        $actualStartTime = $process.StartTime.ToUniversalTime().Ticks
        if (-not [string]::Equals($actualPath, $expectedPath, [System.StringComparison]::OrdinalIgnoreCase) `
            -or $actualStartTime -ne $expectedStartTime) {
            Write-Warning "Skipping cleanup because process $($process.Id) does not match state file: $Path"
            return
        }

        Stop-Process -Id $process.Id -Force
    }
    finally {
        Remove-Item -Path $Path -Force -ErrorAction SilentlyContinue
    }
}

function Invoke-Agent {
    param(
        [Parameter(ValueFromRemainingArguments)]
        [string[]] $Arguments
    )

    & $AgentPath --endpoint $endpoint @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "ironrdp-agent command failed with exit code $LASTEXITCODE"
    }
}

function Invoke-Cleanup {
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue

    if (Test-Path $sessionStatePath) {
        try {
            Invoke-Agent disconnect | Out-Null
        }
        catch {
            Write-Warning "Could not disconnect the agent session: $($_.Exception.Message)"
        }
        finally {
            Remove-Item -Path $sessionStatePath -Force -ErrorAction SilentlyContinue
        }
    }

    try {
        Stop-ProcessFromState -Path $daemonStatePath
    }
    catch {
        Write-Warning "Could not stop agent daemon: $($_.Exception.Message)"
    }

    try {
        Stop-ProcessFromState -Path $psHostEndpointPath
    }
    catch {
        Write-Warning "Could not stop interactive PSHostServer: $($_.Exception.Message)"
    }

    try {
        & (Join-Path $scriptsRoot 'Enable-LocalRdp.ps1') -Cleanup -StatePath $rdpStatePath
    }
    catch {
        Write-Warning "Could not restore local RDP settings: $($_.Exception.Message)"
    }
}

if ($CleanupOnly) {
    Invoke-Cleanup
    return
}

New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null
$rdpInfo = $null
$sessionInfo = $null

try {
    if (Test-ShouldRunStage -Stage 'rdp') {
        Write-Host '::group::Enable local RDP'
        $rdpInfoJson = & (Join-Path $scriptsRoot 'Enable-LocalRdp.ps1') -StatePath $rdpStatePath
        $rdpInfo = $rdpInfoJson | ConvertFrom-Json
        Write-Host '::endgroup::'
    }

    if (Test-ShouldRunStage -Stage 'daemon') {
        Write-Host '::group::Start ironrdp-agent daemon'
        & (Join-Path $scriptsRoot 'Start-AgentDaemon.ps1') `
            -AgentPath $AgentPath `
            -Endpoint $endpoint `
            -ArtifactsDir $ArtifactsDir `
            -StatePath $daemonStatePath | Write-Host
        Write-Host '::endgroup::'
    }

    if (Test-ShouldRunStage -Stage 'remoting-server') {
        Write-Host '::group::Register interactive PSHostServer'
        & (Join-Path $scriptsRoot 'Start-InteractivePSHostServer.ps1') `
            -Register `
            -EndpointPath $psHostEndpointPath `
            -Port $psHostPort `
            -ArtifactsDir $ArtifactsDir `
            -TaskName $taskName | Write-Host
        Write-Host '::endgroup::'
    }

    if (Test-ShouldRunStage -Stage 'connect') {
        if ($null -eq $rdpInfo) {
            throw 'RDP stage did not produce connection information'
        }

        Write-Host '::group::Connect ironrdp-agent session'
        $sessionJson = & (Join-Path $scriptsRoot 'Connect-AgentSession.ps1') `
            -AgentPath $AgentPath `
            -Endpoint $endpoint `
            -UserName $rdpInfo.DomainUserName `
            -Password $rdpInfo.Password `
            -HostName $rdpInfo.HostName `
            -Port ([int] $rdpInfo.Port) `
            -DesktopSize $DesktopSize `
            -ArtifactsDir $ArtifactsDir `
            -StatePath $sessionStatePath
        $sessionInfo = $sessionJson | ConvertFrom-Json
        Write-Host $sessionJson
        Write-Host '::endgroup::'
    }

    if (Test-ShouldRunStage -Stage 'remoting-server') {
        Write-Host '::group::Start interactive PSHostServer task'
        Start-ScheduledTask -TaskName $taskName
        Start-Sleep -Seconds 5
        Get-ScheduledTask -TaskName $taskName | Get-ScheduledTaskInfo | Format-List | Out-String | Write-Host
        Write-Host '::endgroup::'

        Write-Host '::group::Wait for interactive PSHostServer'
        & (Join-Path $scriptsRoot 'Start-InteractivePSHostServer.ps1') `
            -Wait `
            -EndpointPath $psHostEndpointPath `
            -TimeoutSeconds 180 | Write-Host
        Write-Host '::endgroup::'
    }

    if (Test-ShouldRunStage -Stage 'remote-command') {
        Write-Host '::group::Verify interactive remoting'
        & (Join-Path $scriptsRoot 'Invoke-InteractiveCommand.ps1') `
            -Mode Verify `
            -EndpointPath $psHostEndpointPath `
            -ArtifactsDir $ArtifactsDir | Write-Host
        Write-Host '::endgroup::'
    }

    if (Test-ShouldRunStage -Stage 'desktop-scenario') {
        if ($null -eq $sessionInfo) {
            throw 'Connect stage did not produce session information'
        }

        Write-Host '::group::Run agentic desktop scenario'
        & (Join-Path $scriptsRoot 'Invoke-AgenticDesktopScenario.ps1') `
            -AgentPath $AgentPath `
            -Endpoint $endpoint `
            -SessionId $sessionInfo.SessionId `
            -ArtifactsDir $ArtifactsDir `
            -DesktopSize $DesktopSize | Write-Host
        Write-Host '::endgroup::'
    }
}
catch {
    Write-Host '::error::Agentic RDP test failed'
    if ($null -ne $sessionInfo) {
        try {
            $failureScreenshotPath = Join-Path $ArtifactsDir 'agent-failure.png'
            Invoke-Agent screenshot $failureScreenshotPath | Out-Null
            Write-Warning "Captured failure screenshot at $failureScreenshotPath"
        }
        catch {
            Write-Warning "Could not capture failure screenshot: $($_.Exception.Message)"
        }
    }

    throw
}
finally {
    Invoke-Cleanup
}
