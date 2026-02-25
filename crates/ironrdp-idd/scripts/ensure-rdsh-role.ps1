[CmdletBinding()]
param(
    [Parameter()]
    [string]$VmHostname = "IT-HELP-TEST.ad.it-help.ninja",

    [Parameter()]
    [string]$Username = "Administrator@ad.it-help.ninja",

    [Parameter()]
    [string]$Password = "",

    [Parameter()]
    [switch]$AttemptInstall
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($Password)) {
    throw "Password is required (pass -Password)"
}

$securePassword = ConvertTo-SecureString $Password -AsPlainText -Force
$credential = New-Object System.Management.Automation.PSCredential($Username, $securePassword)

$session = New-PSSession -ComputerName $VmHostname -Credential $credential -ErrorAction Stop
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
