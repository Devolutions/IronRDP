[CmdletBinding()]
param(
    [Parameter()]
    [string]$VmHostname = "IT-HELP-TERMSRV",

    [Parameter()]
    [string]$Username = $env:RDP_USERNAME,

    [Parameter()]
    [string]$Password = $env:RDP_PASSWORD
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($Password)) {
    throw "Password is required (pass -Password)"
}

$securePassword = ConvertTo-SecureString $Password -AsPlainText -Force
$credential = New-Object System.Management.Automation.PSCredential($Username, $securePassword)

try {
    $session = New-PSSession -ComputerName $VmHostname -Credential $credential -ErrorAction Stop
}
catch {
    Write-Warning "WinRM over HTTP failed for $VmHostname; trying HTTPS (5986)"
    $session = New-PSSession -ComputerName $VmHostname -Credential $credential -UseSSL -Port 5986 -SessionOption (New-PSSessionOption -SkipCACheck -SkipCNCheck -SkipRevocationCheck) -ErrorAction Stop
}
try {
    Invoke-Command -Session $session -ScriptBlock {
        $ErrorActionPreference = 'Stop'

        $os = Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, ProductType

        $edition = $null
        $editionError = $null
        try {
            $edition = (Get-WindowsEdition -Online).Edition
        } catch {
            $editionError = $_.Exception.Message
        }

        [pscustomobject]@{
            ComputerName = $env:COMPUTERNAME
            OS = $os
            Edition = $edition
            EditionError = $editionError
            Notes = @(
                "Remote IDD testing requires Windows Server with the RDS-RD-Server role."
            )
        }
    }
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}
