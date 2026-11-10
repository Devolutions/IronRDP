[CmdletBinding()]
param(
    [Parameter()]
    [string]$VmHostname = $env:IRONRDP_TEST_VM_HOSTNAME,

    [Parameter()]
    [string]$Username = $env:RDP_USERNAME,

    [Parameter()]
    [string]$Password = $env:RDP_PASSWORD,

    [Parameter()]
    [switch]$AttemptInstall,

    [Parameter()]
    [pscredential]$Credential
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot '..\..\..\scripts\TestVm.psm1') -Force
$credential = Resolve-TestVmCredential -Credential $Credential -Username $Username -Password $Password -Hostname $VmHostname
$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    $obj = Invoke-Command -Session $session -ScriptBlock {
        param([bool]$AttemptInstall)

        $ErrorActionPreference = 'Stop'

        $os = Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, ProductType
        $hasGetWindowsFeature = [bool](Get-Command -Name Get-WindowsFeature -ErrorAction SilentlyContinue)

        $result = [ordered]@{
            ComputerName = $env:COMPUTERNAME
            OS = $os
            HasGetWindowsFeature = $hasGetWindowsFeature
            Feature = 'RDS-RD-Server'
            Installed = $null
            InstallAttempted = $false
            Install = $null
            Notes = @()
        }

        if (-not $hasGetWindowsFeature) {
            $result.Notes += 'Get-WindowsFeature not available (likely not Windows Server or ServerManager missing).'
            $result.Notes += 'On Windows Server, install the Remote Desktop Session Host role: RDS-RD-Server.'
            return [pscustomobject]$result
        }

        Import-Module ServerManager -ErrorAction Stop

        $f = Get-WindowsFeature -Name 'RDS-RD-Server' -ErrorAction Stop
        $result.Installed = [bool]$f.Installed

        if (-not $f.Installed -and $AttemptInstall) {
            $result.InstallAttempted = $true
            $install = Install-WindowsFeature -Name 'RDS-RD-Server' -IncludeManagementTools -ErrorAction Stop
            $result.Install = [pscustomobject]@{
                Success = [bool]$install.Success
                RestartNeeded = [string]$install.RestartNeeded
                ExitCode = [string]$install.ExitCode
            }

            $f2 = Get-WindowsFeature -Name 'RDS-RD-Server' -ErrorAction Stop
            $result.Installed = [bool]$f2.Installed
        }

        [pscustomobject]$result
    } -ArgumentList $AttemptInstall.IsPresent

    $obj | ConvertTo-Json -Depth 6
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}
