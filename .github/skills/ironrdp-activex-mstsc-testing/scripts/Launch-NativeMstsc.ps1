[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$TracePath,

    [string]$ActiveXDllPath = (Join-Path (Get-Location) 'target\release\ironrdpax.dll'),

    [string]$MsRdpExDirectory = (Join-Path $env:ProgramFiles 'Devolutions\MsRdpEx'),

    [string]$Destination,

    [switch]$AutoLogon,

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

if ($AutoLogon) {
    if (!$Destination) {
        throw 'AutoLogon requires Destination'
    }
    if ([string]::IsNullOrEmpty($env:RDP_USERNAME) -or [string]::IsNullOrEmpty($env:RDP_PASSWORD)) {
        throw 'AutoLogon requires nonempty RDP_USERNAME and RDP_PASSWORD environment variables'
    }

    $env:RDP_AUTOLOGON = '1'
}

$rdpFile = $null
$launcherArguments = @()
if ($Destination) {
    if ($Destination.IndexOfAny([char[]]"`r`n") -ge 0) {
        throw 'Destination must not contain a newline'
    }

    $rdpFile = Join-Path $traceDirectory 'ironrdp-native-mstsc.rdp'
    $rdp = "full address:s:$Destination`r`nprompt for credentials:i:1`r`n"
    [System.IO.File]::WriteAllText($rdpFile, $rdp, [System.Text.Encoding]::ASCII)
    # The native form is absent for .rdp launches, so the bridge resolves the destination from this
    # process-local fallback. The temporary file itself contains no credentials.
    $env:RDP_HOSTNAME = $Destination
    $launcherArguments = @($rdpFile)
}

$launcherProcess = Start-Process -FilePath $launcher -WorkingDirectory $MsRdpExDirectory -ArgumentList $launcherArguments -PassThru
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
    RdpFile = $rdpFile
}
