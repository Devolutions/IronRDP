[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$ProviderDllPath,

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ListenerName = "IRDP-Tcp",

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$ProtocolManagerClsid = "{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}",

    [Parameter()]
    [ValidateRange(0, 65535)]
    [int]$PortNumber = 0,

    [Parameter()]
    [switch]$RestartTermService,

    [Parameter()]
    [ValidateRange(5, 600)]
    [int]$TermServiceStopTimeoutSeconds = 60,

    [Parameter()]
    [ValidateRange(5, 600)]
    [int]$TermServiceStartTimeoutSeconds = 60
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$defaultsScript = Join-Path -Path $scriptRoot -ChildPath "side-by-side-defaults.ps1"
. $defaultsScript

$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber -PersistResolvedDefault
$portSettingInfo = Get-SideBySideListenerPortSettingInfo

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (Test-IsAdministrator)) {
    throw "this script must be run from an elevated PowerShell session"
}

if (-not (Test-Path -LiteralPath $ProviderDllPath -PathType Leaf)) {
    throw "provider dll path does not exist: $ProviderDllPath"
}

$providerDllPathResolved = (Resolve-Path -LiteralPath $ProviderDllPath).Path

$winStationsRoot = "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations"
$sourceListener = Join-Path -Path $winStationsRoot -ChildPath "RDP-Tcp"
$targetListener = Join-Path -Path $winStationsRoot -ChildPath $ListenerName

if (-not (Test-Path -LiteralPath $sourceListener)) {
    throw "source listener key not found: $sourceListener"
}

if ($ListenerName -ne "RDP-Tcp" -and $PortNumber -eq 3389) {
    throw "side-by-side listener cannot use port 3389; use a dedicated port such as 4489"
}

$conflictingListeners = @(Get-ChildItem -LiteralPath $winStationsRoot -ErrorAction Stop |
        Where-Object { $_.PSChildName -ne $ListenerName } |
        ForEach-Object {
            $name = $_.PSChildName
            $props = Get-ItemProperty -LiteralPath $_.PSPath -ErrorAction SilentlyContinue

            if ($null -ne $props) {
                $portProperty = $props.PSObject.Properties['PortNumber']
                if ($null -ne $portProperty) {
                    try {
                        if ([int]$portProperty.Value -eq $PortNumber) {
                            $name
                        }
                    }
                    catch {
                    }
                }
            }
        })

if ($conflictingListeners.Count -gt 0) {
    throw "listener port $PortNumber conflicts with existing WinStation listeners: $($conflictingListeners -join ', ')"
}

if (-not (Test-Path -LiteralPath $targetListener)) {
    Copy-Item -Path $sourceListener -Destination $targetListener -Recurse -Force
}

Set-ItemProperty -Path $targetListener -Name "LoadableProtocol_Object" -Type String -Value $ProtocolManagerClsid
Set-ItemProperty -Path $targetListener -Name "PortNumber" -Type DWord -Value $PortNumber

Set-ItemProperty -Path $targetListener -Name "fEnableWinStation" -Type DWord -Value 1
# Remove the kernel-mode transport (TDTCP / tdtcp.sys) from our listener.
# When LoadableProtocol_Object is set, the COM-based protocol manager owns the network connection
# entirely (including TCP accept).  Leaving PdDLL=tdtcp causes the kernel transport to bind our
# port *before* StartListen is called on the DLL, so the companion's TcpListener::bind fails and
# TermService reports "listener stack was down – Catastrophic failure".
Set-ItemProperty -Path $targetListener -Name "PdClass" -Type DWord -Value 0
Set-ItemProperty -Path $targetListener -Name "PdDLL"   -Type String -Value ""
Set-ItemProperty -Path $targetListener -Name "PdName"  -Type String -Value ""
Set-ItemProperty -Path $targetListener -Name "PdFlag"  -Type DWord -Value 0

$clsidRoot = "HKLM:\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid"
$inprocServer32 = Join-Path -Path $clsidRoot -ChildPath "InprocServer32"
$clsidRootRegPath = "HKLM\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid"
$inprocServer32RegPath = "$clsidRootRegPath\InprocServer32"

& reg.exe add $clsidRootRegPath /f | Out-Null
& reg.exe add $clsidRootRegPath /ve /t REG_SZ /d "IronRDP WTS Protocol Manager" /f | Out-Null

& reg.exe add $inprocServer32RegPath /f | Out-Null
& reg.exe add $inprocServer32RegPath /ve /t REG_SZ /d $providerDllPathResolved /f | Out-Null
Set-ItemProperty -Path $inprocServer32 -Name "ThreadingModel" -Type String -Value "Both"

