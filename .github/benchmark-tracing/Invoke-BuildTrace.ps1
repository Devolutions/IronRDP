param(
    [Parameter(Mandatory)]
    [string[]] $CargoArguments,
    [Parameter(Mandatory)]
    [string] $OutputDirectory
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true
$output = (Resolve-Path $OutputDirectory).Path
$intervalPath = Join-Path $output 'build-interval.json'

if ($IsLinux) {
    $PSNativeCommandUseErrorActionPreference = $false
    & bash "$PSScriptRoot/trace-linux.sh" $output cargo @CargoArguments
    $traceExit = $LASTEXITCODE
    if (-not (Test-Path $intervalPath)) { throw 'Linux trace did not launch the build' }
    $interval = Get-Content $intervalPath -Raw | ConvertFrom-Json
    if ($traceExit -ne $interval.exitCode) { throw "Linux trace failed with exit $traceExit" }
    return
}
if (-not $IsWindows) { throw 'build tracing requires Windows or Linux' }

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'WPR tracing requires an administrator'
}
& wpr -profiles | Set-Content (Join-Path $output 'wpr-profiles.txt')
& wpr -profiledetails GeneralProfile | Set-Content (Join-Path $output 'wpr-general-profile.txt')
& wpr -profiledetails FileIO | Set-Content (Join-Path $output 'wpr-fileio-profile.txt')
Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, TotalVisibleMemorySize |
    ConvertTo-Json | Set-Content (Join-Path $output 'os.json')
Get-Volume | Select-Object DriveLetter, FileSystem, Size, SizeRemaining |
    ConvertTo-Json | Set-Content (Join-Path $output 'volumes.json')
Get-MpComputerStatus | Select-Object AntivirusEnabled, RealTimeProtectionEnabled, AMProductVersion, AntivirusSignatureVersion |
    ConvertTo-Json | Set-Content (Join-Path $output 'defender-status.json')

$modules = @(
    foreach ($tool in @('cargo', 'rustc')) {
        $path = & rustup which $tool
        Get-Item $path
        Get-ChildItem (Split-Path $path) -Filter 'rustc_driver*.dll'
    }
    foreach ($tool in @('clang-cl', 'lld-link')) {
        Get-Item (Get-Command $tool).Source
    }
) | Sort-Object FullName -Unique
$symbolDirectory = Join-Path $output 'host-symbols'
New-Item -ItemType Directory $symbolDirectory | Out-Null
$manifest = foreach ($module in $modules) {
    $pdb = [IO.Path]::ChangeExtension($module.FullName, '.pdb')
    $hasPdb = Test-Path $pdb
    if ($hasPdb) { Copy-Item $pdb $symbolDirectory }
    [pscustomobject]@{
        path = $module.FullName
        sha256 = (Get-FileHash $module.FullName -Algorithm SHA256).Hash
        pdbAvailable = $hasPdb
    }
}
$manifest | ConvertTo-Json | Set-Content (Join-Path $output 'host-modules.json')
[ordered]@{
    recorder = (Get-Command wpr).Source
    profiles = @('GeneralProfile', 'FileIO')
    scope = 'system-wide, filter using build-interval.json'
    lostEventsReport = 'trace-summary.txt'
    symbolAvailability = 'host-modules.json; installed host PDBs are copied, unavailable symbols remain unresolved'
} | ConvertTo-Json | Set-Content (Join-Path $output 'trace-capabilities.json')

$instance = 'IronRDPBuild'
& wpr -start GeneralProfile -start FileIO -filemode -instancename $instance |
    Tee-Object -FilePath (Join-Path $output 'wpr-start.txt') | Out-Host
try {
    $PSNativeCommandUseErrorActionPreference = $false
    & python "$PSScriptRoot\measure-build.py" $intervalPath cargo @CargoArguments 2>&1 |
        Tee-Object -FilePath (Join-Path $output 'build.log') | Out-Host
} finally {
    $PSNativeCommandUseErrorActionPreference = $true
    try {
        & wpr -status collectors -details -instancename $instance |
            Set-Content (Join-Path $output 'wpr-status.txt')
    } finally {
        & wpr -stop (Join-Path $output 'build.etl') 'IronRDP clean offline release build' -skipPdbGen -instancename $instance |
            Tee-Object -FilePath (Join-Path $output 'wpr-stop.txt') | Out-Host
    }
}
if (-not (Test-Path $intervalPath)) { throw 'Windows trace did not launch the build' }
& tracerpt (Join-Path $output 'build.etl') -o NUL -summary (Join-Path $output 'trace-summary.txt') `
    -report (Join-Path $output 'trace-report.xml') -y |
    Set-Content (Join-Path $output 'tracerpt.txt')
if ((Get-Item (Join-Path $output 'build.etl')).Length -eq 0) { throw 'WPR produced an empty ETL trace' }
