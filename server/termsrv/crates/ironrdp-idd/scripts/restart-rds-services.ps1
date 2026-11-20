[CmdletBinding()]
param(
    [Parameter()]
    [Alias('ComputerName')]
    [string]$VmHostname = $env:IRONRDP_TEST_VM_HOSTNAME,

    [Parameter()]
    [string]$Username = $env:RDP_USERNAME,

    [Parameter()]
    [pscredential]$Credential,

    [Parameter()]
    [string]$Password = "",

    [Parameter()]
    [switch]$PromptPassword,

    [Parameter()]
    [string]$PasswordEnvVar = 'RDP_PASSWORD',

    [Parameter()]
    [string]$TaskName = 'IronRdpTermSrvSystem',

    [Parameter()]
    [string]$RemoteRoot = $(if ($env:IRONRDP_DEPLOY_ROOT) { $env:IRONRDP_DEPLOY_ROOT } else { 'C:\IronRDPDeploy' }),

    [Parameter()]
    [switch]$SkipTermService,

    [Parameter()]
    [switch]$SkipIronRdpTermSrv
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$testVmModule = Join-Path $PSScriptRoot '..\..\..\scripts\TestVm.psm1'
Import-Module $testVmModule -Force
$credential = Resolve-TestVmCredential -Credential $Credential -Username $Username -Password $Password -PromptPassword:$PromptPassword -PasswordEnvVar $PasswordEnvVar -Hostname $VmHostname

$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    $remoteModule = 'C:\ProgramData\IronRDP\TestVm.psm1'
    Invoke-Command -Session $session -ScriptBlock {
        New-Item -ItemType Directory -Path 'C:\ProgramData\IronRDP' -Force -ErrorAction Stop | Out-Null
    }
    Copy-Item -LiteralPath $testVmModule -Destination $remoteModule -ToSession $session -Force
    Invoke-Command -Session $session -ScriptBlock {
        param([bool]$SkipTermService, [bool]$SkipIronRdpTermSrv, [string]$TaskName, [string]$ModulePath, [string]$RemoteRoot)

        Set-StrictMode -Version Latest
        $ErrorActionPreference = 'Stop'
        Import-Module $ModulePath -Force -ErrorAction Stop

        $termServiceState = if (-not $SkipTermService) { Get-TermServiceLifecycle } else { $null }
        $companionTask = if (-not $SkipIronRdpTermSrv) { Get-ScheduledTask -TaskName $TaskName -ErrorAction Stop } else { $null }
        $ownedPaths = @(
            (Join-Path $RemoteRoot 'bin\ironrdp-termsrv.exe')
            (Join-Path $RemoteRoot 'bin\ironrdp-termsrv.next.exe')
        )

        function Get-OwnedCompanion {
            Get-CimInstance Win32_Process -Filter "Name='ironrdp-termsrv.exe' OR Name='ironrdp-termsrv.next.exe'" -ErrorAction Stop |
                Where-Object { $_.ExecutablePath -in $ownedPaths }
        }

        function Wait-CompanionTask {
            param([bool]$Running)
            $deadline = (Get-Date).AddSeconds(30)
            do {
                $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction Stop
                $processes = @(Get-OwnedCompanion)
                if ($Running -and $processes.Count -eq 1) { return }
                if (-not $Running -and $task.State -ne 'Running' -and $processes.Count -eq 0) { return }
                Start-Sleep -Milliseconds 500
            } while ((Get-Date) -lt $deadline)
            throw "Companion task '$TaskName' did not reach Running=$Running within 30 seconds"
        }

        function Stop-OwnedCompanion {
            Stop-ScheduledTask -TaskName $TaskName -ErrorAction Stop
            foreach ($process in @(Get-OwnedCompanion)) {
                Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop
            }
            Wait-CompanionTask -Running $false
        }

        $companionWasRunning = @(Get-OwnedCompanion).Count -gt 0
        try {
            if (-not $SkipTermService) {
                $null = Stop-TermServiceSafely -TimeoutSeconds 30
            }

            if (-not $SkipIronRdpTermSrv) {
                Stop-OwnedCompanion
                Start-ScheduledTask -TaskName $TaskName -ErrorAction Stop
                Wait-CompanionTask -Running $true
            }

            if (-not $SkipTermService) {
                # UNTESTED: The provider must connect to the restarted companion; verify native control-pipe readiness and listener recovery when changing startup order.
                Start-TermServiceSafely -StartupType $termServiceState.StartupType -TimeoutSeconds 30
            }
        } catch {
            $failure = $_
            $recoveryErrors = @()
            if (-not $SkipIronRdpTermSrv) {
                try {
                    if ($companionWasRunning) {
                        if (@(Get-OwnedCompanion).Count -eq 0) {
                            Start-ScheduledTask -TaskName $TaskName -ErrorAction Stop
                        }
                        Wait-CompanionTask -Running $true
                    } else {
                        Stop-OwnedCompanion
                    }
                } catch { $recoveryErrors += $_.Exception.Message }
            }
            if ($null -ne $termServiceState) {
                try { Restore-TermServiceLifecycle @termServiceState } catch { $recoveryErrors += $_.Exception.Message }
            }
            if ($recoveryErrors.Count -gt 0) {
                throw "RDS restart failed: $($failure.Exception.Message); recovery failed: $($recoveryErrors -join '; ')"
            }
            throw $failure
        }

        $termService = Get-Service -Name 'TermService' -ErrorAction SilentlyContinue
        $companionProcess = Get-OwnedCompanion | Select-Object -First 1

        [pscustomobject]@{
            TermService = if ($null -ne $termService) { $termService.Status.ToString() } else { 'NotFound' }
            CompanionTask = if ($null -ne $companionTask) { (Get-ScheduledTask -TaskName $TaskName -ErrorAction Stop).State.ToString() } else { 'NotFound' }
            CompanionProcessId = if ($null -ne $companionProcess) { [int]$companionProcess.ProcessId } else { 0 }
        }
    } -ArgumentList $SkipTermService.IsPresent, $SkipIronRdpTermSrv.IsPresent, $TaskName, $remoteModule, $RemoteRoot
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}
