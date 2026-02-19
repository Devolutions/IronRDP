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
    [switch]$CheckLocalPortListener
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$defaultsScript = Join-Path -Path $scriptRoot -ChildPath "side-by-side-defaults.ps1"
. $defaultsScript

$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber

$results = New-Object System.Collections.Generic.List[object]
$failed = $false

function Add-Result {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][bool]$Passed,
        [Parameter(Mandatory = $true)][string]$Details
    )

    $script:results.Add([PSCustomObject]@{
            Check = $Name
            Passed = $Passed
            Details = $Details
        })

    if (-not $Passed) {
        $script:failed = $true
    }
}

if (-not (Test-Path -LiteralPath $ProviderDllPath -PathType Leaf)) {
    throw "provider dll path does not exist: $ProviderDllPath"
}

$providerDllPathResolved = (Resolve-Path -LiteralPath $ProviderDllPath).Path
Add-Result -Name "Provider DLL exists" -Passed $true -Details $providerDllPathResolved

$listenerKey = "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\$ListenerName"
if (Test-Path -LiteralPath $listenerKey) {
    $listenerProps = Get-ItemProperty -LiteralPath $listenerKey

    $clsidMatch = $listenerProps.LoadableProtocol_Object -eq $ProtocolManagerClsid
    Add-Result -Name "Listener CLSID wiring" -Passed $clsidMatch -Details "expected=$ProtocolManagerClsid actual=$($listenerProps.LoadableProtocol_Object)"

    $portMatch = $listenerProps.PortNumber -eq $PortNumber
    Add-Result -Name "Listener port" -Passed $portMatch -Details "expected=$PortNumber actual=$($listenerProps.PortNumber)"
} else {
    Add-Result -Name "Listener registry key" -Passed $false -Details "missing key: $listenerKey"
}

$inprocServer32 = "HKLM:\SOFTWARE\Classes\CLSID\$ProtocolManagerClsid\InprocServer32"
if (Test-Path -LiteralPath $inprocServer32) {
    $inprocProps = Get-ItemProperty -LiteralPath $inprocServer32
    $registeredDll = $inprocProps.'(default)'

    if ([string]::IsNullOrWhiteSpace($registeredDll)) {
        Add-Result -Name "COM Inproc DLL path" -Passed $false -Details "empty InprocServer32 default value"
    } elseif (-not (Test-Path -LiteralPath $registeredDll -PathType Leaf)) {
        Add-Result -Name "COM Inproc DLL path" -Passed $false -Details "registered file missing: $registeredDll"
    } else {
        $registeredDllResolved = (Resolve-Path -LiteralPath $registeredDll).Path
        $dllMatch = $registeredDllResolved -eq $providerDllPathResolved
        Add-Result -Name "COM Inproc DLL path" -Passed $dllMatch -Details "expected=$providerDllPathResolved actual=$registeredDllResolved"
    }

    $threadingModel = [string]$inprocProps.ThreadingModel
    $threadingOk = $threadingModel -eq "Both"
    Add-Result -Name "COM threading model" -Passed $threadingOk -Details "expected=Both actual=$threadingModel"
} else {
    Add-Result -Name "COM registration key" -Passed $false -Details "missing key: $inprocServer32"
}

try {
    $termService = Get-Service -Name "TermService" -ErrorAction Stop
    $serviceOk = $termService.Status -eq "Running"
    Add-Result -Name "TermService status" -Passed $serviceOk -Details "status=$($termService.Status)"
} catch {
    Add-Result -Name "TermService status" -Passed $false -Details $_.Exception.Message
}

try {
    $terminalServerProps = Get-ItemProperty -LiteralPath "HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server" -ErrorAction Stop
    $denyTsConnections = [int]$terminalServerProps.fDenyTSConnections
    Add-Result -Name "Remote Desktop enabled" -Passed ($denyTsConnections -eq 0) -Details "fDenyTSConnections=$denyTsConnections"
} catch {
    Add-Result -Name "Remote Desktop enabled" -Passed $false -Details $_.Exception.Message
}

try {
    $matchingRuleCount = (Get-NetFirewallRule -Direction Inbound -Enabled True -Action Allow -ErrorAction Stop |
            Where-Object {
                $rule = $_
                $portFilters = $rule | Get-NetFirewallPortFilter
                $ruleHasPort = $false

                foreach ($filter in $portFilters) {
                    if ($filter.Protocol -eq "TCP" -and ($filter.LocalPort -contains "$PortNumber" -or $filter.LocalPort -eq "Any")) {
                        $ruleHasPort = $true
                        break
                    }
                }

                $ruleHasPort
            }).Count

    Add-Result -Name "Firewall inbound allow" -Passed ($matchingRuleCount -gt 0) -Details "matching_rules=$matchingRuleCount port=$PortNumber"
} catch {
    Add-Result -Name "Firewall inbound allow" -Passed $false -Details $_.Exception.Message
}

if ($CheckLocalPortListener.IsPresent) {
    try {
        $listenCount = (Get-NetTCPConnection -State Listen -LocalPort $PortNumber -ErrorAction Stop).Count
        Add-Result -Name "Local listener socket" -Passed ($listenCount -gt 0) -Details "port=$PortNumber listen_entries=$listenCount"
    } catch {
        Add-Result -Name "Local listener socket" -Passed $false -Details $_.Exception.Message
    }
}

$results | Format-Table -AutoSize | Out-String | Write-Host

if ($failed) {
    throw "side-by-side smoke test failed"
}

Write-Host "side-by-side smoke test passed"
