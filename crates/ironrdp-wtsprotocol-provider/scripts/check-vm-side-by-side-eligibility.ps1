[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string[]]$VmNames,

    [Parameter()]
    [ValidateNotNullOrEmpty()]
    [string]$AdminUser = 'IT-HELP\Administrator',

    [Parameter()]
    [securestring]$AdminPassword,

    [Parameter()]
    [string]$AdminPasswordPlainText = $env:IRONRDP_VM_ADMIN_PASSWORD_PLAINTEXT,

    [Parameter()]
    [ValidateRange(0, 65535)]
    [int]$PortNumber = 0,

    [Parameter()]
    [switch]$RequireActiveSocket,

    [Parameter()]
    [switch]$FailOnIneligible
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$defaultsScript = Join-Path -Path $scriptRoot -ChildPath 'side-by-side-defaults.ps1'
. $defaultsScript

$PortNumber = Resolve-SideBySideListenerPort -PortNumber $PortNumber

if ($null -eq $AdminPassword) {
    if ([string]::IsNullOrWhiteSpace($AdminPasswordPlainText)) {
        throw 'missing VM admin password; pass -AdminPassword or -AdminPasswordPlainText (or set IRONRDP_VM_ADMIN_PASSWORD_PLAINTEXT)'
    }

    $AdminPassword = ConvertTo-SecureString -String $AdminPasswordPlainText -AsPlainText -Force
}

$credential = [pscredential]::new($AdminUser, $AdminPassword)
$results = New-Object System.Collections.Generic.List[object]

