[CmdletBinding()]
param(
    [Alias('ComputerName')]
    [string]$Hostname = $env:IRONRDP_TEST_VM_HOSTNAME,
    [pscredential]$Credential,
    [string]$Username = $env:RDP_USERNAME,
    [string]$PasswordEnvVar = 'RDP_PASSWORD',
    [string]$ListenerName = 'IRDP-Tcp',
    [ValidateRange(1, 65535)][int]$Port = 4489,
    [string]$ProviderDllPath = '',
    [datetime]$Since = (Get-Date).AddHours(-1),
    [string]$RemoteRoot = '',
    [string]$OutputDirectory = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'TestVm.psm1') -Force
. (Join-Path $PSScriptRoot 'diagnostics.helpers.ps1')
$credential = Resolve-TestVmCredential -Credential $Credential -Username $Username `
    -PasswordEnvVar $PasswordEnvVar -Hostname $Hostname
$deploymentRoot = if ($env:IRONRDP_DEPLOY_ROOT) { $env:IRONRDP_DEPLOY_ROOT } else { 'C:\IronRDPDeploy' }
$RemoteRoot = if ([string]::IsNullOrWhiteSpace($RemoteRoot)) {
    Join-Path $deploymentRoot 'diagnostics'
} else {
    $RemoteRoot
}
$ProviderDllPath = if ([string]::IsNullOrWhiteSpace($ProviderDllPath)) {
    Join-Path $deploymentRoot 'provider\ironrdp_wtsprotocol_provider.dll'
} else {
    $ProviderDllPath
}
$productRoot = Split-Path -Parent $PSScriptRoot
$workspaceRoot = (Resolve-Path (Join-Path $productRoot '..\..')).Path
$runId = [guid]::NewGuid().ToString('N')
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $workspaceRoot "artifacts\termsrv-diagnostics-$runId"
}
$OutputDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory).TrimEnd('\')

$collectors = [ordered]@{
    provider = {
        param($directory)
        $session = New-TestVmSession -Hostname $Hostname -Credential $credential
        try {
            $remoteDirectory = Join-Path $RemoteRoot $runId
            $remoteScripts = Join-Path $remoteDirectory 'scripts'
            Invoke-Command -Session $session -ScriptBlock {
                param($Path)
                New-Item -ItemType Directory -Path $Path -ErrorAction Stop | Out-Null
            } -ArgumentList $remoteScripts
            $scriptsRoot = Join-Path $productRoot 'crates\ironrdp-wtsprotocol-provider\scripts'
            foreach ($name in 'collect-side-by-side-diagnostics.ps1', 'side-by-side-defaults.ps1') {
                Copy-Item -ToSession $session -LiteralPath (Join-Path $scriptsRoot $name) -Destination $remoteScripts -ErrorAction Stop
            }
            $remoteOutput = Invoke-Command -Session $session -ScriptBlock {
                param($Scripts, $Output, $Listener, $PortNumber, $DllPath)
                & (Join-Path $Scripts 'collect-side-by-side-diagnostics.ps1') -OutputDirectory $Output `
                    -ListenerName $Listener -PortNumber $PortNumber -ProviderDllPath $DllPath -PassThru
            } -ArgumentList $remoteScripts, (Join-Path $remoteDirectory 'provider'), $ListenerName, $Port, $ProviderDllPath
            Copy-Item -FromSession $session -LiteralPath $remoteOutput -Destination $directory -Recurse -ErrorAction Stop
            $summary = Get-Content (Join-Path $directory 'provider\summary.json') -Raw | ConvertFrom-Json
            [pscustomobject]@{ Warnings = @($summary.warnings) }
        }
        finally { Remove-PSSession -Session $session -ErrorAction SilentlyContinue }
    }
    idd = {
        param($directory)
        $result = & (Join-Path $productRoot 'crates\ironrdp-idd\scripts\collect-idd-diagnostics.ps1') `
            -VmHostname $Hostname -Credential $credential -RemoteDiagRoot (Join-Path $RemoteRoot 'idd') `
            -LocalOutZip (Join-Path $directory 'idd.zip') -PassThru
        [pscustomobject]@{ Warnings = @($result.Warnings) }
    }
    logon = {
        param($directory)
        $result = & (Join-Path $productRoot 'crates\ironrdp-termsrv\scripts\confirm-rdp-type10-logon.ps1') `
            -ComputerName $Hostname -Credential $credential -Since $Since -PassThru
        $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $directory 'type10.json') -Encoding UTF8
        [pscustomobject]@{ Warnings = @($result.Warnings) }
    }
}

Invoke-TermSrvDiagnosticBundle -OutputDirectory $OutputDirectory -ComputerName $Hostname -Collectors $collectors
