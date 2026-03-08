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
    [string]$RemotePath = "C:\Program Files\IronRDP\idd",

    [Parameter()]
    [string]$TestCertsUrl = 'https://raw.githubusercontent.com/Devolutions/devolutions-authenticode/master/data/certs',

    [Parameter()]
    [string]$CodeSignCertPassword = 'CodeSign123!'
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

$certDownloadRoot = Join-Path ([System.IO.Path]::GetTempPath()) 'IronRdpIddCodeSign'
New-Item -ItemType Directory -Path $certDownloadRoot -Force | Out-Null
$rootCertFull = Join-Path $certDownloadRoot 'authenticode-test-ca.crt'
$codeSignPfxFull = Join-Path $certDownloadRoot 'authenticode-test-cert.pfx'

$certArtifacts = @(
    [pscustomobject]@{ Url = "$TestCertsUrl/authenticode-test-ca.crt"; Path = $rootCertFull },
    [pscustomobject]@{ Url = "$TestCertsUrl/authenticode-test-cert.pfx"; Path = $codeSignPfxFull }
)

foreach ($artifact in $certArtifacts) {
    if (-not (Test-Path -LiteralPath $artifact.Path -PathType Leaf)) {
        Write-Host "Downloading test certificate artifact: $($artifact.Url)"
        Invoke-WebRequest -Uri $artifact.Url -OutFile $artifact.Path -ErrorAction Stop
    }
}

$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    Invoke-Command -Session $session -ScriptBlock {
        param($RemotePath)
        New-Item -ItemType Directory -Path $RemotePath -Force | Out-Null
    } -ArgumentList $RemotePath

    Copy-Item -Path $driverDllFull -Destination (Join-Path $RemotePath 'IronRdpIdd.dll') -ToSession $session -Force
    Copy-Item -Path $infFull -Destination (Join-Path $RemotePath 'IronRdpIdd.inf') -ToSession $session -Force
    Copy-Item -Path $catFull -Destination (Join-Path $RemotePath 'IronRdpIdd.cat') -ToSession $session -Force
    Copy-Item -Path $rootCertFull -Destination (Join-Path $RemotePath 'authenticode-test-ca.crt') -ToSession $session -Force
    Copy-Item -Path $codeSignPfxFull -Destination (Join-Path $RemotePath 'authenticode-test-cert.pfx') -ToSession $session -Force

    Invoke-Command -Session $session -ScriptBlock {
        param($RemotePath, $CodeSignCertPassword)

        $infPath = Join-Path $RemotePath 'IronRdpIdd.inf'
        $rootCertPath = Join-Path $RemotePath 'authenticode-test-ca.crt'
        $codeSignPfxPath = Join-Path $RemotePath 'authenticode-test-cert.pfx'
        $codeSignCertPath = Join-Path $RemotePath 'authenticode-test-cert.cer'

        Import-Certificate -FilePath $rootCertPath -CertStoreLocation 'cert:\LocalMachine\Root' | Out-Null

        $secureCodeSignPassword = ConvertTo-SecureString $CodeSignCertPassword -AsPlainText -Force
        Import-PfxCertificate -FilePath $codeSignPfxPath -CertStoreLocation 'cert:\LocalMachine\My' -Password $secureCodeSignPassword | Out-Null

        $codeSigningCert = Get-ChildItem cert:\LocalMachine\My -CodeSigning | Where-Object { $_.Subject -eq 'CN=Test Code Signing Certificate' } | Select-Object -First 1
        if ($null -eq $codeSigningCert) {
            throw 'Code signing cert not found in LocalMachine\\My after PFX import'
        }

        if (Test-Path -LiteralPath $codeSignCertPath) {
            Remove-Item -LiteralPath $codeSignCertPath -Force -ErrorAction SilentlyContinue
        }
        Export-Certificate -Cert $codeSigningCert -FilePath $codeSignCertPath | Out-Null
        Import-Certificate -FilePath $codeSignCertPath -CertStoreLocation 'cert:\LocalMachine\TrustedPublisher' | Out-Null
        Write-Host 'Provisioned LocalMachine certificate trust for IronRdpIdd'

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

        Write-Host 'Driver installed successfully'

        # Ensure WUDFRd (UMDF kernel reflector) is running and set to auto-start.
        # Without WUDFRd, no UMDF driver (including our IDD) can load.
        $wudfrd = Get-Service -Name WUDFRd -ErrorAction SilentlyContinue
        if ($null -ne $wudfrd) {
            if ($wudfrd.StartType -ne 'Automatic') {
                Write-Host 'Setting WUDFRd to auto-start...'
                sc.exe config WUDFRd start= auto | Out-Null
            }
            if ($wudfrd.Status -ne 'Running') {
                Write-Host 'Starting WUDFRd service...'
                Start-Service -Name WUDFRd -ErrorAction Stop
            }
            Write-Host "WUDFRd: Status=$((Get-Service WUDFRd).Status), StartType=$((Get-Service WUDFRd).StartType)"
        } else {
            Write-Warning 'WUDFRd service not found - UMDF drivers will not load'
        }

        # Clean up stale IDD phantom devices from previous sessions.
        # These can accumulate and cause PnP confusion.
        $staleDevices = @(Get-PnpDevice -FriendlyName 'IronRDP Indirect Display*' -ErrorAction SilentlyContinue |
            Where-Object { $_.Status -eq 'Unknown' -or $_.Status -eq 'Error' })
        if ($staleDevices.Count -gt 0) {
            Write-Host "Removing $($staleDevices.Count) stale IDD device(s)..."
            foreach ($dev in $staleDevices) {
                Write-Host "  Removing: $($dev.InstanceId) (Status=$($dev.Status))"
                pnputil /remove-device $dev.InstanceId 2>$null | Out-Null
            }
        }
    } -ArgumentList $RemotePath, $CodeSignCertPassword
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}