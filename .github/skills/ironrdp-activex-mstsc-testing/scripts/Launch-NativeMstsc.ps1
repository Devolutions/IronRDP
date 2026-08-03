[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$TracePath,

    [string]$ActiveXDllPath = (Join-Path (Get-Location) 'target\release\ironrdpax.dll'),

    [string]$MsRdpExDirectory = (Join-Path $env:ProgramFiles 'Devolutions\MsRdpEx'),

    [ValidateRange(1, 60)]
    [int]$StartupTimeoutSeconds = 7
)

$ErrorActionPreference = 'Stop'

$launcher = Join-Path $MsRdpExDirectory 'mstscex.exe'
$shim = Join-Path $MsRdpExDirectory 'MsRdpEx.dll'
foreach ($path in @($launcher, $shim, $ActiveXDllPath)) {
    if (!(Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required file was not found: $path"
    }
}

$traceDirectory = Split-Path -Parent $TracePath
if ($traceDirectory) {
    [System.IO.Directory]::CreateDirectory($traceDirectory) | Out-Null
}

$env:MSRDPEX_MSTSCAX_DLL = (Resolve-Path -LiteralPath $ActiveXDllPath)
$env:MSRDPEX_AX_BACKEND = 'ironrdp'
$env:IRONRDP_ACTIVEX_NATIVE_MSTSC_CREDENTIAL_BRIDGE = '1'
$env:IRONRDP_ACTIVEX_HOST_TRACE = [System.IO.Path]::GetFullPath($TracePath)
$env:IRONRDP_ACTIVEX_RPC = '1'

$launcherProcess = Start-Process -FilePath $launcher -WorkingDirectory $MsRdpExDirectory -PassThru
Start-Sleep -Seconds $StartupTimeoutSeconds

$mstsc = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($launcherProcess.Id)" |
    Where-Object Name -ieq 'mstsc.exe' |
    Select-Object -First 1
if ($null -eq $mstsc) {
    throw "MsRdpEx launcher PID $($launcherProcess.Id) did not launch mstsc.exe"
}

[pscustomobject]@{
    LauncherPid = $launcherProcess.Id
    MstscPid = $mstsc.ProcessId
    Launcher = $launcher
    ActiveXDll = $env:MSRDPEX_MSTSCAX_DLL
    TracePath = $env:IRONRDP_ACTIVEX_HOST_TRACE
}
