[CmdletBinding()]
param(
    [Parameter()]
    [string]$DriverPath = "target\release\ironrdp_idd.dll",

    [Parameter()]
    [string]$InfPath = "crates\ironrdp-idd\IronRdpIdd.inf",

    [Parameter()]
    [string]$CertSubject = "CN=Test Code Signing Certificate"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-WindowsKitTool {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ExeName,

        [Parameter()]
        [string]$WindowsKitsRoot = "C:\Program Files (x86)\Windows Kits\10",

        [Parameter()]
        [string]$Arch = "x64"
    )

    $cmd = Get-Command $ExeName -ErrorAction SilentlyContinue
    if ($cmd) {
        return $cmd.Source
    }

    $binRoot = Join-Path $WindowsKitsRoot 'bin'
    if (-not (Test-Path $binRoot)) {
        return $null
    }

    $candidates = @(Get-ChildItem $binRoot -Directory -ErrorAction SilentlyContinue |
            ForEach-Object { Join-Path $_.FullName (Join-Path $Arch $ExeName) } |
            Where-Object { Test-Path $_ })

    if ($candidates.Count -eq 0) {
        # Fallback: recursive search (slower, but helpful on odd layouts)
        $candidates = @(Get-ChildItem $binRoot -Recurse -Filter $ExeName -ErrorAction SilentlyContinue |
                Select-Object -ExpandProperty FullName)
    }

    if ($candidates.Count -eq 0) {
        return $null
    }

    # Prefer the highest version folder if multiple are present.
    return @($candidates | Sort-Object)[-1]
}

$workspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path

$driverFullPath = (Resolve-Path (Join-Path $workspaceRoot $DriverPath)).Path
$infFullPath = (Resolve-Path (Join-Path $workspaceRoot $InfPath)).Path

$infDir = Split-Path -Parent $infFullPath
$stagedDriverPath = Join-Path $infDir 'IronRdpIdd.dll'

Copy-Item -Path $driverFullPath -Destination $stagedDriverPath -Force

$cert = Get-ChildItem cert:\CurrentUser\My -CodeSigning |
    Where-Object { $_.Subject -eq $CertSubject } |
    Select-Object -First 1

if (-not $cert) {
    throw "Code signing certificate not found in cert:\CurrentUser\My (Subject='$CertSubject')"
}

$signtool = Resolve-WindowsKitTool -ExeName 'signtool.exe'
if (-not $signtool) {
    throw "signtool.exe not found (install Windows SDK/WDK or run from a Developer Command Prompt)"
}

& $signtool sign /v /fd sha256 /sha1 $cert.Thumbprint $stagedDriverPath
if ($LASTEXITCODE -ne 0) {
    throw "signtool failed for driver DLL (exit $LASTEXITCODE)"
}

$inf2cat = Resolve-WindowsKitTool -ExeName 'Inf2Cat.exe'
if (-not $inf2cat) {
    throw "Inf2Cat.exe not found (install WDK)"
}

& $inf2cat /driver:$infDir /os:10_X64
if ($LASTEXITCODE -ne 0) {
    throw "Inf2Cat failed (exit $LASTEXITCODE)"
}

$catPath = Join-Path $infDir 'IronRdpIdd.cat'
if (-not (Test-Path $catPath)) {
    throw "Catalog not found after Inf2Cat: $catPath"
}

& $signtool sign /v /fd sha256 /sha1 $cert.Thumbprint $catPath
if ($LASTEXITCODE -ne 0) {
    throw "signtool failed for catalog (exit $LASTEXITCODE)"
}

Write-Host "Signed: $driverFullPath"
Write-Host "Staged: $stagedDriverPath"
Write-Host "Signed: $catPath"
