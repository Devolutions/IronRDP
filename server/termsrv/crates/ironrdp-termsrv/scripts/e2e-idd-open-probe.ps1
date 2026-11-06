param(
    [Parameter(Mandatory = $true)]
    [string]$LogPath,

    [Parameter()]
    [string]$RuntimeConfigPath = 'C:\ProgramData\IronRDP\Idd\runtime-config.txt',

    [Parameter()]
    [int]$RunSeconds = 90,

    [Parameter()]
    [string]$InterfaceGuid = '{1ea642e3-6a78-4f4b-9a19-2eb4f0f33b82}'
)

$ErrorActionPreference = 'Continue'

New-Item -ItemType Directory -Path (Split-Path -Parent $LogPath) -Force | Out-Null

$nativeSource = @"
using System;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class IronRdpProbeNative
{
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern SafeFileHandle CreateFileW(
        string lpFileName,
        uint dwDesiredAccess,
        uint dwShareMode,
        IntPtr lpSecurityAttributes,
        uint dwCreationDisposition,
        uint dwFlagsAndAttributes,
        IntPtr hTemplateFile
    );

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern uint QueryDosDeviceW(string lpDeviceName, StringBuilder lpTargetPath, int ucchMax);
}
"@

Add-Type -TypeDefinition $nativeSource -ErrorAction Stop

function Write-ProbeLine {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Message
    )

    Add-Content -LiteralPath $LogPath -Value ("{0} {1}" -f (Get-Date -Format o), $Message)
}

function Read-RuntimeConfigMap {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path
    )

    $map = @{}
    if (-not (Test-Path -LiteralPath $Path)) {
        return $map
    }

    foreach ($line in Get-Content -LiteralPath $Path -ErrorAction SilentlyContinue) {
        if ($line -match '^(?<key>[^=]+)=(?<value>.*)$') {
            $map[$Matches['key']] = $Matches['value']
        }
    }

    $map
}

function Query-DosTarget {
    param(
        [Parameter(Mandatory = $true)]
        [string]$DeviceName
    )

    $buffer = New-Object System.Text.StringBuilder 4096
    $result = [IronRdpProbeNative]::QueryDosDeviceW($DeviceName, $buffer, $buffer.Capacity)
    $lastError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    $target = $buffer.ToString().Trim([char]0)
    Write-ProbeLine ("query name={0} result={1} last_error=0x{2:X8} target={3}" -f $DeviceName, $result, $lastError, $target)

    if ($result -eq 0 -or [string]::IsNullOrWhiteSpace($target)) {
        return $null
    }

    return $target
}

function Test-OpenPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,

        [Parameter(Mandatory = $true)]
        [string]$AccessName,

        [Parameter(Mandatory = $true)]
        [uint32]$Access
    )

    $handle = [IronRdpProbeNative]::CreateFileW($Path, $Access, 3, [IntPtr]::Zero, 3, 0, [IntPtr]::Zero)
    $lastError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    $ok = ($null -ne $handle) -and (-not $handle.IsInvalid)
    Write-ProbeLine ("open path={0} access={1} ok={2} last_error=0x{3:X8}" -f $Path, $AccessName, $ok, $lastError)

    if ($ok) {
        $handle.Dispose()
    }
}

$identityName = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
Write-ProbeLine ("start identity={0} run_seconds={1} interface_guid={2}" -f $identityName, $RunSeconds, $InterfaceGuid)

$deadline = (Get-Date).AddSeconds($RunSeconds)
$lastRuntimeMarker = ''
$interfaceGuidNormalized = $InterfaceGuid.ToLowerInvariant()

while ((Get-Date) -lt $deadline) {
    $runtimeConfig = Read-RuntimeConfigMap -Path $RuntimeConfigPath
    $sessionId = 0
    if ($runtimeConfig.ContainsKey('session_id')) {
        [void][int]::TryParse([string]$runtimeConfig['session_id'], [ref]$sessionId)
    }

    $runtimeMarker = "runtime session_id=$sessionId driver_loaded=$($runtimeConfig['driver_loaded']) updated_source=$($runtimeConfig['updated_source']) active_video_source=$($runtimeConfig['active_video_source'])"
    if ($runtimeMarker -ne $lastRuntimeMarker) {
        Write-ProbeLine $runtimeMarker
        $lastRuntimeMarker = $runtimeMarker
    }

    $candidatePaths = [System.Collections.Generic.List[string]]::new()
    $candidatePaths.Add('\\.\Global\IronRdpIddVideo')
    $candidatePaths.Add('\\.\IronRdpIddVideo')

    foreach ($deviceName in @('IronRdpIddVideo', 'Global\IronRdpIddVideo')) {
        $target = Query-DosTarget -DeviceName $deviceName
        if (-not [string]::IsNullOrWhiteSpace($target)) {
            $candidatePaths.Add("\\?\GLOBALROOT$target")
        }
    }

    if ($sessionId -gt 0) {
        $candidatePaths.Add(('\\?\swd#remotedisplayenum#ironrdpidd&sessionid_{0:d4}#{1}' -f $sessionId, $interfaceGuidNormalized))
    }

    foreach ($path in ($candidatePaths | Select-Object -Unique)) {
        foreach ($access in @(
            @{ Name = 'rw'; Value = [uint32]3221225472 },
            @{ Name = 'r'; Value = [uint32]2147483648 },
            @{ Name = 'none'; Value = [uint32]0x00000000 }
        )) {
            Test-OpenPath -Path $path -AccessName $access.Name -Access $access.Value
        }
    }

    Start-Sleep -Milliseconds 500
}

Write-ProbeLine 'complete'