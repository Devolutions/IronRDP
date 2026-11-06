[CmdletBinding()]
param(
    [Parameter()]
    [string]$VmHostname = "IT-HELP-TERMSRV",

    [Parameter()]
    [string]$Username = $env:RDP_USERNAME,

    [Parameter()]
    [string]$Password = $env:RDP_PASSWORD,

    [Parameter()]
    [pscredential]$Credential
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot '..\..\..\scripts\TestVm.psm1') -Force
$credential = Resolve-TestVmCredential -Credential $Credential -Username $Username -Password $Password -Hostname $VmHostname
$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
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
