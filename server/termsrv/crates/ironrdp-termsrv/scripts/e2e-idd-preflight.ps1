param($RuntimeRoot, $RuntimeStateFile, $DebugTraceFile, $DumpDir, $DriverInfPath)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$result = @{}
$issues = New-Object System.Collections.Generic.List[string]
$result['issues'] = @()
if (Test-Path -LiteralPath $DumpDir) {
    Remove-Item -LiteralPath $DumpDir -Recurse -Force -ErrorAction SilentlyContinue
}

function Grant-RuntimePathAccess {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [switch]$Container
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }

    $inheritanceFlags = if ($Container) {
        [System.Security.AccessControl.InheritanceFlags]'ContainerInherit, ObjectInherit'
    } else {
        [System.Security.AccessControl.InheritanceFlags]::None
    }
    $propagationFlags = [System.Security.AccessControl.PropagationFlags]::None
    $rights = [System.Security.AccessControl.FileSystemRights]::Modify
    $accessControlType = [System.Security.AccessControl.AccessControlType]::Allow
    $acl = Get-Acl -LiteralPath $Path

    foreach ($identity in @(
        (New-Object System.Security.Principal.NTAccount 'NT AUTHORITY', 'NETWORK SERVICE'),
        (New-Object System.Security.Principal.NTAccount 'NT SERVICE', 'TermService')
    )) {
        try {
            $null = $identity.Translate([System.Security.Principal.SecurityIdentifier])
            $rule = New-Object System.Security.AccessControl.FileSystemAccessRule(
                $identity,
                $rights,
                $inheritanceFlags,
                $propagationFlags,
                $accessControlType
            )
            $acl.SetAccessRule($rule)
        }
        catch {
        }
    }

    Set-Acl -LiteralPath $Path -AclObject $acl
}

New-Item -ItemType Directory -Path $RuntimeRoot -Force | Out-Null
New-Item -ItemType Directory -Path $DumpDir -Force | Out-Null
Grant-RuntimePathAccess -Path $RuntimeRoot -Container
Grant-RuntimePathAccess -Path $DumpDir -Container
Remove-Item -LiteralPath $DebugTraceFile -Force -ErrorAction SilentlyContinue

@(
    '# managed by e2e-test-screenshot.ps1',
    "dump_dir=$DumpDir",
    'session_id=0',
    'wddm_idd_enabled=false',
    'driver_loaded=false',
    'hardware_id=IronRdpIdd',
    'active_video_source=unknown'
) | Set-Content -Path $RuntimeStateFile
Grant-RuntimePathAccess -Path $RuntimeStateFile

$currentVersion = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' -ErrorAction SilentlyContinue
$productName = if ($null -ne $currentVersion) { [string]$currentVersion.ProductName } else { '' }
$installationType = if ($null -ne $currentVersion) { [string]$currentVersion.InstallationType } else { '' }
$isWindowsServer = ($installationType -eq 'Server') -or ($productName -like 'Windows Server*')
$rdsSessionHostInstalled = $true
if ($isWindowsServer) {
    $rdsSessionHost = Get-WindowsFeature -Name 'RDS-RD-Server' -ErrorAction SilentlyContinue
    $rdsSessionHostInstalled = ($null -ne $rdsSessionHost) -and [bool]$rdsSessionHost.Installed
    if (-not $rdsSessionHostInstalled) {
        $issues.Add('RDS-RD-Server (Remote Desktop Session Host) is not installed on this Windows Server host')
    }
}
$result['product_name'] = $productName
$result['installation_type'] = $installationType
$result['is_windows_server'] = $isWindowsServer
$result['rds_session_host_installed'] = $rdsSessionHostInstalled

$testSigningEnabled = ((bcdedit /enum | Out-String) -match '(?im)testsigning\s+Yes')
if (-not $testSigningEnabled) {
    $issues.Add('test-signing is disabled')
}
$result['testsigning_enabled'] = $testSigningEnabled

$trustedPublisher = Get-ChildItem cert:\LocalMachine\TrustedPublisher -ErrorAction SilentlyContinue |
    Where-Object {
        $_.Subject -eq 'CN=Test Code Signing Certificate' -and
        @($_.EnhancedKeyUsageList | Where-Object { $_.ObjectId -eq '1.3.6.1.5.5.7.3.3' }).Count -ge 1
    } |
    Select-Object -First 1
$trustedPublisherPresent = ($null -ne $trustedPublisher)
if (-not $trustedPublisherPresent) {
    $issues.Add('LocalMachine\\TrustedPublisher is missing the test code-signing certificate')
}
$result['trusted_publisher_present'] = $trustedPublisherPresent

$wudfrd = Get-Service -Name 'WUDFRd' -ErrorAction SilentlyContinue
$wudfrdPresent = ($null -ne $wudfrd)
if ($wudfrdPresent) {
    if ($wudfrd.StartType -ne 'Automatic') {
        & sc.exe config WUDFRd start= auto | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Could not configure WUDFRd (exit $LASTEXITCODE)" }
    }
    # WUDFRd can stop when no UMDF devices are active; prepare it before judging readiness.
    if ($wudfrd.Status -ne 'Running') {
        Start-Service -Name 'WUDFRd' -ErrorAction Stop
        $wudfrd = Get-Service -Name 'WUDFRd' -ErrorAction Stop
    }
}
$wudfrdRunning = $wudfrdPresent -and ($wudfrd.Status -eq 'Running')
if (-not $wudfrdPresent) {
    $issues.Add('WUDFRd service is missing')
} elseif (-not $wudfrdRunning) {
    $issues.Add("WUDFRd is not running (status=$($wudfrd.Status))")
}
$result['wudfrd_present'] = $wudfrdPresent
$result['wudfrd_running'] = $wudfrdRunning

