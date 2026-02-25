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
    [string]$PasswordEnvVar = 'IRONRDP_TESTVM_PASSWORD'
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


