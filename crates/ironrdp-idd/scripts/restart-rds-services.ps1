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
    [switch]$SkipTermService,

    [Parameter()]
    [switch]$SkipIronRdpTermSrv
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

$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    Invoke-Command -Session $session -ScriptBlock {
        param([bool]$SkipTermService, [bool]$SkipIronRdpTermSrv)

        Set-StrictMode -Version Latest
        $ErrorActionPreference = 'Stop'

        $termService = Get-Service -Name 'TermService' -ErrorAction SilentlyContinue
        $ironRdpTermSrv = Get-Service -Name 'IronRdpTermSrv' -ErrorAction SilentlyContinue

        if (-not $SkipIronRdpTermSrv) {
            if ($null -ne $ironRdpTermSrv) {
                try {
                    Stop-Service -Name 'IronRdpTermSrv' -Force -ErrorAction Stop
                } catch {
                    Write-Host "IronRdpTermSrv stop skipped/failed: $($_.Exception.Message)"
                }
            } else {
                Write-Host 'IronRdpTermSrv service not found (skipping stop)'
            }
        }

        if (-not $SkipTermService) {
            if ($null -eq $termService) {
                throw 'TermService not found'
            }

            Stop-Service -Name 'TermService' -Force -ErrorAction Stop
            Start-Sleep -Seconds 1
            Start-Service -Name 'TermService' -ErrorAction Stop
        }

        if (-not $SkipIronRdpTermSrv) {
            if ($null -ne $ironRdpTermSrv) {
                try {
                    Start-Service -Name 'IronRdpTermSrv' -ErrorAction Stop
                } catch {
                    Write-Host "IronRdpTermSrv start failed: $($_.Exception.Message)"
                }
            } else {
                Write-Host 'IronRdpTermSrv service not found (skipping start)'
            }
        }

        $termService = Get-Service -Name 'TermService' -ErrorAction SilentlyContinue
        $ironRdpTermSrv = Get-Service -Name 'IronRdpTermSrv' -ErrorAction SilentlyContinue

        [pscustomobject]@{
            TermService = if ($null -ne $termService) { $termService.Status.ToString() } else { 'NotFound' }
            IronRdpTermSrv = if ($null -ne $ironRdpTermSrv) { $ironRdpTermSrv.Status.ToString() } else { 'NotFound' }
        }
    } -ArgumentList $SkipTermService.IsPresent, $SkipIronRdpTermSrv.IsPresent
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}

