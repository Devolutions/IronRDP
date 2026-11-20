[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateNotNullOrEmpty()]
    [string]$BackupDirectory,

    [Parameter()]
    [switch]$RestartTermService
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Test-IsAdministrator {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

if (-not (Test-IsAdministrator)) {
    throw "this script must be run from an elevated PowerShell session"
}

if (-not (Test-Path -LiteralPath $BackupDirectory -PathType Container)) {
    throw "backup directory does not exist: $BackupDirectory"
}

$resolvedBackupDirectory = (Resolve-Path -LiteralPath $BackupDirectory).Path
$regFiles = Get-ChildItem -LiteralPath $resolvedBackupDirectory -Filter "*.reg" -File | Sort-Object -Property Name

if ($regFiles.Count -eq 0) {
    throw "no .reg files found in backup directory: $resolvedBackupDirectory"
}

foreach ($file in $regFiles) {
    & reg.exe import $file.FullName | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "failed to import registry file: $($file.FullName)"
    }

    Write-Host "Imported: $($file.FullName)"
}

Write-Host "Restore complete from: $resolvedBackupDirectory"

if ($RestartTermService.IsPresent) {
    Write-Warning "Restarting TermService now"
    Restart-Service -Name "TermService" -Force
}
