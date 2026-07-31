[CmdletBinding(DefaultParameterSetName = 'Enable')]
param(
    [Parameter(ParameterSetName = 'Enable')]
    [Parameter(ParameterSetName = 'Cleanup')]
    [string] $StatePath = (Join-Path $(
            if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
                [System.IO.Path]::GetTempPath()
            }
            else {
                $env:RUNNER_TEMP
            }
        ) 'ironrdp-agentic-rdp-state.json'),

    [Parameter(ParameterSetName = 'Cleanup')]
    [switch] $Cleanup
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$terminalServerPath = 'HKLM:\System\CurrentControlSet\Control\Terminal Server'
$rdpTcpPath = Join-Path $terminalServerPath 'WinStations\RDP-Tcp'
$rdpGroupName = 'Remote Desktop Users'

function Get-RegistryValue {
    param(
        [Parameter(Mandatory)]
        [string] $Path,

        [Parameter(Mandatory)]
        [string] $Name
    )

    $property = Get-ItemProperty -Path $Path -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $property) {
        return $null
    }

    return $property.$Name
}

function Write-JsonFile {
    param(
        [Parameter(Mandatory)]
        [string] $Path,

        [Parameter(Mandatory)]
        [object] $Value
    )

    $directory = Split-Path -Path $Path -Parent
    New-Item -Path $directory -ItemType Directory -Force | Out-Null
    $Value | ConvertTo-Json -Depth 8 | Set-Content -Path $Path -Encoding utf8NoBOM
}

function Test-TcpPort {
    param(
        [Parameter(Mandatory)]
        [string] $HostName,

        [Parameter(Mandatory)]
        [int] $Port
    )

    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $connect = $client.BeginConnect($HostName, $Port, $null, $null)
        if (-not $connect.AsyncWaitHandle.WaitOne([TimeSpan]::FromSeconds(1))) {
            return $false
        }

        $client.EndConnect($connect)
        return $true
    }
    catch {
        return $false
    }
    finally {
        $client.Dispose()
    }
}

function Wait-TcpPort {
    param(
        [Parameter(Mandatory)]
        [string] $HostName,

        [Parameter(Mandatory)]
        [int] $Port,

        [int] $TimeoutSeconds = 30
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        if (Test-TcpPort -HostName $HostName -Port $Port) {
            return
        }

        Start-Sleep -Seconds 1
    } while ((Get-Date) -lt $deadline)

    throw "Timed out waiting for $HostName`:$Port to accept TCP connections"
}

if ($Cleanup) {
    if (-not (Test-Path $StatePath)) {
        return
    }

    $state = Get-Content -Path $StatePath -Raw | ConvertFrom-Json

    if ($null -ne $state.fDenyTSConnections) {
        Set-ItemProperty -Path $terminalServerPath -Name 'fDenyTSConnections' -Value ([int] $state.fDenyTSConnections)
    }

    if ($null -ne $state.UserAuthentication) {
        Set-ItemProperty -Path $rdpTcpPath -Name 'UserAuthentication' -Value ([int] $state.UserAuthentication)
    }

    if ($state.PSObject.Properties.Name -contains 'FirewallRuleName') {
        Remove-NetFirewallRule -Name $state.FirewallRuleName -ErrorAction SilentlyContinue
    }
    else {
        foreach ($rule in @($state.FirewallRules)) {
            if ($null -ne $rule.Name -and $null -ne $rule.Enabled) {
                Set-NetFirewallRule -Name $rule.Name -Enabled $rule.Enabled -ErrorAction SilentlyContinue
            }
        }
    }

    if ($state.PSObject.Properties.Name -contains 'TemporaryUserName') {
        $temporaryUser = Get-LocalUser -Name $state.TemporaryUserName -ErrorAction SilentlyContinue
        if ($null -ne $temporaryUser `
            -and $state.PSObject.Properties.Name -contains 'TemporaryUserSid' `
            -and $temporaryUser.SID.Value -eq $state.TemporaryUserSid) {
            Remove-LocalGroupMember -Group $rdpGroupName -Member $state.TemporaryUserName -ErrorAction SilentlyContinue
            Remove-LocalUser -Name $state.TemporaryUserName -ErrorAction SilentlyContinue
        }
        elseif ($null -ne $temporaryUser) {
            Write-Warning "Skipping cleanup because temporary RDP user does not match state file: $($state.TemporaryUserName)"
        }
    }
    elseif ($state.AddedToRemoteDesktopUsers) {
        # Preserve cleanup compatibility with state files created by older versions of this script.
        Remove-LocalGroupMember -Group $rdpGroupName -Member $state.LocalUserName -ErrorAction SilentlyContinue
    }

    Remove-Item -Path $StatePath -Force -ErrorAction SilentlyContinue
    return
}

$temporaryUserName = "IronRdpAgent$PID"
if (Get-LocalUser -Name $temporaryUserName -ErrorAction SilentlyContinue) {
    throw "Temporary RDP user already exists: $temporaryUserName"
}

$passwordBytes = [System.Security.Cryptography.RandomNumberGenerator]::GetBytes(24)
$temporaryPassword = 'RdpAgent!' + [Convert]::ToBase64String($passwordBytes) + 'aA1!'
Write-Host "::add-mask::$temporaryPassword"

$firewallRuleName = "IronRdpAgenticRdp-$PID"

$state = [pscustomobject]@{
    TemporaryUserName = $temporaryUserName
    TemporaryUserSid = $null
    fDenyTSConnections = Get-RegistryValue -Path $terminalServerPath -Name 'fDenyTSConnections'
    UserAuthentication = Get-RegistryValue -Path $rdpTcpPath -Name 'UserAuthentication'
    FirewallRuleName = $firewallRuleName
}
Write-JsonFile -Path $StatePath -Value $state

$securePassword = ConvertTo-SecureString -String $temporaryPassword -AsPlainText -Force
$temporaryUser = New-LocalUser -Name $temporaryUserName -Password $securePassword -Description 'Temporary IronRDP agentic RDP test account'
$state.TemporaryUserSid = $temporaryUser.SID.Value
Write-JsonFile -Path $StatePath -Value $state
Add-LocalGroupMember -Group $rdpGroupName -Member $temporaryUserName

Set-ItemProperty -Path $terminalServerPath -Name 'fDenyTSConnections' -Value 0
Set-ItemProperty -Path $rdpTcpPath -Name 'UserAuthentication' -Value 0
Set-Service -Name TermService -StartupType Automatic
Start-Service -Name TermService
New-NetFirewallRule `
    -Name $firewallRuleName `
    -DisplayName 'IronRDP agentic RDP loopback' `
    -Direction Inbound `
    -Action Allow `
    -Protocol TCP `
    -LocalPort 3389 `
    -RemoteAddress '127.0.0.1' | Out-Null
Wait-TcpPort -HostName '127.0.0.1' -Port 3389

[pscustomobject]@{
    UserName = $temporaryUserName
    DomainUserName = "$env:COMPUTERNAME\$temporaryUserName"
    Password = $temporaryPassword
    HostName = '127.0.0.1'
    Port = 3389
    StatePath = $StatePath
} | ConvertTo-Json -Compress
