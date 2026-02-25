[CmdletBinding()]
param(
    [Parameter()]
    [Alias('ComputerName')]
    [string]$VmHostname = "IT-HELP-TEST",

    [Parameter()]
    [string]$Username = "IT-HELP\Administrator",

    [Parameter()]
    [pscredential]$Credential,

    [Parameter()]
    [string]$Password = "",

    [Parameter()]
    [switch]$PromptPassword,

    [Parameter()]
    [string]$PasswordEnvVar = 'IRONRDP_TESTVM_PASSWORD',

    [Parameter()]
    [string]$CertsUrl = "https://raw.githubusercontent.com/Devolutions/devolutions-authenticode/master/data/certs"
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

 $session = $null
 $session = New-TestVmSession -Hostname $VmHostname -Credential $credential
 try {
     Invoke-Command -Session $session -ScriptBlock {
         param($CertsUrl)

    $ErrorActionPreference='Stop'

    $TempPath = "C:\Temp\IronRdpCerts"
    New-Item -ItemType Directory -Path $TempPath -Force | Out-Null

    @('authenticode-test-ca.crt','authenticode-test-cert.pfx') | ForEach-Object {
        Invoke-WebRequest -Uri "$CertsUrl/$_" -OutFile "$TempPath\$_" -ErrorAction Stop
    }

    Import-Certificate -FilePath "$TempPath\authenticode-test-ca.crt" -CertStoreLocation "cert:\LocalMachine\Root" | Out-Null

    $CodeSignPassword = ConvertTo-SecureString "CodeSign123!" -AsPlainText -Force
    Import-PfxCertificate -FilePath "$TempPath\authenticode-test-cert.pfx" -CertStoreLocation 'cert:\LocalMachine\My' -Password $CodeSignPassword | Out-Null

    # Driver packages require the catalog signer to be trusted as a publisher.
    # Import the public code signing certificate into LocalMachine\TrustedPublisher.
    try {
        $codeSigningCert = Get-ChildItem cert:\LocalMachine\My -CodeSigning | Where-Object { $_.Subject -eq 'CN=Test Code Signing Certificate' } | Select-Object -First 1
        if ($null -ne $codeSigningCert) {
            $pubCer = Join-Path $TempPath 'authenticode-test-cert.cer'
            Export-Certificate -Cert $codeSigningCert -FilePath $pubCer | Out-Null
            Import-Certificate -FilePath $pubCer -CertStoreLocation 'cert:\LocalMachine\TrustedPublisher' | Out-Null
        } else {
            Write-Warning 'Code signing cert not found in LocalMachine\\My after PFX import (TrustedPublisher not updated)'
        }
    } catch {
        Write-Warning "Failed to import code signing cert into TrustedPublisher: $($_.Exception.Message)"
    }

    bcdedit /set testsigning on | Out-Null

         Write-Host "Test signing enabled. A reboot is required."
     } -ArgumentList $CertsUrl
 }
 finally {
     Remove-PSSession -Session $session -ErrorAction SilentlyContinue
 }