# Enable DLL debug logging by injecting the env var into TermService's per-service environment.
# TermService runs inside svchost.exe; per-service env vars are set via the Environment
# REG_MULTI_SZ value under the service's registry key.
$termServiceRegPath = "HKLM:\SYSTEM\CurrentControlSet\Services\TermService"
$debugLogPath = "C:\IronRDPDeploy\logs\wts-provider-debug.log"
$debugEnvEntry = "IRONRDP_WTS_PROVIDER_DEBUG_LOG=$debugLogPath"
try {
    $existingValue = $null
    try {
        $existingValue = Get-ItemPropertyValue -Path $termServiceRegPath -Name "Environment" -ErrorAction Stop
    }
    catch [System.Management.Automation.ItemNotFoundException], [System.Management.Automation.PSArgumentException] {
        $existingValue = $null
    }

    $existing = if ($null -eq $existingValue) { @() } else { @($existingValue) }

    # Keep other vars, replace or add our entry.
    $newEnv = @($existing | Where-Object { $_ -notlike "IRONRDP_WTS_PROVIDER_DEBUG_LOG=*" })
    $newEnv += $debugEnvEntry

    New-ItemProperty -Path $termServiceRegPath -Name "Environment" -PropertyType MultiString -Value $newEnv -Force | Out-Null
    Write-Host "  dll debug log: $debugLogPath (via TermService env)"
} catch {
    Write-Warning "Could not set TermService environment for DLL debug log: $($_.Exception.Message)"
}

Write-Host "Installed side-by-side protocol provider"
Write-Host "  listener: $ListenerName"
Write-Host "  port: $PortNumber"
Write-Host "  default-listener-port registry: $($portSettingInfo.RegistryPath)\\$($portSettingInfo.RegistryValueName)"
Write-Host "  clsid: $ProtocolManagerClsid"
Write-Host "  dll: $providerDllPathResolved"

if ($RestartTermService.IsPresent) {
    Write-Warning "Restarting TermService now"

    & sc.exe config TermService start= disabled | Out-Null

    $termServiceStopped = $false
    $termServicePid = 0
    $stopDeadline = (Get-Date).AddSeconds($TermServiceStopTimeoutSeconds)
    while ((Get-Date) -lt $stopDeadline) {
        & sc.exe stop TermService | Out-Null

        $waitDeadline = (Get-Date).AddSeconds(8)
        while ((Get-Date) -lt $waitDeadline) {
            $service = Get-Service -Name "TermService" -ErrorAction SilentlyContinue
            if ($null -eq $service -or $service.Status -eq "Stopped") {
                $termServiceStopped = $true
                break
            }

            Start-Sleep -Milliseconds 500
        }

        if ($termServiceStopped) {
            break
        }

        $serviceCim = Get-CimInstance Win32_Service -Filter "Name='TermService'" -ErrorAction SilentlyContinue
        $termServicePid = 0
        if ($null -ne $serviceCim) {
            $termServicePid = [int]$serviceCim.ProcessId
        }

        if ($termServicePid -gt 0) {
            Write-Warning "TermService still running; force-stopping host process PID $termServicePid"
            Stop-Process -Id $termServicePid -Force -ErrorAction SilentlyContinue

            $processStillRunning = $false
            try {
                $null = Get-Process -Id $termServicePid -ErrorAction Stop
                $processStillRunning = $true
            }
            catch {
                $processStillRunning = $false
            }

            if ($processStillRunning) {
                $taskkillOutput = & taskkill.exe /PID $termServicePid /F /T 2>&1
                if (($LASTEXITCODE -ne 0) -and ($LASTEXITCODE -ne 128)) {
                    $taskkillMessage = ($taskkillOutput | Out-String).Trim()
                    throw "taskkill failed for TermService host PID $termServicePid (exit_code=$LASTEXITCODE): $taskkillMessage"
                }
            }
        }
        Start-Sleep -Seconds 1
    }

    if (-not $termServiceStopped) {
        $service = Get-Service -Name "TermService" -ErrorAction SilentlyContinue
        $statusText = if ($null -eq $service) { 'missing' } else { $service.Status }
        throw "TermService did not stop within ${TermServiceStopTimeoutSeconds}s during provider install restart (status=$statusText, last_pid=$termServicePid)"
    }

    & sc.exe config TermService start= demand | Out-Null

    & sc.exe start TermService | Out-Null

    $startDeadline = (Get-Date).AddSeconds($TermServiceStartTimeoutSeconds)
    while ((Get-Date) -lt $startDeadline) {
        $service = Get-Service -Name "TermService" -ErrorAction SilentlyContinue
        if ($null -ne $service -and $service.Status -eq "Running") {
            return
        }

        Start-Sleep -Seconds 2
    }

    $service = Get-Service -Name "TermService" -ErrorAction SilentlyContinue
    if ($null -eq $service) {
        throw "TermService service not found after provider install restart"
    }

    throw "TermService did not reach Running within ${TermServiceStartTimeoutSeconds}s during provider install restart (status=$($service.Status))"
}
