[CmdletBinding()]
param(
    [string]$Hostname = 'IT-HELP-TERMSRV',
    [ValidateRange(1, 65535)]
    [int]$Port = 3389,
    [string]$Endpoint = '',
    [string]$AgentPath = 'ironrdp-agent'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

foreach ($name in 'RDP_USERNAME', 'RDP_PASSWORD') {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        throw "Missing credential environment variable: $name"
    }
}

$arguments = @('connect', '--server', "${Hostname}:$Port", '--no-prompt')
if (-not [string]::IsNullOrWhiteSpace($Endpoint)) {
    $arguments += @('--endpoint', $Endpoint, '--auto-start')
}
& $AgentPath @arguments
if ($LASTEXITCODE -ne 0) {
    throw "ironrdp-agent connect failed (exit $LASTEXITCODE)"
}