$driverEnumText = (& pnputil /enum-drivers 2>$null | Out-String)
$iddDriverPackagePresent = ($driverEnumText -match '(?i)IronRdpIdd\.inf') -or ($driverEnumText -match '(?i)IronRDP Project')
if (-not $iddDriverPackagePresent) {
    $issues.Add('IronRdpIdd driver package is not installed')
}
$result['idd_driver_package_present'] = $iddDriverPackagePresent

$iddInfPresent = Test-Path -LiteralPath $DriverInfPath -PathType Leaf
$iddInfText = if ($iddInfPresent) { Get-Content -LiteralPath $DriverInfPath -Raw -ErrorAction SilentlyContinue } else { '' }
$iddInfUsesIddCx0102 = (-not [string]::IsNullOrWhiteSpace($iddInfText)) -and ($iddInfText -match '(?im)^\s*UmdfExtensions\s*=\s*IddCx0102\s*$')
$iddInfDisablesHostSharing = (-not [string]::IsNullOrWhiteSpace($iddInfText)) -and ($iddInfText -match '(?im)^\s*UmdfHostProcessSharing\s*=\s*ProcessSharingDisabled\s*$')
if (-not $iddInfPresent) {
    $issues.Add("IronRdpIdd INF not found at expected deploy path '$DriverInfPath'")
} else {
    if (-not $iddInfUsesIddCx0102) {
        $issues.Add('IronRdpIdd INF is not configured for UmdfExtensions=IddCx0102')
    }
    if ($isWindowsServer -and (-not $iddInfDisablesHostSharing)) {
        $issues.Add('IronRdpIdd INF does not set UmdfHostProcessSharing=ProcessSharingDisabled on this Windows Server host')
    }
}
$result['idd_inf_present'] = $iddInfPresent
$result['idd_inf_uses_iddcx0102'] = $iddInfUsesIddCx0102
$result['idd_inf_disables_host_sharing'] = $iddInfDisablesHostSharing

$iddDevices = @(Get-PnpDevice -Class Display -ErrorAction SilentlyContinue |
    Where-Object { $_.FriendlyName -like 'IronRDP Indirect Display*' -or $_.InstanceId -like '*IronRdpIdd*' })
$iddDevicePresent = ($iddDevices.Count -ge 1)
$iddDeviceOk = @($iddDevices | Where-Object { $_.Status -eq 'OK' }).Count -ge 1
$staleIddDeviceCount = @($iddDevices | Where-Object { $_.Status -in @('Unknown', 'Error') }).Count
if ($iddDevicePresent -and -not $iddDeviceOk) {
    $issues.Add('no healthy IronRdpIdd display device is present')
}
if ($staleIddDeviceCount -gt 0) {
    $issues.Add("stale/conflicting IronRDP display devices remain (count=$staleIddDeviceCount)")
}
$result['idd_device_present'] = $iddDevicePresent
$result['idd_device_ok'] = $iddDeviceOk
$result['stale_idd_device_count'] = $staleIddDeviceCount

$activationNotificationMode = $false
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
    $activationNotificationMode = ([int]$license.LicenseStatus -eq 5)
    $result['activation_license_status'] = [int]$license.LicenseStatus
    $result['activation_license_name'] = [string]$license.Name
    $result['activation_license_description'] = [string]$license.Description
    $result['activation_source'] = 'cim'
    if ($null -ne $license.LicenseStatusReason) {
        $reasonInt = [int64]$license.LicenseStatusReason
        if ($reasonInt -lt 0) {
            $reasonInt = $reasonInt -band 0xFFFFFFFF
        }
        $result['activation_license_status_reason_hex'] = ('0x{0:X8}' -f $reasonInt)
    }
} elseif (Get-Command cscript.exe -ErrorAction SilentlyContinue) {
    $slmgrText = (& cscript.exe //NoLogo "$env:SystemRoot\System32\slmgr.vbs" /dlv 2>$null | Out-String)
    if (-not [string]::IsNullOrWhiteSpace($slmgrText)) {
        $result['activation_source'] = 'slmgr'
        if ($slmgrText -match '(?im)^License Status:\s*Notification\b') {
            $activationNotificationMode = $true
            $result['activation_license_status'] = 5
        } elseif ($slmgrText -match '(?im)^License Status:\s*Licensed\b') {
            $result['activation_license_status'] = 1
        }

        if ($slmgrText -match '(?im)^Notification Reason:\s*(0x[0-9A-F]+)') {
            $result['activation_license_status_reason_hex'] = $Matches[1].ToUpperInvariant()
        }
    }
}
$result['activation_notification_mode'] = $activationNotificationMode
$result['runtime_state'] = if (Test-Path -LiteralPath $RuntimeStateFile) { Get-Content -LiteralPath $RuntimeStateFile -Raw } else { '' }
$result['issues'] = @($issues)
$result
