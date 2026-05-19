[CmdletBinding()]
param(
    [ValidateSet('Verify')]
    [string] $Mode = 'Verify',

    [string] $EndpointPath = (Join-Path $env:GITHUB_WORKSPACE 'artifacts\agentic-rdp\pshost-endpoint.json'),

    [string] $ArtifactsDir = (Join-Path $env:GITHUB_WORKSPACE 'artifacts\agentic-rdp')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not (Test-Path $EndpointPath)) {
    throw "Endpoint marker was not found: $EndpointPath"
}

New-Item -Path $ArtifactsDir -ItemType Directory -Force | Out-Null
Import-Module AwakeCoding.PSRemoting -ErrorAction Stop

$endpoint = Get-Content -Path $EndpointPath -Raw | ConvertFrom-Json
$session = New-PSHostSession -HostName $endpoint.HostName -Port ([int] $endpoint.Port)

try {
    $result = Invoke-Command -Session $session -ScriptBlock {
        $process = Get-Process -Id $PID
        $notepad = Start-Process -FilePath (Join-Path $env:WINDIR 'System32\notepad.exe') -PassThru
        Start-Sleep -Seconds 3
        $notepadProcess = Get-Process -Id $notepad.Id -ErrorAction Stop
        Stop-Process -Id $notepad.Id -Force

        [pscustomobject]@{
            ProcessId = $PID
            SessionId = $process.SessionId
            UserName = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
            UserInteractive = [Environment]::UserInteractive
            Desktop = $env:USERPROFILE
            GuiProbeProcessId = $notepadProcess.Id
            GuiProbeSessionId = $notepadProcess.SessionId
        }
    }

    if (-not $result.UserInteractive) {
        throw 'Remote command did not report an interactive user session'
    }

    $result | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $ArtifactsDir 'remote-command-verification.json') -Encoding utf8NoBOM
    $result | ConvertTo-Json -Compress
}
finally {
    Remove-PSSession -Session $session -ErrorAction SilentlyContinue
}
