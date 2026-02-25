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
    [string]$DriverDll = "crates\ironrdp-idd\IronRdpIdd.dll",

    [Parameter()]
    [string]$InfPath = "crates\ironrdp-idd\IronRdpIdd.inf",

    [Parameter()]
    [string]$CatPath = "crates\ironrdp-idd\IronRdpIdd.cat",

    [Parameter()]
    [string]$RemotePath = "C:\Program Files\IronRDP\idd"
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

$workspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path

$driverDllFull = (Resolve-Path (Join-Path $workspaceRoot $DriverDll)).Path
$infFull = (Resolve-Path (Join-Path $workspaceRoot $InfPath)).Path
$catFull = (Resolve-Path (Join-Path $workspaceRoot $CatPath)).Path

$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    Invoke-Command -Session $session -ScriptBlock {
        param($RemotePath)
        New-Item -ItemType Directory -Path $RemotePath -Force | Out-Null
    } -ArgumentList $RemotePath

    Copy-Item -Path $driverDllFull -Destination (Join-Path $RemotePath "IronRdpIdd.dll") -ToSession $session -Force
    Copy-Item -Path $infFull -Destination (Join-Path $RemotePath "IronRdpIdd.inf") -ToSession $session -Force
    Copy-Item -Path $catFull -Destination (Join-Path $RemotePath "IronRdpIdd.cat") -ToSession $session -Force

    Invoke-Command -Session $session -ScriptBlock {
        param($RemotePath)

        $infPath = Join-Path $RemotePath "IronRdpIdd.inf"

        # Ensure we don't leave multiple IronRDP driver packages in the DriverStore.
        # When multiple versions are present, PnP can keep some device instances bound
        # to an older published oem*.inf even after installing a newer package.
        $enum = pnputil /enum-drivers
        if ($LASTEXITCODE -ne 0) {
            throw "pnputil /enum-drivers failed with exit code $LASTEXITCODE"
        }

        $entries = @()
        $cur = @{}
        foreach ($line in $enum) {
            if ($line -match '^Published Name:\s*(\S+)$') {
                if ($cur.ContainsKey('PublishedName')) {
                    $entries += [pscustomobject]$cur
                    $cur = @{}
                }
                $cur.PublishedName = $Matches[1]
                continue
            }
            if ($line -match '^Original Name:\s*(\S+)$') {
                $cur.OriginalName = $Matches[1]
                continue
            }
            if ($line -match '^Provider Name:\s*(.+)$') {
                $cur.ProviderName = $Matches[1].Trim()
                continue
            }
        }
        if ($cur.ContainsKey('PublishedName')) {
            $entries += [pscustomobject]$cur
        }

        $toDelete = @($entries | Where-Object { $_.ProviderName -eq 'IronRDP Project' })
        foreach ($drv in $toDelete) {
            Write-Host "Removing existing IronRDP driver package: $($drv.PublishedName) (Original=$($drv.OriginalName))"
            pnputil /delete-driver $drv.PublishedName /uninstall /force
            if ($LASTEXITCODE -ne 0) {
                throw "pnputil /delete-driver $($drv.PublishedName) failed with exit code $LASTEXITCODE"
            }
        }

        pnputil /add-driver $infPath /install
        if ($LASTEXITCODE -ne 0) {
            throw "Driver installation failed with exit code $LASTEXITCODE"
        }

        Write-Host "Driver installed successfully"
    } -ArgumentList $RemotePath
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}