foreach ($vmName in $VmNames) {
    $result = [ordered]@{
        VmName = $vmName
        VmState = 'Unknown'
        CredentialAccess = $false
        ComputerName = $null
        UserName = $null
        TermDDPresent = $false
        RdpTransportPresent = $false
        TermServiceRunning = $false
        RemoteDesktopEnabled = $false
        RdpTcpEnabled = $false
        RdpTcpPort = $null
        SideBySideListenerExists = $false
        SideBySidePort = $null
        ActiveRdpTcpSocket = $false
        ActiveSideBySideSocket = $false
        Eligible = $false
        Reason = $null
    }

    try {
        $vm = Get-VM -Name $vmName -ErrorAction Stop
        $result.VmState = [string]$vm.State
    }
    catch {
        $result.Reason = "vm not found: $($_.Exception.Message)"
        $results.Add([PSCustomObject]$result)
        continue
    }

    if ($result.VmState -ne 'Running') {
        $result.Reason = "vm is not running (state=$($result.VmState))"
        $results.Add([PSCustomObject]$result)
        continue
    }

    $session = $null
    try {
        $session = New-PSSession -VMName $vmName -Credential $credential -ErrorAction Stop
        $remote = Invoke-Command -Session $session -ScriptBlock {
            $terminalServerPath = 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server'
            $rdpTcpPath = 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\RDP-Tcp'
            $sideBySidePath = 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations\IRDP-Tcp'

            $terminalServer = Get-ItemProperty -LiteralPath $terminalServerPath -ErrorAction SilentlyContinue
            $rdpTcp = Get-ItemProperty -LiteralPath $rdpTcpPath -ErrorAction SilentlyContinue
            $sideBySide = Get-ItemProperty -LiteralPath $sideBySidePath -ErrorAction SilentlyContinue
            $termService = Get-Service -Name 'TermService' -ErrorAction SilentlyContinue

            $listen3389 = @(Get-NetTCPConnection -State Listen -LocalPort 3389 -ErrorAction SilentlyContinue).Count
            $listenCustom = @(Get-NetTCPConnection -State Listen -LocalPort $using:PortNumber -ErrorAction SilentlyContinue).Count

            [pscustomobject]@{
                ComputerName = $env:COMPUTERNAME
                UserName = (whoami)
                TermDDPresent = (Test-Path -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Services\TermDD')
                RdpTransportPresent = (
                    (Test-Path -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Services\TermDD') -or
                    (Test-Path -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Services\UmRdpService') -or
                    (Test-Path -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Services\rdpbus') -or
                    (Test-Path -LiteralPath 'HKLM:\SYSTEM\CurrentControlSet\Services\RDPNP')
                )
                TermServiceRunning = ($null -ne $termService -and $termService.Status -eq 'Running')
                RemoteDesktopEnabled = ($null -ne $terminalServer -and [int]$terminalServer.fDenyTSConnections -eq 0)
                RdpTcpEnabled = ($null -ne $rdpTcp -and [int]$rdpTcp.fEnableWinStation -eq 1)
                RdpTcpPort = if ($null -ne $rdpTcp) { [int]$rdpTcp.PortNumber } else { $null }
                SideBySideListenerExists = ($null -ne $sideBySide)
                SideBySidePort = if ($null -ne $sideBySide) { [int]$sideBySide.PortNumber } else { $null }
                ActiveRdpTcpSocket = ($listen3389 -gt 0)
                ActiveSideBySideSocket = ($listenCustom -gt 0)
            }
        }

        $result.CredentialAccess = $true
        $result.ComputerName = $remote.ComputerName
        $result.UserName = $remote.UserName
        $result.TermDDPresent = [bool]$remote.TermDDPresent
        $result.RdpTransportPresent = [bool]$remote.RdpTransportPresent
        $result.TermServiceRunning = [bool]$remote.TermServiceRunning
        $result.RemoteDesktopEnabled = [bool]$remote.RemoteDesktopEnabled
        $result.RdpTcpEnabled = [bool]$remote.RdpTcpEnabled
        $result.RdpTcpPort = $remote.RdpTcpPort
        $result.SideBySideListenerExists = [bool]$remote.SideBySideListenerExists
        $result.SideBySidePort = $remote.SideBySidePort
        $result.ActiveRdpTcpSocket = [bool]$remote.ActiveRdpTcpSocket
        $result.ActiveSideBySideSocket = [bool]$remote.ActiveSideBySideSocket

        $problems = New-Object System.Collections.Generic.List[string]

        if (-not $result.RdpTransportPresent) {
            $problems.Add('RDP transport missing')
        }

        if (-not $result.TermServiceRunning) {
            $problems.Add('TermService not running')
        }

        if (-not $result.RemoteDesktopEnabled) {
            $problems.Add('Remote Desktop disabled')
        }

        if (-not $result.RdpTcpEnabled) {
            $problems.Add('RDP-Tcp listener disabled')
        }

        if ($RequireActiveSocket.IsPresent) {
            if (-not $result.ActiveRdpTcpSocket -and -not $result.ActiveSideBySideSocket) {
                $problems.Add('no active TCP listener socket on 3389 or configured side-by-side port')
            }
        }

        if ($problems.Count -eq 0) {
            $result.Eligible = $true
            $result.Reason = 'eligible'
        }
        else {
            $result.Reason = ($problems -join '; ')
        }
    }
    catch {
        $result.Reason = "credential/session failure: $($_.Exception.Message)"
    }
    finally {
        if ($null -ne $session) {
            Remove-PSSession -Session $session -ErrorAction SilentlyContinue
        }
    }

    $results.Add([PSCustomObject]$result)
}

$results | Sort-Object VmName | Format-Table VmName,VmState,CredentialAccess,RdpTransportPresent,TermDDPresent,TermServiceRunning,RemoteDesktopEnabled,RdpTcpEnabled,RdpTcpPort,SideBySideListenerExists,SideBySidePort,ActiveRdpTcpSocket,ActiveSideBySideSocket,Eligible,Reason -AutoSize | Out-String | Write-Host

$results

if ($FailOnIneligible.IsPresent) {
    $ineligibleCount = @($results | Where-Object { -not $_.Eligible }).Count
    if ($ineligibleCount -gt 0) {
        throw "$ineligibleCount VM(s) are not eligible for side-by-side mstsc TCP validation"
    }
}
