[CmdletBinding()]
param(
    [Parameter()]
    [Alias('ComputerName')]
    [string]$VmHostname = "IT-HELP-TERMSRV",

    [Parameter()]
    [string]$Username = $env:RDP_USERNAME,

    [Parameter()]
    [pscredential]$Credential,

    [Parameter()]
    [string]$Password = "",

    [Parameter()]
    [switch]$PromptPassword,

    [Parameter()]
    [string]$PasswordEnvVar = 'RDP_PASSWORD'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot '..\..\..\scripts\TestVm.psm1') -Force
$credential = Resolve-TestVmCredential -Credential $Credential -Username $Username -Password $Password -PromptPassword:$PromptPassword -PasswordEnvVar $PasswordEnvVar -Hostname $VmHostname

$session = $null
$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    Invoke-Command -Session $session -ScriptBlock {
        $ErrorActionPreference = 'Stop'

        $bcdeditOutput = bcdedit /enum | Select-String "testsigning"
        if ($bcdeditOutput -match "Yes") {
            Write-Host "Test signing is enabled"
        } else {
            throw "Test signing is NOT enabled - driver will not load"
        }
    }
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}
