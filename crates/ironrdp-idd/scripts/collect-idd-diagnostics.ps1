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
    [string]$RemoteDiagRoot = "C:\Temp\IddDiagnostics",

    [Parameter()]
    [string]$LocalOutZip = ""
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

if ([string]::IsNullOrWhiteSpace($LocalOutZip)) {
    $timestamp = Get-Date -Format 'yyyyMMdd-HHmm'
    $LocalOutZip = Join-Path (Get-Location) "IddDiagnostics-$timestamp.zip"
}

$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    $remoteZip = Invoke-Command -Session $session -ScriptBlock {
        param($RemoteDiagRoot)

        $ErrorActionPreference = 'Stop'

        $diagPath = $RemoteDiagRoot
        $zipPath = "C:\Temp\IddDiagnostics.zip"

        New-Item -ItemType Directory -Path $diagPath -Force | Out-Null

        pnputil /enum-devices /class Display | Out-File (Join-Path $diagPath 'devices.txt')

        try {
            Get-WinEvent -LogName System -MaxEvents 200 -ErrorAction Stop |
                Where-Object { $_.ProviderName -match 'Idd|Display|WudfRd|TermDD|dxgkrnl' } |
                Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message |
                Out-File (Join-Path $diagPath 'system-events.txt')
        } catch {
            "Failed to collect system events: $_" | Out-File (Join-Path $diagPath 'system-events.txt')
        }

        try {
            Get-ChildItem "C:\Windows\System32\DriverStore\FileRepository\*IronRdp*" -Recurse -ErrorAction SilentlyContinue |
                Select-Object FullName, Length, LastWriteTime |
                Out-File (Join-Path $diagPath 'driver-files.txt')
        } catch {
            "Failed to enumerate driver store: $_" | Out-File (Join-Path $diagPath 'driver-files.txt')
        }

        Compress-Archive -Path $diagPath -DestinationPath $zipPath -Force
        $zipPath
    } -ArgumentList $RemoteDiagRoot

    Copy-Item -FromSession $session -Path $remoteZip -Destination $LocalOutZip -Force

    Write-Host "Downloaded diagnostics: $LocalOutZip"
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}

