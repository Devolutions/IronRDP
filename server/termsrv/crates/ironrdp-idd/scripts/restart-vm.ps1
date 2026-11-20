[CmdletBinding()]
param(
    [Parameter()]
    [Alias('ComputerName')]
    [string]$VmHostname = $env:IRONRDP_TEST_VM_HOSTNAME,

    [Parameter()]
    [string]$Username = $env:RDP_USERNAME,

    [Parameter()]
    [pscredential]$Credential,

    [Parameter()]
    [string]$Password = "",

    [Parameter()]
    [switch]$PromptPassword,

    [Parameter()]
    [string]$PasswordEnvVar = 'RDP_PASSWORD',

    [Parameter()]
    [ValidateRange(1, 3600)]
    [int]$TimeoutSeconds = 180
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot '..\..\..\scripts\TestVm.psm1') -Force
$credential = Resolve-TestVmCredential -Credential $Credential -Username $Username -Password $Password -PromptPassword:$PromptPassword -PasswordEnvVar $PasswordEnvVar -Hostname $VmHostname

Write-Host "Restarting $VmHostname..." -ForegroundColor Cyan

$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    Invoke-Command -Session $session -ScriptBlock { Restart-Computer -Force -ErrorAction Stop }
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}

$settleSeconds = [Math]::Min(20, [Math]::Max(0, $TimeoutSeconds - 1))
Start-Sleep -Seconds $settleSeconds
$session = Wait-TestVmSession -Hostname $VmHostname -Credential $credential -TimeoutSeconds ($TimeoutSeconds - $settleSeconds)
try {
    Write-Host "VM is back online (WinRM ready)." -ForegroundColor Green
} finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}
