[CmdletBinding(DefaultParameterSetName = 'ByName')]
param(
    [Parameter(Mandatory = $true, ParameterSetName = 'ByName')]
    [string]$VmName,

    [Parameter(Mandatory = $true, ParameterSetName = 'ById')]
    [Guid]$VmId,

    [string]$WorkspaceRoot,
    [string]$VmDeployRoot = 'C:\IronRDPDeploy',
    [string]$AdminUser = 'IT-HELP\Administrator',
    [securestring]$AdminPassword,
    [string]$AdminPasswordPlainText = $env:IRONRDP_VM_ADMIN_PASSWORD_PLAINTEXT,
    [string]$ListenerName = 'IRDP-Tcp',
    [string]$ProtocolManagerClsid = '{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}',
    [int]$PortNumber = 4489,
    [switch]$SkipServiceInstall,
    [switch]$OpenMstsc
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ironRdpClsid = '{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}'
$useIronRdpProvider = ($ProtocolManagerClsid.Trim('{}').ToUpperInvariant() -eq $ironRdpClsid.Trim('{}').ToUpperInvariant())

$installService = -not $SkipServiceInstall.IsPresent

$WorkspaceRoot = if (-not [string]::IsNullOrWhiteSpace($WorkspaceRoot)) {
    $WorkspaceRoot
}
elseif (-not [string]::IsNullOrWhiteSpace($PSScriptRoot)) {
    Join-Path $PSScriptRoot '..\..\..'
}
else {
    (Get-Location).Path
}

$WorkspaceRoot = (Resolve-Path $WorkspaceRoot).Path
$providerScriptsDir = Join-Path $WorkspaceRoot 'crates\ironrdp-wtsprotocol-provider\scripts'
$providerDll = Join-Path $WorkspaceRoot 'target\release\ironrdp_wtsprotocol_provider.dll'
$serviceExe = Join-Path $WorkspaceRoot 'target\release\ironrdp-termsrv.exe'
$msiBuilderDir = Join-Path $WorkspaceRoot 'package\CevicheServiceWindowsManaged'
$msiPath = Join-Path $msiBuilderDir 'Release\IronRdpTermSrv.msi'

if (-not (Test-Path $providerScriptsDir)) {
    throw "Provider scripts directory not found: $providerScriptsDir"
}

if ($useIronRdpProvider) {
    Write-Host 'Building provider DLL (release)...' -ForegroundColor Cyan
    cargo build -p ironrdp-wtsprotocol-provider --release
    if (-not (Test-Path $providerDll)) {
        throw "Provider DLL not found after build: $providerDll"
    }
}

if ($installService) {
    if (-not (Test-Path $serviceExe)) {
        Write-Host 'Service executable not found. Attempting to build a companion service (release)...' -ForegroundColor Cyan

        $servicePackageCandidates = @('ironrdp-termsrv')
        foreach ($pkg in $servicePackageCandidates) {
            try {
                cargo build -p $pkg --release | Out-Null
                if (Test-Path $serviceExe) {
                    break
                }
            }
            catch {
                Write-Verbose "Failed to build package '$pkg': $($_.Exception.Message)"
            }
        }
    }

    if (-not (Test-Path $serviceExe)) {
        Write-Warning "Service executable not found ($serviceExe). Continuing with provider-only deployment."
        $installService = $false
    }
}

if ($installService) {
    if (-not (Test-Path $msiBuilderDir)) {
        Write-Warning "MSI builder directory not found ($msiBuilderDir). Continuing with provider-only deployment."
        $installService = $false
    }
}

if ($installService) {
    Write-Host 'Building TermSrv service MSI...' -ForegroundColor Cyan
    Push-Location $msiBuilderDir
    try {
        .\build-ceviche-service-msi.ps1 -ServiceExePath $serviceExe -ProviderDllPath $providerDll -Platform x64 -Configuration Release
    }
    finally {
        Pop-Location
    }

    if (-not (Test-Path $msiPath)) {
        Write-Warning "MSI not found after build ($msiPath). Continuing with provider-only deployment."
        $installService = $false
    }
}

$resolvedAdminPassword = if ($AdminPassword) {
    $AdminPassword
}
elseif (-not [string]::IsNullOrWhiteSpace($AdminPasswordPlainText)) {
    ConvertTo-SecureString -String $AdminPasswordPlainText -AsPlainText -Force
}
else {
    throw "Missing VM admin password. Pass -AdminPassword (SecureString) or set -AdminPasswordPlainText / env:IRONRDP_VM_ADMIN_PASSWORD_PLAINTEXT."
}

$credential = [pscredential]::new($AdminUser, $resolvedAdminPassword)

$session = if ($PSCmdlet.ParameterSetName -eq 'ByName') {
    New-PSSession -VMName $VmName -Credential $credential
} else {
    New-PSSession -VMId $VmId -Credential $credential
}

try {
    Invoke-Command -Session $session -ScriptBlock {
        param($Root)

        New-Item -ItemType Directory -Path $Root -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $Root 'scripts') -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $Root 'bin') -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $Root 'logs') -Force | Out-Null
    } -ArgumentList $VmDeployRoot

    Invoke-Command -Session $session -ScriptBlock {
        Set-StrictMode -Version Latest
        $ErrorActionPreference = 'Stop'

        $svc = Get-Service -Name 'TermService' -ErrorAction SilentlyContinue
        if ($null -ne $svc -and $svc.Status -ne 'Stopped') {
            Write-Host 'Stopping TermService before deployment...' -ForegroundColor Yellow
            Stop-Service -Name 'TermService' -Force
            $svc.WaitForStatus('Stopped', (New-TimeSpan -Seconds 30))
        }
    }

    Write-Host 'Copying scripts and artifacts to VM over PSDirect...' -ForegroundColor Cyan
    Copy-Item -ToSession $session -Path (Join-Path $providerScriptsDir '*') -Destination (Join-Path $VmDeployRoot 'scripts') -Recurse -Force
    if ($useIronRdpProvider) {
        Copy-Item -ToSession $session -Path $providerDll -Destination (Join-Path $VmDeployRoot 'bin\ironrdp_wtsprotocol_provider.dll') -Force
    }

    if ($installService) {
        Copy-Item -ToSession $session -Path $msiPath -Destination (Join-Path $VmDeployRoot 'bin\IronRdpTermSrv.msi') -Force
    }

    if (-not $useIronRdpProvider) {
        Write-Host 'Configuring listener and starting TermService in VM...' -ForegroundColor Cyan

        $deployment = Invoke-Command -Session $session -ScriptBlock {
            param($Root, $Listener, $Clsid, $Port)

            Set-StrictMode -Version Latest
            $ErrorActionPreference = 'Stop'

            $scripts = Join-Path $Root 'scripts'
            $winStationsRoot = 'HKLM:\SYSTEM\CurrentControlSet\Control\Terminal Server\WinStations'
            $listenerKey = Join-Path -Path $winStationsRoot -ChildPath $Listener

            if (-not (Test-Path -LiteralPath $listenerKey)) {
                throw "listener key not found: $listenerKey"
            }

            Set-ItemProperty -LiteralPath $listenerKey -Name 'LoadableProtocol_Object' -Type String -Value $Clsid
            Set-ItemProperty -LiteralPath $listenerKey -Name 'PortNumber' -Type DWord -Value $Port

            & (Join-Path $scripts 'configure-side-by-side-firewall.ps1') -Mode Add -PortNumber $Port

            Restart-Service -Name TermService -Force
            & (Join-Path $scripts 'wait-termservice-ready.ps1') -PortNumber $Port -TimeoutSeconds 180

            $ipv4 = Get-NetIPAddress -AddressFamily IPv4 |
                Where-Object {
                    $_.IPAddress -ne '127.0.0.1' -and
                    $_.IPAddress -notlike '169.254.*' -and
                    $_.PrefixOrigin -ne 'WellKnown'
                } |
                Sort-Object -Property InterfaceMetric |
                Select-Object -First 1 -ExpandProperty IPAddress

            [pscustomobject]@{
                VmIp = $ipv4
                Listener = $Listener
                Port = $Port
                ServiceStatus = 'TermService'
                DeployRoot = $Root
            }
        } -ArgumentList $VmDeployRoot, $ListenerName, $ProtocolManagerClsid, $PortNumber
    }
    else {
        Write-Host 'Running provider install/verify workflow in VM...' -ForegroundColor Cyan
        $deployment = Invoke-Command -Session $session -ScriptBlock {
            param($Root, $Listener, $Clsid, $Port, $InstallService)

        Set-StrictMode -Version Latest
        $ErrorActionPreference = 'Stop'

        function Invoke-DeploymentStep {
            param(
                [Parameter(Mandatory = $true)]
                [string]$Name,

                [Parameter(Mandatory = $true)]
                [scriptblock]$Action
            )

            Write-Host "==> $Name"

            try {
                & $Action
            }
            catch {
                throw "step '$Name' failed: $($_.Exception.Message)"
            }
        }

        $scripts = Join-Path $Root 'scripts'
        $dll = Join-Path $Root 'bin\ironrdp_wtsprotocol_provider.dll'
        $msi = Join-Path $Root 'bin\IronRdpTermSrv.msi'

        Invoke-DeploymentStep -Name 'Backup side-by-side state' -Action {
            & (Join-Path $scripts 'backup-side-by-side-state.ps1') -ListenerName $Listener -ProtocolManagerClsid $Clsid -OutputDirectory (Join-Path $Root 'logs') | Out-Null
        }

        Invoke-DeploymentStep -Name 'Run preflight checks' -Action {
            & (Join-Path $scripts 'preflight-side-by-side.ps1') -ProviderDllPath $dll -ListenerName $Listener -ProtocolManagerClsid $Clsid -PortNumber $Port
        }

        Invoke-DeploymentStep -Name 'Install side-by-side provider' -Action {
            & (Join-Path $scripts 'install-side-by-side.ps1') -ProviderDllPath $dll -ListenerName $Listener -ProtocolManagerClsid $Clsid -PortNumber $Port
        }

        Invoke-DeploymentStep -Name 'Verify side-by-side install' -Action {
            & (Join-Path $scripts 'verify-side-by-side.ps1') -ProviderDllPath $dll -ListenerName $Listener -ProtocolManagerClsid $Clsid -PortNumber $Port
        }

        Invoke-DeploymentStep -Name 'Configure firewall rule' -Action {
            & (Join-Path $scripts 'configure-side-by-side-firewall.ps1') -Mode Add -PortNumber $Port
        }

        Invoke-DeploymentStep -Name 'Restart TermService' -Action {
            Restart-Service -Name TermService -Force
        }

        Invoke-DeploymentStep -Name 'Wait for TermService ready' -Action {
            & (Join-Path $scripts 'wait-termservice-ready.ps1') -PortNumber $Port -TimeoutSeconds 120
        }

        Invoke-DeploymentStep -Name 'Run smoke test' -Action {
            & (Join-Path $scripts 'smoke-test-side-by-side.ps1') -ProviderDllPath $dll -ListenerName $Listener -ProtocolManagerClsid $Clsid -PortNumber $Port -CheckLocalPortListener
        }

        $serviceStatus = 'Skipped'
        if ($InstallService) {
            Invoke-DeploymentStep -Name 'Install companion service MSI' -Action {
                Start-Process -FilePath 'msiexec.exe' -ArgumentList @('/i', $msi, '/qn', '/norestart') -Wait -NoNewWindow
            }

            Invoke-DeploymentStep -Name 'Start companion service' -Action {
                Start-Service -Name 'IronRdpTermSrv'
                $script:serviceStatus = (Get-Service -Name 'IronRdpTermSrv').Status.ToString()
            }
        }

        $ipv4 = Get-NetIPAddress -AddressFamily IPv4 |
            Where-Object {
                $_.IPAddress -ne '127.0.0.1' -and
                $_.IPAddress -notlike '169.254.*' -and
                $_.PrefixOrigin -ne 'WellKnown'
            } |
            Sort-Object -Property InterfaceMetric |
            Select-Object -First 1 -ExpandProperty IPAddress

        [pscustomobject]@{
            VmIp = $ipv4
            Listener = $Listener
            Port = $Port
            ServiceStatus = $serviceStatus
            DeployRoot = $Root
        }
        } -ArgumentList $VmDeployRoot, $ListenerName, $ProtocolManagerClsid, $PortNumber, $installService
    }

    $targetIp = if ([string]::IsNullOrWhiteSpace($deployment.VmIp)) { '<vm-ip>' } else { $deployment.VmIp }
    $rdpTarget = "$targetIp`:$PortNumber"

    Write-Host "Deployment completed." -ForegroundColor Green
    Write-Host "Listener: $($deployment.Listener)"
    Write-Host "Port: $($deployment.Port)"
    Write-Host "Deploy root in VM: $($deployment.DeployRoot)"
    Write-Host "Service status: $($deployment.ServiceStatus)"
    if (-not $installService) {
        Write-Host 'Companion service deployment: skipped (provider-only mode)' -ForegroundColor Yellow
    }
    Write-Host "Connect with: mstsc /v:$rdpTarget" -ForegroundColor Green

    if ($OpenMstsc -and $targetIp -ne '<vm-ip>') {
        Start-Process -FilePath 'mstsc.exe' -ArgumentList "/v:$rdpTarget"
    }
}
finally {
    if ($null -ne $session) {
        Remove-PSSession -Session $session
    }
}
