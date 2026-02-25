[CmdletBinding()]
param(
    [Parameter()]
    [string]$WindowsKitsRoot = "C:\Program Files (x86)\Windows Kits\10"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

try {
    if ($WindowsKitsRoot -eq "C:\Program Files (x86)\Windows Kits\10") {
        $installedRoots = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue
        if ($null -ne $installedRoots -and $null -ne $installedRoots.KitsRoot10 -and $installedRoots.KitsRoot10 -is [string] -and $installedRoots.KitsRoot10.Length -gt 0) {
            $WindowsKitsRoot = $installedRoots.KitsRoot10
        }
    }
} catch {
    # best-effort only
}

$includeRoot = Join-Path $WindowsKitsRoot 'Include'
$libRoot = Join-Path $WindowsKitsRoot 'Lib'
$binRoot = Join-Path $WindowsKitsRoot 'bin'

$results = [ordered]@{
    WindowsKitsRoot = $WindowsKitsRoot
    Exists = [ordered]@{
        Include = (Test-Path $includeRoot)
        Lib = (Test-Path $libRoot)
        Bin = (Test-Path $binRoot)
    }
    IddCxHeader = @()
    IddCxLib = @()
    IddCxStubLib = @()
    Signtool = @()
    Inf2Cat = @()
    Notes = @()
}

if (Test-Path $includeRoot) {
    $results.IddCxHeader = @(Get-ChildItem $includeRoot -Recurse -Filter 'iddcx.h' -ErrorAction SilentlyContinue |
            Select-Object -ExpandProperty FullName)
} else {
    $results.Notes += "Windows Kits Include folder not found: $includeRoot"
}

if (Test-Path $libRoot) {
    $results.IddCxLib = @(Get-ChildItem $libRoot -Recurse -Filter 'iddcx.lib' -ErrorAction SilentlyContinue |
            Select-Object -ExpandProperty FullName)

    $results.IddCxStubLib = @(Get-ChildItem $libRoot -Recurse -Filter 'iddcxstub.lib' -ErrorAction SilentlyContinue |
            Select-Object -ExpandProperty FullName)
} else {
    $results.Notes += "Windows Kits Lib folder not found: $libRoot"
}

$maybeSigntool = @(Get-Command signtool.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source)
if ($maybeSigntool.Count -gt 0) {
    $results.Signtool = $maybeSigntool
} elseif (Test-Path $binRoot) {
    $results.Signtool = @(Get-ChildItem $binRoot -Recurse -Filter 'signtool.exe' -ErrorAction SilentlyContinue |
            Select-Object -ExpandProperty FullName)
}

$maybeInf2Cat = @(Get-Command Inf2Cat.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source)
if ($maybeInf2Cat.Count -gt 0) {
    $results.Inf2Cat = $maybeInf2Cat
} elseif (Test-Path $binRoot) {
    $results.Inf2Cat = @(Get-ChildItem $binRoot -Recurse -Filter 'Inf2Cat.exe' -ErrorAction SilentlyContinue |
            Select-Object -ExpandProperty FullName)
}

if ($results.IddCxHeader.Count -eq 0) {
    $results.Notes += 'iddcx.h not found (install WDK to enable real IddCx bindings)'
}

if ($results.IddCxLib.Count -eq 0) {
    if ($results.IddCxStubLib.Count -eq 0) {
        $results.Notes += 'iddcxstub.lib not found (WDK import libs missing)'
    } else {
        $results.Notes += 'iddcx.lib not found (using iddcxstub.lib instead)'
    }
}

if ($results.Inf2Cat.Count -eq 0) {
    $results.Notes += 'Inf2Cat.exe not found (WDK tools missing; required to generate driver catalog)'
}

$results | ConvertTo-Json -Depth 6
