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
    [string]$PasswordEnvVar = 'RDP_PASSWORD',

    [Parameter()]
    [string]$CertsUrl = 'https://raw.githubusercontent.com/Devolutions/psign/2a7a928a7db8479ca4c96273579aac8c85f60a09/tests/fixtures/devolutions-authenticode'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function New-TestVmSession {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Hostname,

        [Parameter(Mandatory = $true)]
        [pscredential]$Credential
    )

    try {
        return New-PSSession -ComputerName $Hostname -Credential $Credential -ErrorAction Stop
    }
    catch {
        Write-Warning "WinRM over HTTP failed for $Hostname; trying WinRM over HTTPS (5986)"
        $sessOpts = New-PSSessionOption -SkipCACheck -SkipCNCheck -SkipRevocationCheck
        return New-PSSession -ComputerName $Hostname -Credential $Credential -UseSSL -Port 5986 -SessionOption $sessOpts -ErrorAction Stop
    }
}

if ($PSBoundParameters.ContainsKey('Credential') -and ($null -ne $Credential)) {
    $credential = $Credential
} else {
    $passwordEffective = $Password
    if ([string]::IsNullOrWhiteSpace($passwordEffective) -and $PromptPassword.IsPresent) {
        $secure = Read-Host -Prompt "Password for $Username@$VmHostname" -AsSecureString
        $credential = New-Object System.Management.Automation.PSCredential($Username, $secure)
    } else {
        if ([string]::IsNullOrWhiteSpace($passwordEffective)) {
            $passwordEffective = [Environment]::GetEnvironmentVariable($PasswordEnvVar)
        }
        if ([string]::IsNullOrWhiteSpace($passwordEffective)) {
            throw "Password is required (pass -Password, pass -Credential, or set $PasswordEnvVar)"
        }
        $securePassword = ConvertTo-SecureString $passwordEffective -AsPlainText -Force
        $credential = New-Object System.Management.Automation.PSCredential($Username, $securePassword)
    }
}

$certificates = & (Join-Path $PSScriptRoot 'get-test-certificates.ps1') -CertsUrl $CertsUrl
$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    $remotePath = 'C:\ProgramData\IronRDP\TestSigning'
    Invoke-Command -Session $session -ScriptBlock {
        param($Path)
        New-Item -ItemType Directory -Path $Path -Force | Out-Null
    } -ArgumentList $remotePath
    Copy-Item -LiteralPath $certificates.RootCertificate -Destination "$remotePath\authenticode-test-ca.crt" -ToSession $session -Force
    Copy-Item -LiteralPath $certificates.SignerCertificate -Destination "$remotePath\authenticode-test-cert.cer" -ToSession $session -Force
    Invoke-Command -Session $session -ScriptBlock {
        param($Path)
        $ErrorActionPreference = 'Stop'
        Import-Certificate -FilePath "$Path\authenticode-test-ca.crt" -CertStoreLocation cert:\LocalMachine\Root | Out-Null
        Import-Certificate -FilePath "$Path\authenticode-test-cert.cer" -CertStoreLocation cert:\LocalMachine\TrustedPublisher | Out-Null
        bcdedit /set testsigning on
        if ($LASTEXITCODE -ne 0) {
            throw "Could not enable test signing (exit $LASTEXITCODE); check Secure Boot policy"
        }
        Write-Host 'Target-only certificate trust provisioned; reboot if test signing was not already enabled'
    } -ArgumentList $remotePath
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}
