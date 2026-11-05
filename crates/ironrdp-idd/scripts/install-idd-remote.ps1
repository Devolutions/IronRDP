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
    [string]$DriverDll = "crates\ironrdp-idd\IronRdpIdd.dll",

    [Parameter()]
    [string]$InfPath = "crates\ironrdp-idd\IronRdpIdd.inf",

    [Parameter()]
    [string]$CatPath = "crates\ironrdp-idd\IronRdpIdd.cat",

    [Parameter()]
    [string]$RemotePath = "C:\Program Files\IronRDP\idd",

    [Parameter()]
    [string]$TestCertsUrl = 'https://raw.githubusercontent.com/Devolutions/psign/2a7a928a7db8479ca4c96273579aac8c85f60a09/tests/fixtures/devolutions-authenticode',

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

$certificates = & (Join-Path $PSScriptRoot 'get-test-certificates.ps1') -CertsUrl $TestCertsUrl -CodeSignCertPassword $CodeSignCertPassword

$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    Invoke-Command -Session $session -ScriptBlock {
        param($RemotePath)
        New-Item -ItemType Directory -Path $RemotePath -Force | Out-Null
    } -ArgumentList $RemotePath

    Copy-Item -Path $driverDllFull -Destination (Join-Path $RemotePath 'IronRdpIdd.dll') -ToSession $session -Force
    Copy-Item -Path $infFull -Destination (Join-Path $RemotePath 'IronRdpIdd.inf') -ToSession $session -Force
    Copy-Item -Path $catFull -Destination (Join-Path $RemotePath 'IronRdpIdd.cat') -ToSession $session -Force
    Copy-Item -Path $certificates.RootCertificate -Destination (Join-Path $RemotePath 'authenticode-test-ca.crt') -ToSession $session -Force
    Copy-Item -Path $certificates.SignerCertificate -Destination (Join-Path $RemotePath 'authenticode-test-cert.cer') -ToSession $session -Force

    Invoke-Command -Session $session -ScriptBlock {
        param($RemotePath)

        $ErrorActionPreference = 'Stop'

        $displayDirectory = 'C:\ProgramData\IronRDPDisplay'
        if (Test-Path -LiteralPath $displayDirectory) {
            $directory = Get-Item -LiteralPath $displayDirectory
            $owner = (Get-Acl -LiteralPath $displayDirectory).GetOwner([Security.Principal.SecurityIdentifier]).Value
            if (($directory.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
                $owner -notin @('S-1-5-18', 'S-1-5-32-544')) {
                throw 'The display request directory is not owned by SYSTEM or Administrators'
            }
        } else {
            New-Item -ItemType Directory -Path $displayDirectory | Out-Null
        }
        $acl = [Security.AccessControl.DirectorySecurity]::new()
        $acl.SetAccessRuleProtection($true, $false)
        foreach ($sid in 'S-1-5-18', 'S-1-5-32-544', 'S-1-5-19', 'S-1-5-20') {
            $rights = if ($sid -in @('S-1-5-18', 'S-1-5-32-544')) { 'FullControl' } else { 'ReadAndExecute' }
            $rule = [Security.AccessControl.FileSystemAccessRule]::new(
                [Security.Principal.SecurityIdentifier]::new($sid), $rights,
                'ContainerInherit,ObjectInherit', 'None', 'Allow')
            $acl.AddAccessRule($rule)
        }
        Set-Acl -LiteralPath $displayDirectory -AclObject $acl

        $infPath = Join-Path $RemotePath 'IronRdpIdd.inf'
        $rootCertPath = Join-Path $RemotePath 'authenticode-test-ca.crt'
        $codeSignCertPath = Join-Path $RemotePath 'authenticode-test-cert.cer'

        Import-Certificate -FilePath $rootCertPath -CertStoreLocation 'cert:\LocalMachine\Root' | Out-Null

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
            pnputil /delete-driver $drv.PublishedName /uninstall
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
    } -ArgumentList $RemotePath
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}