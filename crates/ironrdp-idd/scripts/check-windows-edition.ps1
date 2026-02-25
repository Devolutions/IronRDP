[CmdletBinding()]
param(
    [Parameter()]
    [string]$VmHostname = "IT-HELP-TEST.ad.it-help.ninja",

    [Parameter()]
    [string]$Username = "Administrator@ad.it-help.ninja",

    [Parameter()]
    [string]$Password = ""
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
                "Windows 11 Enterprise/Professional are required for 3rd party protocol providers."
            )
        }
    }
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}
