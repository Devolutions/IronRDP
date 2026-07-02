<#
.SYNOPSIS
    Enables loopback RDP on the local machine for the live-RDP smoke test, or restores prior state.

.DESCRIPTION
    Default mode snapshots the current registry/firewall/group-membership state to -StatePath, then:
    enables RDP connections, disables NLA (so autologon works without CredSSP), sets a random
    password on the current user, adds the user to "Remote Desktop Users" if needed, enables the
    "Remote Desktop" firewall rule group, starts TermService, and waits for 127.0.0.1:3389 to accept
    connections. Results are written to $env:GITHUB_OUTPUT (username, domain_username, password,
    hostname, port).

    -Cleanup restores every setting from -StatePath and removes the file.
#>
[CmdletBinding()]
param(
    [switch]$Cleanup,
    [string]$StatePath = "$env:RUNNER_TEMP\live-rdp-state.json"
)

$ErrorActionPreference = 'Stop'

$TerminalServerPath = 'HKLM:\System\CurrentControlSet\Control\Terminal Server'
$RdpTcpPath = 'HKLM:\System\CurrentControlSet\Control\Terminal Server\WinStations\RDP-Tcp'
$FirewallGroup = 'Remote Desktop'
$RdpUsersGroup = 'Remote Desktop Users'
$Username = $env:USERNAME

function Get-RegistryDwordOrNull {
    param([string]$Path, [string]$Name)
    $Item = Get-ItemProperty -Path $Path -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $Item) {
        return $null
    }
    return $Item.$Name
}

if ($Cleanup) {
    if (-not (Test-Path $StatePath)) {
        Write-Host "No state file at $StatePath, nothing to clean up."
        return
    }

    $State = Get-Content -Path $StatePath -Raw | ConvertFrom-Json

    if ($null -ne $State.DenyTSConnections) {
        Set-ItemProperty -Path $TerminalServerPath -Name 'fDenyTSConnections' -Value $State.DenyTSConnections
    }
    if ($null -ne $State.UserAuthentication) {
        Set-ItemProperty -Path $RdpTcpPath -Name 'UserAuthentication' -Value $State.UserAuthentication
    }

    foreach ($Rule in $State.FirewallRules) {
        if ($Rule.WasEnabled) {
            Enable-NetFirewallRule -Name $Rule.Name
        }
        else {
            Disable-NetFirewallRule -Name $Rule.Name
        }
    }

    if (-not $State.UserWasMember) {
        Remove-LocalGroupMember -Group $RdpUsersGroup -Member $State.Username -ErrorAction SilentlyContinue
    }

    Remove-Item -Path $StatePath -Force
    Write-Host "Restored local RDP configuration from $StatePath."
    return
}

$UserWasMember = [bool](Get-LocalGroupMember -Group $RdpUsersGroup -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like "*\$Username" })

$State = [PSCustomObject]@{
    DenyTSConnections  = Get-RegistryDwordOrNull -Path $TerminalServerPath -Name 'fDenyTSConnections'
    UserAuthentication = Get-RegistryDwordOrNull -Path $RdpTcpPath -Name 'UserAuthentication'
    FirewallRules      = @(Get-NetFirewallRule -DisplayGroup $FirewallGroup | ForEach-Object {
            [PSCustomObject]@{ Name = $_.Name; WasEnabled = ($_.Enabled -eq 'True') }
        })
    UserWasMember      = $UserWasMember
    Username           = $Username
}
$State | ConvertTo-Json -Depth 5 | Set-Content -Path $StatePath
Write-Host "Saved prior RDP configuration to $StatePath."

Set-ItemProperty -Path $TerminalServerPath -Name 'fDenyTSConnections' -Value 0

# Disable NLA so the daemon's own overlay (`enablecredsspsupport:i:0`) can authenticate via
# autologon without a CredSSP round-trip.
Set-ItemProperty -Path $RdpTcpPath -Name 'UserAuthentication' -Value 0

if (-not $UserWasMember) {
    Add-LocalGroupMember -Group $RdpUsersGroup -Member $Username
}

Enable-NetFirewallRule -DisplayGroup $FirewallGroup

$PasswordBytes = New-Object byte[] 24
[System.Security.Cryptography.RandomNumberGenerator]::Fill($PasswordBytes)
$Password = ([Convert]::ToBase64String($PasswordBytes) -replace '[^a-zA-Z0-9]', 'x') + '!A1'
Write-Host "::add-mask::$Password"

Set-LocalUser -Name $Username -Password (ConvertTo-SecureString -String $Password -AsPlainText -Force)

Start-Service -Name TermService

$Deadline = (Get-Date).AddSeconds(60)
$Connected = $false
while ((Get-Date) -lt $Deadline) {
    $Test = Test-NetConnection -ComputerName '127.0.0.1' -Port 3389 -WarningAction SilentlyContinue
    if ($Test.TcpTestSucceeded) {
        $Connected = $true
        break
    }
    Start-Sleep -Seconds 2
}
if (-not $Connected) {
    throw 'RDP did not become reachable on 127.0.0.1:3389 within the timeout.'
}

# ".\" denotes the local computer as the login authority, robust regardless of hostname length.
$DomainUsername = ".\$Username"

Add-Content -Path $env:GITHUB_OUTPUT -Value "username=$Username"
Add-Content -Path $env:GITHUB_OUTPUT -Value "domain_username=$DomainUsername"
Add-Content -Path $env:GITHUB_OUTPUT -Value "password=$Password"
Add-Content -Path $env:GITHUB_OUTPUT -Value "hostname=$env:COMPUTERNAME"
Add-Content -Path $env:GITHUB_OUTPUT -Value 'port=3389'

Write-Host "Local RDP enabled for user $Username."
