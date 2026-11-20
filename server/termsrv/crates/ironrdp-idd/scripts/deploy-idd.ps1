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
    [string]$CertsUrl = 'https://raw.githubusercontent.com/Devolutions/psign/2a7a928a7db8479ca4c96273579aac8c85f60a09/tests/fixtures/devolutions-authenticode',

    [Parameter()]
    [string]$TestCertificatesDirectory = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot '..\..\..\scripts\TestVm.psm1') -Force
$credential = Resolve-TestVmCredential -Credential $Credential -Username $Username -Password $Password -PromptPassword:$PromptPassword -PasswordEnvVar $PasswordEnvVar -Hostname $VmHostname

$certificates = & (Join-Path $PSScriptRoot 'get-test-certificates.ps1') -CertsUrl $CertsUrl `
    -CertificateDirectory $TestCertificatesDirectory
$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    $remotePath = 'C:\ProgramData\IronRDP\TestSigning'
    Invoke-Command -Session $session -ScriptBlock {
        param($Path)
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    } -ArgumentList $remotePath
    Copy-Item -LiteralPath $certificates.RootCertificate -Destination "$remotePath\authenticode-test-ca.crt" -ToSession $session -Force
    Copy-Item -LiteralPath $certificates.SignerCertificate -Destination "$remotePath\authenticode-test-cert.cer" -ToSession $session -Force
    $deployment = Invoke-Command -Session $session -ScriptBlock {
        param($Path)
        $ErrorActionPreference = 'Stop'
        Import-Certificate -FilePath "$Path\authenticode-test-ca.crt" -CertStoreLocation cert:\LocalMachine\Root | Out-Null
        Import-Certificate -FilePath "$Path\authenticode-test-cert.cer" -CertStoreLocation cert:\LocalMachine\TrustedPublisher | Out-Null
        $bootSettings = bcdedit /enum '{current}' | Out-String
        if ($LASTEXITCODE -ne 0) {
            throw "Could not inspect boot configuration (exit $LASTEXITCODE)"
        }
        $testSigningWasEnabled = $bootSettings -match '(?im)^\s*testsigning\s+Yes\s*$'

        bcdedit /set testsigning on | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "Could not enable test signing (exit $LASTEXITCODE); check Secure Boot policy"
        }
        [pscustomobject]@{
            RebootRequired = -not $testSigningWasEnabled
        }
    } -ArgumentList $remotePath
    Write-Host 'Target-only certificate trust provisioned'
    if ($deployment.RebootRequired) {
        Write-Host 'A reboot is required before Windows will load test-signed drivers'
    }
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}

$deployment
