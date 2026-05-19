[CmdletBinding(DefaultParameterSetName = 'Enable')]
param(
    [Parameter(ParameterSetName = 'Enable')]
    [Parameter(ParameterSetName = 'Cleanup')]
    [string] $StatePath = (Join-Path $env:RUNNER_TEMP 'ironrdp-agentic-rdp-state.json'),

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

    foreach ($rule in @($state.FirewallRules)) {
        if ($null -ne $rule.Name -and $null -ne $rule.Enabled) {
            Set-NetFirewallRule -Name $rule.Name -Enabled $rule.Enabled -ErrorAction SilentlyContinue
        }
    }

    if ($state.AddedToRemoteDesktopUsers) {
        Remove-LocalGroupMember -Group $rdpGroupName -Member $state.LocalUserName -ErrorAction SilentlyContinue
    }

    Remove-Item -Path $StatePath -Force -ErrorAction SilentlyContinue
    return
}

$localUserName = $env:USERNAME
if ([string]::IsNullOrWhiteSpace($localUserName)) {
    throw 'USERNAME is not set; cannot configure a local RDP user'
}

$passwordBytes = [System.Security.Cryptography.RandomNumberGenerator]::GetBytes(24)
$temporaryPassword = 'RdpAgent!' + [Convert]::ToBase64String($passwordBytes) + 'aA1!'
Write-Host "::add-mask::$temporaryPassword"

$currentMembers = @(Get-LocalGroupMember -Group $rdpGroupName -ErrorAction SilentlyContinue | ForEach-Object { $_.Name })
$memberNames = @($localUserName, "$env:COMPUTERNAME\$localUserName")
$wasRdpMember = [bool]($currentMembers | Where-Object { $memberNames -contains $_ } | Select-Object -First 1)

$state = [pscustomobject]@{
    LocalUserName = $localUserName
    DomainUserName = "$env:COMPUTERNAME\$localUserName"
    fDenyTSConnections = Get-RegistryValue -Path $terminalServerPath -Name 'fDenyTSConnections'
    UserAuthentication = Get-RegistryValue -Path $rdpTcpPath -Name 'UserAuthentication'
    FirewallRules = @(Get-NetFirewallRule -DisplayGroup 'Remote Desktop' -ErrorAction SilentlyContinue | Select-Object -Property Name, Enabled)
    AddedToRemoteDesktopUsers = (-not $wasRdpMember)
}
Write-JsonFile -Path $StatePath -Value $state

$securePassword = ConvertTo-SecureString -String $temporaryPassword -AsPlainText -Force
Set-LocalUser -Name $localUserName -Password $securePassword

if (-not $wasRdpMember) {
    Add-LocalGroupMember -Group $rdpGroupName -Member $localUserName
}

Set-ItemProperty -Path $terminalServerPath -Name 'fDenyTSConnections' -Value 0
Set-ItemProperty -Path $rdpTcpPath -Name 'UserAuthentication' -Value 0
Set-Service -Name TermService -StartupType Automatic
Start-Service -Name TermService
Enable-NetFirewallRule -DisplayGroup 'Remote Desktop' | Out-Null
Wait-TcpPort -HostName '127.0.0.1' -Port 3389

[pscustomobject]@{
    UserName = $localUserName
    DomainUserName = "$env:COMPUTERNAME\$localUserName"
    Password = $temporaryPassword
    HostName = '127.0.0.1'
    Port = 3389
    StatePath = $StatePath
} | ConvertTo-Json -Compress
