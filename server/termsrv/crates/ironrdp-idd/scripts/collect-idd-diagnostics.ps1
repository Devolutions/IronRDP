[CmdletBinding()]
param(
    [Parameter()]
    [Alias('ComputerName')]
    [string]$VmHostname = $env:IRONRDP_TEST_VM_HOSTNAME,

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
    [string]$RemoteDiagRoot = "C:\Temp\IddDiagnostics",

    [Parameter()]
    [string]$LocalOutZip = "",

    [Parameter()]
    [switch]$PassThru
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Import-Module (Join-Path $PSScriptRoot '..\..\..\scripts\TestVm.psm1') -Force
$credential = Resolve-TestVmCredential -Credential $Credential -Username $Username -Password $Password -PromptPassword:$PromptPassword -PasswordEnvVar $PasswordEnvVar -Hostname $VmHostname

if ([string]::IsNullOrWhiteSpace($LocalOutZip)) {
    $LocalOutZip = Join-Path (Get-Location) "IddDiagnostics-$([guid]::NewGuid().ToString('N')).zip"
}

$session = New-TestVmSession -Hostname $VmHostname -Credential $credential
try {
    $runId = [guid]::NewGuid().ToString('N')
    $remoteDiagPath = Join-Path $RemoteDiagRoot $runId
    $remoteZipPath = Join-Path $RemoteDiagRoot "$runId.zip"
    $collection = Invoke-Command -Session $session -ScriptBlock {
        param($diagPath, $zipPath)

        $ErrorActionPreference = 'Stop'
        $warnings = @()

        New-Item -ItemType Directory -Path $diagPath -Force | Out-Null

        try {
            $deviceOutput = & pnputil.exe /enum-devices /class Display 2>&1
            $deviceExitCode = $LASTEXITCODE
            $deviceOutput | Out-File (Join-Path $diagPath 'devices.txt')
            if ($deviceExitCode -ne 0) {
                $warnings += "pnputil /enum-devices /class Display failed (exit $deviceExitCode)"
            }
        } catch {
            $warning = "Failed to enumerate display devices: $($_.Exception.Message)"
            $warnings += $warning
            $warning | Out-File (Join-Path $diagPath 'devices.txt')
        }

        try {
            Get-WinEvent -LogName System -MaxEvents 200 -ErrorAction Stop |
                Where-Object { $_.ProviderName -match 'Idd|Display|WudfRd|TermDD|dxgkrnl' } |
                Select-Object TimeCreated, Id, LevelDisplayName, ProviderName, Message |
                Out-File (Join-Path $diagPath 'system-events.txt')
        } catch {
            $warning = "Failed to collect system events: $($_.Exception.Message)"
            $warnings += $warning
            $warning | Out-File (Join-Path $diagPath 'system-events.txt')
        }

        try {
            Get-ChildItem "C:\Windows\System32\DriverStore\FileRepository\*IronRdp*" -Recurse -ErrorAction Stop |
                Select-Object FullName, Length, LastWriteTime |
                Out-File (Join-Path $diagPath 'driver-files.txt')
        } catch {
            $warning = "Failed to enumerate driver store: $($_.Exception.Message)"
            $warnings += $warning
            $warning | Out-File (Join-Path $diagPath 'driver-files.txt')
        }

        Compress-Archive -Path $diagPath -DestinationPath $zipPath -Force
        [pscustomobject]@{ ArchivePath = $zipPath; Warnings = @($warnings) }
    } -ArgumentList $remoteDiagPath, $remoteZipPath
    $remoteZip = $collection.ArchivePath

    Copy-Item -FromSession $session -Path $remoteZip -Destination $LocalOutZip -Force

    Write-Host "Downloaded diagnostics: $LocalOutZip"
    if ($PassThru) {
        [pscustomobject]@{
            LocalArchivePath = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($LocalOutZip)
            RemoteArchivePath = $remoteZip
            RemoteDirectory = $remoteDiagPath
            Warnings = @($collection.Warnings)
        }
    }
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}
